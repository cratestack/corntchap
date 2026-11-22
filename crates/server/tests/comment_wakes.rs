//! A comment on a task wakes the cook responsible for it (on the session holding it) and the
//! cooks it @mentions, never the commenter.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

async fn wakes_after(app: &common::TestApp, from: i64) -> Vec<Value> {
    sqlx::query_scalar("select payload from events where kind = 'wake' and seq > $1 order by seq")
        .bind(from)
        .fetch_all(&app.state.db)
        .await
        .unwrap()
}

async fn cursor(app: &common::TestApp) -> i64 {
    sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn comments_wake_the_responsible_cook() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
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
    let mut tok = std::collections::HashMap::new();
    for name in ["c1", "c2"] {
        let (_, v) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local", "max_leases": 3}),
            )
            .await;
        tok.insert(name, v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let (c1, c2) = (tok["c1"].clone(), tok["c2"].clone());
    let (_, a) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": "ses_a"}),
        )
        .await;
    let (_, b) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": "ses_b"}),
        )
        .await;
    for t in ["refunds", "receipts", "old idea"] {
        app.post(&alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    // c1 holds PAY-1 from session b (and session a is its main station)
    app.post(
        &alice,
        &format!("/stations/{}/main", a["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    let claim = ok(
        &app.mcp(
            &c1,
            "claim_task",
            json!({"id": "PAY-1", "station": b["id"]}),
        )
        .await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();

    // a person comments: c1 is woken on the session holding the task (not its main station)
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "comment",
            json!({"id": "PAY-1", "body": "Can you read this?"}),
        )
        .await,
        "comment",
    );
    let w = wakes_after(&app, c).await;
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(
        (&w[0]["agent"], &w[0]["reason"], &w[0]["station"]["id"]),
        (&json!("c1"), &json!("comment"), &b["id"])
    );
    let text = w[0]["text"].as_str().unwrap();
    assert!(
        text.contains("alice commented on PAY-1")
            && text.contains("you hold it")
            && text.contains("Can you read this?"),
        "{text}"
    );

    // the holder commenting on its own task wakes nobody
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &c1,
            "comment",
            json!({"id": "PAY-1", "body": "working on it", "epoch": epoch}),
        )
        .await,
        "own",
    );
    assert!(wakes_after(&app, c).await.is_empty());

    // served: the cook that worked on it still hears about it
    ok(
        &app.mcp(
            &c1,
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "done"}),
        )
        .await,
        "complete",
    );
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "comment",
            json!({"id": "PAY-1", "body": "This broke the export"}),
        )
        .await,
        "after done",
    );
    let w = wakes_after(&app, c).await;
    assert_eq!(w.len(), 1);
    assert!(w[0]["text"].as_str().unwrap().contains("you worked on it"));

    // nobody has held PAY-2: only @mentions wake (c2 here), and a cook mentioning itself does not
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "comment",
            json!({"id": "PAY-2", "body": "unowned for now"}),
        )
        .await,
        "unowned",
    );
    assert!(wakes_after(&app, c).await.is_empty());
    ok(
        &app.mcp(
            &c2,
            "comment",
            json!({"id": "PAY-2", "body": "@c1 can you take this? cc @c2"}),
        )
        .await,
        "mention",
    );
    let w = wakes_after(&app, c).await;
    let agents: Vec<&str> = w.iter().map(|x| x["agent"].as_str().unwrap()).collect();
    assert_eq!(agents, ["c1"]);
    assert!(
        w[0]["text"]
            .as_str()
            .unwrap()
            .contains("c2 mentioned you in a comment on PAY-2")
    );

    // tossed tasks stay quiet
    app.post(&alice, "/tasks/PAY-3/cancel", json!({})).await;
    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "comment",
            json!({"id": "PAY-3", "body": "@c1 never mind"}),
        )
        .await,
        "tossed",
    );
    assert!(wakes_after(&app, c).await.is_empty());
}

