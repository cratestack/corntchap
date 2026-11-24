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

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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

use uuid::Uuid;

use crate::{auth, connect, error::Error, procs, state::AppState, usage};

/// Revisions answered by this module, newest first (the first is the fallback on negotiation).
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const MAX_BODY: usize = 4 * 1024 * 1024;

const INSTRUCTIONS: &str = "corntchap coordinates coding agents over one task list per project. \
Loop: call claim_next with a project key (or claim_task for one named task); you get the task and a \
lease {epoch, expires_at}. Work the task, tick checklist items with check_item and keep your lease \
alive (every call carrying the epoch extends it, or call heartbeat). The lease lasts the pot's lease TTL: \
for long work claim with ttlS, and before a command that may outlast your lease (a long build or test run) \
heartbeat with a ttlS that covers it. Finish with complete, passing the \
epoch and a result summary, or give the task back with release / block. A lease_lost error means \
another agent holds the task now: stop and report, your work is not counted. Call claim_next again \
until it returns {none: true}. A task you release is left to the other cooks for 15 minutes (claim_task \
takes it back by name). When a claim carries a `warning` about a lapsed lease, the last holder's work may \
still be in its branch, worktree or an open pull request: look before you start over. get_task and get_task_markdown read a task, get_board reads a project. \
Each project has a pantry, the memory all agents share: read it with read_pantry when you start on a \
project and search it with recall before exploring or deciding. When you learn something durable the \
next agent should know (a convention, a gotcha, a decision and why, how to build/test/deploy), save it \
with remember, updating the existing memory (same key) rather than adding a duplicate. When claim_next \
or claim_task returns `pantry` news (memories saved or changed since you last read the pantry), read them \
before you start; notify=true on remember wakes everyone, only for what cannot wait. Task progress \
goes in comments, never secrets in the pantry. \
When a wake prompt names your station (st_...), pass it as `station` to claim_next, claim_task, ask, \
pass and comment, so leases and answers come back to this very session and people see which of your \
sessions wrote what. Several sessions can share your cook name: \
touch only the leases whose station is yours (whoami lists current_leases with their station), and treat \
max_leases as another session holding the slot, not as a lease of yours to clean up. When you need a human decision, ask it into the \
pot's question pool with ask (wait=true on the task you hold parks it for you and the answer wakes you \
with a fresh lease; do not ask and then block, a blocked task waits for a person). When you are chatting with your owner, check list_questions and offer to go \
through them: pick_question, ask them, then answer_question with their words. \
The pass is where a pot's chefs (people and cooks) talk: read_pass when you start on a pot, and use \
pass to coordinate with the others (who takes what, a heads-up, a handoff), with `to` for a direct \
message only you and they read. A direct message or an @mention wakes the cook it is for; \
broadcast=true (or @all) wakes every session of every cook, your own other sessions included when \
you pass your station: answer a broadcast only if you have something to add. \
When people ask for a view of the tasks (\"what is urgent on the MVP\"), build it with save_view: a SQL \
condition over the pot's tasks, tried first with run_view. \
A pot may have several repositories (list_projects shows them by name). When a task is about specific \
repositories, name them in `repos` on create_task / update_task; claim_next respects your repository \
scope (set_cook_repos: you may declare which repositories you handle), so you are given only tasks about \
those or about none. split_by_repo turns a task naming several repositories into one subtask each. \
Serving a task (complete) in a repository others depend on (link_repo) tells their cooks, or opens a \
follow-up task for them; pass noPropagate=true when nothing downstream is affected.";

pub struct Compat<T> {
    tools: T,
    state: AppState,
    /// Per token: whether the client said at `initialize` that it renders MCP Apps. Handshake-era
    /// requests are stateless, so this is how a later `connect_cloud` knows if its form is shown.
    renders_ui: Mutex<HashMap<Uuid, bool>>,
}

impl<T: McpTools> Compat<T> {
    pub fn new(tools: T, state: AppState) -> Arc<Self> {
        Arc::new(Self {
            tools,
            state,
            renders_ui: Mutex::new(HashMap::new()),
        })
    }

    fn remember_ui(&self, token: Option<Uuid>, renders: bool) {
        if let (Some(t), Ok(mut m)) = (token, self.renders_ui.lock()) {
            if m.len() > 10_000 {
                m.clear();
            }
            m.insert(t, renders);
        }
    }

