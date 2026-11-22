//! A cloud cook remembers nothing between runs: each wake carries its recent exchange on the pass,
//! so "how far?" arrives with the "ping"/"pong" before it.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) {
    assert_ne!(r["isError"], true, "{what}: {r}");
}

#[tokio::test]
async fn a_cloud_wake_carries_the_conversation_so_far() {
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
    let (_, c) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "rita", "kind": "claude-code", "runtime": "cloud"}),
        )
        .await;
    let rita = c["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "rita"}))
        .await;

    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "Ping", "to": "rita"}),
        )
        .await,
        "ping",
    );
    ok(
        &app.mcp(
            &rita,
            "pass",
            json!({"project": "PAY", "message": "Pong, awake", "to": "alice"}),
        )
        .await,
        "pong",
    );
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "Lunch at noon, everyone"}),
        )
        .await,
        "noise",
    );
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "@rita can you look at refunds?"}),
        )
        .await,
        "mention",
    );
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "How far?", "to": "rita"}),
        )
        .await,
        "follow-up",
    );

    let (agent, project): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'rita'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    let mut conn = app.state.db.acquire().await.unwrap();
    let text = corntchap::cloud::conversation(&mut conn, agent, "rita", project).await;
    let pos = |s: &str| {
        text.find(s)
            .unwrap_or_else(|| panic!("{s} missing in {text}"))
    };
    assert!(pos("alice → rita: Ping") < pos("rita → alice: Pong, awake"));
    assert!(pos("Pong, awake") < pos("@rita can you look at refunds?"));
    assert!(pos("refunds?") < pos("alice → rita: How far?"));
    assert!(
        !text.contains("Lunch at noon"),
        "pot chatter that does not involve the cook stays out: {text}"
    );

    // and it is part of what a cloud run receives
    let wake = json!({"reason": "message", "text": "alice wrote to you: How far?"});
    let full = corntchap::cloud::messages_text(&mut conn, agent, "rita", project, "PAY", &[wake])
        .await
        .full;
    assert!(
        full.contains("The conversation so far") && full.contains("Pong, awake"),
        "{full}"
    );

    // a cook with no exchange gets no section
    let none =
        corntchap::cloud::conversation(&mut conn, uuid::Uuid::new_v4(), "nobody", project).await;
    assert!(none.is_empty());
}
