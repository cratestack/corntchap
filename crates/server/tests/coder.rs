//! Coder as a cloud wake target: a wake starts a Coder run (a chat, or a task on older Coder), a
//! later wake for the same pot is a follow-up message to it, the session token is never shown
//! back, and a direct message from the cook's owner follows up.
//! Runs against a fake Coder (axum).
mod common;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
};
use corntchap::admin;
use serde_json::{Value, json};

const TOKEN: &str = "coder-session-secret-123";
const ORG: &str = "11111111-2222-3333-4444-555555555555";

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    token: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Req>>>,
    /// follow-ups answer 404 (the run is gone)
    gone: Arc<AtomicBool>,
    /// everything answers 401
    deny: Arc<AtomicBool>,
}

async fn handle(
    State(f): State<Fake>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let path = uri.path().to_string();
    let n = {
        let mut s = f.seen.lock().unwrap();
        s.push(Req {
            method: method.to_string(),
            path: path.clone(),
            token: headers
                .get("coder-session-token")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        });
        s.iter().filter(|r| r.method == "POST").count()
    };
    if f.deny.load(Ordering::SeqCst) {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"message": "Invalid session token"}).to_string(),
        );
    }
    let ok = |v: Value| (StatusCode::OK, v.to_string());
    if method == Method::GET && path == "/api/v2/templates" {
        return ok(json!([
            {"id": "t-0", "name": "other", "active_version_id": "99999999-0000-0000-0000-000000000000"},
            {"id": "t-1", "name": "agent-box", "active_version_id": "aaaaaaaa-0000-0000-0000-000000000001"}
        ]));
    }
    if path.ends_with("/messages") || path.ends_with("/send") {
        if f.gone.load(Ordering::SeqCst) {
            return (
                StatusCode::NOT_FOUND,
                json!({"message": "not found"}).to_string(),
            );
        }
        return ok(json!({"queued": false}));
    }
    if path == "/api/v2/chats" {
        return ok(json!({"id": format!("chat-{n}"), "status": "running"}));
    }
    if path.starts_with("/api/v2/tasks/") {
        return ok(json!({"id": format!("task-{n}"), "name": format!("corn-{n}")}));
    }
    (StatusCode::NOT_FOUND, "{}".into())
}

async fn fake_coder() -> (String, Fake) {
    let fake = Fake::default();
    let app = Router::new().fallback(handle).with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(l, app).await.unwrap();
    });
    (base, fake)
}