    fn known_ui(&self, token: Option<Uuid>) -> Option<bool> {
        let t = token?;
        self.renders_ui.lock().ok()?.get(&t).copied()
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
    // `/mcp/<KEY>` (the router strips `/mcp`): one pot's MCP URL
    let sub = req.uri().path().trim_matches('/').to_string();
    if !sub.is_empty() {
        return c.pot_door(sub, req, next).await;
    }
    if is_legacy(req.headers()) {
        c.handle(req).await
    } else {
        c.modern(req, next).await
    }
}

/// The pot whose MCP URL is being served.
#[derive(Clone, Debug)]
pub struct Pot {
    pub id: Uuid,
    pub key: String,
}

tokio::task_local! {
    static CURRENT_POT: Option<Pot>;
}

fn current_pot() -> Option<Pot> {
    CURRENT_POT.try_with(|p| p.clone()).ok().flatten()
}

/// `PAY-12`, `PAY-Q3`, `PAY-F7`: the pot key of a task, question or file reference.
fn ref_key(s: &str) -> Option<&str> {
    let (key, rest) = s.split_once('-')?;
    let rest = rest.strip_prefix(['Q', 'F']).unwrap_or(rest);
    let key_ok = !key.is_empty()
        && key.len() <= 16
        && key.starts_with(|c: char| c.is_ascii_uppercase())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    (key_ok && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())).then_some(key)
}

/// The pot a tool call is about, for the usage count: its `project` argument, else the pot of the
/// MCP URL, else the key of a task, question or file reference in the arguments.
fn usage_pot(args: &Value) -> Option<String> {
    if let Some(p) = args["project"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        return Some(p.to_uppercase());
    }
    if let Some(p) = current_pot() {
        return Some(p.key);
    }
    args.as_object()?
        .values()
        .filter_map(Value::as_str)
        .flat_map(|s| s.split(',').map(str::trim))
        .find_map(|part| ref_key(part).map(String::from))
}

/// Characters of text in a tool result's `content` (what the agent reads).
/// Characters of a tool listing as a model sees it: name, description and input schema. Output
/// schemas (two thirds of the listing) stay on the wire for clients that validate results, but the
/// model APIs (Anthropic, OpenAI, the AI SDK) take only the input side, so they are not counted.
fn catalog_chars(result: &Value) -> u64 {
    let mut r = result.clone();
    for t in r["tools"].as_array_mut().into_iter().flatten() {
        if let Some(o) = t.as_object_mut() {
            o.remove("outputSchema");
            o.remove("_meta");
            o.remove("annotations");
        }
    }
    r.to_string().chars().count() as u64
}

fn content_chars(result: &Value) -> u64 {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .map(|t| t.chars().count() as u64)
        .sum()
}

/// The JSON-RPC messages of a response body: plain JSON, or the `data:` lines of an event stream.
fn body_messages(body: &[u8]) -> Vec<Value> {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        return match v {
            Value::Array(items) => items,
            v => vec![v],
        };
    }
    std::str::from_utf8(body)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str(d.trim_start()).ok())
        .collect()
}

/// On a pot's URL: `project` defaults to the pot, and naming another pot (or another pot's task,
/// question or file) is refused.
pub fn pot_args(schema: &Value, args: &mut Value, key: &str) -> Result<(), String> {
    let Some(obj) = args.as_object_mut() else {
        return Ok(());
    };
    match obj.get("project").and_then(Value::as_str) {
        Some(p) if !p.trim().eq_ignore_ascii_case(key) => {
            return Err(format!(
                "this MCP URL is for pot {key}: use the URL of pot {} (or /mcp) for another pot",
                p.trim().to_uppercase()
            ));
        }
        Some(_) => {}
        None => {
            if schema["properties"].get("project").is_some() {
                obj.insert("project".into(), json!(key));
            }
        }
    }
    for v in obj.values() {
        let Some(s) = v.as_str() else { continue };
        for part in s.split(',').map(str::trim) {
            if let Some(k) = ref_key(part)
                && k != key
            {
                return Err(format!(
                    "{part} is in pot {k}, and this MCP URL is for pot {key}"
                ));
            }
        }
    }
    Ok(())
}

