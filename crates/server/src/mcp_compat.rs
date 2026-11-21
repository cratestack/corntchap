//! MCP compatibility front door for the handshake-era revisions.
//!
//! cratestack serves MCP revision 2026-07-28 only (`MCP-Protocol-Version` + `Mcp-Method`
//! headers, no `initialize`). Most shipped clients (the TypeScript SDK up to 1.x, OpenCode,
//! Claude Code, Codex, ...) still speak 2025-11-25 and older: an `initialize` handshake, then
//! requests. This module answers those on the very same `/mcp` path, in the stateless
//! Streamable HTTP style (JSON responses, no sessions, no SSE), on top of the same generated
//! tool table so tool names, arguments, policies and error envelopes are identical.
//!
//! Routing (see [`front_door`]): a request whose `MCP-Protocol-Version` header is one of
//! [`LEGACY_VERSIONS`], or absent, is handled here; anything else goes to cratestack.

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use cratestack::{
    CratestackContext, CratestackError,
    mcp::{McpTools, ToolDescriptor},
};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use serde_json::{Value, json};

use crate::{auth, error::Error, procs, state::AppState};

/// Revisions answered by this module, newest first (the first is the fallback on negotiation).
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const MAX_BODY: usize = 4 * 1024 * 1024;

const INSTRUCTIONS: &str = "corntchap coordinates coding agents over one task list per project. \
Loop: call claim_next with a project key (or claim_task for one named task); you get the task and a \
lease {epoch, expires_at}. Work the task, tick checklist items with check_item and keep your lease \
alive (every call carrying the epoch extends it, or call heartbeat). Finish with complete, passing the \
epoch and a result summary, or give the task back with release / block. A lease_lost error means \
another agent holds the task now: stop and report, your work is not counted. Call claim_next again \
until it returns {none: true}. get_task and get_task_markdown read a task, get_board reads a project.";

pub struct Compat<T> {
    tools: T,
    state: AppState,
}

impl<T: McpTools> Compat<T> {
    pub fn new(tools: T, state: AppState) -> Arc<Self> {
        Arc::new(Self { tools, state })
    }
}

fn is_legacy(headers: &HeaderMap) -> bool {
    match headers.get("mcp-protocol-version") {
        None => true,
        Some(v) => v
            .to_str()
            .is_ok_and(|v| LEGACY_VERSIONS.contains(&v.trim())),
    }
}