#[tokio::test]
async fn unanswered_comments_reach_the_main_station() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
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
    let (_, v) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local", "max_leases": 3}),
        )
        .await;
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    let mut st = vec![];
    for s in ["ses_main", "ses_worker"] {
        let (_, v) = app
            .post(
                &alice,
                "/stations",
                json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": s}),
            )
            .await;
        st.push(v["id"].as_str().unwrap().to_string());
    }
    app.post(&alice, &format!("/stations/{}/main", st[0]), json!({}))
        .await;
    for t in ["one", "two"] {
        app.post(&alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    for id in ["PAY-1", "PAY-2"] {
        ok(
            &app.mcp(&c1, "claim_task", json!({"id": id, "station": st[1]}))
                .await,
            "claim",
        );
        ok(
            &app.mcp(
                &alice,
                "comment",
                json!({"id": id, "body": format!("status of {id}?")}),
            )
            .await,
            "comment",
        );
    }
    // c1 answers on PAY-2 only
    ok(
        &app.mcp(
            &c1,
            "comment",
            json!({"id": "PAY-2", "body": "almost there"}),
        )
        .await,
        "answer",
    );
    // not due yet: nothing
    assert_eq!(
        corntchap::jobs::nudge_unanswered(&app.state).await.unwrap(),
        0
    );
    sqlx::query("update comment_replies set created_at = now() - interval '11 minutes'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let c = cursor(&app).await;
    assert_eq!(
        corntchap::jobs::nudge_unanswered(&app.state).await.unwrap(),
        1
    );
    let w = wakes_after(&app, c).await;
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(
        (&w[0]["agent"], &w[0]["reason"], &w[0]["station"]["id"]),
        (&json!("c1"), &json!("comment-nudge"), &json!(st[0])),
        "the main station, not the worker that stayed silent"
    );
    let text = w[0]["text"].as_str().unwrap();
    assert!(
        text.contains("Still unanswered") && text.contains("status of PAY-1?"),
        "{text}"
    );
    // once only
    assert_eq!(
        corntchap::jobs::nudge_unanswered(&app.state).await.unwrap(),
        0
    );
    // a cook's own comment never waits for an answer
    let n: i64 = sqlx::query_scalar("select count(*) from comment_replies")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

/// Issue #34: a comment on a task parked on a question (blocked, reserved for its cook) wakes the
/// station it is parked with, once.
#[tokio::test]
async fn a_comment_on_a_parked_task_wakes_its_station() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    // A person exists once they sign in (no pre-provisioning); she then takes the handle `alice`.
    app.sign_in_as("sub-alice", "alice", false);
    app.browser_login("/").await;
    let first = app.handle_of("sub-alice").await;
    let first_token = admin::operator_token(&app.state.db, &first, None)
        .await
        .unwrap();
    let (st, v) = app
        .rest(
            &first_token,
            reqwest::Method::PATCH,
            "/me",
            Some(json!({"name": "alice"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
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
    let (_, v) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    let (_, s) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": "ses_p"}),
        )
        .await;
    app.post(&alice, "/projects/PAY/tasks", json!({"title": "refunds"}))
        .await;
    ok(
        &app.mcp(
            &c1,
            "claim_task",
            json!({"id": "PAY-1", "station": s["id"]}),
        )
        .await,
        "claim",
    );
    ok(
        &app.mcp(&c1, "ask", json!({"task": "PAY-1", "question": "Round down or up?", "wait": true, "station": s["id"]})).await,
        "ask",
    );
    let status: String = sqlx::query_scalar("select status from tasks where number = 1")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(status, "blocked");

    let c = cursor(&app).await;
    ok(
        &app.mcp(
            &alice,
            "comment",
            json!({"id": "PAY-1", "body": "Down, like the ledger."}),
        )
        .await,
        "comment",
    );
    let w = wakes_after(&app, c).await;
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(
        (&w[0]["agent"], &w[0]["reason"], &w[0]["station"]["id"]),
        (&json!("c1"), &json!("comment"), &s["id"])
    );
    let text = w[0]["text"].as_str().unwrap();
    assert!(
        text.contains("parked with you") && text.contains("Down, like the ledger."),
        "{text}"
    );
}
