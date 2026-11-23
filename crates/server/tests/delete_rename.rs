//! Renaming and deleting cooks, fires, pots and teams: who may, what survives, what goes.
mod common;
use common::TestApp;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    assert!(r.is_object(), "{what}: {r}");
    TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should be refused: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

struct World {
    app: TestApp,
    root: String,
    alice: String,
    bob: String,
    c1: String,
}

/// octo (admin), alice (owns team acme and the pot PAY, owns cook c1 that is in acme), bob (owns team other).
async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
    app.add_user("bob").await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    for (team, owner) in [("acme", "alice"), ("other", "bob")] {
        ok(
            &app.mcp(&root, "create_team", json!({"name": team})).await,
            "create_team",
        );
        ok(
            &app.mcp(
                &root,
                "add_team_member",
                json!({"team": team, "principal": owner, "role": "owner"}),
            )
            .await,
            "owner",
        );
    }
    ok(
        &app.mcp(
            &alice,
            "create_project",
            json!({"team": "acme", "key": "pay", "name": "Payments"}),
        )
        .await,
        "create_project",
    );
    let c1 = ok(
        &app.mcp(&alice, "create_cook", json!({"name": "c1"})).await,
        "create_cook",
    )["token"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &app.mcp(
            &alice,
            "add_team_member",
            json!({"team": "acme", "principal": "c1"}),
        )
        .await,
        "c1 joins",
    );
    World {
        app,
        root,
        alice,
        bob,
        c1,
    }
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_cook_can_be_renamed_and_keeps_working() {
    let w = world().await;
    let app = &w.app;
    app.post(&w.alice, "/projects/PAY/tasks", json!({"title": "refunds"}))
        .await;
    let claim = ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();

    // another person cannot; a bad or taken name is refused
    let e = refused(
        &app.mcp(
            &w.bob,
            "rename_cook",
            json!({"name": "c1", "newName": "x1"}),
        )
        .await,
        "bob renames alice's cook",
    );
    assert!(e.contains("no agent or relay named c1"), "{e}");
    let e = refused(
        &app.mcp(
            &w.alice,
            "rename_cook",
            json!({"name": "c1", "newName": "st_bad"}),
        )
        .await,
        "reserved prefix",
    );
    assert!(e.contains("handles"), "{e}");
    let e = refused(
        &app.mcp(
            &w.alice,
            "rename_cook",
            json!({"name": "c1", "newName": "bob"}),
        )
        .await,
        "a person's handle",
    );
    assert!(e.contains("not available"), "{e}");
    ok(
        &app.mcp(&w.alice, "create_cook", json!({"name": "c2"}))
            .await,
        "c2",
    );
    let e = refused(
        &app.mcp(
            &w.alice,
            "rename_cook",
            json!({"name": "c1", "newName": "c2"}),
        )
        .await,
        "taken",
    );
    assert!(e.contains("already"), "{e}");
    // a fire is not a cook
    ok(
        &app.mcp(&w.alice, "create_fire", json!({"name": "f1"}))
            .await,
        "fire",
    );
    refused(
        &app.mcp(
            &w.alice,
            "rename_cook",
            json!({"name": "f1", "newName": "f9"}),
        )
        .await,
        "rename_cook on a fire",
    );

    // its owner renames it; the same token works, whoami shows the new name, the lease is still its own
    let r = ok(
        &app.mcp(
            &w.alice,
            "rename_cook",
            json!({"name": "c1", "newName": "Sous-Chef"}),
        )
        .await,
        "rename",
    );
    assert_eq!(
        (&r["name"], &r["previous"]),
        (&json!("sous-chef"), &json!("c1"))
    );
    let me = ok(&app.mcp(&w.c1, "whoami", json!({})).await, "whoami");
    assert_eq!(me["name"], "sous-chef");
    assert_eq!(me["teams"][0]["name"], "acme");
    assert_eq!(me["current_leases"][0]["id"], "PAY-1");
    ok(
        &app.mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
            .await,
        "heartbeat on the old lease",
    );
    let cooks = ok(&app.mcp(&w.alice, "list_cooks", json!({})).await, "list");
    let names: Vec<_> = cooks
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["c2", "sous-chef"]);
    // an admin may rename anybody's cook
    ok(
        &app.mcp(
            &w.root,
            "rename_cook",
            json!({"name": "sous-chef", "newName": "c1"}),
        )
        .await,
        "admin rename",
    );
}

