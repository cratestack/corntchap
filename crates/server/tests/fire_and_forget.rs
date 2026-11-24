//! A cook with cloud wake targets only is fire and forget: ready work and its own owner's direct
//! message start it, nothing else does. The calls that would reach it are refused or say it was
//! not woken, and the cook's nature shows in the team and cook listings.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// The error text of a refused call.
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should be refused: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

struct World {
    app: common::TestApp,
    root: String,
    alice: String,
    carol: String,
    rita: String,
    bob: String,
}

/// alice owns `rita` (a cloud cook: a Claude routine target) and `bob` (a local cook); carol is
/// another person of the team.
async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "carol"] {
        app.add_user(u).await;
    }
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    for u in ["alice", "carol"] {
        app.post(&root, "/teams/acme/members", json!({"principal": u}))
            .await;
    }
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let carol = admin::operator_token(&app.state.db, "carol", None)
        .await
        .unwrap();
    let mut toks = vec![];
    for (name, runtime) in [("rita", "cloud"), ("bob", "local")] {
        let (_, c) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "claude-code", "runtime": runtime}),
            )
            .await;
        toks.push(c["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    sqlx::query("insert into wake_targets (agent_id, type, projects, labels, config) select id, 'claude-routine', '{}', '{}', '{}' from principals where name = 'rita'")
        .execute(&app.state.db)
        .await
        .unwrap();
    World {
        app,
        root,
        alice,
        carol,
        rita: toks[0].clone(),
        bob: toks[1].clone(),
    }
}

/// A cook of alice with one claude-routine target serving `projects` (empty: every pot).
async fn cloud_cook(w: &World, name: &str, projects: &str) {
    w.app
        .post(
            &w.alice,
            "/agents",
            json!({"name": name, "kind": "claude-code", "runtime": "cloud"}),
        )
        .await;
    w.app
        .post(&w.root, "/teams/acme/members", json!({"principal": name}))
        .await;
    sqlx::query(
        "insert into wake_targets (agent_id, type, projects, labels, config) select id, 'claude-routine', $2::text[], '{}', '{}' from principals where name = $1",
    )
    .bind(name)
    .bind(projects)
    .execute(&w.app.state.db)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_fire_and_forget_cook_cannot_speak_for_its_owner() {
    let w = world().await;
    cloud_cook(&w, "sam", "{}").await;
    // rita (fire and forget) posts as alice to sam (fire and forget): refused, nobody started
    let r = w
        .app
        .mcp(
            &w.rita,
            "pass",
            json!({"project": "PAY", "message": "go", "to": "sam", "fromOwner": true}),
        )
        .await;
    let e = refused(&r, "relay by a fire-and-forget cook");
    assert!(e.contains("sam is fire-and-forget"), "{e}");
    assert_eq!(wakes_for(&w.app, "sam").await, 0);
    // bob (a local cook of alice) relaying alice's words is allowed
    let sent = ok(
        &w.app
            .mcp(
                &w.bob,
                "pass",
                json!({"project": "PAY", "message": "go", "to": "sam", "fromOwner": true}),
            )
            .await,
        "relay by a local cook",
    );
    assert_eq!(sent["woken"][0]["cloud"], true, "{sent}");
    assert_eq!(wakes_for(&w.app, "sam").await, 1);
}

#[tokio::test]
async fn an_owner_message_in_a_pot_no_target_serves_says_so() {
    let w = world().await;
    cloud_cook(&w, "tess", "{OTHER}").await;
    let sent = ok(
        &w.app
            .mcp(
                &w.alice,
                "pass",
                json!({"project": "PAY", "message": "hello", "to": "tess"}),
            )
            .await,
        "owner dm",
    );
    let note = sent["note"].as_str().unwrap();
    assert!(
        note.contains("tess is fire-and-forget but no cloud target serves PAY")
            && !note.contains("starts a fresh run"),
        "{note}"
    );
    assert_eq!(wakes_for(&w.app, "tess").await, 0);
}

async fn wakes_for(app: &common::TestApp, agent: &str) -> i64 {
    sqlx::query_scalar("select count(*) from events where kind = 'wake' and payload->>'agent' = $1")
        .bind(agent)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn only_the_owner_can_message_a_fire_and_forget_cook() {
    let w = world().await;
    // the owner's direct message starts a run, and the answer says what that means
    let sent = ok(
        &w.app
            .mcp(
                &w.alice,
                "pass",
                json!({"project": "PAY", "message": "how far?", "to": "rita"}),
            )
            .await,
        "owner dm",
    );
    assert_eq!(sent["woken"][0]["cloud"], true, "{sent}");
    assert!(
        sent["note"]
            .as_str()
            .unwrap()
            .contains("rita is fire-and-forget"),
        "{sent}"
    );
    assert_eq!(wakes_for(&w.app, "rita").await, 1);
    // another person's message is refused, and so is an agent's: nothing is stored or woken
    let before: i64 = sqlx::query_scalar("select count(*) from pass_messages")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    for (who, tok) in [("carol", &w.carol), ("bob", &w.bob)] {
        let r = w
            .app
            .mcp(
                tok,
                "pass",
                json!({"project": "PAY", "message": "hello", "to": "rita"}),
            )
            .await;
        let e = refused(&r, who);
        assert!(
            e.contains("rita is fire-and-forget: it only takes ready work; give it a task"),
            "{e}"
        );
    }
    let after: i64 = sqlx::query_scalar("select count(*) from pass_messages")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(wakes_for(&w.app, "rita").await, 1);
    // a local cook still takes anyone's message
    let sent = ok(
        &w.app
            .mcp(
                &w.carol,
                "pass",
                json!({"project": "PAY", "message": "hi bob", "to": "bob"}),
            )
            .await,
        "dm bob",
    );
    assert!(
        sent["woken"][0]["cloud"].is_null() && sent["note"].is_null(),
        "{sent}"
    );
}

#[tokio::test]
async fn a_mention_or_a_comment_wakes_it_not_and_says_so() {
    let w = world().await;
    let sent = ok(
        &w.app
            .mcp(
                &w.alice,
                "pass",
                json!({"project": "PAY", "message": "@rita can you look at refunds?"}),
            )
            .await,
        "mention",
    );
    assert_eq!(sent["woken"], json!([]), "{sent}");
    assert!(
        sent["note"]
            .as_str()
            .unwrap()
            .contains("rita is fire-and-forget and was not woken"),
        "{sent}"
    );
    // rita holds a task: comments (a person's, an agent's, a mention) wake nobody, nag nobody
    ok(
        &w.app
            .mcp(
                &w.alice,
                "create_task",
                json!({"project": "PAY", "title": "Refund screen"}),
            )
            .await,
        "create",
    );
    let c = ok(
        &w.app
            .mcp(&w.rita, "claim_task", json!({"id": "PAY-1"}))
            .await,
        "claim",
    );
    assert!(c["lease"]["epoch"].is_number());
    for (tok, body) in [
        (&w.alice, "any news?"),
        (&w.carol, "@rita ping"),
        (&w.bob, "I looked too"),
    ] {
        let r = ok(
            &w.app
                .mcp(tok, "comment", json!({"id": "PAY-1", "body": body}))
                .await,
            "comment",
        );
        assert_eq!(r["woken"], json!([]), "{r}");
        assert!(
            r["note"]
                .as_str()
                .unwrap()
                .contains("rita is fire-and-forget and was not woken"),
            "{r}"
        );
    }
    assert_eq!(wakes_for(&w.app, "rita").await, 0);
    let replies: i64 = sqlx::query_scalar("select count(*) from comment_replies")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(replies, 0, "nothing to nudge about");
    assert_eq!(
        corntchap::jobs::nudge_unanswered(&w.app.state)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn ask_and_review_refuse_a_fire_and_forget_cook() {
    let w = world().await;
    let e = refused(
        &w.app
            .mcp(
                &w.bob,
                "ask",
                json!({"project": "PAY", "question": "Cents or units?", "to": "rita"}),
            )
            .await,
        "ask to",
    );
    assert!(
        e.contains("rita is fire-and-forget: it only takes ready work"),
        "{e}"
    );

    ok(
        &w.app
            .mcp(
                &w.alice,
                "create_task",
                json!({"project": "PAY", "title": "One"}),
            )
            .await,
        "create",
    );
    ok(
        &w.app
            .mcp(&w.rita, "claim_task", json!({"id": "PAY-1"}))
            .await,
        "claim",
    );
    let e = refused(
        &w.app
            .mcp(
                &w.rita,
                "ask",
                json!({"project": "PAY", "task": "PAY-1", "question": "Cents or units?", "wait": true}),
            )
            .await,
        "ask wait",
    );
    assert!(
        e.contains("fire-and-forget") && e.contains("Block the task instead"),
        "{e}"
    );
    // asking without waiting is fine: the cook says what it needs and releases the task
    ok(
        &w.app
            .mcp(
                &w.rita,
                "ask",
                json!({"project": "PAY", "task": "PAY-1", "question": "Cents or units?"}),
            )
            .await,
        "ask no wait",
    );

    // a review cannot be assigned to it
    ok(
        &w.app
            .mcp(
                &w.alice,
                "create_task",
                json!({"project": "PAY", "title": "Two"}),
            )
            .await,
        "create 2",
    );
    ok(
        &w.app
            .mcp(&w.bob, "claim_task", json!({"id": "PAY-2"}))
            .await,
        "claim 2",
    );
    let e = refused(
        &w.app
            .mcp(
                &w.bob,
                "request_review",
                json!({"task": "PAY-2", "reviewer": "rita", "note": "look"}),
            )
            .await,
        "review",
    );
    assert!(
        e.contains("rita is fire-and-forget: it only takes ready work"),
        "{e}"
    );
    assert_eq!(wakes_for(&w.app, "rita").await, 0);
}

#[tokio::test]
async fn the_listings_show_fire_and_forget() {
    let w = world().await;
    let members = ok(
        &w.app
            .mcp(&w.alice, "list_team_members", json!({"team": "acme"}))
            .await,
        "members",
    );
    let flag = |name: &str| {
        members
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing in {members}"))["fireAndForget"]
            .clone()
    };
    assert_eq!(flag("rita"), true, "{members}");
    assert_eq!(flag("bob"), false);
    assert_eq!(flag("alice"), false);
    let cooks = ok(&w.app.mcp(&w.alice, "list_cooks", json!({})).await, "cooks");
    let by = |name: &str| {
        cooks
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap()["fireAndForget"]
            .clone()
    };
    assert_eq!(by("rita"), true, "{cooks}");
    assert_eq!(by("bob"), false);
    // a relay target next to the cloud one makes it present again
    sqlx::query("insert into wake_targets (agent_id, type, projects, labels, config) select id, 'relay', '{}', '{}', '{}' from principals where name = 'rita'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let cooks = ok(&w.app.mcp(&w.alice, "list_cooks", json!({})).await, "cooks");
    let rita = cooks
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "rita")
        .unwrap();
    assert_eq!(rita["fireAndForget"], false, "{rita}");
}

#[tokio::test]
async fn the_migration_strips_on_call() {
    let w = world().await;
    sqlx::query("update wake_targets set config = '{\"url\": \"http://x\", \"on_call\": true}'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0034_drop_on_call.sql"))
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let c: Value = sqlx::query_scalar("select config from wake_targets")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(c, json!({"url": "http://x"}));
}
