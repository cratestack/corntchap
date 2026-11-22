mod common;
use corntchap::admin;
use serde_json::{Value, json};

/// A parent with open subtasks is not ready: claiming it would strand an agent with one lease,
/// because `complete` refuses a parent whose subtasks are open.
#[tokio::test]
async fn parent_is_not_claimable_until_subtasks_are_terminal() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let t = tok.unwrap();
    app.post(&t, "/teams", json!({"name": "acme"})).await;
    app.post(
        &t,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &t,
            "/agents",
            json!({"name": "a1", "kind": "other", "runtime": "local"}),
        )
        .await;
    let agent = ag["token"].as_str().unwrap().to_string();
    app.post(&t, "/teams/acme/members", json!({"principal": "a1"}))
        .await;

    // PAY-1 is the parent (highest priority), PAY-2 and PAY-3 its subtasks.
    for (title, parent, prio) in [
        ("parent", None, 9),
        ("sub a", Some("PAY-1"), 0),
        ("sub b", Some("PAY-1"), 0),
    ] {
        let mut a = json!({"project": "PAY", "title": title, "priority": prio});
        if let Some(p) = parent {
            a["parent"] = json!(p);
        }
        let r = app.mcp(&t, "create_task", a).await;
        assert_ne!(r["isError"], true, "{r}");
    }

    let (_, parent) = app.get(&agent, "/tasks/PAY-1").await;
    assert_eq!(parent["ready"], false, "{parent}");
    let (_, ready) = app.get(&agent, "/projects/PAY/tasks?ready_only=true").await;
    let ids: Vec<_> = ready
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["PAY-2", "PAY-3"], "{ready}");
    let (_, projects) = app.get(&agent, "/projects").await;
    assert_eq!(projects[0]["ready"], 2, "{projects}");

    // Naming the parent explicitly is refused with a clear reason.
    let r = app.mcp(&agent, "claim_task", json!({"id": "PAY-1"})).await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("subtasks"),
        "{r}"
    );

    // claim_next skips the higher-priority parent and takes a subtask, then finishes both.
    for expect in ["PAY-2", "PAY-3"] {
        let c = common::TestApp::payload(
            &app.mcp(&agent, "claim_next", json!({"project": "PAY"}))
                .await,
        );
        assert_eq!(c["task"]["id"], expect, "{c}");
        let epoch = c["lease"]["epoch"].as_i64().unwrap();
        let done = app
            .mcp(
                &agent,
                "complete",
                json!({"id": expect, "epoch": epoch, "result": "ok"}),
            )
            .await;
        assert_ne!(done["isError"], true, "{done}");
        if expect == "PAY-2" {
            // one subtask is still open: the parent stays unclaimable
            let (_, p) = app.get(&agent, "/tasks/PAY-1").await;
            assert_eq!(p["ready"], false);
        }
    }

    // Both subtasks done: now the parent is claimable and completable by the same agent.
    let (_, p) = app.get(&agent, "/tasks/PAY-1").await;
    assert_eq!(p["ready"], true, "{p}");
    let c = common::TestApp::payload(
        &app.mcp(&agent, "claim_next", json!({"project": "PAY"}))
            .await,
    );
    assert_eq!(c["task"]["id"], "PAY-1", "{c}");
    let epoch = c["lease"]["epoch"].as_i64().unwrap();
    let done: Value = app
        .mcp(
            &agent,
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "all done"}),
        )
        .await;
    assert_ne!(done["isError"], true, "{done}");
}

/// A cancelled subtask counts as finished.
#[tokio::test]
async fn cancelled_subtask_unblocks_the_parent() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let t = tok.unwrap();
    app.post(&t, "/teams", json!({"name": "acme"})).await;
    app.post(
        &t,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    app.mcp(
        &t,
        "create_task",
        json!({"project": "PAY", "title": "parent"}),
    )
    .await;
    app.mcp(
        &t,
        "create_task",
        json!({"project": "PAY", "title": "sub", "parent": "PAY-1"}),
    )
    .await;
    assert_eq!(app.get(&t, "/tasks/PAY-1").await.1["ready"], false);
    assert_eq!(app.post(&t, "/tasks/PAY-2/cancel", json!({})).await.0, 200);
    assert_eq!(app.get(&t, "/tasks/PAY-1").await.1["ready"], true);
}
