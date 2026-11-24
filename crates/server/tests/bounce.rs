//! A task a cook keeps giving back is not handed to it again (3 give-backs with nobody touching it
//! in between); given back 5 times it is blocked for a person. A release made while a pull request
//! is open does not count, and a parked pull request makes its task work again.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

struct World {
    app: common::TestApp,
    root: String,
    cooks: Vec<String>,
}

async fn world() -> World {
    // a release is not held back by the cooldown here: bouncing is the rule under test
    corntchap::service::RELEASE_COOLDOWN_SECS.store(0, std::sync::atomic::Ordering::Relaxed);
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
    let mut cooks = vec![];
    for name in ["c1", "c2"] {
        let (_, v) = app
            .post(
                &root,
                "/agents",
                json!({"name": name, "kind": "claude-code", "runtime": "local", "max_leases": 3}),
            )
            .await;
        cooks.push(v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    app.post(&root, "/projects/PAY/tasks", json!({"title": "stubborn"}))
        .await;
    World { app, root, cooks }
}

impl World {
    /// Claim with claim_next and give it back.
    async fn bounce(&self, cook: usize) {
        let c = ok(
            &self
                .app
                .mcp(&self.cooks[cook], "claim_next", json!({"project": "PAY"}))
                .await,
            "claim_next",
        );
        assert_eq!(c["task"]["id"], "PAY-1", "{c}");
        ok(
            &self
                .app
                .mcp(
                    &self.cooks[cook],
                    "release",
                    json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "reason": "not for me"}),
                )
                .await,
            "release",
        );
    }

    async fn status(&self) -> String {
        sqlx::query_scalar("select status from tasks where number = 1")
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_cook_that_gave_a_task_back_three_times_is_not_handed_it_again() {
    let w = world().await;
    for _ in 0..3 {
        w.bounce(0).await;
    }
    // claim_next: nothing, and it says why
    let none = ok(
        &w.app
            .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["bounced"], "PAY-1", "{none}");
    let hint = none["hint"].as_str().unwrap();
    assert!(hint.contains("PAY-1") && hint.contains("3 times"), "{hint}");
    assert!(
        !hint.contains("Take one by name"),
        "claim_task refuses it too: {hint}"
    );
    // claim_task by name is refused
    let r = w
        .app
        .mcp(&w.cooks[0], "claim_task", json!({"id": "PAY-1"}))
        .await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("given PAY-1 back 3 times"),
        "{r}"
    );
    // another cook is not held back
    let c = ok(
        &w.app
            .mcp(&w.cooks[1], "claim_next", json!({"project": "PAY"}))
            .await,
        "other cook",
    );
    assert_eq!(c["task"]["id"], "PAY-1");
    ok(
        &w.app
            .mcp(
                &w.cooks[1],
                "release",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "reason": "no"}),
            )
            .await,
        "release",
    );
    // still closed to the first cook, but a person's comment opens it again
    let none = ok(
        &w.app
            .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true, "{none}");
    ok(
        &w.app
            .mcp(
                &w.root,
                "comment",
                json!({"id": "PAY-1", "body": "use the v2 client, see PAY-9"}),
            )
            .await,
        "person comments",
    );
    let c = ok(
        &w.app
            .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
            .await,
        "after the comment",
    );
    assert_eq!(c["task"]["id"], "PAY-1", "{c}");
}