#[tokio::test]
async fn deleting_a_cook_releases_its_leases_and_keeps_history() {
    let w = world().await;
    let app = &w.app;
    app.post(
        &w.alice,
        "/projects/PAY/tasks",
        json!({"title": "refunds", "items": ["a", "b"]}),
    )
    .await;
    let claim = ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    ok(
        &app.mcp(
            &w.c1,
            "comment",
            json!({"id": "PAY-1", "body": "on it", "epoch": epoch}),
        )
        .await,
        "comment",
    );
    ok(
        &app.mcp(
            &w.c1,
            "check_item",
            json!({"id": "PAY-1", "position": 1, "done": true, "epoch": epoch}),
        )
        .await,
        "check",
    );
    ok(
        &app.mcp(
            &w.c1,
            "register_station",
            json!({"project": "PAY", "runtime": "opencode", "session": "ses_a"}),
        )
        .await,
        "station",
    );
    ok(
        &app.mcp(
            &w.alice,
            "add_wake_target",
            json!({"cook": "c1", "type": "relay"}),
        )
        .await,
        "wake target",
    );
    assert_eq!(count(app, "select count(*) from stations").await, 1);

    // somebody else cannot; a cook cannot delete itself, even one granted `manage`
    let e = refused(
        &app.mcp(&w.bob, "delete_cook", json!({"name": "c1"})).await,
        "bob",
    );
    assert!(e.contains("no agent or relay named c1"), "{e}");
    let e = refused(
        &app.mcp(&w.c1, "delete_cook", json!({"name": "c1"})).await,
        "cook without manage",
    );
    assert!(e.contains("manage"), "{e}");
    ok(
        &app.mcp(
            &w.alice,
            "set_grants",
            json!({"name": "c1", "grants": "manage"}),
        )
        .await,
        "grant",
    );
    let e = refused(
        &app.mcp(&w.c1, "delete_cook", json!({"name": "c1"})).await,
        "itself",
    );
    assert!(e.contains("cannot delete itself"), "{e}");
    assert_eq!(
        count(app, "select count(*) from principals where name = 'c1'").await,
        1
    );

    // its owner deletes it
    let r = ok(
        &app.mcp(&w.alice, "delete_cook", json!({"name": "c1"}))
            .await,
        "delete",
    );
    assert_eq!((&r["name"], &r["released"]), (&json!("c1"), &json!(1)));
    assert_eq!(
        count(app, "select count(*) from principals where name = 'c1'").await,
        0
    );
    assert_eq!(count(app, "select count(*) from stations").await, 0);
    assert_eq!(count(app, "select count(*) from wake_targets").await, 0);
    assert_eq!(app.get(&w.c1, "/me").await.0, 401, "its token is gone");

    // the task is open again and can be claimed anew; nothing it wrote is gone
    let (status, by): (String, Option<uuid::Uuid>) =
        sqlx::query_as("select status, claimed_by from tasks where number = 1")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), by), ("open", None));
    assert_eq!(
        count(
            app,
            "select count(*) from items where done and done_by is null"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            app,
            "select count(*) from tasks where created_by is not null"
        )
        .await,
        1,
        "alice's task keeps its author; only c1's marks went null"
    );
    assert_eq!(
        count(
            app,
            "select count(*) from comments where author_id is null and body = 'on it'"
        )
        .await,
        1
    );
    assert!(
        count(
            app,
            "select count(*) from events where payload->>'actor_name' = 'c1'"
        )
        .await
            > 0,
        "its events keep its name"
    );
    assert_eq!(
        count(app, "select count(*) from events where kind = 'task.released' and payload->>'reason' like '%deleted%'").await,
        1
    );
    ok(
        &app.mcp(&w.alice, "create_cook", json!({"name": "c1"}))
            .await,
        "the name is free again",
    );
}

