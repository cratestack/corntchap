//! Devin as a cloud wake target: a wake starts a Devin session, a later wake for the same pot is a
//! message to it, a gone session starts a new one, the API key is never shown back, and
//! a direct message from the cook's owner follows up.
//! Runs against a fake Devin (axum).
mod common;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
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

const TOKEN: &str = "cog_devin-service-secret-123";
const ORG: &str = "org-abc123";

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    auth: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Req>>>,
    /// 0 normal, 1 messages answer 404, 2 messages answer 410, 3 messages answer a session in
    /// `exit`, 4 everything answers 401
    mode: Arc<AtomicU8>,
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
            auth: headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        });
        s.iter().filter(|r| r.method == "POST").count()
    };
    let mode = f.mode.load(Ordering::SeqCst);
    if mode == 4 {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"title": "Unauthorized", "status": 401, "detail": "Invalid token"}).to_string(),
        );
    }
    let ok = |v: Value| (StatusCode::OK, v.to_string());
    if path.ends_with("/messages") {
        match mode {
            1 => {
                return (
                    StatusCode::NOT_FOUND,
                    json!({"title": "Not Found"}).to_string(),
                );
            }
            2 => return (StatusCode::GONE, json!({"title": "Gone"}).to_string()),
            3 => return ok(json!({"session_id": "devin-old", "status": "exit"})),
            _ => {}
        }
        return ok(json!({"session_id": "devin-x", "status": "running",
            "url": "https://app.devin.ai/sessions/same"}));
    }
    if path.ends_with("/sessions") {
        return ok(json!({"session_id": format!("devin-{n}"), "status": "new",
            "url": format!("https://app.devin.ai/sessions/{n}")}));
    }
    (StatusCode::NOT_FOUND, "{}".into())
}

