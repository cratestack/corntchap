//! Connecting a cloud cook in steps (connect_cloud / connect_cloud_save), over the
//! handshake-era MCP that Claude Code and Codex speak: the MCP App form is announced on the
//! tools, served as a ui:// resource, and the secret never comes back in an answer.
mod common;
use std::sync::{Arc, Mutex};

use corntchap::admin;
use serde_json::{Value, json};

/// (method, path, headers lowercased, body) of each request the fake endpoint got.
type Seen = Arc<Mutex<Vec<(String, String, Vec<(String, String)>, String)>>>;

/// A tiny HTTP server answering 200 with a Claude-routine-like body.
async fn fake_endpoint() -> (String, Seen) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen: Seen = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = l.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let mut lines = head.lines();
                let first: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                    .collect();
                log.lock().unwrap().push((
                    first.first().unwrap_or(&"").to_string(),
                    first.get(1).unwrap_or(&"").to_string(),
                    headers,
                    body.to_string(),
                ));
                let out = json!({"type": "routine_fire", "claude_code_session_id": "session_01X",
                                 "claude_code_session_url": "https://claude.ai/code/session_01X"})
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{out}",
                    out.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, seen)
}

async fn rpc(app: &common::TestApp, token: &str, method: &str, params: Value) -> Value {
    let v: Value = app
        .http
        .post(format!("{}/mcp", app.base))
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v
}

async fn call(app: &common::TestApp, token: &str, tool: &str, args: Value) -> Value {
    rpc(
        app,
        token,
        "tools/call",
        json!({"name": tool, "arguments": args}),
    )
    .await["result"]
        .clone()
}

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    r["structuredContent"].clone()
}

fn err(r: &Value, has: &str) {
    assert_eq!(
        r["isError"], true,
        "expected an error containing {has:?}: {r}"
    );
    assert!(r.to_string().contains(has), "expected {has:?} in {r}");
}