#[tokio::test]
async fn a_cook_with_manage_deletes_a_sibling_but_a_person_cannot_be_deleted() {
    let w = world().await;
    let app = &w.app;
    ok(
        &app.mcp(&w.alice, "create_cook", json!({"name": "c2"}))
            .await,
        "c2",
    );
    ok(
        &app.mcp(
            &w.alice,
            "set_grants",
            json!({"name": "c1", "grants": "manage"}),
        )
        .await,
        "grant",
    );
    ok(
        &app.mcp(&w.c1, "delete_cook", json!({"name": "c2"})).await,
        "c1 acts as alice",
    );
    refused(
        &app.mcp(&w.root, "delete_cook", json!({"name": "alice"}))
            .await,
        "a person is not a cook",
    );
    refused(
        &app.mcp(&w.root, "delete_fire", json!({"name": "c1"})).await,
        "a cook is not a fire",
    );
}

#[tokio::test]
async fn fires_rename_and_delete() {
    let w = world().await;
    let app = &w.app;
    let f = ok(
        &app.mcp(&w.alice, "create_fire", json!({"name": "laptop"}))
            .await,
        "fire",
    );
    let ft = f["token"].as_str().unwrap().to_string();
    assert_eq!(app.get(&ft, "/me").await.0, 200);
    refused(
        &app.mcp(
            &w.bob,
            "rename_fire",
            json!({"name": "laptop", "newName": "x"}),
        )
        .await,
        "bob",
    );
    refused(
        &app.mcp(&w.bob, "delete_fire", json!({"name": "laptop"}))
            .await,
        "bob",
    );
    let r = ok(
        &app.mcp(
            &w.alice,
            "rename_fire",
            json!({"name": "laptop", "newName": "desk"}),
        )
        .await,
        "rename",
    );
    assert_eq!(r["name"], "desk");
    assert_eq!(app.get(&ft, "/me").await.1["name"], "desk");
    let fires = ok(&app.mcp(&w.alice, "list_fires", json!({})).await, "list");
    assert_eq!(fires[0]["name"], "desk");
    ok(
        &app.mcp(&w.alice, "delete_fire", json!({"name": "desk"}))
            .await,
        "delete",
    );
    assert_eq!(app.get(&ft, "/me").await.0, 401);
    assert_eq!(
        ok(&app.mcp(&w.alice, "list_fires", json!({})).await, "list"),
        json!([])
    );
    // the cook is untouched
    assert_eq!(app.get(&w.c1, "/me").await.0, 200);
}