#[tokio::test]
async fn five_give_backs_block_the_task_for_a_person() {
    let w = world().await;
    for _ in 0..3 {
        w.bounce(0).await;
    }
    w.bounce(1).await;
    assert_eq!(w.status().await, "open");
    w.bounce(1).await;
    assert_eq!(w.status().await, "blocked");
    let reason: String = sqlx::query_scalar(
        "select payload->>'reason' from events where kind = 'task.blocked' and actor_id is null",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(reason, "bounced 5×: needs a person");
    let note: String = sqlx::query_scalar("select body from comments where author_id is null")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert!(note.contains("bounced 5"), "{note}");
    // nobody is handed it any more
    let none = ok(
        &w.app
            .mcp(&w.cooks[1], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true);
    // a person unblocks it and the count starts over
    let (s, _) = w.app.post(&w.root, "/tasks/PAY-1/unblock", json!({})).await;
    assert_eq!(s, 200);
    w.bounce(1).await;
    assert_eq!(w.status().await, "open");
}

#[tokio::test]
async fn a_lapsed_lease_counts_as_a_give_back() {
    let w = world().await;
    for _ in 0..3 {
        let c = ok(
            &w.app
                .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
                .await,
            "claim_next",
        );
        assert_eq!(c["task"]["id"], "PAY-1", "{c}");
        sqlx::query(
            "update tasks set lease_expires_at = now() - interval '1 second' where number = 1",
        )
        .execute(&w.app.state.db)
        .await
        .unwrap();
        corntchap::jobs::reap(&w.app.state).await.unwrap();
    }
    let none = ok(
        &w.app
            .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["bounced"], "PAY-1");
}

async fn open_pr(w: &World, checks: &str) {
    sqlx::query(
        "insert into forge_prs (provider, repo, norm_url, number, task_id, state, checks, head_branch)
         values ('github', 'acme/shop', 'github.com/acme/shop', 7, (select id from tasks where number = 1), 'open', $1, 'pay-1-x')",
    )
    .bind(checks)
    .execute(&w.app.state.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_release_with_an_open_pr_is_not_a_bounce() {
    let w = world().await;
    open_pr(&w, "success").await;
    // a cook that opens a PR and releases "PR open" over and over is waiting, not bouncing
    for _ in 0..5 {
        let c = ok(
            &w.app
                .mcp(&w.cooks[0], "claim_task", json!({"id": "PAY-1"}))
                .await,
            "claim_task",
        );
        ok(
            &w.app
                .mcp(
                    &w.cooks[0],
                    "release",
                    json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "reason": "PR open: https://github.com/acme/shop/pull/7"}),
                )
                .await,
            "release",
        );
    }
    assert_eq!(w.status().await, "open", "never blocked");
    let flagged: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'task.released' and payload->>'on_pr' = 'true'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(flagged, 5);
    // a healthy PR keeps the task out of claim_next
    let none = ok(
        &w.app
            .mcp(&w.cooks[1], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["open_prs"], "PAY-1 (acme/shop#7)");
    // failing CI parks the PR: the task is work again
    sqlx::query("update forge_prs set checks = 'failure', failing = array['build']")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let c = ok(
        &w.app
            .mcp(&w.cooks[1], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(c["task"]["id"], "PAY-1", "{c}");
}

#[tokio::test]
async fn changes_requested_and_conflicts_park_a_pr_too() {
    for (set, what) in [
        ("review = 'changes_requested'", "changes requested"),
        ("mergeable = 'false'", "conflict"),
    ] {
        let w = world().await;
        open_pr(&w, "success").await;
        let none = ok(
            &w.app
                .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
                .await,
            "claim_next",
        );
        assert_eq!(none["none"], true, "{what}: {none}");
        sqlx::query(sqlx::AssertSqlSafe(format!("update forge_prs set {set}")))
            .execute(&w.app.state.db)
            .await
            .unwrap();
        let c = ok(
            &w.app
                .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
                .await,
            "claim_next",
        );
        assert_eq!(c["task"]["id"], "PAY-1", "{what}: {c}");
    }
}

#[tokio::test]
async fn giving_back_a_task_with_a_failing_pr_is_bouncing() {
    let w = world().await;
    open_pr(&w, "failure").await;
    sqlx::query("update forge_prs set failing = array['build']")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    // the PR is parked: the task is work, and releasing it is a give-back, not "waiting on CI"
    for _ in 0..3 {
        w.bounce(0).await;
    }
    let flagged: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'task.released' and payload->>'on_pr' = 'true'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(flagged, 0);
    let none = ok(
        &w.app
            .mcp(&w.cooks[0], "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(none["none"], true, "{none}");
    assert_eq!(none["bounced"], "PAY-1", "{none}");
    // the other cook is still offered it; two more give-backs by it block it for a person
    w.bounce(1).await;
    assert_eq!(w.status().await, "open");
    w.bounce(1).await;
    assert_eq!(w.status().await, "blocked");
    let note: String = sqlx::query_scalar("select body from comments where author_id is null")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert!(note.contains("bounced 5"), "{note}");
}

#[tokio::test]
async fn a_person_may_claim_a_task_a_cook_keeps_giving_back() {
    let w = world().await;
    for _ in 0..3 {
        w.bounce(0).await;
    }
    let (s, v) = w.app.post(&w.root, "/tasks/PAY-1/claim", json!({})).await;
    assert_eq!(s, 200, "{v}");
}
