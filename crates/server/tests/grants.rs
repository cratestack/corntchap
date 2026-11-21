//! Everything the console does is an MCP tool, and a cook may do what its owner granted it
//! (plan, manage, admin), never more than the owner, and never raise its own grants.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    assert!(r.is_object(), "{what}: {r}");
    common::TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should be refused: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

#[tokio::test]
async fn console_actions_are_tools_and_cooks_act_within_their_grants() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    let call = |tok: &str, tool: &str, args: Value| {
        let (tok, tool, app) = (tok.to_string(), tool.to_string(), &app);
        async move { app.mcp(&tok, &tool, args).await }
    };

    // --- the admin drives everything over MCP (every console action has a tool) ---
    ok(
        &call(&root, "create_user", json!({"login": "alice"})).await,
        "create_user",
    );
    let users = ok(&call(&root, "list_users", json!({})).await, "list_users");
    assert!(
        users
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["name"] == "alice")
    );
    ok(
        &call(&root, "create_team", json!({"name": "acme"})).await,
        "create_team",
    );
    ok(
        &call(
            &root,
            "add_team_member",
            json!({"team": "acme", "principal": "alice", "role": "owner"}),
        )
        .await,
        "add_team_member",
    );
    let teams = ok(&call(&root, "list_teams", json!({})).await, "list_teams");
    assert_eq!(teams[0]["name"], "acme");
    let p = ok(
        &call(
            &root,
            "create_project",
            json!({"team": "acme", "key": "pay", "name": "Payments", "leaseTtlS": 120}),
        )
        .await,
        "create_project",
    );
    assert_eq!((&p["key"], &p["lease_ttl_s"]), (&json!("PAY"), &json!(120)));
    let fire = ok(
        &call(&root, "create_fire", json!({"name": "laptop"})).await,
        "create_fire",
    );
    assert!(fire["token"].as_str().unwrap().starts_with("ct_"));
    let fires = ok(&call(&root, "list_fires", json!({})).await, "list_fires");
    assert_eq!(fires[0]["name"], "laptop");
    let members = ok(
        &call(&root, "list_team_members", json!({"team": "acme"})).await,
        "list_team_members",
    );
    assert!(
        members
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["name"] == "alice" && m["role"] == "owner")
    );

    // alice (a person, team owner, not an admin) makes her cook over MCP
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let c1 = ok(
        &call(
            &alice,
            "create_cook",
            json!({"name": "c1", "agentKind": "opencode", "runtime": "local", "maxLeases": 3}),
        )
        .await,
        "create_cook",
    );
    let c1 = c1["token"].as_str().unwrap().to_string();
    ok(
        &call(
            &alice,
            "add_team_member",
            json!({"team": "acme", "principal": "c1"}),
        )
        .await,
        "add c1",
    );
    let cooks = ok(&call(&alice, "list_cooks", json!({})).await, "list_cooks");
    assert_eq!(
        (&cooks[0]["name"], &cooks[0]["grants"]),
        (&json!("c1"), &json!([]))
    );
    let wt = ok(
        &call(
            &alice,
            "add_wake_target",
            json!({"cook": "c1", "type": "relay", "projects": "PAY"}),
        )
        .await,
        "add_wake_target",
    );
    let wts = ok(
        &call(&alice, "list_wake_targets", json!({"cook": "c1"})).await,
        "list_wake_targets",
    );
    assert_eq!(
        (&wts[0]["type"], &wts[0]["projects"]),
        (&json!("relay"), &json!(["PAY"]))
    );
    ok(
        &call(&alice, "delete_wake_target", json!({"id": wt["id"]})).await,
        "delete_wake_target",
    );
    let hook = ok(
        &call(
            &root,
            "add_wake_target",
            json!({"cook": "c1", "type": "webhook", "url": "https://hooks.example/x"}),
        )
        .await,
        "webhook target",
    );
    let wts = ok(
        &call(&root, "list_wake_targets", json!({"cook": "c1"})).await,
        "list webhook",
    );
    assert_eq!(wts[0]["target"], "https://hooks.example/x");
    ok(
        &call(&root, "delete_wake_target", json!({"id": hook["id"]})).await,
        "delete webhook",
    );

    for t in ["one", "two", "three"] {
        ok(
            &call(
                &alice,
                "create_task",
                json!({"project": "PAY", "title": t, "items": "a\nb"}),
            )
            .await,
            "create_task",
        );
    }

    // --- c1 with no grants: works tasks, nothing more ---
    let e = refused(
        &call(&c1, "toss_task", json!({"id": "PAY-1", "reason": "dup"})).await,
        "toss",
    );
    assert!(e.contains("grant") && e.contains("plan"), "{e}");
    let e = refused(
        &call(
            &c1,
            "create_project",
            json!({"team": "acme", "key": "X", "name": "X"}),
        )
        .await,
        "create_project",
    );
    assert!(e.contains("manage"), "{e}");
    refused(
        &call(
            &c1,
            "update_task",
            json!({"id": "PAY-2", "title": "renamed"}),
        )
        .await,
        "edit unheld",
    );
    refused(
        &call(
            &c1,
            "check_item",
            json!({"id": "PAY-2", "position": 1, "done": true}),
        )
        .await,
        "tick w/o lease",
    );
    let e = refused(
        &call(&c1, "set_grants", json!({"name": "c1", "grants": "plan"})).await,
        "self-grant",
    );
    assert!(e.contains("only a person"), "{e}");
    assert_eq!(
        ok(&call(&c1, "list_cooks", json!({})).await, "c1 list_cooks"),
        json!([])
    );

    // --- plan: the people-side task actions ---
    let g = ok(
        &call(
            &alice,
            "set_grants",
            json!({"name": "c1", "grants": "plan"}),
        )
        .await,
        "grant plan",
    );
    assert_eq!(g["grants"], json!(["plan"]));
    let me = ok(&call(&c1, "whoami", json!({})).await, "whoami");
    assert_eq!(me["grants"], json!(["plan"]));
    let t = ok(
        &call(&c1, "toss_task", json!({"id": "PAY-1", "reason": "probe"})).await,
        "toss with plan",
    );
    assert_eq!(t["status"], "cancelled");
    ok(
        &call(&c1, "restore_task", json!({"id": "PAY-1"})).await,
        "restore",
    );
    ok(
        &call(
            &c1,
            "update_task",
            json!({"id": "PAY-2", "title": "renamed"}),
        )
        .await,
        "edit any task",
    );
    ok(
        &call(
            &c1,
            "check_item",
            json!({"id": "PAY-2", "position": 1, "done": true}),
        )
        .await,
        "tick w/o lease",
    );
    let claim = ok(
        &call(&c1, "claim_task", json!({"id": "PAY-3"})).await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    ok(
        &call(
            &c1,
            "block",
            json!({"id": "PAY-3", "epoch": epoch, "reason": "needs a key"}),
        )
        .await,
        "block",
    );
    ok(
        &call(&c1, "unblock_task", json!({"id": "PAY-3"})).await,
        "unblock",
    );
    // still no admin side
    refused(
        &call(
            &c1,
            "create_project",
            json!({"team": "acme", "key": "X", "name": "X"}),
        )
        .await,
        "plan != manage",
    );

    // --- manage: acts as its owner (alice is a team owner, not an admin) ---
    ok(
        &call(
            &alice,
            "set_grants",
            json!({"name": "c1", "grants": "plan, manage, admin"}),
        )
        .await,
        "grant manage",
    );
    let p = ok(
        &call(
            &c1,
            "create_project",
            json!({"team": "acme", "key": "ops", "name": "Ops"}),
        )
        .await,
        "create_project as owner",
    );
    assert_eq!(p["key"], "OPS");
    let made = ok(
        &call(&c1, "create_cook", json!({"name": "c1-helper"})).await,
        "create_cook as owner",
    );
    assert!(made["token"].as_str().unwrap().starts_with("ct_"));
    let cooks: Vec<String> = ok(&call(&c1, "list_cooks", json!({})).await, "owner's cooks")
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(cooks, ["c1", "c1-helper"]);
    ok(
        &call(
            &c1,
            "set_enabled",
            json!({"name": "c1-helper", "enabled": false}),
        )
        .await,
        "disable",
    );
    ok(
        &call(&c1, "rotate_token", json!({"name": "c1-helper"})).await,
        "rotate",
    );
    // never more than its owner: alice is no admin, so `admin` adds nothing
    let e = refused(
        &call(&c1, "create_team", json!({"name": "rogue"})).await,
        "owner not admin",
    );
    assert!(e.contains("admin"), "{e}");
    refused(&call(&c1, "list_users", json!({})).await, "owner not admin");
    // and still cannot raise its own grants while acting for its owner
    refused(
        &call(&c1, "set_grants", json!({"name": "c1", "grants": ""})).await,
        "self-grant with manage",
    );

    // --- an admin's cook granted admin has the admin powers ---
    let c2 = ok(
        &call(&root, "create_cook", json!({"name": "c2"})).await,
        "root cook",
    );
    let c2 = c2["token"].as_str().unwrap().to_string();
    ok(
        &call(
            &root,
            "set_grants",
            json!({"name": "c2", "grants": "manage,admin"}),
        )
        .await,
        "grant c2",
    );
    ok(
        &call(&c2, "create_team", json!({"name": "platform"})).await,
        "admin cook creates a team",
    );
    ok(
        &call(&c2, "list_users", json!({})).await,
        "admin cook lists users",
    );
    let e = refused(
        &call(&root, "set_grants", json!({"name": "c2", "grants": "root"})).await,
        "bad grant",
    );
    assert!(e.contains("unknown grant"), "{e}");

    // the cook stays the actor in the feed
    let feed = ok(
        &call(
            &alice,
            "changes_since",
            json!({"cursor": 0, "project": "PAY", "limit": 500}),
        )
        .await,
        "feed",
    );
    let tossed = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "task.cancelled")
        .unwrap();
    assert_eq!(
        (&tossed["actor"], &tossed["payload"]["reason"]),
        (&json!("c1"), &json!("probe"))
    );

    // REST matches: people set grants, cooks cannot
    let (st, _) = app
        .post(&c1, "/agents/c1/grants", json!({"grants": ["admin"]}))
        .await;
    assert_eq!(st, 403);
    let (st, v) = app
        .post(&alice, "/agents/c1/grants", json!({"grants": []}))
        .await;
    assert_eq!((st, &v["grants"]), (200, &json!([])));
    refused(
        &call(&c1, "toss_task", json!({"id": "PAY-2", "reason": "x"})).await,
        "grants revoked",
    );
}
