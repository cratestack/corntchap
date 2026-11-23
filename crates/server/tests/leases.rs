//! Leases for long work: a cook asks for a longer lease when it claims, a pot's owner changes its
//! default, and an agent's tool use (hook activity) keeps the leases of its session alive.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

async fn seconds_left(app: &common::TestApp, number: i64) -> f64 {
    sqlx::query_scalar(
        "select extract(epoch from lease_expires_at - now())::float8 from tasks where number = $1",
    )
    .bind(number)
    .fetch_one(&app.state.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn leases_fit_long_work() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
    app.add_user("eve").await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/teams/acme/members",
        json!({"principal": "alice", "role": "owner"}),
    )
    .await;
    app.post(&root, "/teams/acme/members", json!({"principal": "eve"}))
        .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let eve = admin::operator_token(&app.state.db, "eve", None)
        .await
        .unwrap();
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay", "lease_ttl_s": 300}),
    )
    .await;
    let (_, c) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "claude-code", "runtime": "local", "max_leases": 3}),
        )
        .await;
    let c1 = c["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    for t in ["quick", "build", "migrate"] {
        app.post(&alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }

    // the pot's default
    ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-1"})).await,
        "default",
    );
    let s = seconds_left(&app, 1).await;
    assert!((250.0..=300.0).contains(&s), "{s}");
    // a longer lease asked for at claim time
    ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-2", "ttlS": 7200}))
            .await,
        "long",
    );
    let s = seconds_left(&app, 2).await;
    assert!((7100.0..=7200.0).contains(&s), "{s}");
    let n = ok(
        &app.mcp(&c1, "claim_next", json!({"project": "PAY", "ttlS": 999999}))
            .await,
        "capped",
    );
    assert_eq!(n["task"]["id"], "PAY-3");
    let s = seconds_left(&app, 3).await;
    assert!((86000.0..=86400.0).contains(&s), "at most a day: {s}");

    // the pot's default changes for its team's owner, not for a chef
    let e = app
        .mcp(
            &eve,
            "set_lease_ttl",
            json!({"project": "PAY", "ttlS": 1800}),
        )
        .await;
    assert_eq!(e["isError"], true, "{e}");
    let v = ok(
        &app.mcp(
            &alice,
            "set_lease_ttl",
            json!({"project": "pay", "ttlS": 1800}),
        )
        .await,
        "owner",
    );
    assert_eq!(
        (&v["key"], &v["lease_ttl_s"]),
        (&json!("PAY"), &json!(1800))
    );
    let (_, pots) = app.get(&alice, "/projects").await;
    assert_eq!(pots[0]["lease_ttl_s"], 1800.0);

    // an agent at work keeps its session's leases: hook activity renews them, never shortens them
    let (_, st) = app
        .post(&alice, "/stations", json!({"cook": "c1", "project": "PAY", "runtime": "claude-code", "session": "sess-1", "directory": "/w"}))
        .await;
    sqlx::query("update tasks set station_id = (select id from stations where session_ref = 'sess-1'), lease_expires_at = now() + interval '20 seconds' where number = 1")
        .execute(&app.state.db)
        .await
        .unwrap();
    let rec = app
        .http
        .post(format!("{}/api/v2/$procs/activityRecord", app.base))
        .bearer_auth(&c1)
        .json(&json!({"runtime": "claude-code", "session": "sess-1", "tool": "Bash", "files": [], "summary": "cargo test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rec.status().as_u16(), 200, "{}", rec.text().await.unwrap());
    let s = seconds_left(&app, 1).await;
    assert!(
        (1750.0..=1800.0).contains(&s),
        "renewed to the pot's new default: {s}"
    );
    assert!(st["id"].is_string());
    let s2 = seconds_left(&app, 2).await;
    assert!(s2 > 7000.0, "a longer lease it asked for is kept: {s2}");
}
