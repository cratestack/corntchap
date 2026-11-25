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
    )["items"]
        .clone();
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
    let cooks = ok(&w.app.mcp(&w.alice, "list_cooks", json!({})).await, "cooks")["items"].clone();
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
    let cooks = ok(&w.app.mcp(&w.alice, "list_cooks", json!({})).await, "cooks")["items"].clone();
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

async fn id_of(app: &common::TestApp, name: &str) -> uuid::Uuid {
    sqlx::query_scalar("select id from principals where name = $1")
        .bind(name)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

async fn wake(
    conn: &mut sqlx::PgConnection,
    project: uuid::Uuid,
    cook: uuid::Uuid,
    from: Option<uuid::Uuid>,
    reason: &str,
) -> Value {
    corntchap::questions::wake_from(conn, project, "PAY", cook, None, None, "look", reason, from)
        .await
        .unwrap()
}

/// The restaurant's `wake.withheld` events for one reason: (actor, text, repeated).
async fn withheld(app: &common::TestApp, reason: &str) -> Vec<(String, String, Option<i32>)> {
    sqlx::query_as(
        "select a.name, e.payload->>'text', (e.payload->>'repeated')::int
         from events e join principals a on a.id = e.actor_id
         where e.kind = 'restaurant.decided' and e.payload->>'decision' = 'wake.withheld' and e.payload->>'reason' = $1
         order by e.seq",
    )
    .bind(reason)
    .fetch_all(&app.state.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_wake_the_server_withholds_is_a_decision_it_says_once_in_a_while() {
    let w = world().await;
    let (rita, alice) = (id_of(&w.app, "rita").await, id_of(&w.app, "alice").await);
    let project: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    let mut conn = w.app.state.db.acquire().await.unwrap();
    // a forge wake for a fire-and-forget cook: nothing is sent, and the restaurant says why
    let r = wake(&mut conn, project, rita, None, "forge").await;
    assert_eq!(r["woken"], false, "{r}");
    let rows = withheld(&w.app, "fire_and_forget").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "restaurant");
    assert!(
        rows[0].1.contains("rita") && rows[0].1.contains("fire-and-forget"),
        "{rows:?}"
    );
    assert_eq!(wakes_for(&w.app, "rita").await, 0);
    // the same again within the window: only counted
    wake(&mut conn, project, rita, None, "forge").await;
    wake(&mut conn, project, rita, None, "forge").await;
    assert_eq!(withheld(&w.app, "fire_and_forget").await.len(), 1);
    let suppressed: i32 = sqlx::query_scalar(
        "select suppressed from restaurant_quiet where key like 'wake.withheld:fire_and_forget:%'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(suppressed, 2);
    // after the window the next one carries the count
    sqlx::query("update restaurant_quiet set last_at = now() - interval '31 minutes'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    wake(&mut conn, project, rita, None, "forge").await;
    let rows = withheld(&w.app, "fire_and_forget").await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].2, Some(2), "{rows:?}");

    // its owner's message to a cook no cloud target serves in this pot
    cloud_cook(&w, "tess", "{OTHER}").await;
    let tess = id_of(&w.app, "tess").await;
    let r = wake(&mut conn, project, tess, Some(alice), "pass").await;
    assert_eq!(r["uncovered"], true, "{r}");
    let rows = withheld(&w.app, "uncovered").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].1.contains("tess") && rows[0].1.contains("PAY"),
        "{rows:?}"
    );
    // the cook is told by what_now: its latest decision holds it back
    let now = ok(
        &w.app
            .mcp(&w.rita, "what_now", json!({"project": "PAY"}))
            .await,
        "what_now",
    );
    assert!(
        now["restaurant"]
            .as_str()
            .unwrap_or("")
            .contains("fire-and-forget"),
        "{now}"
    );
}

