//! Cloud cooks: a Claude routine, a GitHub comment (@codex) and a signed webhook get new work and
//! every message for the cook, each prompt carrying the context a fresh session needs; GitHub
//! comments carry a pointer, never the messages.
mod common;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

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

fn header<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
    h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

async fn wait_for(
    seen: &Seen,
    path_part: &str,
    n: usize,
) -> Vec<(String, String, Vec<(String, String)>, String)> {
    for _ in 0..60 {
        let got: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.1.contains(path_part))
            .cloned()
            .collect();
        if got.len() >= n {
            return got;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("no request to {path_part}: {:?}", seen.lock().unwrap());
}

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

#[tokio::test]
async fn cloud_cooks_get_work_and_messages() {
    let (base, seen) = fake_endpoint().await;
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams/acme/members", json!({"principal": "octo"}))
        .await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let mut tokens = std::collections::HashMap::new();
    for name in ["claude-web", "codex-cloud", "hooked"] {
        let (_, v) = app
            .post(
                &root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "cloud"}),
            )
            .await;
        tokens.insert(name, v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    // a Claude routine, a GitHub comment (Codex cloud) and a signed webhook
    let (code, added) = app
        .post(
            &root,
            "/agents/claude-web/wake-targets",
            json!({"type": "claude-routine", "config": {"url": format!("{base}/v1/claude_code/routines/trig_1/fire"), "token": "sk-ant-oat01-x"}}),
        )
        .await;
    assert_eq!(code, 200, "{added}");
    assert!(
        added["routine_prompt"]
            .as_str()
            .unwrap()
            .contains("routine-fire-payload")
    );
    app.post(
        &root,
        "/agents/codex-cloud/wake-targets",
        json!({"type": "github-comment", "config": {"repo": "acme/pay", "issue": 7, "token": "ghp_x", "api_url": base}}),
    )
    .await;
    app.post(
        &root,
        "/agents/hooked/wake-targets",
        json!({"type": "webhook", "config": {"url": format!("{base}/hook"), "secret": "s3cret"}}),
    )
    .await;
    // the token and the secret are never shown back
    let (_, list) = app.get(&root, "/agents/hooked/wake-targets").await;
    assert!(!list.to_string().contains("s3cret"), "{list}");

    // new work: each cloud cook is fired, the routine with the work prompt
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Refund screen"}),
    )
    .await;
    let fires = wait_for(&seen, "/routines/trig_1/fire", 1).await;
    let (_, _, h, body) = &fires[0];
    assert_eq!(header(h, "authorization"), Some("Bearer sk-ant-oat01-x"));
    assert_eq!(header(h, "anthropic-version"), Some("2023-06-01"));
    let text = serde_json::from_str::<Value>(body).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        text.contains("new work is ready for claude-web in PAY") && text.contains("claim_next"),
        "{text}"
    );

    // claude-web takes the task; then messages arrive for it: a direct message and a comment,
    // batched into one run whose prompt carries the task and its comments
    let claim = ok(
        &app.mcp(&tokens["claude-web"], "claim_task", json!({"id": "PAY-1"}))
            .await,
        "claim",
    );
    assert!(claim["lease"]["epoch"].is_number());
    let before = seen.lock().unwrap().len();
    ok(
        &app.mcp(&root, "pass", json!({"project": "PAY", "message": "secret plan: ship on Tuesday", "to": "claude-web"}))
            .await,
        "dm",
    );
    ok(
        &app.mcp(
            &root,
            "comment",
            json!({"id": "PAY-1", "body": "Use the new refund API"}),
        )
        .await,
        "comment",
    );
    let fires = wait_for(&seen, "/routines/trig_1/fire", 2).await;
    assert_eq!(
        seen.lock().unwrap().len() - before,
        1,
        "one batched run for both messages"
    );
    let text = serde_json::from_str::<Value>(&fires[1].3).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        text.contains("2 messages for claude-web in PAY")
            && text.contains("secret plan: ship on Tuesday")
            && text.contains("Use the new refund API")
            && text.contains("PAY-1 \"Refund screen\": claimed, held by claude-web")
            && text.contains("You are a cloud session"),
        "{text}"
    );
    // the run's session is on record
    let mut url = None;
    for _ in 0..20 {
        url = sqlx::query_scalar::<_, Option<String>>(
            "select payload->>'session_url' from events where kind = 'wake.delivered' and payload->>'messages' = '2'",
        )
        .fetch_optional(&app.state.db)
        .await
        .unwrap()
        .flatten();
        if url.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(url.as_deref(), Some("https://claude.ai/code/session_01X"));

    // a GitHub comment mentions the server's default (@cook) and points at the messages, without their content
    ok(
        &app.mcp(&root, "pass", json!({"project": "PAY", "message": "the vault password is hunter2", "to": "codex-cloud"}))
            .await,
        "dm codex",
    );
    let comments = wait_for(&seen, "/repos/acme/pay/issues/7/comments", 2).await;
    let last = serde_json::from_str::<Value>(&comments.last().unwrap().3).unwrap();
    let body = last["body"].as_str().unwrap();
    assert!(
        body.starts_with("@cook ") && body.contains("read_pass (project PAY)"),
        "{body}"
    );
    assert!(
        !body.contains("hunter2"),
        "a comment must not carry the message: {body}"
    );

    // the webhook gets the structured body, signed
    ok(
        &app.mcp(
            &root,
            "pass",
            json!({"project": "PAY", "message": "@hooked ping"}),
        )
        .await,
        "mention hooked",
    );
    let hooks = wait_for(&seen, "/hook", 2).await;
    let (_, _, h, body) = hooks.last().unwrap();
    let v: Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        (&v["agent"], &v["reasons"]),
        (&json!("hooked"), &json!(["pass"]))
    );
    let sig = header(h, "x-corntchap-signature").unwrap();
    let want = format!(
        "sha256={}",
        hex::encode(corntchap::cloud::hmac_sha256(b"s3cret", body.as_bytes()))
    );
    assert_eq!(sig, want);
}