/// A pot's tool list: `project` is optional and says its default.
fn pot_tool(t: &mut Value, key: &str) {
    let schema = &mut t["inputSchema"];
    if schema["properties"].get("project").is_none() {
        return;
    }
    schema["properties"]["project"]["description"] = json!(format!(
        "Pot key; on this MCP URL it is {key}, so leave it out."
    ));
    if let Some(req) = schema["required"].as_array_mut() {
        req.retain(|r| r != "project");
    }
}

/// The methods where corntchap adds to what cratestack answers on 2026-07-28: the connect form
/// (MCP App metadata, its ui:// resource, client-aware guidance) and the prompts.
const MODERN_EXTRAS: &[&str] = &[
    "tools/list",
    "tools/call",
    "resources/list",
    "resources/read",
    "prompts/list",
    "prompts/get",
];

/// Rewrite every JSON-RPC message in a body: plain JSON, or the `data:` lines of an event stream.
fn rewrite_body(body: &[u8], f: &dyn Fn(&mut Value)) -> Option<Vec<u8>> {
    if let Ok(mut v) = serde_json::from_slice::<Value>(body) {
        f(&mut v);
        return Some(v.to_string().into_bytes());
    }
    let text = std::str::from_utf8(body).ok()?;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (content, nl) = match line.strip_suffix('\n') {
            Some(c) => (c, "\n"),
            None => (line, ""),
        };
        let (content, cr) = match content.strip_suffix('\r') {
            Some(c) => (c, "\r"),
            None => (content, ""),
        };
        match content
            .strip_prefix("data:")
            .and_then(|d| serde_json::from_str::<Value>(d.trim_start()).ok())
        {
            Some(mut v) => {
                f(&mut v);
                out.push_str(&format!("data: {v}{cr}{nl}"));
            }
            None => out.push_str(line),
        }
    }
    Some(out.into_bytes())
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
    /// A tool call's params on a pot's URL: arguments defaulted to and checked against the pot.
    fn pot_call(&self, params: &Value) -> Result<Value, String> {
        let Some(p) = current_pot() else {
            return Ok(params.clone());
        };
        let mut params = params.clone();
        let tool = params["name"].as_str().unwrap_or_default().to_string();
        if params.get("arguments").is_none_or(Value::is_null) {
            params["arguments"] = json!({});
        }
        pot_args(&self.schema_of(&tool), &mut params["arguments"], &p.key)?;
        Ok(params)
    }

    /// Count a tool result sent to `who` (see [`usage`]).
    fn note_call(&self, who: Uuid, params: &Value, result: &Value) {
        let name = params["name"].as_str().unwrap_or_default();
        usage::record(
            &self.state,
            who,
            usage_pot(&params["arguments"]),
            usage::TOOL,
            name,
            content_chars(result),
        );
    }

    /// Count the `tools/call` and `tools/list` responses cratestack sent on 2026-07-28.
    async fn note_modern(&self, headers: &HeaderMap, method: &str, params: &Value, body: &[u8]) {
        for m in body_messages(body) {
            let Some(result) = m.get("result") else {
                continue;
            };
            // identified like any MCP request; the count is skipped when that fails
            let Ok(p) = auth::authenticate_headers(&self.state, headers).await else {
                return;
            };
            if method == "tools/list" {
                usage::record(
                    &self.state,
                    p.id,
                    current_pot().map(|p| p.key),
                    usage::LIST,
                    "tools/list",
                    catalog_chars(result),
                );
            } else {
                self.note_call(p.id, params, result);
            }
        }
    }

    fn schema_of(&self, tool: &str) -> Value {
        self.tools
            .tools()
            .iter()
            .find(|d| d.name == tool)
            .and_then(|d| serde_json::from_str(d.input_schema).ok())
            .unwrap_or(Value::Null)
    }

    /// `/mcp/<KEY>`: the same MCP server, for one pot. The request is authenticated here first (a
    /// token bound to this pot is accepted only inside this scope), then served as `/mcp`.
    async fn pot_door(&self, sub: String, req: Request, next: Next) -> Response {
        let key = sub.to_uppercase();
        let valid =
            !key.is_empty() && key.len() <= 16 && key.chars().all(|c| c.is_ascii_alphanumeric());
        let pot: Option<(Uuid, String)> = if valid {
            sqlx::query_as("select id, key from projects where key = $1")
                .bind(&key)
                .fetch_optional(&self.state.db)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let pot = pot.map(|(id, key)| Pot { id, key });
        let scope = pot.as_ref().map(|p| p.id);
        let (mut parts, body) = req.into_parts();
        let fut = async {
            if let Err(e) = auth::authenticate_headers(&self.state, &parts.headers).await {
                return self.auth_failure(e, &parts.headers);
            }
            if pot.is_none() {
                return json_response(
                    StatusCode::NOT_FOUND,
                    &json!({"code": "NOT_FOUND", "message": format!("no pot {key}"), "details": null}),
                );
            }
            // served as /mcp from here on
            let q = parts
                .uri
                .query()
                .map(|q| format!("/?{q}"))
                .unwrap_or_else(|| "/".into());
            parts.uri = q.parse().unwrap_or_default();
            let req = Request::from_parts(parts, body);
            if is_legacy(req.headers()) {
                self.handle(req).await
            } else {
                self.modern(req, next).await
            }
        };
        // the key is known even for an unknown pot: a 401 still points at this URL's metadata
        let ctx = Some(Pot {
            id: scope.unwrap_or(Uuid::nil()),
            key: pot
                .as_ref()
                .map(|p| p.key.clone())
                .unwrap_or_else(|| key.clone()),
        });
        auth::POT_SCOPE
            .scope(scope, CURRENT_POT.scope(ctx, fut))
            .await
    }

    /// 2026-07-28 requests: cratestack answers, corntchap adds the connect form and the prompts.
    async fn modern(&self, req: Request, next: Next) -> Response {
        let method = req
            .headers()
            .get("mcp-method")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let Some(method) = method.filter(|m| MODERN_EXTRAS.contains(&m.as_str())) else {
            return next.run(req).await;
        };
        let (parts, body) = req.into_parts();
        let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
            return json_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                &rpc_error(Value::Null, -32600, "request body too large"),
            );
        };
        let msg: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        // what cratestack does not serve at all: answered here, after the same authentication
        let own = match method.as_str() {
            "prompts/list" | "prompts/get" => true,
            "resources/read" => params["uri"].as_str() == Some(connect::UI_URI),
            _ => false,
        };
        if own {
            if let Err(e) = auth::authenticate_headers(&self.state, &parts.headers).await {
                return self.auth_failure(e, &parts.headers);
            }
            let r = match method.as_str() {
                "prompts/list" => rpc_result(id, prompts_list()),
                "prompts/get" => match prompt_get(&params) {
                    Ok(r) => rpc_result(id, r),
                    Err(m) => rpc_error(id, -32602, m),
                },
                _ => rpc_result(id, ui_resource()),
            };
            return json_response(StatusCode::OK, &r);
        }
        let pot = current_pot();
        if let Some(p) = &pot {
            if method == "resources/read" {
                return json_response(
                    StatusCode::OK,
                    &rpc_error(
                        id,
                        -32002,
                        "records are read through the tools on a pot's MCP URL (or on /mcp)",
                    ),
                );
            }
            let _ = p;
        }
        // on a pot's URL, a tool call's arguments are defaulted and checked first
        let bytes = if method == "tools/call" && pot.is_some() {
            match self.pot_call(&params) {
                Err(m) => {
                    return json_response(
                        StatusCode::OK,
                        &rpc_result(
                            id,
                            json!({"content": [{"type": "text", "text": m}], "isError": true}),
                        ),
                    );
                }
                Ok(np) => {
                    let mut m = msg.clone();
                    m["params"] = np;
                    axum::body::Bytes::from(m.to_string())
                }
            }
        } else {
            bytes
        };
        let tool = params["name"].as_str().unwrap_or_default().to_string();
        // a tool other than the connect form: its response is only counted, never rewritten
        let counted_only = method == "tools/call" && !UI_TOOLS.contains(&tool.as_str());
        let headers = parts.headers.clone();
        let renders =
            connect::renders_ui(&params["_meta"]["io.modelcontextprotocol/clientCapabilities"]);
        let resp = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        if !resp.status().is_success() {
            return resp;
        }
        let (mut rparts, rbody) = resp.into_parts();
        let Ok(rbytes) = to_bytes(rbody, MAX_BODY * 4).await else {
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        };
        if counted_only {
            self.note_modern(&headers, &method, &params, &rbytes).await;
            return Response::from_parts(rparts, Body::from(rbytes));
        }
        let patch = |v: &mut Value| {
            let Some(result) = v.get_mut("result") else {
                return;
            };
            match method.as_str() {
                "tools/list" => {
                    for t in result["tools"].as_array_mut().into_iter().flatten() {
                        if t["name"].as_str().is_some_and(|n| UI_TOOLS.contains(&n)) {
                            t["_meta"] = ui_meta();
                        }
                        if let Some(p) = &pot {
                            pot_tool(t, &p.key);
                        }
                    }
                }
                "resources/list" if pot.is_some() => {
                    result["resources"] = json!([ui_resource_entry()]);
                }
                "resources/list" => {
                    if let Some(list) = result["resources"].as_array_mut() {
                        list.push(ui_resource_entry());
                    }
                }
                "tools/call" if result["isError"] != true => connect::guide_client(result, renders),
                _ => {}
            }
        };
        let out = rewrite_body(&rbytes, &patch).unwrap_or_else(|| rbytes.to_vec());
        if matches!(method.as_str(), "tools/list" | "tools/call") {
            self.note_modern(&headers, &method, &params, &out).await;
        }
        rparts.headers.remove(header::CONTENT_LENGTH);
        Response::from_parts(rparts, Body::from(out))
    }

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
        let (ctx, token, who) = match auth::authenticate_headers(&self.state, req.headers()).await {
            Ok(p) => (
                CratestackContext::authenticated(procs::ctx_fields(&p)),
                p.token_id,
                p.id,
            ),
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
                    if let Some(r) = self.message(&ctx, token, who, m).await {
                        out.push(r);
                    }
                }
                if out.is_empty() {
                    empty(StatusCode::ACCEPTED)
                } else {
                    json_response(StatusCode::OK, &Value::Array(out))
                }
            }
            m => match self.message(&ctx, token, who, m).await {
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
                "{}/.well-known/oauth-protected-resource/mcp{}",
                self.state.config.public_url.trim_end_matches('/'),
                current_pot()
                    .map(|p| format!("/{}", p.key))
                    .unwrap_or_default()
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
    async fn message(
        &self,
        ctx: &CratestackContext,
        token: Option<Uuid>,
        who: Uuid,
        msg: Value,
    ) -> Option<Value> {
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
            "initialize" => {
                self.remember_ui(token, connect::renders_ui(&params["capabilities"]));
                let mut r = self.initialize(&params);
                if let Some(p) = current_pot() {
                    r["instructions"] = json!(format!(
                        "{} This MCP URL is for pot {k}: tools default to it (leave `project` out), and other pots are refused here.",
                        INSTRUCTIONS,
                        k = p.key
                    ));
                }
                rpc_result(id, r)
            }
            "ping" => rpc_result(id, json!({})),
            "tools/list" => {
                let mut l = self.list_tools();
                if let Some(p) = current_pot() {
                    for t in l["tools"].as_array_mut().into_iter().flatten() {
                        pot_tool(t, &p.key);
                    }
                }
                usage::record(
                    &self.state,
                    who,
                    current_pot().map(|p| p.key),
                    usage::LIST,
                    "tools/list",
                    catalog_chars(&l),
                );
                rpc_result(id, l)
            }
            "tools/call" => match self.pot_call(&params) {
                Err(m) => {
                    let r = json!({"content": [{"type": "text", "text": m}], "isError": true});
                    self.note_call(who, &params, &r);
                    rpc_result(id, r)
                }
                Ok(params) => match self.call_tool(ctx, &params).await {
                    Ok(mut r) => {
                        let name = params["name"].as_str().unwrap_or_default();
                        if UI_TOOLS.contains(&name)
                            && r["isError"] != true
                            && let Some(renders) = self.known_ui(token)
                        {
                            connect::guide_client(&mut r, renders);
                        }
                        self.note_call(who, &params, &r);
                        rpc_result(id, r)
                    }
                    Err(m) => rpc_error(id, -32602, m),
                },
            },
            // a pot's URL lists no cross-pot record collections: its tools are the way in
            "resources/list" if current_pot().is_some() => {
                rpc_result(id, json!({"resources": [ui_resource_entry()]}))
            }
            "resources/templates/list" if current_pot().is_some() => {
                rpc_result(id, json!({"resourceTemplates": []}))
            }
            "resources/read"
                if current_pot().is_some() && params["uri"].as_str() != Some(connect::UI_URI) =>
            {
                rpc_error(
                    id,
                    -32002,
                    "records are read through the tools on a pot's MCP URL (or on /mcp)",
                )
            }
            "resources/list" => rpc_result(id, self.list_resources()),
            "resources/templates/list" => rpc_result(id, self.list_templates()),
            "resources/read" => match self.read_resource(ctx, &params).await {
                Ok(r) => rpc_result(id, r),
                Err((code, m)) => rpc_error(id, code, m),
            },
            "prompts/list" => rpc_result(id, prompts_list()),
            "prompts/get" => match prompt_get(&params) {
                Ok(r) => rpc_result(id, r),
                Err(m) => rpc_error(id, -32602, m),
            },
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
            "capabilities": {"tools": {"listChanged": false}, "resources": {"listChanged": false, "subscribe": false},
                             "prompts": {"listChanged": false}},
            "serverInfo": {
                "name": "corntchap",
                "title": "corntchap",
                "version": env!("CARGO_PKG_VERSION"),
                "websiteUrl": self.state.config.public_url,
                // the console's logo (MCP 2025-11-25 `icons`), shown by clients that support it
                "icons": [{
                    "src": format!("{}/favicon.svg", self.state.config.public_url),
                    "mimeType": "image/svg+xml",
                    "sizes": ["any"],
                }],
            },
            "instructions": INSTRUCTIONS,
        })
    }

    /// Model collections, the same ones cratestack serves on 2026-07-28 (`cratestack://<name>/<segment>`).
    fn list_resources(&self) -> Value {
        let resources: Vec<Value> = self
            .tools
            .resources()
            .iter()
            .map(|r| {
                json!({
                    "uri": r.collection_uri(),
                    "name": r.segment,
                    "description": format!("Every `{}` record you may read, a page at a time (?limit, ?cursor).", r.segment),
                    "mimeType": "application/json",
                })
            })
            .collect();
        let mut resources = resources;
        resources.push(ui_resource_entry());
        json!({"resources": resources})
    }

    fn list_templates(&self) -> Value {
        let templates: Vec<Value> = self
            .tools
            .resources()
            .iter()
            .flat_map(|r| {
                let base = r.collection_uri();
                [
                    json!({"uriTemplate": format!("{base}/{{id}}"), "name": r.segment,
                        "description": format!("One `{}` record by id.", r.segment), "mimeType": "application/json"}),
                    json!({"uriTemplate": format!("{base}{{?limit,cursor}}"), "name": r.segment,
                        "description": format!("A page of `{}` records.", r.segment), "mimeType": "application/json"}),
                ]
            })
            .collect();
        json!({"resourceTemplates": templates})
    }

    /// `cratestack://<name>/<segment>[/<id>][?limit=&cursor=]`, read under the caller's policies.
    async fn read_resource(
        &self,
        ctx: &CratestackContext,
        params: &Value,
    ) -> Result<Value, (i64, String)> {
        const NOT_FOUND: i64 = -32002;
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or((-32602, "missing uri".to_string()))?;
        if uri == connect::UI_URI {
            return Ok(ui_resource());
        }
        let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
        let resource = self
            .tools
            .resources()
            .iter()
            .find(|r| {
                path == r.collection_uri() || path.starts_with(&format!("{}/", r.collection_uri()))
            })
            .ok_or((NOT_FOUND, "resource not found".to_string()))?;
        let rest = &path[resource.collection_uri().len()..];
        let body = if let Some(id) = rest.strip_prefix('/').filter(|id| !id.is_empty()) {
            if !query.is_empty() {
                return Err((-32602, "a record URI takes no query".into()));
            }
            match self.tools.read_record(resource.segment, id, ctx).await {
                Ok(Some(v)) => v,
                Ok(None) => return Err((NOT_FOUND, "resource not found".into())),
                Err(e) => return Err((-32603, e.to_string())),
            }
        } else if rest.is_empty() {
            let mut limit: Option<u64> = None;
            let mut offset: u64 = 0;
            for pair in query.split('&').filter(|p| !p.is_empty()) {
                match pair.split_once('=') {
                    Some(("limit", v)) => {
                        limit = Some(
                            v.parse()
                                .map_err(|_| (-32602, "limit must be a number".to_string()))?,
                        )
                    }
                    Some(("cursor", v)) => {
                        offset = decode_cursor(resource.segment, v).ok_or((
                            -32602,
                            "cursor was not issued for this resource".to_string(),
                        ))?
                    }
                    _ => return Err((-32602, format!("unknown query parameter {pair}"))),
                }
            }
            let size = limit
                .unwrap_or(50)
                .clamp(1, 200)
                .min(resource.max_page_size as u64) as u32;
            let items = self
                .tools
                .read_page(resource.segment, size + 1, offset, ctx)
                .await
                .map_err(|e| (-32603, e.to_string()))?;
            let more = items.len() as u32 > size;
            let items: Vec<Value> = items.into_iter().take(size as usize).collect();
            let mut page = json!({"items": items});
            if more {
                page["nextCursor"] = json!(encode_cursor(resource.segment, offset + size as u64));
            }
            page
        } else {
            return Err((NOT_FOUND, "resource not found".into()));
        };
        Ok(
            json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": body.to_string()}]}),
        )
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
    // MCP Apps: hosts that render them show the connect form next to these tools
    if UI_TOOLS.contains(&d.name) {
        t["_meta"] = ui_meta();
    }
    t
}