async fn fake_devin() -> (String, Fake) {
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
    fn posts(&self, suffix: &str) -> Vec<Req> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST" && r.path.ends_with(suffix))
            .cloned()
            .collect()
    }
    async fn wait(&self, suffix: &str, n: usize) -> Vec<Req> {
        for _ in 0..60 {
            let got = self.posts(suffix);
            if got.len() >= n {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("no POST *{suffix} x{n}: {:?}", self.seen.lock().unwrap());
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// An app with an admin, a team, a pot PAY and a cook; returns the app and the admin's token.
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

async fn remote(app: &common::TestApp, want_not: &str) -> String {
    let mut remote = String::new();
    for _ in 0..40 {
        remote = sqlx::query_scalar("select remote_id from coder_runs where project = 'PAY'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
        if remote != want_not {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    remote
}

#[tokio::test]
async fn a_wake_starts_a_session_and_the_next_one_follows_up() {
    let (base, fake) = fake_devin().await;
    let (app, root) = world("devin-cook").await;
    let (code, added) = app
        .post(
            &root,
            "/agents/devin-cook/wake-targets",
            json!({"type": "devin", "config": {"url": format!("{base}/"), "token": TOKEN,
                   "organization": ORG}}),
        )
        .await;
    assert_eq!(code, 200, "{added}");
    assert!(!added.to_string().contains(TOKEN), "{added}");

    // new work: the target takes work, so it starts a session
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Refund screen"}),
    )
    .await;
    let created = fake.wait("/sessions", 1).await;
    let r = &created[0];
    assert_eq!(r.path, format!("/v3/organizations/{ORG}/sessions"));
    assert_eq!(r.auth.as_deref(), Some(&*format!("Bearer {TOKEN}")));
    let prompt = r.body["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("You are a corntchap cook")
            && prompt.contains("link_pr")
            && prompt.contains("new work is ready for devin-cook in PAY")
            && prompt.contains("claim_next"),
        "{prompt}"
    );

    // a direct message goes to that session as a message, not into a second session
    dm(&app, &root, "devin-cook", "secret plan: ship on Tuesday").await;
    let msgs = fake.wait("/devin-1/messages", 1).await;
    assert_eq!(
        msgs[0].path,
        format!("/v3/organizations/{ORG}/sessions/devin-1/messages")
    );
    let text = msgs[0].body["message"].as_str().unwrap();
    assert!(
        text.contains("secret plan: ship on Tuesday") && !text.contains("You are a corntchap cook"),
        "{text}"
    );
    assert_eq!(msgs[0].auth.as_deref(), Some(&*format!("Bearer {TOKEN}")));
    assert_eq!(fake.posts("/sessions").len(), 1, "one session only");
    assert_eq!(remote(&app, "").await, "devin-1");

    // the cook page's wake target shows the session's link, and no key anywhere
    let (_, list) = app.get(&root, "/agents/devin-cook/wake-targets").await;
    assert_eq!(
        list[0]["config"]["session_url"].as_str(),
        Some("https://app.devin.ai/sessions/1"),
        "{list}"
    );
    let mut delivered = vec![];
    for _ in 0..40 {
        delivered = sqlx::query_scalar::<_, Value>(
            "select payload from events where kind = 'wake.delivered' and payload->>'type' = 'devin'",
        )
        .fetch_all(&app.state.db)
        .await
        .unwrap();
        if delivered.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        delivered
            .iter()
            .any(|p| p["session_url"] == "https://app.devin.ai/sessions/1"),
        "{delivered:?}"
    );

    // the session is gone (404): a new one starts and replaces it
    fake.mode.store(1, Ordering::SeqCst);
    dm(&app, &root, "devin-cook", "are you there").await;
    let sessions = fake.wait("/sessions", 2).await;
    assert!(
        sessions[1].body["prompt"]
            .as_str()
            .unwrap()
            .contains("are you there")
    );
    assert_eq!(remote(&app, "devin-1").await, "devin-4");

    // 410 does the same
    fake.mode.store(2, Ordering::SeqCst);
    dm(&app, &root, "devin-cook", "again").await;
    fake.wait("/sessions", 3).await;
    assert_eq!(remote(&app, "devin-4").await, "devin-6");

    // an accepted message to a session that has exited woke nothing: a new session starts
    fake.mode.store(3, Ordering::SeqCst);
    dm(&app, &root, "devin-cook", "one more").await;
    fake.wait("/sessions", 4).await;

    // the key is in no output
    let (_, list) = app.get(&root, "/agents/devin-cook/wake-targets").await;
    let mcp = ok(
        &app.mcp(&root, "list_wake_targets", json!({"cook": "devin-cook"}))
            .await,
        "list",
    );
    assert_eq!(mcp[0]["type"], "devin");
    let (_, cook_page) = app.get(&root, "/agents/devin-cook").await;
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
        assert!(!out.contains(TOKEN), "{what} shows the key: {out}");
    }
}

#[tokio::test]
async fn a_broadcast_starts_nothing_and_failures_are_recorded() {
    let (base, fake) = fake_devin().await;
    let (app, root) = world("onduty").await;
    // no ready work: a broadcast starts nothing, a direct message from the owner does
    app.post(
        &root,
        "/agents/onduty/wake-targets",
        json!({"type": "devin", "config": {"url": base, "token": TOKEN, "organization": ORG}}),
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
    let created = fake.wait("/sessions", 1).await;
    assert!(
        created[0].body["prompt"]
            .as_str()
            .unwrap()
            .contains("can you look at PAY-1?")
    );

    // a Devin error is a wake.failed event with Devin's message, and no key
    fake.mode.store(4, Ordering::SeqCst);
    dm(&app, &root, "onduty", "second message").await;
    let mut err = None;
    for _ in 0..60 {
        err = sqlx::query_scalar::<_, Option<String>>(
            "select payload->>'error' from events where kind = 'wake.failed' and payload->>'type' = 'devin'",
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
        err.contains("401") && err.contains("Invalid token") && !err.contains(TOKEN),
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
                json!({"type": "devin", "config": cfg}),
            )
            .await
        }
    };
    let (c, v) = add(json!({"token": "t"})).await;
    assert_eq!(c, 400, "the organization is required: {v}");
    let (c, v) = add(json!({"organization": ORG})).await;
    assert_eq!(c, 400, "the key is required: {v}");
    let (c, v) = add(json!({"token": "t", "organization": "org/x"})).await;
    assert_eq!(c, 400, "{v}");
    let (c, v) = add(json!({"url": "ftp://x", "token": "t", "organization": ORG})).await;
    assert_eq!(c, 400, "{v}");
    // the default URL is filled in
    let (c, v) = add(json!({"token": "t", "organization": ORG})).await;
    assert_eq!(c, 200, "{v}");
    let url: String =
        sqlx::query_scalar("select config->>'url' from wake_targets where type = 'devin'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(url, "https://api.devin.ai");
}

#[tokio::test]
async fn only_an_admin_changes_the_url() {
    let app = common::start(false).await;
    app.add_user("mia").await;
    let mia = admin::operator_token(&app.state.db, "mia", None)
        .await
        .unwrap();
    app.post(
        &mia,
        "/agents",
        json!({"name": "mias", "kind": "other", "runtime": "cloud"}),
    )
    .await;
    let (c, v) = app
        .post(
            &mia,
            "/agents/mias/wake-targets",
            json!({"type": "devin", "config": {"url": "http://127.0.0.1:1", "token": "t", "organization": ORG}}),
        )
        .await;
    assert_eq!(c, 403, "{v}");
    let (c, v) = app
        .post(
            &mia,
            "/agents/mias/wake-targets",
            json!({"type": "devin", "config": {"token": "t", "organization": ORG}}),
        )
        .await;
    assert_eq!(c, 200, "{v}");
}
