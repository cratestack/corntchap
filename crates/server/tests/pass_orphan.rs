//! A direct message from a chef who was deleted afterwards: its author becomes null, and the pass
//! must still read (it used to fail decoding a null conversation key).
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

#[tokio::test]
async fn the_pass_reads_after_a_direct_message_author_is_deleted() {
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
    let (st, v) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    ok(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "PAY", "message": "done, see PAY-1", "to": "alice"}),
        )
        .await,
        "dm",
    );
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "hello pot"}),
        )
        .await,
        "pot",
    );
    ok(
        &app.mcp(&alice, "delete_cook", json!({"name": "c1"})).await,
        "delete c1",
    );
    // everything alice can read, the pot only, and the counts: no null conversation anywhere
    let all = ok(
        &app.mcp(&alice, "read_pass", json!({"project": "PAY"}))
            .await,
        "read all",
    );
    let bodies: Vec<&str> = all["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert!(bodies.contains(&"done, see PAY-1"), "{all}");
    assert!(
        all["unread"].to_string().find("gone").is_none(),
        "a deleted chef's conversation is not listed: {all}"
    );
    ok(
        &app.mcp(
            &alice,
            "read_pass",
            json!({"project": "PAY", "chef": "pot"}),
        )
        .await,
        "read pot",
    );
    // reading again (the read marks are stored per conversation) still works
    ok(
        &app.mcp(&alice, "read_pass", json!({"project": "PAY"}))
            .await,
        "read again",
    );
}