fn ui_meta() -> Value {
    json!({"ui": {"resourceUri": connect::UI_URI, "visibility": ["model", "app"]}})
}

fn ui_resource_entry() -> Value {
    json!({
        "uri": connect::UI_URI,
        "name": "connect_cloud",
        "description": "The form to connect a cook to a cloud wake (MCP App for connect_cloud).",
        "mimeType": UI_MIME,
    })
}

fn ui_resource() -> Value {
    json!({"contents": [{
        "uri": connect::UI_URI,
        "mimeType": UI_MIME,
        "text": connect::UI_HTML,
        "_meta": {"ui": {"prefersBorder": true}},
    }]})
}

/// The MCP Apps (`io.modelcontextprotocol/ui`) HTML profile.
pub const UI_MIME: &str = "text/html;profile=mcp-app";
const UI_TOOLS: &[&str] = &["connect_cloud", "connect_cloud_save"];

/// Slash commands in clients that list prompts (Claude Code: `/mcp__corntchap__connect-cloud`).
fn prompts_list() -> Value {
    json!({"prompts": [{
        "name": "connect-cloud",
        "title": "Connect a cloud cook",
        "description": "Connect a cook to a Claude Code routine, mentions in GitHub comments (Codex cloud, Copilot, …) or a webhook, so corntchap wakes it in the cloud.",
        "arguments": [
            {"name": "kind", "description": "claude-routine, github-comment or webhook (default claude-routine)", "required": false},
            {"name": "cook", "description": "The cook to connect (default: this one)", "required": false},
        ],
    }]})
}

