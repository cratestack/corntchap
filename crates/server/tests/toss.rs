//! Tossing (status `cancelled`, shown as "Tossed"): people only, with a reason, also while a cook
//! holds the task (its lease ends), and restoring.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

#[tokio::test]
async fn people_toss_and_restore_tasks() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    app.post(&admin, "/teams", json!({"name": "acme"})).await;
    app.post(
        &admin,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "P"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &admin,
            "/agents",
            json!({"name": "a1", "kind": "other", "runtime": "local"}),
        )
        .await;
    let a1 = ag["token"].as_str().unwrap().to_string();
    app.post(&admin, "/teams/acme/members", json!({"principal": "a1"}))
        .await;
    for t in ["one", "two"] {
        app.post(&admin, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }

    // toss an open task with a reason
    let (st, t) = app
        .post(
            &admin,
            "/tasks/PAY-1/toss",
            json!({"reason": "duplicate of PAY-2"}),
        )
        .await;
    assert_eq!(
        (st, &t["status"], &t["result"]),
        (200, &json!("cancelled"), &json!("duplicate of PAY-2")),
        "{t}"
    );
    // agents cannot toss or restore
    let (st, _) = app.post(&a1, "/tasks/PAY-2/toss", json!({})).await;
    assert_eq!(st, 403);
    let (st, _) = app.post(&a1, "/tasks/PAY-1/restore", json!({})).await;
    assert_eq!(st, 403);

    // toss a task a cook holds: its lease ends and its next lease call fails
    let claim = common::TestApp::payload(&app.mcp(&a1, "claim_task", json!({"id": "PAY-2"})).await);
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    let (st, t) = app.post(&admin, "/tasks/PAY-2/cancel", json!({})).await;
    assert_eq!((st, &t["status"]), (200, &json!("cancelled")), "{t}");
    assert!(t["claimed_by"].is_null());
    let r = app
        .mcp(
            &a1,
            "complete",
            json!({"id": "PAY-2", "epoch": epoch, "result": "done anyway"}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("lease_lost"),
        "{r}"
    );

    // tossing twice is a conflict; restore puts it back on the counter without the reason
    let (st, _) = app.post(&admin, "/tasks/PAY-1/toss", json!({})).await;
    assert_eq!(st, 409);
    let (st, t) = app.post(&admin, "/tasks/PAY-1/restore", json!({})).await;
    assert_eq!(
        (st, &t["status"], &t["result"]),
        (200, &json!("open"), &Value::Null),
        "{t}"
    );
    let (st, _) = app.post(&admin, "/tasks/PAY-1/restore", json!({})).await;
    assert_eq!(st, 409);

    // the feed says why
    let feed = common::TestApp::payload(
        &app.mcp(&a1, "changes_since", json!({"cursor": 0, "project": "PAY"}))
            .await,
    );
    let tossed: Vec<&Value> = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "task.cancelled" || e["kind"] == "task.restored")
        .collect();
    assert_eq!(tossed.len(), 3);
    assert_eq!(tossed[0]["payload"]["reason"], "duplicate of PAY-2");
    assert_eq!(tossed[2]["kind"], "task.restored");
}
