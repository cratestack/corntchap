//! No duplicate wakes: a cook is not woken by its own actions (it is mid-turn and loops on
//! claim_next by itself), only by other chefs' work.
mod common;
use std::time::Duration;

use corntchap::admin;
use serde_json::json;

async fn wakes_for(app: &common::TestApp, agent: &str) -> i64 {
    sqlx::query_scalar("select count(*) from events where kind = 'wake' and payload->>'agent' = $1")
        .bind(agent)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_cook_is_not_woken_by_its_own_work() {
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &root,
            "/agents",
            json!({"name": "doug", "kind": "opencode", "runtime": "local", "max_leases": 3}),
        )
        .await;
    let doug = ag["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "doug"}))
        .await;
    let (st, _) = app
        .post(&root, "/agents/doug/wake-targets", json!({"type": "relay"}))
        .await;
    assert_eq!(st, 200);
    // let the dispatcher's start-up pass go by
    tokio::time::sleep(Duration::from_millis(500)).await;

    // doug files work itself: it is busy right now, no wake
    let r = app
        .mcp(
            &doug,
            "create_task",
            json!({"project": "PAY", "title": "my own"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(wakes_for(&app, "doug").await, 0, "woken by its own task");

    // someone else's work wakes it
    app.post(&root, "/projects/PAY/tasks", json!({"title": "from octo"}))
        .await;
    let mut n = 0;
    for _ in 0..30 {
        n = wakes_for(&app, "doug").await;
        if n > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(n, 1, "one wake for another chef's work");
}
