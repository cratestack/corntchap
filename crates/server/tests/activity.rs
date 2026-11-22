//! Agent activity: hooks report what a session did (`activityRecord`); the server stores `activity`
//! events for registered stations only, coalesces bursts, keeps them out of the change feed, wakes
//! nobody, and prunes them after 14 days.
mod common;
use std::time::Duration;

use corntchap::admin;
use serde_json::{Value, json};

async fn record(app: &common::TestApp, token: &str, session: &str, args: Value) -> (u16, Value) {
    let mut body =
        json!({"runtime": "claude-code", "session": session, "tool": "Edit", "files": []});
    for (k, v) in args.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = app
        .http
        .post(format!("{}/api/v2/$procs/activityRecord", app.base))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let st = r.status().as_u16();
    (st, r.json().await.unwrap_or(Value::Null))
}

async fn count(app: &common::TestApp, kind: &str) -> i64 {
    sqlx::query_scalar("select count(*) from events where kind = $1")
        .bind(kind)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn activity_is_recorded_coalesced_and_private() {
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob"] {
        app.add_user(u).await;
    }
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    for u in ["alice", "bob"] {
        app.post(&root, "/teams/acme/members", json!({"principal": u}))
            .await;
    }
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay"}),
    )
    .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    let (_, cook) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "doug", "kind": "claude-code", "runtime": "local"}),
        )
        .await;
    assert!(cook["token"].is_string(), "{cook}");
    app.post(&root, "/teams/acme/members", json!({"principal": "doug"}))
        .await;
    let (code, st) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "doug", "project": "PAY", "runtime": "claude-code", "session": "ses_a"}),
        )
        .await;
    assert_eq!(code, 200, "{st}");
    let station = st["id"].as_str().unwrap().to_string();

    // open work exists but nobody is subscribed yet; then doug gets a wake target. An `activity`
    // event must not wake anyone (it is no dispatcher trigger).
    app.post(&root, "/projects/PAY/tasks", json!({"title": "ready work"}))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (c, _) = app
        .post(&root, "/agents/doug/wake-targets", json!({"type": "relay"}))
        .await;
    assert_eq!(c, 200);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let wakes_before = count(&app, "wake").await;

    // alice (the cook's owner) records an edit
    let (c, v) = record(
        &app,
        &alice,
        "ses_a",
        json!({"tool": "Edit", "files": ["src/a.rs"], "checkpoint": "abc1234def", "summary": "src/a.rs"}),
    )
    .await;
    assert_eq!(
        (c, &v["recorded"], &v["station"]),
        (200, &json!(true), &json!(station)),
        "{v}"
    );
    assert_eq!(count(&app, "activity").await, 1);
    let p: Value = sqlx::query_scalar("select payload from events where kind = 'activity'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(p["station"], json!(station));
    assert_eq!(p["cook"], "doug");
    assert_eq!(p["tool"], "Edit");
    assert_eq!(p["files"], json!(["src/a.rs"]));
    assert_eq!(p["checkpoint"], "abc1234def");

    // a burst inside two seconds is merged into the same event
    record(
        &app,
        &alice,
        "ses_a",
        json!({"tool": "Bash", "files": ["src/b.rs", "src/a.rs"], "checkpoint": "bbb2222", "summary": "cargo test"}),
    )
    .await;
    assert_eq!(count(&app, "activity").await, 1, "coalesced");
    let p: Value = sqlx::query_scalar("select payload from events where kind = 'activity'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(p["files"], json!(["src/a.rs", "src/b.rs"]));
    assert_eq!(
        (&p["tool"], &p["checkpoint"], &p["count"]),
        (&json!("Bash"), &json!("bbb2222"), &json!(2))
    );

    // after the window, a new event
    sqlx::query("update events set at = at - interval '3 seconds' where kind = 'activity'")
        .execute(&app.state.db)
        .await
        .unwrap();
    record(
        &app,
        &alice,
        "ses_a",
        json!({"files": ["c.txt"], "summary": "c.txt"}),
    )
    .await;
    assert_eq!(count(&app, "activity").await, 2);

    // another person's session is not recorded; neither is an unknown one; nothing is stored
    let (c, v) = record(&app, &bob, "ses_a", json!({"files": ["evil"]})).await;
    assert_eq!((c, &v["recorded"]), (200, &json!(false)), "{v}");
    assert!(
        v["message"].as_str().unwrap().contains("station scan"),
        "{v}"
    );
    let (_, v) = record(&app, &alice, "ses_nope", json!({})).await;
    assert_eq!(v["recorded"], false);
    assert_eq!(count(&app, "activity").await, 2);
    // a garbage checkpoint id is refused
    let (c, _) = record(&app, &alice, "ses_a", json!({"checkpoint": "not a sha"})).await;
    assert_eq!(c, 400);

    // list_activity: newest first, filterable
    let r = app
        .mcp(&alice, "list_activity", json!({"project": "PAY"}))
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let l = common::TestApp::payload(&r);
    let l = l.as_array().unwrap();
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!(l[0]["files"], json!(["c.txt"]));
    assert_eq!(l[1]["files"], json!(["src/a.rs", "src/b.rs"]));
    assert_eq!(l[1]["checkpoint"], "bbb2222");
    assert_eq!(l[1]["count"], 2);
    let r = app
        .mcp(
            &alice,
            "list_activity",
            json!({"project": "PAY", "station": "st_other"}),
        )
        .await;
    assert_eq!(common::TestApp::payload(&r), json!([]));
    let r = app
        .mcp(
            &alice,
            "list_activity",
            json!({"project": "PAY", "limit": 1}),
        )
        .await;
    assert_eq!(common::TestApp::payload(&r).as_array().unwrap().len(), 1);

    // not part of the change feed agents poll
    let r = app
        .mcp(
            &alice,
            "changes_since",
            json!({"cursor": 0, "project": "PAY"}),
        )
        .await;
    let feed = common::TestApp::payload(&r);
    assert!(
        feed["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["kind"] != "activity"),
        "{feed}"
    );

    // nobody was woken by any of it
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        count(&app, "wake").await,
        wakes_before,
        "activity woke someone"
    );

    // retention: 14 days
    sqlx::query("update events set at = now() - interval '15 days' where kind = 'activity' and seq = (select min(seq) from events where kind = 'activity')")
        .execute(&app.state.db)
        .await
        .unwrap();
    corntchap::jobs::retain(&app.state).await.unwrap();
    assert_eq!(count(&app, "activity").await, 1);
}
