//! Broadcast on the pass: one message wakes every session of every cook in the pot (the
//! sender's own other sessions included when it names its station), not gated by lease slots.
mod common;
use std::collections::BTreeSet;

use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// (agent, station ref or "-") of the wakes after `from`.
async fn wakes_after(app: &common::TestApp, from: i64) -> BTreeSet<(String, String)> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "select payload from events where kind = 'wake' and seq > $1 order by seq",
    )
    .bind(from)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    rows.iter()
        .map(|p| {
            (
                p["agent"].as_str().unwrap().to_string(),
                p["station"]["id"].as_str().unwrap_or("-").to_string(),
            )
        })
        .collect()
}

async fn cursor(app: &common::TestApp) -> i64 {
    sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn broadcast_reaches_every_session() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/users", json!({"login": "alice"})).await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams/acme/members", json!({"principal": "alice"}))
        .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    // doug has three sessions (max_leases 1: a broadcast is not work and is not gated by it),
    // c2 one, c3 none (a relay target only)
    let mut tokens = std::collections::HashMap::new();
    for name in ["doug", "c2", "c3"] {
        let (_, v) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local"}),
            )
            .await;
        tokens.insert(name, v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let mut st = std::collections::HashMap::new();
    for (cook, sess) in [("doug", "a"), ("doug", "b"), ("doug", "c"), ("c2", "x")] {
        let (_, v) = app
            .post(
                &alice,
                "/stations",
                json!({"cook": cook, "project": "PAY", "runtime": "opencode", "session": format!("ses_{sess}")}),
            )
            .await;
        st.insert(sess, v["id"].as_str().unwrap().to_string());
    }
    let doug = tokens["doug"].clone();
    let pair = |a: &str, s: &str| (a.to_string(), s.to_string());

    // doug's session a broadcasts, naming its station: its two sibling sessions, c2's session
    // and c3 (no station) are woken; session a itself is not
    let c = cursor(&app).await;
    let m = ok(
        &app.mcp(
            &doug,
            "pass",
            json!({"project": "PAY", "message": "heads-up: I am rebasing main", "broadcast": true, "station": st["a"]}),
        )
        .await,
        "broadcast",
    );
    let expected: BTreeSet<_> = [
        pair("doug", &st["b"]),
        pair("doug", &st["c"]),
        pair("c2", &st["x"]),
        pair("c3", "-"),
    ]
    .into();
    assert_eq!(wakes_after(&app, c).await, expected);
    assert_eq!(m["woken"].as_array().unwrap().len(), 4, "{m}");
    let text: String = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' order by seq desc limit 1",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert!(
        text.contains("wrote to everyone") && text.contains("only if you have something to add"),
        "{text}"
    );

    // without its station, doug's own sessions are all skipped
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &doug,
            "pass",
            json!({"project": "PAY", "message": "@all lunch", "task": null}),
        )
        .await,
        "@all",
    );
    let expected: BTreeSet<_> = [pair("c2", &st["x"]), pair("c3", "-")].into();
    assert_eq!(wakes_after(&app, c).await, expected, "@all is a broadcast");

    // a person broadcasting reaches every session
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "standup in 5", "broadcast": true}),
        )
        .await,
        "alice",
    );
    assert_eq!(wakes_after(&app, c).await.len(), 5);

    // a broadcast is not a direct message; a station must be the sender's own
    let r = app
        .mcp(
            &doug,
            "pass",
            json!({"project": "PAY", "message": "x", "broadcast": true, "to": "c2"}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let r = app
        .mcp(
            &doug,
            "pass",
            json!({"project": "PAY", "message": "x", "broadcast": true, "station": st["x"]}),
        )
        .await;
    assert_eq!(r["isError"], true, "someone else's station: {r}");
    // plain posts still wake nobody but mentions
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &doug,
            "pass",
            json!({"project": "PAY", "message": "just saying"}),
        )
        .await,
        "plain",
    );
    assert!(wakes_after(&app, c).await.is_empty());
}

#[tokio::test]
async fn sessions_sign_what_they_write() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, v) = app
        .post(
            &root,
            "/agents",
            json!({"name": "doug", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    let doug = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "doug"}))
        .await;
    app.post(&root, "/teams/acme/members", json!({"principal": "octo"}))
        .await;
    let mut st = vec![];
    for (s, label) in [("ses_a", "quiet-planet"), ("ses_b", "hidden-nebula")] {
        let (_, v) = app
            .post(
                &root,
                "/stations",
                json!({"cook": "doug", "project": "PAY", "runtime": "opencode", "session": s, "label": label}),
            )
            .await;
        st.push(v["id"].as_str().unwrap().to_string());
    }
    app.post(&root, "/projects/PAY/tasks", json!({"title": "t"}))
        .await;

    // a broadcast from a person: every wake tells its session who it is
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &root,
            "pass",
            json!({"project": "PAY", "message": "ping", "broadcast": true}),
        )
        .await,
        "ping",
    );
    let texts: Vec<String> = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' and seq > $1 order by seq",
    )
    .bind(c)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(texts.len(), 2);
    for (t, id) in texts.iter().zip(&st) {
        assert!(t.contains(&format!("You are station {id}")), "{t}");
    }

    // each session answers with its station: the answers say which session wrote them
    for id in &st {
        ok(
            &app.mcp(
                &doug,
                "pass",
                json!({"project": "PAY", "message": "pong", "station": id}),
            )
            .await,
            "pong",
        );
    }
    let page = ok(
        &app.mcp(&root, "read_pass", json!({"project": "PAY", "chef": "pot"}))
            .await,
        "read",
    );
    let signed: Vec<(String, String)> = page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["body"] == "pong")
        .map(|m| {
            (
                m["station"].as_str().unwrap().to_string(),
                m["station_label"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        signed,
        [
            (st[0].clone(), "quiet-planet".to_string()),
            (st[1].clone(), "hidden-nebula".to_string())
        ]
    );
    // comments too
    ok(
        &app.mcp(
            &doug,
            "comment",
            json!({"id": "PAY-1", "body": "on it", "station": st[1]}),
        )
        .await,
        "comment",
    );
    let t = ok(
        &app.mcp(&root, "get_task", json!({"id": "PAY-1"})).await,
        "task",
    );
    assert_eq!(t["comments"][0]["station_label"], "hidden-nebula");
    // a station that is not yours is refused
    let r = app
        .mcp(
            &root,
            "comment",
            json!({"id": "PAY-1", "body": "x", "station": st[0]}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
}