impl Fake {
    fn posts(&self, path_part: &str) -> Vec<Req> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" && r.path.contains(path_part))
            .cloned()
            .collect()
    }
    async fn wait(&self, path_part: &str, n: usize) -> Vec<Req> {
        for _ in 0..60 {
            let got = self.posts(path_part);
            if got.len() >= n {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("no POST {path_part} x{n}: {:?}", self.seen.lock().unwrap());
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// An app with an admin, a team, a pot PAY and a cook `coder-cook` in it; returns the app and the
/// admin's token.
async fn world(cook: &str) -> (common::TestApp, String) {
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
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
    app.post(
        &root,
        "/agents",
        json!({"name": cook, "kind": "other", "runtime": "cloud"}),
    )
    .await;
    app.post(&root, "/teams/acme/members", json!({"principal": cook}))
        .await;
    (app, root)
}

async fn dm(app: &common::TestApp, root: &str, cook: &str, text: &str) {
    ok(
        &app.mcp(
            root,
            "pass",
            json!({"project": "PAY", "message": text, "to": cook}),
        )
        .await,
        "dm",
    );
}

fn input_text(r: &Req) -> String {
    r.body["content"][0]["text"]
        .as_str()
        .or(r.body["input"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_wake_starts_a_chat_and_the_next_one_follows_up() {
    let (base, fake) = fake_coder().await;
    let (app, root) = world("coder-cook").await;
    let (code, added) = app
        .post(
            &root,
            "/agents/coder-cook/wake-targets",
            json!({"type": "coder", "config": {"url": format!("{base}/"), "token": TOKEN,
                   "organization": ORG, "workspace": "ws-1"}}),
        )
        .await;
    assert_eq!(code, 200, "{added}");
    assert!(!added.to_string().contains(TOKEN), "{added}");

    // new work: the target takes work, so it starts a chat in the pinned workspace
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Refund screen"}),
    )
    .await;
    let chats = fake.wait("/api/v2/chats", 1).await;
    let r = &chats[0];
    assert_eq!(r.path, "/api/v2/chats");
    assert_eq!(r.token.as_deref(), Some(TOKEN));
    assert_eq!(r.body["organization_id"], ORG);
    assert_eq!(r.body["workspace_id"], "ws-1");
    assert_eq!(r.body["content"][0]["type"], "text");
    let text = input_text(r);
    assert!(
        text.contains("You are a corntchap cook")
            && text.contains("new work is ready for coder-cook in PAY")
            && text.contains("claim_next"),
        "{text}"
    );

    // a direct message goes to that chat as a follow-up, not into a second chat
    dm(&app, &root, "coder-cook", "secret plan: ship on Tuesday").await;
    let msgs = fake.wait("/api/v2/chats/chat-1/messages", 1).await;
    let text = input_text(&msgs[0]);
    assert!(
        text.contains("secret plan: ship on Tuesday") && !text.contains("You are a corntchap cook"),
        "{text}"
    );
    assert_eq!(msgs[0].token.as_deref(), Some(TOKEN));
    assert_eq!(
        fake.posts("/api/v2/chats").len(),
        2,
        "one create, one follow-up"
    );

    // the run is remembered per cook and pot
    let remote: String =
        sqlx::query_scalar("select remote_id from coder_runs where project = 'PAY'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(remote, "chat-1");

    // Coder lost the chat: the next wake starts a new one and remembers it
    fake.gone.store(true, Ordering::SeqCst);
    dm(&app, &root, "coder-cook", "are you there").await;
    let chats: Vec<_> = {
        let mut got = vec![];
        for _ in 0..60 {
            got = fake
                .posts("/api/v2/chats")
                .into_iter()
                .filter(|r| r.path == "/api/v2/chats")
                .collect();
            if got.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        got
    };
    assert_eq!(chats.len(), 2, "a new chat after the 404");
    assert!(input_text(&chats[1]).contains("are you there"));
    let mut remote = String::new();
    for _ in 0..40 {
        remote = sqlx::query_scalar("select remote_id from coder_runs where project = 'PAY'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
        if remote == "chat-3" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_ne!(remote, "chat-1", "the new chat replaces the lost one");

    // the token is in no output
    let (_, list) = app.get(&root, "/agents/coder-cook/wake-targets").await;
    let mcp = ok(
        &app.mcp(&root, "list_wake_targets", json!({"cook": "coder-cook"}))
            .await,
        "list",
    )["items"]
        .clone();
    assert_eq!(mcp[0]["type"], "coder");
    let (_, cook_page) = app.get(&root, "/agents/coder-cook").await;
    let (_, cooks) = app.get(&root, "/agents").await;
    let events: Vec<Value> = sqlx::query_scalar("select payload from events")
        .fetch_all(&app.state.db)
        .await
        .unwrap();
    for (what, out) in [
        ("wake targets", list.to_string()),
        ("mcp wake targets", mcp.to_string()),
        ("cook page", cook_page.to_string()),
        ("cooks", cooks.to_string()),
        ("events", json!(events).to_string()),
    ] {
        assert!(!out.contains(TOKEN), "{what} shows the token: {out}");
    }
}

#[tokio::test]
async fn a_task_on_older_coder_resolves_the_template_and_follows_up() {
    let (base, fake) = fake_coder().await;
    let (app, root) = world("tasker").await;
    let added = ok(
        &app.mcp(
            &root,
            "add_wake_target",
            json!({"cook": "tasker", "type": "coder", "url": base, "token": TOKEN,
                   "template": "agent-box", "preset": "bbbbbbbb-0000-0000-0000-000000000002",
                   "owner": "svc-agent"}),
        )
        .await,
        "add",
    );
    assert!(!added.to_string().contains(TOKEN), "{added}");
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Refund screen"}),
    )
    .await;
    let created = fake.wait("/api/v2/tasks/svc-agent", 1).await;
    let r = &created[0];
    assert_eq!(r.path, "/api/v2/tasks/svc-agent");
    assert_eq!(r.token.as_deref(), Some(TOKEN));
    assert_eq!(
        r.body["template_version_id"],
        "aaaaaaaa-0000-0000-0000-000000000001"
    );
    assert_eq!(
        r.body["template_version_preset_id"],
        "bbbbbbbb-0000-0000-0000-000000000002"
    );
    assert!(input_text(r).contains("new work is ready for tasker in PAY"));

    dm(&app, &root, "tasker", "also update the docs").await;
    let sent = fake.wait("/api/v2/tasks/svc-agent/task-", 1).await;
    assert!(sent[0].path.ends_with("/task-1/send"), "{}", sent[0].path);
    assert!(input_text(&sent[0]).contains("also update the docs"));
}

#[tokio::test]
async fn a_broadcast_starts_nothing_and_failures_are_recorded() {
    let (base, fake) = fake_coder().await;
    let (app, root) = world("onduty").await;
    // no ready work: a broadcast starts nothing, a direct message from the owner does
    app.post(
        &root,
        "/agents/onduty/wake-targets",
        json!({"type": "coder", "config": {"url": base, "token": TOKEN, "organization": ORG}}),
    )
    .await;
    ok(
        &app.mcp(
            &root,
            "pass",
            json!({"project": "PAY", "message": "standup", "broadcast": true}),
        )
        .await,
        "broadcast",
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(fake.count(), 0, "a broadcast does not");
    dm(&app, &root, "onduty", "can you look at PAY-1?").await;
    let chats = fake.wait("/api/v2/chats", 1).await;
    assert!(input_text(&chats[0]).contains("can you look at PAY-1?"));

    // a Coder error is a wake.failed event with Coder's message, and no token
    fake.deny.store(true, Ordering::SeqCst);
    fake.gone.store(false, Ordering::SeqCst);
    dm(&app, &root, "onduty", "second message").await;
    let mut err = None;
    for _ in 0..60 {
        err = sqlx::query_scalar::<_, Option<String>>(
            "select payload->>'error' from events where kind = 'wake.failed' and payload->>'type' = 'coder'",
        )
        .fetch_optional(&app.state.db)
        .await
        .unwrap()
        .flatten();
        if err.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let err = err.expect("a wake.failed event");
    assert!(
        err.contains("401") && err.contains("Invalid session token") && !err.contains(TOKEN),
        "{err}"
    );
}

#[tokio::test]
async fn the_config_is_checked() {
    let (app, root) = world("checked").await;
    let add = |cfg: Value| {
        let (app, root) = (&app, root.clone());
        async move {
            app.post(
                &root,
                "/agents/checked/wake-targets",
                json!({"type": "coder", "config": cfg}),
            )
            .await
        }
    };
    let (c, v) = add(json!({"url": "https://coder.example.com", "token": "t"})).await;
    assert_eq!(c, 400, "organization or template is required: {v}");
    let (c, v) = add(
        json!({"url": "https://coder.example.com", "token": "t", "organization": ORG, "template": "x"}),
    )
    .await;
    assert_eq!(c, 400, "not both apis: {v}");
    let (c, v) = add(json!({"url": "ftp://x", "token": "t", "organization": ORG})).await;
    assert_eq!(c, 400, "{v}");
    let (c, v) = add(json!({"url": "https://coder.example.com", "organization": ORG})).await;
    assert_eq!(c, 400, "the token is required: {v}");
    let (c, v) =
        add(json!({"url": "https://coder.example.com", "token": "t", "organization": ORG})).await;
    assert_eq!(c, 200, "{v}");
}