/// Axum middleware: handshake-era requests are answered here, the rest fall through to cratestack.
pub async fn front_door<T: McpTools>(
    State(c): State<Arc<Compat<T>>>,
    req: Request,
    next: Next,
) -> Response {
    if is_legacy(req.headers()) {
        c.handle(req).await
    } else {
        next.run(req).await
    }
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn json_response(status: StatusCode, v: &Value) -> Response {
    let mut r = Response::new(Body::from(v.to_string()));
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    r
}

fn empty(status: StatusCode) -> Response {
    let mut r = Response::new(Body::empty());
    *r.status_mut() = status;
    r
}

impl<T: McpTools> Compat<T> {
    async fn handle(&self, req: Request) -> Response {
        if req.method() != Method::POST {
            let mut r = empty(StatusCode::METHOD_NOT_ALLOWED);
            r.headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("POST"));
            return r;
        }
        if let Some(origin) = req.headers().get(header::ORIGIN)
            && !self.origin_allowed(origin)
        {
            return empty(StatusCode::FORBIDDEN);
        }
        let ctx = match auth::authenticate_headers(&self.state, req.headers()).await {
            Ok(p) => CratestackContext::authenticated(procs::ctx_fields(&p)),
            Err(e) => return self.auth_failure(e, req.headers()),
        };
        let body = match to_bytes(req.into_body(), MAX_BODY).await {
            Ok(b) => b,
            Err(_) => {
                return json_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    &rpc_error(Value::Null, -32600, "request body too large"),
                );
            }
        };
        let msg: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                return json_response(
                    StatusCode::BAD_REQUEST,
                    &rpc_error(Value::Null, -32700, format!("parse error: {e}")),
                );
            }
        };
        match msg {
            Value::Array(items) => {
                if items.is_empty() {
                    return json_response(
                        StatusCode::BAD_REQUEST,
                        &rpc_error(Value::Null, -32600, "empty batch"),
                    );
                }
                let mut out = Vec::new();
                for m in items {
                    if let Some(r) = self.message(&ctx, m).await {
                        out.push(r);
                    }
                }
                if out.is_empty() {
                    empty(StatusCode::ACCEPTED)
                } else {
                    json_response(StatusCode::OK, &Value::Array(out))
                }
            }
            m => match self.message(&ctx, m).await {
                Some(r) => json_response(StatusCode::OK, &r),
                None => empty(StatusCode::ACCEPTED),
            },
        }
    }

    /// Browsers only: a present `Origin` must be this server's own (DNS-rebinding guard).
    fn origin_allowed(&self, origin: &HeaderValue) -> bool {
        let public = self.state.config.public_url.trim_end_matches('/');
        let own = public.split('/').take(3).collect::<Vec<_>>().join("/");
        origin
            .to_str()
            .is_ok_and(|o| o.trim_end_matches('/').eq_ignore_ascii_case(&own))
    }

    fn auth_failure(&self, e: Error, headers: &HeaderMap) -> Response {
        let (status, code) = match &e {
            Error::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            Error::Unauthorized | Error::Insecure(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        };
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            let _ = e.body(); // logs the cause
            "internal error".to_string()
        } else {
            e.to_string()
        };
        let mut r = json_response(
            status,
            &json!({"code": code, "message": message, "details": null}),
        );
        if status == StatusCode::UNAUTHORIZED {
            let meta = format!(
                "{}/.well-known/oauth-protected-resource/mcp",
                self.state.config.public_url.trim_end_matches('/')
            );
            let challenge = if headers.contains_key(header::AUTHORIZATION) {
                format!("Bearer error=\"invalid_token\", resource_metadata=\"{meta}\"")
            } else {
                format!("Bearer resource_metadata=\"{meta}\"")
            };
            if let Ok(v) = HeaderValue::from_str(&challenge) {
                r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
            }
        }
        r
    }

    /// One JSON-RPC message; `None` for notifications and stray responses (HTTP 202).
    async fn message(&self, ctx: &CratestackContext, msg: Value) -> Option<Value> {
        let Value::Object(obj) = msg else {
            return Some(rpc_error(Value::Null, -32600, "invalid request"));
        };
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A client response (result/error) or garbage: nothing to answer.
            return if obj.contains_key("result") || obj.contains_key("error") {
                None
            } else {
                Some(rpc_error(Value::Null, -32600, "invalid request: no method"))
            };
        };
        // No id: a notification (notifications/initialized, notifications/cancelled, ...).
        let id = obj.get("id").cloned()?;
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => rpc_result(id, self.initialize(&params)),
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, self.list_tools()),
            "tools/call" => match self.call_tool(ctx, &params).await {
                Ok(r) => rpc_result(id, r),
                Err(m) => rpc_error(id, -32602, m),
            },
            "resources/list" => rpc_result(id, json!({"resources": []})),
            "resources/templates/list" => rpc_result(id, json!({"resourceTemplates": []})),
            "prompts/list" => rpc_result(id, json!({"prompts": []})),
            m => rpc_error(id, -32601, format!("method not found: {m}")),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let wanted = params.get("protocolVersion").and_then(Value::as_str);
        let version = wanted
            .and_then(|w| LEGACY_VERSIONS.iter().find(|v| **v == w))
            .copied()
            .unwrap_or(LEGACY_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "corntchap", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })
    }

    fn list_tools(&self) -> Value {
        let tools: Vec<Value> = self.tools.tools().iter().map(tool_json).collect();
        json!({"tools": tools})
    }

    /// `Err` is a protocol error (-32602); tool failures are `isError` results.
    async fn call_tool(&self, ctx: &CratestackContext, params: &Value) -> Result<Value, String> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("missing tool name")?;
        let descriptor = self
            .tools
            .tools()
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| format!("unknown tool `{name}`"))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(a @ Value::Object(_)) => a.clone(),
            Some(_) => return Err("arguments must be an object".into()),
        };
        let call = match self.tools.decode(descriptor.name, arguments) {
            Ok(c) => c,
            Err(e) => {
                return Ok(failure(
                    descriptor,
                    CratestackError::Validation(e.to_string()),
                ));
            }
        };
        Ok(match self.tools.execute(call, ctx).await {
            Ok(v) => success(v),
            Err(e) => failure(descriptor, e),
        })
    }
}

fn tool_json(d: &ToolDescriptor) -> Value {
    let mut t = json!({
        "name": d.name,
        "inputSchema": serde_json::from_str::<Value>(d.input_schema)
            .unwrap_or_else(|_| json!({"type": "object"})),
        "annotations": {"readOnlyHint": d.read_only},
    });
    if let Some(desc) = d.description {
        t["description"] = json!(desc);
    }
    if let Some(Ok(schema)) = d.output_schema.map(serde_json::from_str::<Value>) {
        t["outputSchema"] = schema;
    }
    t
}

fn success(v: Value) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false});
    if v.is_object() {
        r["structuredContent"] = v;
    }
    r
}

/// The text is exactly the JSON error envelope REST and cratestack's MCP binding send.
fn failure(d: &ToolDescriptor, e: CratestackError) -> Value {
    tracing::warn!(
        target: "cratestack",
        cratestack_operation = "mcp_compat_tool_call",
        cratestack_tool = d.name,
        cratestack_error = e.code(),
        cratestack_detail = e.detail().unwrap_or(""),
        "mcp compat tool call failed",
    );
    let envelope = e.into_response();
    let text = serde_json::to_string(&envelope).unwrap_or_else(|_| {
        format!(
            r#"{{"code":"{}","message":"internal error"}}"#,
            envelope.code
        )
    });
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}
