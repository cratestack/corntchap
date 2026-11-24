//! An agent back after an absence checks the board first: its first claim in a pot after 30 minutes
//! away returns a `catch_up` and no lease; reading `what_now` first counts as the briefing; people
//! are never gated.
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
}

async fn world() -> World {
    let app = common::start(false).await;
    // this file is about the gate: no helper reads what_now for us
    app.without_briefing();
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
    for t in ["one", "two", "three", "four", "five"] {
        app.post(&root, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    World { app, root }
}

impl World {
    async fn cook(&self, name: &str) -> String {
        let (_, v) = self
            .app
            .post(
                &self.root,
                "/agents",
                json!({"name": name, "kind": "claude-code", "runtime": "local", "max_leases": 3}),
            )
            .await;
        self.app
            .post(
                &self.root,
                "/teams/acme/members",
                json!({"principal": name}),
            )
            .await;
        v["token"].as_str().unwrap().to_string()
    }

    async fn live_leases(&self) -> i64 {
        sqlx::query_scalar("select count(*) from tasks where status = 'claimed'")
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_new_agents_first_claim_is_a_catch_up_not_a_lease() {
    let w = world().await;
    let c1 = w.cook("c1").await;
    let first = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "first claim",
    );
    assert!(first["none"].is_null(), "loops stop on none: {first}");
    assert!(
        first["task"].is_null() && first["lease"].is_null(),
        "{first}"
    );
    assert!(first["catch_up"].is_object(), "{first}");
    assert!(
        first["next"].as_str().unwrap().starts_with("You were away"),
        "{first}"
    );
    assert_eq!(w.live_leases().await, 0, "no lease was taken");
    // the second claim takes work
    let second = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "second claim",
    );
    assert_eq!(second["task"]["id"], "PAY-1", "{second}");
    assert!(second["lease"]["epoch"].is_i64());
    assert!(second["catch_up"].is_null());
    // claim_task is gated the same way for another agent, and so is REST
    let c2 = w.cook("c2").await;
    let t = ok(
        &w.app.mcp(&c2, "claim_task", json!({"id": "PAY-2"})).await,
        "claim_task",
    );
    assert!(t["catch_up"].is_object() && t["task"].is_null(), "{t}");
    let (_, rest) = w.app.post(&c2, "/projects/PAY/claim-next", json!({})).await;
    assert_eq!(rest["task"]["id"], "PAY-2", "{rest}");
}

#[tokio::test]
async fn an_absence_of_two_hours_gates_it_again() {
    let w = world().await;
    let c1 = w.cook("c1").await;
    ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "catch up",
    );
    let got = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(got["task"]["id"], "PAY-1");
    // calls inside the half hour change nothing
    ok(&w.app.mcp(&c1, "whoami", json!({})).await, "whoami");
    let more = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(more["task"]["id"], "PAY-2", "{more}");
    // 2 hours of silence
    sqlx::query(
        "update principals set last_seen_at = now() - interval '2 hours' where name = 'c1'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    let back = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "back",
    );
    assert!(back["catch_up"].is_object(), "{back}");
    assert!(back["none"].is_null() && back["lease"].is_null());
    let cu = &back["catch_up"];
    assert!(cu["away_for"].as_str().unwrap().starts_with("2 h"), "{cu}");
    assert_eq!(
        cu["holding"].as_array().unwrap().len(),
        2,
        "it still holds two: {cu}"
    );
    assert!(
        back["next"].as_str().unwrap().contains("still hold"),
        "{back}"
    );
    // and then it goes through
    let again = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(again["task"]["id"], "PAY-3", "{again}");
}

#[tokio::test]
async fn what_now_first_is_the_briefing() {
    let w = world().await;
    let c1 = w.cook("c1").await;
    ok(
        &w.app.mcp(&c1, "what_now", json!({"project": "PAY"})).await,
        "what_now",
    );
    let got = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(got["task"]["id"], "PAY-1", "{got}");

    // the other reads brief too: get_board, my_work, changes_since with a project
    for (i, (tool, args)) in [
        ("get_board", json!({"project": "PAY"})),
        ("my_work", json!({"project": "PAY"})),
        ("changes_since", json!({"cursor": 0, "project": "PAY"})),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("r{i}");
        let tok = w.cook(&name).await;
        ok(&w.app.mcp(&tok, tool, args).await, tool);
        let got = ok(
            &w.app
                .mcp(&tok, "claim_task", json!({"id": format!("PAY-{}", i + 2)}))
                .await,
            "claim",
        );
        assert!(
            got["lease"].is_object() && got["catch_up"].is_null(),
            "{tool}: {got}"
        );
    }
}

#[tokio::test]
async fn my_work_without_a_pot_is_not_a_briefing() {
    let w = world().await;
    let c1 = w.cook("c1").await;
    ok(&w.app.mcp(&c1, "my_work", json!({})).await, "my_work");
    let got = ok(
        &w.app.mcp(&c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    assert!(got["catch_up"].is_object(), "{got}");
}

#[tokio::test]
async fn the_catch_up_says_what_waits() {
    let w = world().await;
    let c1 = w.cook("c1").await;
    // c1 was here once, and was briefed
    ok(
        &w.app.mcp(&c1, "what_now", json!({"project": "PAY"})).await,
        "what_now",
    );
    sqlx::query(
        "update principals set last_seen_at = now() - interval '3 hours' where name = 'c1'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    // meanwhile: a direct message, a question to it, a convention, and a new task
    ok(
        &w.app
            .mcp(
                &w.root,
                "pass",
                json!({"project": "PAY", "message": "please look at PAY-2 first", "to": "c1"}),
            )
            .await,
        "pass",
    );
    ok(
        &w.app
            .mcp(
                &w.root,
                "ask",
                json!({"project": "PAY", "title": "Which PSP first?", "to": "c1"}),
            )
            .await,
        "ask",
    );
    ok(
        &w.app
            .mcp(&w.root, "remember", json!({"project": "PAY", "title": "Never deploy on Fridays", "body": "Freeze from Friday noon.", "kind": "convention"}))
            .await,
        "remember",
    );
    w.app
        .post(&w.root, "/projects/PAY/tasks", json!({"title": "four"}))
        .await;

    let back = ok(
        &w.app
            .mcp(&c1, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    let cu = &back["catch_up"];
    assert!(cu.is_object(), "{back}");
    assert!(cu["unread_pass"].as_i64().unwrap() >= 1, "{cu}");
    let qs = cu["waiting_on_you"]["questions"].as_array().unwrap();
    assert_eq!(qs.len(), 1, "{cu}");
    assert_eq!(qs[0]["title"], "Which PSP first?");
    assert!(cu["pantry_news"].as_i64().unwrap() >= 1, "{cu}");
    let mem = cu["memories"].as_array().unwrap();
    assert!(
        mem.iter().any(|m| m["key"] == "never-deploy-on-fridays"),
        "{cu}"
    );
    let changes = &cu["changes"];
    assert!(changes["total"].as_i64().unwrap() >= 3, "{changes}");
    let recent = changes["recent"].as_array().unwrap();
    assert!(!recent.is_empty() && recent.len() <= 10, "{changes}");
    assert!(
        recent
            .iter()
            .any(|l| l.as_str().unwrap().contains("task.created")),
        "{changes}"
    );
}

#[tokio::test]
async fn people_are_never_gated() {
    let w = world().await;
    // the admin signs in with a token like a cook does, with no history: no catch-up
    let got = ok(
        &w.app
            .mcp(&w.root, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(got["task"]["id"], "PAY-1", "{got}");
    ok(
        &w.app
            .mcp(
                &w.root,
                "complete",
                json!({"id": "PAY-1", "epoch": got["lease"]["epoch"], "result": "x"}),
            )
            .await,
        "complete",
    );
    sqlx::query(
        "update principals set last_seen_at = now() - interval '5 hours' where name = 'octo'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    let got = ok(
        &w.app
            .mcp(&w.root, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim",
    );
    assert_eq!(got["task"]["id"], "PAY-2", "{got}");
}

#[test]
fn what_now_comes_first_in_every_prompt() {
    assert!(corntchap::cloud::CLOUD_CONTRACT.contains("(0) Start with what_now"));
    assert!(
        corntchap::cloud::CLOUD_CONTRACT.find("what_now").unwrap()
            < corntchap::cloud::CLOUD_CONTRACT.find("claim_next").unwrap()
    );
}