#[tokio::test]
async fn pots_rename_and_delete() {
    let w = world().await;
    let app = &w.app;
    for t in ["refunds", "receipts"] {
        app.post(&w.alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    ok(
        &app.mcp(&w.alice, "comment", json!({"id": "PAY-2", "body": "hi"}))
            .await,
        "comment",
    );
    app.post(
        &w.alice,
        "/projects/PAY/pantry",
        json!({"key": "style", "body": "tabs"}),
    )
    .await;
    ok(
        &app.mcp(
            &w.c1,
            "register_station",
            json!({"project": "PAY", "runtime": "opencode", "session": "s1"}),
        )
        .await,
        "station",
    );
    ok(
        &app.mcp(
            &w.alice,
            "pass",
            json!({"project": "PAY", "message": "hello"}),
        )
        .await,
        "pass",
    );
    sqlx::query(
        "insert into files (id, project_id, number, name, content_type, storage_key, status, upload_expires_at)
         select gen_random_uuid(), id, 1, 'a.txt', 'text/plain', 'pay/test-delete-key', 'ready', now() from projects where key = 'PAY'",
    )
    .execute(&app.state.db)
    .await
    .unwrap();
    // with storage configured, the object really exists and must go with the pot
    if let Some(st) = &app.state.storage {
        st.put(
            "pay/test-delete-key",
            "text/plain",
            2,
            reqwest::Body::from("hi"),
        )
        .await
        .unwrap();
        assert_eq!(st.size_of("pay/test-delete-key").await.unwrap(), Some(2));
    }
    // a second pot elsewhere must survive, and a dependency across pots must not block the delete
    ok(
        &app.mcp(
            &w.bob,
            "create_project",
            json!({"team": "other", "key": "OPS", "name": "Ops"}),
        )
        .await,
        "OPS",
    );
    app.post(&w.bob, "/projects/OPS/tasks", json!({"title": "keep me"}))
        .await;
    sqlx::query(
        "insert into task_deps (task_id, depends_on)
         select (select t.id from tasks t join projects p on p.id = t.project_id where p.key = 'OPS'),
                (select t.id from tasks t join projects p on p.id = t.project_id where p.key = 'PAY' and t.number = 1)",
    )
    .execute(&app.state.db)
    .await
    .unwrap();

    // rename: the key stays, the name changes; others cannot
    refused(
        &app.mcp(
            &w.bob,
            "rename_project",
            json!({"key": "PAY", "name": "Hax"}),
        )
        .await,
        "bob",
    );
    let r = ok(
        &app.mcp(
            &w.alice,
            "rename_project",
            json!({"key": "pay", "name": "Payments v2"}),
        )
        .await,
        "rename",
    );
    assert_eq!(
        (&r["key"], &r["name"], &r["previous"]),
        (&json!("PAY"), &json!("Payments v2"), &json!("Payments"))
    );
    let pots = ok(&app.mcp(&w.alice, "list_projects", json!({})).await, "list");
    assert_eq!(pots[0]["name"], "Payments v2");
    assert_eq!(pots[0]["key"], "PAY");
    refused(
        &app.mcp(
            &w.alice,
            "rename_project",
            json!({"key": "PAY", "name": "  "}),
        )
        .await,
        "blank",
    );

    // delete: wrong confirm, wrong person, then right
    let e = refused(
        &app.mcp(
            &w.alice,
            "delete_project",
            json!({"key": "PAY", "confirm": "pay2"}),
        )
        .await,
        "wrong confirm",
    );
    assert!(e.contains("confirm"), "{e}");
    refused(
        &app.mcp(
            &w.bob,
            "delete_project",
            json!({"key": "PAY", "confirm": "PAY"}),
        )
        .await,
        "bob",
    );
    refused(
        &app.mcp(
            &w.c1,
            "delete_project",
            json!({"key": "PAY", "confirm": "PAY"}),
        )
        .await,
        "a cook without manage",
    );
    assert_eq!(count(&w.app, "select count(*) from tasks").await, 3);
    let r = ok(
        &app.mcp(
            &w.alice,
            "delete_project",
            json!({"key": "PAY", "confirm": "PAY"}),
        )
        .await,
        "delete",
    );
    assert_eq!((&r["key"], &r["tasks"]), (&json!("PAY"), &json!(2)));
    // the one stored object is removed (or reported as left behind when no storage is configured)
    assert_eq!(
        r["files"].as_i64().unwrap() + r["files_left"].as_i64().unwrap(),
        1,
        "{r}"
    );
    assert_eq!(
        count(app, "select count(*) from projects where key = 'PAY'").await,
        0
    );
    if let Some(st) = &app.state.storage {
        assert_eq!(
            st.size_of("pay/test-delete-key").await.unwrap(),
            None,
            "the object is gone"
        );
        assert_eq!((&r["files"], &r["files_left"]), (&json!(1), &json!(0)));
    }
    for t in ["comments", "memories", "stations", "pass_messages", "files"] {
        assert_eq!(
            count(app, &format!("select count(*) from {t}")).await,
            0,
            "{t}"
        );
    }
    // nothing of PAY is left anywhere, not even orphaned
    assert_eq!(
        count(app, "select count(*) from events e where e.project_id is not null and not exists (select 1 from projects p where p.id = e.project_id)").await,
        0
    );
    assert_eq!(
        count(app, "select count(*) from comments c where not exists (select 1 from tasks t where t.id = c.task_id)").await,
        0
    );
    refused(
        &app.mcp(&w.alice, "get_task", json!({"id": "PAY-1"})).await,
        "gone",
    );
    // the other pot, the cook and the team are fine
    assert_eq!(count(app, "select count(*) from tasks").await, 1);
    assert_eq!(count(app, "select count(*) from task_deps").await, 0);
    assert_eq!(app.get(&w.c1, "/me").await.0, 200);
    // the key is free again
    ok(
        &app.mcp(
            &w.alice,
            "create_project",
            json!({"team": "acme", "key": "PAY", "name": "Again"}),
        )
        .await,
        "recreate",
    );
}

#[tokio::test]
async fn teams_rename_and_delete() {
    let w = world().await;
    let app = &w.app;
    // not a member: it does not even exist for bob; a plain member is forbidden
    let e = refused(
        &app.mcp(
            &w.bob,
            "rename_team",
            json!({"name": "acme", "newName": "x"}),
        )
        .await,
        "bob",
    );
    assert!(e.contains("not found"), "{e}");
    let e = refused(
        &app.mcp(
            &w.c1,
            "rename_team",
            json!({"name": "acme", "newName": "x"}),
        )
        .await,
        "a cook",
    );
    assert!(e.contains("manage") || e.contains("owner"), "{e}");
    let e = refused(
        &app.mcp(
            &w.alice,
            "rename_team",
            json!({"name": "acme", "newName": "other"}),
        )
        .await,
        "taken",
    );
    assert!(e.contains("already exists"), "{e}");
    refused(
        &app.mcp(
            &w.alice,
            "rename_team",
            json!({"name": "acme", "newName": "no spaces"}),
        )
        .await,
        "bad name",
    );
    let r = ok(
        &app.mcp(
            &w.alice,
            "rename_team",
            json!({"name": "acme", "newName": "Payments-Co"}),
        )
        .await,
        "rename",
    );
    assert_eq!(
        (&r["name"], &r["previous"]),
        (&json!("payments-co"), &json!("acme"))
    );
    let teams = ok(&app.mcp(&w.alice, "list_teams", json!({})).await, "list");
    assert_eq!(teams[0]["name"], "payments-co");
    assert_eq!(teams[0]["projects"], json!(["PAY"]));
    // the pot and the membership followed
    assert_eq!(app.get(&w.c1, "/me").await.1["projects"], json!(["PAY"]));

    // delete: refused while it has pots, and with a wrong confirm
    let e = refused(
        &app.mcp(
            &w.alice,
            "delete_team",
            json!({"name": "payments-co", "confirm": "payments-co"}),
        )
        .await,
        "has pots",
    );
    assert!(e.contains("still has pots") && e.contains("PAY"), "{e}");
    refused(
        &app.mcp(
            &w.alice,
            "delete_team",
            json!({"name": "payments-co", "confirm": "nope"}),
        )
        .await,
        "wrong confirm",
    );
    refused(
        &app.mcp(
            &w.bob,
            "delete_team",
            json!({"name": "payments-co", "confirm": "payments-co"}),
        )
        .await,
        "bob",
    );
    ok(
        &app.mcp(
            &w.alice,
            "delete_project",
            json!({"key": "PAY", "confirm": "PAY"}),
        )
        .await,
        "empty it",
    );
    ok(
        &app.mcp(
            &w.alice,
            "delete_team",
            json!({"name": "payments-co", "confirm": "payments-co"}),
        )
        .await,
        "delete",
    );
    assert_eq!(
        count(app, "select count(*) from teams where name = 'payments-co'").await,
        0
    );
    assert_eq!(
        count(app, "select count(*) from team_members m where not exists (select 1 from teams t where t.id = m.team_id)").await,
        0
    );
    // the people and the cook stay
    assert_eq!(app.get(&w.c1, "/me").await.0, 200);
    assert_eq!(
        count(
            app,
            "select count(*) from principals where name in ('alice', 'c1')"
        )
        .await,
        2
    );
}
