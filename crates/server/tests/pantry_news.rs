//! Pantry news: what was saved or changed since a cook last read the pantry comes with its next
//! claim (and wake); `notify` broadcasts a memory to every session at once.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

#[tokio::test]
async fn pantry_news_and_notify() {
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
    let mut tok = std::collections::HashMap::new();
    for name in ["c1", "c2"] {
        let (_, v) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local", "max_leases": 5}),
            )
            .await;
        tok.insert(name, v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let (c1, c2) = (tok["c1"].clone(), tok["c2"].clone());
    for t in ["one", "two", "three"] {
        app.post(&alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    // c1 reads the pantry; then alice and c1 each save something
    ok(
        &app.mcp(&c1, "read_pantry", json!({"project": "PAY"})).await,
        "read",
    );
    ok(
        &app.mcp(&alice, "remember", json!({"project": "PAY", "title": "Never deploy on Fridays", "body": "Freeze from Friday noon.", "kind": "convention"}))
            .await,
        "alice saves",
    );
    ok(
        &app.mcp(
            &c1,
            "remember",
            json!({"project": "PAY", "title": "Ledger needs VPN", "body": "Connect first."}),
        )
        .await,
        "c1 saves",
    );
    // c1's next claim carries alice's memory, not its own
    let claim = ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    assert_eq!(claim["pantry"]["count"], 1, "{claim}");
    assert_eq!(
        claim["pantry"]["items"][0]["key"],
        "never-deploy-on-fridays"
    );
    assert_eq!(claim["pantry"]["items"][0]["by"], "alice");
    // c2 never read it: everything is news
    let claim2 = ok(
        &app.mcp(&c2, "claim_next", json!({"project": "PAY"})).await,
        "claim c2",
    );
    assert_eq!(claim2["pantry"]["count"], 2);
    // reading clears it
    ok(
        &app.mcp(
            &c1,
            "get_memory",
            json!({"project": "PAY", "key": "never-deploy-on-fridays"}),
        )
        .await,
        "get",
    );
    let claim = ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-3"})).await,
        "claim again",
    );
    assert!(
        claim.get("pantry").is_none() || claim["pantry"].is_null(),
        "{claim}"
    );

    // notify: a broadcast on the pass wakes every cook (alice is a person: both cooks)
    let before: i64 = sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    ok(
        &app.mcp(&alice, "remember", json!({"project": "PAY", "title": "Do not touch the ledger schema", "body": "A migration is in flight.", "kind": "gotcha", "notify": true}))
            .await,
        "notify",
    );
    let woken: Vec<String> = sqlx::query_scalar(
        "select payload->>'agent' from events where kind = 'wake' and seq > $1 order by payload->>'agent'",
    )
    .bind(before)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(woken, ["c1", "c2"]);
    let page = ok(
        &app.mcp(&c2, "read_pass", json!({"project": "PAY", "chef": "pot"}))
            .await,
        "pass",
    );
    let body = page["messages"][0]["body"].as_str().unwrap();
    assert!(
        body.contains("Pantry, new gotcha") && body.contains("do-not-touch-the-ledger-schema"),
        "{body}"
    );
    // and the wake itself mentions the pantry news
    let text: String = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' and payload->>'agent' = 'c2' order by seq desc limit 1",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert!(
        text.contains("Pantry:") && text.contains("do-not-touch-the-ledger-schema"),
        "{text}"
    );

    // removals are news too: quietly with the next claim, or to everyone at once with notify
    ok(
        &app.mcp(&c1, "read_pantry", json!({"project": "PAY"})).await,
        "c1 reads",
    );
    ok(
        &app.mcp(
            &alice,
            "forget",
            json!({"project": "PAY", "key": "never-deploy-on-fridays"}),
        )
        .await,
        "forget quietly",
    );
    app.post(&alice, "/projects/PAY/tasks", json!({"title": "four"}))
        .await;
    let claim = ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-4"})).await,
        "claim after removal",
    );
    assert_eq!(claim["pantry"]["count"], 1, "{claim}");
    assert_eq!(
        (
            &claim["pantry"]["items"][0]["key"],
            &claim["pantry"]["items"][0]["deleted"]
        ),
        (&json!("never-deploy-on-fridays"), &json!(true))
    );
    let before: i64 = sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    ok(
        &app.mcp(
            &alice,
            "forget",
            json!({"project": "PAY", "key": "do-not-touch-the-ledger-schema", "notify": true}),
        )
        .await,
        "forget loudly",
    );
    let woken: Vec<String> = sqlx::query_scalar(
        "select payload->>'agent' from events where kind = 'wake' and seq > $1 order by payload->>'agent'",
    )
    .bind(before)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(woken, ["c1", "c2"], "a notified removal reaches every cook");
    let page = ok(
        &app.mcp(
            &c1,
            "read_pass",
            json!({"project": "PAY", "chef": "pot", "limit": 1}),
        )
        .await,
        "pass",
    );
    assert!(
        page["messages"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Pantry, removed: **Do not touch the ledger schema**"),
        "{page}"
    );
}
