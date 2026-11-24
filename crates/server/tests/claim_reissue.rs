//! A cook that gives a task back must not get it straight back from `claim_next`, and a cook that
//! takes a task whose last lease lapsed is told who held it (that work may still be in a worktree).
mod common;
use corntchap::{admin, jobs};
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

async fn expire(app: &common::TestApp, number: i64) {
    sqlx::query(
        "update tasks set lease_expires_at = now() - interval '3 minutes' where number = $1",
    )
    .bind(number)
    .execute(&app.state.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn released_tasks_go_to_other_cooks_and_lapses_are_named() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay"}),
    )
    .await;
    let mut cook = Vec::new();
    for name in ["c1", "c2"] {
        let (_, c) = app
            .post(
                &root,
                "/agents",
                json!({"name": name, "kind": "claude-code", "runtime": "local", "max_leases": 5}),
            )
            .await;
        cook.push(c["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let (c1, c2) = (&cook[0], &cook[1]);
    for (t, prio) in [("first", 9), ("second", 5), ("third", 3), ("fourth", 1)] {
        app.post(
            &root,
            "/projects/PAY/tasks",
            json!({"title": t, "priority": prio}),
        )
        .await;
    }
    let next = |tok: &str| {
        let tok = tok.to_string();
        let app = &app;
        async move {
            ok(
                &app.mcp(&tok, "claim_next", json!({"project": "PAY"})).await,
                "claim_next",
            )
        }
    };
    let release = |tok: &str, v: &Value| {
        let (tok, id, epoch) = (
            tok.to_string(),
            v["task"]["id"].clone(),
            v["lease"]["epoch"].clone(),
        );
        let app = &app;
        async move {
            ok(
                &app.mcp(
                    &tok,
                    "release",
                    json!({"id": id, "epoch": epoch, "reason": "not mine"}),
                )
                .await,
                "release",
            )
        }
    };

    // c1 gives PAY-1 back: its next claim_next is PAY-2, not PAY-1 again
    let a = next(c1).await;
    assert_eq!(a["task"]["id"], "PAY-1");
    let r = release(c1, &a).await;
    assert_eq!(r["status"], "open");
    // a released task shows no live station; the last one stays readable
    assert!(
        r["station"].is_null() && r.get("last_station").is_some(),
        "{r}"
    );
    let b = next(c1).await;
    assert_eq!(b["task"]["id"], "PAY-2", "{b}");
    release(c1, &b).await;

    // wait_for_work does not offer c1 what it gave back (else it would spin on claim_next)
    let w = ok(
        &app.mcp(
            c1,
            "wait_for_work",
            json!({"project": "PAY", "timeoutS": 0}),
        )
        .await,
        "wait_for_work",
    );
    let ready: Vec<&str> = w["ready"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ready, ["PAY-3", "PAY-4"], "{w}");

    // the other cook takes it at once
    let c = next(c2).await;
    assert_eq!(c["task"]["id"], "PAY-1");
    assert!(c["warning"].is_null(), "a release is no lapse: {c}");

    // when only released tasks are left, claim_next says so and how to take one back
    let three = next(c1).await;
    let four = next(c1).await;
    assert_eq!(
        (&three["task"]["id"], &four["task"]["id"]),
        (&json!("PAY-3"), &json!("PAY-4"))
    );
    let none = next(c1).await;
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["released_recently"], "PAY-2", "{none}");
    assert!(
        none["hint"].as_str().unwrap().contains("claim_task"),
        "{none}"
    );
    // ...and claim_task by name is never held back
    let back = ok(
        &app.mcp(c1, "claim_task", json!({"id": "PAY-2"})).await,
        "claim_task",
    );
    assert_eq!(back["task"]["id"], "PAY-2");

    // a lease that lapsed and was reaped: the next holder is told who had it
    expire(&app, 1).await;
    jobs::reap(&app.state).await.unwrap();
    let lapsed = next(c1).await;
    // c1 released PAY-1 a moment ago, so nothing for it; c2 takes its own lapsed task back
    assert_eq!(lapsed["none"], true, "{lapsed}");
    let mine = next(c2).await;
    assert_eq!(mine["task"]["id"], "PAY-1");
    assert_eq!(mine["previous_lease"]["yours"], true, "{mine}");
    assert!(
        mine["warning"]
            .as_str()
            .unwrap()
            .contains("Your last lease on PAY-1"),
        "{mine}"
    );

    // a lease that lapsed and was not reaped yet: claim_task names the holder too
    expire(&app, 3).await;
    let other = ok(
        &app.mcp(c2, "claim_task", json!({"id": "PAY-3"})).await,
        "claim_task lapsed",
    );
    assert_eq!(other["previous_lease"]["holder"], "c1", "{other}");
    assert_eq!(other["previous_lease"]["yours"], false, "{other}");
    assert!(
        other["warning"].as_str().unwrap().contains("held by c1"),
        "{other}"
    );
}