#[tokio::test]
async fn a_cook_connects_itself_through_the_form_the_link_or_the_tool() {
    let (base, seen) = fake_endpoint().await;
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    for u in ["alice", "eve"] {
        app.post(&root, "/users", json!({"login": u})).await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let eve = admin::operator_token(&app.state.db, "eve", None)
        .await
        .unwrap();
    let (_, c) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "rita", "kind": "claude-code", "runtime": "cloud"}),
        )
        .await;
    let rita = c["token"].as_str().unwrap().to_string();

    // the MCP App is announced on both tools and served as a ui:// resource
    let tools = rpc(&app, &rita, "tools/list", json!({})).await;
    let tool = |n: &str| {
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == n)
            .unwrap()
            .clone()
    };
    for n in ["connect_cloud", "connect_cloud_save"] {
        assert_eq!(
            tool(n)["_meta"]["ui"]["resourceUri"],
            "ui://corntchap/connect-cloud",
            "{n}"
        );
        assert_eq!(
            tool(n)["_meta"]["ui"]["visibility"],
            json!(["model", "app"])
        );
    }
    assert!(tool("claim_next").get("_meta").is_none());
    let res = rpc(&app, &rita, "resources/list", json!({})).await;
    assert!(
        res.to_string().contains("ui://corntchap/connect-cloud"),
        "{res}"
    );
    let html = rpc(
        &app,
        &rita,
        "resources/read",
        json!({"uri": "ui://corntchap/connect-cloud"}),
    )
    .await;
    let content = &html["result"]["contents"][0];
    assert_eq!(content["mimeType"], "text/html;profile=mcp-app");
    let page = content["text"].as_str().unwrap();
    assert!(
        page.starts_with("<!doctype html>")
            && page.contains("ui/initialize")
            && page.contains("connect_cloud_save")
    );
    // and a prompt (a slash command in Claude Code)
    let prompts = rpc(&app, &rita, "prompts/list", json!({})).await;
    assert_eq!(prompts["result"]["prompts"][0]["name"], "connect-cloud");
    let p = rpc(
        &app,
        &rita,
        "prompts/get",
        json!({"name": "connect-cloud", "arguments": {"kind": "github-comment"}}),
    )
    .await;
    let text = p["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("connect_cloud with kind \"github-comment\""),
        "{text}"
    );
    let init = rpc(
        &app,
        &rita,
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}}),
    )
    .await;
    assert!(
        init["result"]["capabilities"]["prompts"].is_object(),
        "{init}"
    );

    // the cook opens a setup for itself
    let s = ok(
        &call(
            &app,
            &rita,
            "connect_cloud",
            json!({"kind": "claude-routine", "projects": "pay"}),
        )
        .await,
        "open",
    );
    let code = s["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("cs_"), "{s}");
    assert_eq!(s["cook"], "rita");
    assert_eq!(s["status"], "waiting");
    assert_eq!(s["projects"], json!(["PAY"]));
    assert!(
        s["link"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/connect/{code}"))
    );
    assert!(s["routine_prompt"].as_str().unwrap().contains("corntchap"));
    let names: Vec<&str> = s["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["url", "token"]);
    assert_eq!(s["fields"][1]["secret"], true);

    // its owner sees it (the console link), nobody else does
    ok(
        &call(&app, &alice, "connect_cloud", json!({"code": code})).await,
        "owner",
    );
    err(
        &call(&app, &eve, "connect_cloud", json!({"code": code})).await,
        "no cloud setup",
    );
    err(
        &call(
            &app,
            &eve,
            "connect_cloud_save",
            json!({"code": code, "url": "x", "token": "y"}),
        )
        .await,
        "no cloud setup",
    );
    // nothing to test before it is connected
    err(
        &call(
            &app,
            &rita,
            "connect_cloud",
            json!({"code": code, "test": true}),
        )
        .await,
        "not connected yet",
    );

    // a non-admin's trigger must be Anthropic's
    err(
        &call(&app, &rita, "connect_cloud_save", json!({"code": code, "url": format!("{base}/v1/claude_code/routines/trig_1/fire"), "token": "sk-ant-oat01-secret"})).await,
        "routine's API trigger URL",
    );
    err(&call(&app, &rita, "connect_cloud_save", json!({"code": code, "url": "https://api.anthropic.com/v1/claude_code/routines/trig_1/fire"})).await, "token is required");
    // an admin may point it elsewhere (here: the fake endpoint)
    let done = call(&app, &root, "connect_cloud_save", json!({"code": code, "url": format!("{base}/v1/claude_code/routines/trig_1/fire"), "token": "sk-ant-oat01-secret"})).await;
    let s = ok(&done, "save");
    assert_eq!(s["status"], "connected");
    assert_eq!(s["target"]["type"], "claude-routine");
    assert!(
        !done.to_string().contains("sk-ant-oat01-secret"),
        "the secret came back: {done}"
    );
    err(&call(&app, &rita, "connect_cloud_save", json!({"code": code, "url": "https://api.anthropic.com/v1/claude_code/routines/trig_2/fire", "token": "t"})).await, "already connected");
    let (_, targets) = app.get(&alice, "/agents/rita/wake-targets").await;
    assert!(
        !targets.to_string().contains("sk-ant-oat01-secret"),
        "{targets}"
    );
    assert_eq!(targets.as_array().unwrap().len(), 1, "{targets}");
    assert_eq!(targets[0]["projects"], json!(["PAY"]));

    // a test wake goes through the routine with its token
    let t = call(
        &app,
        &rita,
        "connect_cloud",
        json!({"code": code, "test": true}),
    )
    .await;
    let s = ok(&t, "test");
    assert_eq!(s["test"]["result"]["ok"], true, "{s}");
    assert_eq!(
        s["test"]["result"]["session_url"],
        "https://claude.ai/code/session_01X"
    );
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].1.ends_with("/trig_1/fire"));
    assert!(
        got[0]
            .2
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer sk-ant-oat01-secret")
    );
    assert!(
        got[0].3.contains("connection test for cook rita"),
        "{}",
        got[0].3
    );

    // the owner connects GitHub comments without being an admin; the token never comes back
    let s = ok(
        &call(
            &app,
            &alice,
            "connect_cloud",
            json!({"kind": "github-comment", "cook": "rita"}),
        )
        .await,
        "gh",
    );
    let code2 = s["code"].as_str().unwrap().to_string();
    assert_eq!(s["fields"][2]["name"], "mention");
    assert_eq!(
        s["fields"][2]["placeholder"], "@cook",
        "the server's default mention"
    );
    err(
        &call(
            &app,
            &alice,
            "connect_cloud_save",
            json!({"code": code2, "repo": "nope", "issue": 3, "token": "ghp_x"}),
        )
        .await,
        "owner/name",
    );
    let done = call(&app, &alice, "connect_cloud_save", json!({"code": code2, "repo": "acme/pay", "issue": 3, "token": "ghp_secret", "mention": "codex"})).await;
    assert_eq!(ok(&done, "gh save")["status"], "connected");
    assert!(!done.to_string().contains("ghp_secret"));
    let cfg: Value =
        sqlx::query_scalar("select config from wake_targets where type = 'github-comment'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(cfg["mention"], "@codex");
    assert_eq!(cfg["issue"], 3);

    // webhooks stay an admin's call
    let s = ok(
        &call(
            &app,
            &alice,
            "connect_cloud",
            json!({"kind": "webhook", "cook": "rita"}),
        )
        .await,
        "wh",
    );
    let code3 = s["code"].as_str().unwrap().to_string();
    err(
        &call(
            &app,
            &alice,
            "connect_cloud_save",
            json!({"code": code3, "url": "https://example.com/hook", "secret": "s"}),
        )
        .await,
        "only an admin",
    );
    // setups expire
    sqlx::query("update cloud_setups set expires_at = now() - interval '1 minute' where code = $1")
        .bind(&code3)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        ok(
            &call(&app, &alice, "connect_cloud", json!({"code": code3})).await,
            "exp"
        )["status"],
        "expired"
    );
    err(
        &call(
            &app,
            &root,
            "connect_cloud_save",
            json!({"code": code3, "url": "https://example.com/hook"}),
        )
        .await,
        "expired",
    );
    // a cook cannot open a setup for a cook it does not manage
    err(
        &call(
            &app,
            &rita,
            "connect_cloud",
            json!({"kind": "webhook", "cook": "octo"}),
        )
        .await,
        "",
    );
    err(
        &call(&app, &rita, "connect_cloud", json!({})).await,
        "kind is required",
    );
    // a removed target shows as removed
    sqlx::query("delete from wake_targets where type = 'claude-routine'")
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        ok(
            &call(&app, &rita, "connect_cloud", json!({"code": code})).await,
            "rm"
        )["status"],
        "removed"
    );
}