#[tokio::test]
async fn decisions_show_in_the_feed_and_are_kept_a_month_and_never_rolled_up() {
    let w = world().await;
    let rita = id_of(&w.app, "rita").await;
    let project: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    let mut conn = w.app.state.db.acquire().await.unwrap();
    wake(&mut conn, project, rita, None, "forge").await;
    drop(conn);
    // changes_since lists it, with the restaurant as its actor and kind
    let feed = ok(
        &w.app
            .mcp(
                &w.alice,
                "changes_since",
                json!({"cursor": 0, "project": "PAY"}),
            )
            .await,
        "changes_since",
    );
    let e = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "restaurant.decided")
        .unwrap_or_else(|| panic!("{feed}"));
    assert_eq!(
        (e["actor"].as_str(), e["actor_kind"].as_str()),
        (Some("restaurant"), Some("restaurant"))
    );
    assert_eq!(e["payload"]["decision"], "wake.withheld", "{e}");
    // the digest and the catch-up count work, not decisions
    let (code, digest) = w.app.get(&w.alice, "/projects/PAY/digest?since=0").await;
    assert_eq!(code, 200, "{digest}");
    assert!(digest["counts"]["restaurant.decided"].is_null(), "{digest}");
    // a month on, it goes; other events are kept to 90 days and are rolled into daily counts, this one is not
    sqlx::query(
        "update events set at = now() - interval '31 days' where kind = 'restaurant.decided'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    sqlx::query("update restaurant_quiet set last_at = now() - interval '3 days'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    corntchap::jobs::retain(&w.app.state).await.unwrap();
    let left: i64 =
        sqlx::query_scalar("select count(*) from events where kind = 'restaurant.decided'")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(left, 0);
    let rolled: i64 = sqlx::query_scalar(
        "select count(*) from event_daily_counts where kind = 'restaurant.decided'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(rolled, 0);
    let quiet: i64 = sqlx::query_scalar("select count(*) from restaurant_quiet")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(quiet, 0, "the quiet rows are pruned");
    common::assert_no_anonymous_events(&w.app.state.db).await;
}

#[tokio::test]
async fn the_cook_page_reads_what_the_restaurant_did_about_it_a_page_at_a_time() {
    let w = world().await;
    let rita = id_of(&w.app, "rita").await;
    let project: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    // five decisions about rita (the quiet window is 0 for the test: each says something), one about bob
    let mut conn = w.app.state.db.acquire().await.unwrap();
    for i in 0..5 {
        sqlx::query("delete from restaurant_quiet")
            .execute(&mut *conn)
            .await
            .unwrap();
        wake(&mut conn, project, rita, None, &format!("forge{i}")).await;
    }
    drop(conn);
    let proc = |args: Value| {
        let (app, tok) = (&w.app, w.alice.clone());
        async move {
            let r = app
                .http
                .post(format!("{}/api/v2/$procs/cookDecisions", app.base))
                .bearer_auth(tok)
                .json(&args)
                .send()
                .await
                .unwrap();
            let st = r.status().as_u16();
            (st, r.json::<Value>().await.unwrap_or(Value::Null))
        }
    };
    let (st, first) = proc(json!({"agent": "rita", "page": {"limit": 2}})).await;
    assert_eq!(st, 200, "{first}");
    let items = first["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{first}");
    assert!(
        items
            .iter()
            .all(|e| e["actor"] == "restaurant" && e["actor_kind"] == "restaurant"),
        "{first}"
    );
    assert!(
        items[0]["seq"].as_i64() > items[1]["seq"].as_i64(),
        "newest first"
    );
    // the rest, by cursor
    let mut seen = items.len();
    let mut cursor = first["nextCursor"].as_str().map(String::from);
    while let Some(c) = cursor {
        let (st, v) = proc(json!({"agent": "rita", "page": {"limit": 2, "cursor": c}})).await;
        assert_eq!(st, 200, "{v}");
        seen += v["items"].as_array().unwrap().len();
        cursor = v["nextCursor"].as_str().map(String::from);
    }
    assert_eq!(seen, 5);
    // another cook has none, and an unknown one is refused
    let (st, none) = proc(json!({"agent": "bob", "page": {}})).await;
    assert_eq!(
        (st, none["items"].as_array().map(|a| a.len())),
        (200, Some(0)),
        "{none}"
    );
    let (st, _) = proc(json!({"agent": "nobody", "page": {}})).await;
    assert_eq!(st, 404);
    // a forged cursor is a 400, not a 500
    let (st, _) = proc(json!({"agent": "rita", "page": {"cursor": "garbage"}})).await;
    assert_eq!(st, 400);
}