fn prompt_get(params: &Value) -> Result<Value, String> {
    if params.get("name").and_then(Value::as_str) != Some("connect-cloud") {
        return Err("unknown prompt".into());
    }
    let arg = |k: &str| {
        params["arguments"][k]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let kind = arg("kind").unwrap_or_else(|| "claude-routine".into());
    let cook = arg("cook")
        .map(|c| format!(" for the cook {c}"))
        .unwrap_or_else(|| " for this cook (you)".into());
    let text = format!(
        "Connect a cloud wake ({kind}){cook} with corntchap: call connect_cloud with kind \"{kind}\". \
Then walk me through its steps. If my client shows the corntchap form, I enter the values there. \
Otherwise give me the setup's link so I can enter them on corntchap; only if I paste the values to you, \
call connect_cloud_save, and tell me first that the secret then stays in this conversation. \
Finish by calling connect_cloud with the code to check, and offer a test wake (test=true)."
    );
    Ok(json!({
        "description": "Connect a cloud cook",
        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
    }))
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

/// Opaque, resource-bound page cursor (`<segment>:<offset>`, base64url).
fn encode_cursor(segment: &str, offset: u64) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(format!("{segment}:{offset}"))
}

fn decode_cursor(segment: &str, raw: &str) -> Option<u64> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let text = String::from_utf8(URL_SAFE_NO_PAD.decode(raw).ok()?).ok()?;
    let (seg, offset) = text.rsplit_once(':')?;
    (seg == segment).then(|| offset.parse().ok()).flatten()
}
