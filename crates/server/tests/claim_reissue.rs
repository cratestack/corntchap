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

#[tokio::test]
async fn tasks_with_an_open_pull_request_are_not_handed_out() {
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
    let (_, c) = app
        .post(
            &root,
            "/agents",
            json!({"name": "c1", "kind": "claude-code", "runtime": "local", "max_leases": 5}),
        )
        .await;
    let c1 = c["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    for (t, prio) in [("has a PR", 9), ("plain", 1)] {
        app.post(
            &root,
            "/projects/PAY/tasks",
            json!({"title": t, "priority": prio}),
        )
        .await;
    }
    sqlx::query(
        "insert into forge_prs (provider, repo, norm_url, number, url, author, head_branch, task_id)
         select 'github', 'acme/pay', 'github.com/acme/pay', 12, 'https://github.com/acme/pay/pull/12',
                'c1[bot]', 'pay-1-fix', id from tasks where number = 1",
    )
    .execute(&app.state.db)
    .await
    .unwrap();

    // the open PR keeps PAY-1 out of claim_next and wait_for_work, though it outranks PAY-2
    let w = ok(
        &app.mcp(
            &c1,
            "wait_for_work",
            json!({"project": "PAY", "timeoutS": 0}),
        )
        .await,
        "wait_for_work",
    );
    assert_eq!(w["ready"].as_array().unwrap().len(), 1, "{w}");
    assert_eq!(w["ready"][0]["id"], "PAY-2", "{w}");
    let two = ok(
        &app.mcp(&c1, "claim_next", json!({"project": "PAY"})).await,
        "claim_next",
    );
    assert_eq!(two["task"]["id"], "PAY-2");
    let none = ok(
        &app.mcp(&c1, "claim_next", json!({"project": "PAY"})).await,
        "claim_next none",
    );
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["open_prs"], "PAY-1 (acme/pay#12)", "{none}");
    assert!(none["released_recently"].is_null(), "{none}");

    // claim_task takes it, and says where the work is
    let one = ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim_task",
    );
    let w = one["warning"].as_str().unwrap();
    assert!(
        w.contains("acme/pay#12") && w.contains("pay-1-fix"),
        "{one}"
    );

    // once the PR is closed, the task is ordinary work again
    sqlx::query("update forge_prs set state = 'merged'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let free: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select count(*) from tasks t where t.number = 1 and {}",
        corntchap::service::NO_OPEN_PR_SQL
    )))
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(free, 1);
}
