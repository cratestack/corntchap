//! A list is for choosing: `list_tasks` cuts long bodies and results, `get_task` keeps them whole.
mod common;
use corntchap::admin;
use serde_json::json;

#[tokio::test]
async fn list_tasks_cuts_long_bodies_and_get_task_keeps_them() {
    let app = common::start(false).await;
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
    let long = "x".repeat(2_000);
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Long", "body": long}),
    )
    .await;
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Short", "body": "tiny"}),
    )
    .await;

    let list = common::TestApp::payload(
        &app.mcp(&root, "list_tasks", json!({"project": "PAY"}))
            .await,
    );
    let body = |t: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|x| x["title"] == t)
            .unwrap()["body"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let cut = body("Long");
    assert_eq!(
        cut.chars().count(),
        corntchap::service::LIST_CUT + 1,
        "280 characters and an ellipsis"
    );
    assert!(cut.ends_with('…'));
    assert_eq!(body("Short"), "tiny", "short bodies are untouched");

    let one = common::TestApp::payload(&app.mcp(&root, "get_task", json!({"id": "PAY-1"})).await);
    let full = one["task"]["body"]
        .as_str()
        .or(one["body"].as_str())
        .unwrap();
    assert_eq!(
        full.chars().count(),
        2_000,
        "get_task has the whole body: {one}"
    );
}
