//! Roles: super_admin > orchestrator > member. An orchestrator creates and manages their own teams, pots,
//! cooks, fires and wake targets, but not other people's resources and nothing instance-wide.
mod common;
use common::TestApp;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should be refused: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

struct World {
    app: TestApp,
    /// The super-admin (`init`).
    root: String,
    /// Two orchestrators and a member.
    orch: String,
    orch2: String,
    mem: String,
}

async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    for u in ["orch", "orch2", "mem"] {
        app.add_user(u).await;
    }
    app.set_role("orch", "orchestrator").await;
    app.set_role("orch2", "orchestrator").await;
    let t = |n: &'static str| {
        let db = app.state.db.clone();
        async move { admin::operator_token(&db, n, None).await.unwrap() }
    };
    let (orch, orch2, mem) = (t("orch").await, t("orch2").await, t("mem").await);
    World {
        app,
        root: tok.unwrap(),
        orch,
        orch2,
        mem,
    }
}

/// orch owns team kitchen with pot KIT and the cook `oc`; orch2 owns the cook `pc`.
async fn kitchen(w: &World) {
    ok(
        &w.app
            .mcp(&w.orch, "create_team", json!({"name": "kitchen"}))
            .await,
        "team",
    );
    ok(
        &w.app
            .mcp(
                &w.orch,
                "create_project",
                json!({"team": "kitchen", "key": "KIT", "name": "Kit"}),
            )
            .await,
        "pot",
    );
    ok(
        &w.app
            .mcp(&w.orch, "create_cook", json!({"name": "oc"}))
            .await,
        "cook",
    );
    ok(
        &w.app
            .mcp(&w.orch2, "create_cook", json!({"name": "pc"}))
            .await,
        "other cook",
    );
}

#[tokio::test]
async fn an_orchestrator_creates_and_manages_their_own_things() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    kitchen(&w).await;
    let teams = ok(&app.mcp(o, "list_teams", json!({})).await, "teams");
    assert_eq!(teams[0]["name"], "kitchen");
    assert_eq!(teams[0]["role"], "owner");
    // pots, their settings and repositories
    ok(
        &app.mcp(o, "set_lease_ttl", json!({"project": "KIT", "ttlS": 120}))
            .await,
        "lease",
    );
    ok(
        &app.mcp(
            o,
            "set_pot_guide",
            json!({"project": "KIT", "guide": "be kind"}),
        )
        .await,
        "guide",
    );
    ok(
        &app.mcp(
            o,
            "add_repo",
            json!({"project": "KIT", "url": "https://github.com/acme/api"}),
        )
        .await,
        "repo",
    );
    ok(
        &app.mcp(
            o,
            "rename_project",
            json!({"key": "KIT", "name": "Kitchen pot"}),
        )
        .await,
        "rename pot",
    );
    // their own cooks and fires, and the wake targets of the cooks
    ok(
        &app.mcp(o, "create_fire", json!({"name": "of"})).await,
        "fire",
    );
    ok(
        &app.mcp(o, "add_wake_target", json!({"cook": "oc", "type": "relay"}))
            .await,
        "relay target",
    );
    ok(
        &app.mcp(
            o,
            "add_wake_target",
            json!({"cook": "oc", "type": "claude-routine",
                   "url": "https://api.anthropic.com/v1/claude_code/routines/trig_1/fire", "token": "sk-ant-oat01-x"}),
        )
        .await,
        "routine to Anthropic",
    );
    ok(
        &app.mcp(
            o,
            "add_wake_target",
            json!({"cook": "oc", "type": "github-comment", "repo": "acme/api", "issue": 3, "token": "t"}),
        )
        .await,
        "github comment, default API",
    );
    ok(
        &app.mcp(
            o,
            "add_wake_target",
            json!({"cook": "oc", "type": "devin", "token": "apk_x", "organization": "org-1"}),
        )
        .await,
        "devin, default API",
    );
    let targets = ok(
        &app.mcp(o, "list_wake_targets", json!({"cook": "oc"})).await,
        "targets",
    );
    assert_eq!(targets.as_array().unwrap().len(), 4);
    ok(
        &app.mcp(o, "set_grants", json!({"name": "oc", "grants": "plan"}))
            .await,
        "grants",
    );
    ok(
        &app.mcp(o, "rotate_token", json!({"name": "oc"})).await,
        "rotate",
    );
    ok(
        &app.mcp(o, "set_enabled", json!({"name": "oc", "enabled": false}))
            .await,
        "disable",
    );
    ok(
        &app.mcp(o, "set_enabled", json!({"name": "oc", "enabled": true}))
            .await,
        "enable",
    );
    ok(
        &app.mcp(o, "rename_cook", json!({"name": "oc", "newName": "oc2"}))
            .await,
        "rename cook",
    );
    ok(
        &app.mcp(o, "rename_fire", json!({"name": "of", "newName": "of2"}))
            .await,
        "rename fire",
    );
    // people in the team
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        )
        .await,
        "add a member",
    );
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "orch2", "role": "owner"}),
        )
        .await,
        "make an orchestrator an owner",
    );
    // and take it all down again
    ok(
        &app.mcp(o, "delete_project", json!({"key": "KIT", "confirm": "KIT"}))
            .await,
        "delete pot",
    );
    ok(
        &app.mcp(
            o,
            "rename_team",
            json!({"name": "kitchen", "newName": "galley"}),
        )
        .await,
        "rename team",
    );
    ok(
        &app.mcp(
            o,
            "delete_team",
            json!({"name": "galley", "confirm": "galley"}),
        )
        .await,
        "delete team",
    );
    ok(
        &app.mcp(o, "delete_cook", json!({"name": "oc2"})).await,
        "delete cook",
    );
    ok(
        &app.mcp(o, "delete_fire", json!({"name": "of2"})).await,
        "delete fire",
    );
}

#[tokio::test]
async fn an_orchestrator_cannot_touch_another_ones_team_pot_or_cooks() {
    let w = world().await;
    let (app, o2) = (&w.app, &w.orch2);
    kitchen(&w).await;
    for (tool, args) in [
        ("rename_team", json!({"name": "kitchen", "newName": "x"})),
        (
            "delete_team",
            json!({"name": "kitchen", "confirm": "kitchen"}),
        ),
        (
            "add_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        ),
        (
            "remove_team_member",
            json!({"team": "kitchen", "principal": "orch"}),
        ),
        (
            "create_project",
            json!({"team": "kitchen", "key": "NEW", "name": "New"}),
        ),
        ("rename_project", json!({"key": "KIT", "name": "x"})),
        ("delete_project", json!({"key": "KIT", "confirm": "KIT"})),
        ("set_lease_ttl", json!({"project": "KIT", "ttlS": 90})),
        ("set_pot_guide", json!({"project": "KIT", "guide": "x"})),
        (
            "add_repo",
            json!({"project": "KIT", "url": "https://github.com/acme/api"}),
        ),
        ("rotate_token", json!({"name": "oc"})),
        ("set_enabled", json!({"name": "oc", "enabled": false})),
        ("rename_cook", json!({"name": "oc", "newName": "x"})),
        ("delete_cook", json!({"name": "oc"})),
        ("set_grants", json!({"name": "oc", "grants": "plan"})),
        ("add_wake_target", json!({"cook": "oc", "type": "relay"})),
        ("list_wake_targets", json!({"cook": "oc"})),
    ] {
        refused(&app.mcp(o2, tool, args).await, tool);
    }
    // not even visible: REST says 404 for the team, 404 for the cook
    assert_eq!(app.get(o2, "/teams/kitchen/members").await.0, 404);
    assert_eq!(
        app.post(o2, "/teams/kitchen/members", json!({"principal": "mem"}))
            .await
            .0,
        404
    );
    assert_eq!(app.post(o2, "/agents/oc/rotate", json!({})).await.0, 404);
    // the other orchestrator's lists hold only their own
    let cooks = ok(&app.mcp(o2, "list_cooks", json!({})).await, "cooks");
    assert_eq!(cooks.as_array().unwrap().len(), 1);
    assert_eq!(cooks[0]["name"], "pc");
    assert_eq!(
        ok(&app.mcp(o2, "list_teams", json!({})).await, "teams"),
        json!([])
    );
    // the teams and pots of the REST model are theirs alone, too
    assert_eq!(app.get(o2, "/projects").await.1, json!([]));
    // a super-admin may do any of it
    ok(
        &app.mcp(
            &w.root,
            "rename_team",
            json!({"name": "kitchen", "newName": "galley"}),
        )
        .await,
        "super-admin",
    );
    ok(
        &app.mcp(&w.root, "rotate_token", json!({"name": "oc"}))
            .await,
        "super-admin rotates",
    );
}

#[tokio::test]
async fn an_orchestrator_cannot_do_instance_wide_things() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    kitchen(&w).await;
    // roles and people
    refused(
        &app.mcp(
            o,
            "set_role",
            json!({"user": "mem", "role": "orchestrator"}),
        )
        .await,
        "set_role",
    );
    refused(
        &app.mcp(
            o,
            "set_role",
            json!({"user": "orch", "role": "super_admin"}),
        )
        .await,
        "self-promotion",
    );
    assert_eq!(
        app.rest(
            o,
            reqwest::Method::PATCH,
            "/users/mem",
            Some(json!({"role": "super_admin"}))
        )
        .await
        .0,
        403
    );
    for body in [json!({"disabled": true}), json!({"disabled": false})] {
        assert_eq!(
            app.rest(o, reqwest::Method::PATCH, "/users/mem", Some(body))
                .await
                .0,
            403,
            "disabling or enabling people"
        );
    }
    assert_eq!(role_of(app, "mem").await, "member");
    // forges
    assert_eq!(app.get(o, "/forges").await.0, 403);
    assert_eq!(app.get(o, "/forges/repos").await.0, 403, "without a pot");
    assert_eq!(
        app.post(o, "/forges/unfollowed/dismiss", json!({"repo": "a/b"}))
            .await
            .0,
        403
    );
    assert_eq!(
        app.rest(
            o,
            reqwest::Method::DELETE,
            "/forges/00000000-0000-0000-0000-000000000000",
            None
        )
        .await
        .0,
        403
    );
    // with a pot of their own the repository list is theirs
    assert_eq!(app.get(o, "/forges/repos?project=KIT").await.0, 200);
    refused(
        &app.mcp(&w.orch2, "list_forge_repos", json!({"project": "KIT"}))
            .await,
        "another orchestrator's pot",
    );
    // any-URL cloud targets stay with super-admins
    for (what, args) in [
        (
            "webhook",
            json!({"cook": "oc", "type": "webhook", "url": "https://example.com/h"}),
        ),
        (
            "coder",
            json!({"cook": "oc", "type": "coder", "url": "https://coder.example.com", "token": "t", "organization": "o"}),
        ),
        (
            "devin at a custom URL",
            json!({"cook": "oc", "type": "devin", "url": "https://devin.example.com", "token": "t", "organization": "org-1"}),
        ),
    ] {
        let e = refused(&app.mcp(o, "add_wake_target", args).await, what);
        assert!(e.contains("super-admin"), "{what}: {e}");
    }
    let (st, _) = app
        .post(o, "/agents/oc/wake-targets", json!({"type": "github-comment", "config": {"repo": "a/b", "issue": 1, "token": "t", "api_url": "http://169.254.169.254"}}))
        .await;
    assert_eq!(st, 403, "github at a custom API");
    let e = refused(
        &app.mcp(
            o,
            "add_wake_target",
            json!({"cook": "oc", "type": "claude-routine",
                   "url": "https://hooks.example.com/fire", "token": "t"}),
        )
        .await,
        "routine at another host",
    );
    assert!(e.contains("routine"), "{e}");
    // ... while a super-admin may
    ok(
        &app.mcp(
            &w.root,
            "add_wake_target",
            json!({"cook": "oc", "type": "webhook", "url": "https://example.com/h"}),
        )
        .await,
        "super-admin webhook",
    );
    // a cook for someone else
    let e = refused(
        &app.mcp(o, "create_cook", json!({"name": "forme", "owner": "mem"}))
            .await,
        "cook for someone else",
    );
    assert!(e.contains("super-admin"), "{e}");
    ok(
        &app.mcp(
            &w.root,
            "create_cook",
            json!({"name": "forme", "owner": "mem"}),
        )
        .await,
        "a super-admin may",
    );
    ok(
        &app.mcp(
            o,
            "create_cook",
            json!({"name": "forself", "owner": "orch"}),
        )
        .await,
        "owner is oneself",
    );
}

async fn role_of(app: &TestApp, who: &str) -> String {
    sqlx::query_scalar("select role from principals where name = $1")
        .bind(who)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_super_admin_sets_roles_and_they_stick() {
    let w = world().await;
    let (app, root) = (&w.app, &w.root);
    let v = ok(
        &app.mcp(
            root,
            "set_role",
            json!({"user": "mem", "role": "orchestrator"}),
        )
        .await,
        "promote",
    );
    assert_eq!(
        (v["role"].as_str(), v["role_source"].as_str()),
        (Some("orchestrator"), Some("manual"))
    );
    assert_eq!(role_of(app, "mem").await, "orchestrator");
    // the member is an orchestrator now and may found a team
    ok(
        &app.mcp(&w.mem, "create_team", json!({"name": "late"}))
            .await,
        "new orchestrator",
    );
    // REST takes the same, with either spelling
    let (st, _) = app
        .rest(
            root,
            reqwest::Method::PATCH,
            "/users/mem",
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(role_of(app, "mem").await, "member");
    let (st, _) = app
        .rest(
            root,
            reqwest::Method::PATCH,
            "/users/mem",
            Some(json!({"role": "wizard"})),
        )
        .await;
    assert_eq!(st, 400);
    // the old flag is gone
    let (st, _) = app
        .rest(
            root,
            reqwest::Method::PATCH,
            "/users/mem",
            Some(json!({"is_admin": true})),
        )
        .await;
    assert_eq!(st, 400);
    // whoami says what you are
    assert_eq!(app.get(root, "/me").await.1["role"], "super_admin");
    assert_eq!(app.get(&w.orch, "/me").await.1["role"], "orchestrator");
    assert_eq!(app.get(&w.mem, "/me").await.1["role"], "member");
    let me = ok(&app.mcp(&w.orch, "whoami", json!({})).await, "whoami");
    assert_eq!(me["role"], "orchestrator");
    // the last super-admin who can sign in cannot be demoted
    let e = refused(
        &app.mcp(
            root,
            "set_role",
            json!({"user": "octo", "role": "orchestrator"}),
        )
        .await,
        "last super-admin",
    );
    assert!(e.contains("last super-admin"), "{e}");
    // with a second one the first may step down
    ok(
        &app.mcp(
            root,
            "set_role",
            json!({"user": "orch", "role": "super_admin"}),
        )
        .await,
        "second super-admin",
    );
    assert_eq!(role_of(app, "orch").await, "super_admin");
    // `orch` has no sign-in link: it does not count as one who could undo the demotion
    let e = refused(
        &app.mcp(root, "set_role", json!({"user": "octo", "role": "member"}))
            .await,
        "an unlinked super-admin does not count",
    );
    assert!(e.contains("last super-admin"), "{e}");
}

#[tokio::test]
async fn only_orchestrators_and_up_create_teams_and_own_them() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    refused(
        &app.mcp(&w.mem, "create_team", json!({"name": "nope"}))
            .await,
        "a member",
    );
    assert_eq!(
        app.post(&w.mem, "/teams", json!({"name": "nope"})).await.0,
        403
    );
    ok(
        &app.mcp(o, "create_team", json!({"name": "acme"})).await,
        "orchestrator",
    );
    ok(
        &app.mcp(&w.root, "create_team", json!({"name": "super"}))
            .await,
        "super-admin",
    );
    // the owner role is for orchestrators and super-admins
    for who in ["mem"] {
        let e = refused(
            &app.mcp(
                o,
                "add_team_member",
                json!({"team": "acme", "principal": who, "role": "owner"}),
            )
            .await,
            "a member as owner",
        );
        assert!(e.contains("orchestrator"), "{e}");
        let e = refused(
            &app.mcp(
                &w.root,
                "add_team_member",
                json!({"team": "acme", "principal": who, "role": "owner"}),
            )
            .await,
            "a member as owner, even by a super-admin",
        );
        assert!(e.contains("orchestrator"), "{e}");
    }
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "acme", "principal": "orch2", "role": "owner"}),
        )
        .await,
        "an orchestrator as owner",
    );
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "acme", "principal": "mem", "role": "chef"}),
        )
        .await,
        "a member as chef",
    );
    // the second owner has the powers
    ok(
        &app.mcp(
            &w.orch2,
            "create_project",
            json!({"team": "acme", "key": "ACM", "name": "Acme"}),
        )
        .await,
        "a second owner",
    );
    // a plain chef has not
    refused(
        &app.mcp(
            &w.mem,
            "create_project",
            json!({"team": "acme", "key": "ZZZ", "name": "Zzz"}),
        )
        .await,
        "a chef",
    );
}

/// A demoted owner keeps the row, but the powers only count while they are an orchestrator.
#[tokio::test]
async fn a_demoted_owners_ownership_goes_dormant_and_returns_with_the_role() {
    let w = world().await;
    let (app, o, root) = (&w.app, &w.orch, &w.root);
    kitchen(&w).await;
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        )
        .await,
        "a chef",
    );
    let (st, _) = app
        .rest(
            root,
            reqwest::Method::PATCH,
            "/users/orch",
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(st, 200);
    let owner_rows = || async {
        sqlx::query_scalar::<_, i64>(
            "select count(*) from team_members m join principals p on p.id = m.principal_id
             where p.name = 'orch' and m.role = 'owner'",
        )
        .fetch_one(&app.state.db)
        .await
        .unwrap()
    };
    assert_eq!(owner_rows().await, 1, "the owner row is not deleted");
    for (tool, args) in [
        ("create_team", json!({"name": "second"})),
        (
            "create_project",
            json!({"team": "kitchen", "key": "NEW", "name": "New"}),
        ),
        ("rename_team", json!({"name": "kitchen", "newName": "x"})),
        ("rename_project", json!({"key": "KIT", "name": "x"})),
        ("set_lease_ttl", json!({"project": "KIT", "ttlS": 90})),
        (
            "add_team_member",
            json!({"team": "kitchen", "principal": "orch2"}),
        ),
        (
            "remove_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        ),
    ] {
        refused(&app.mcp(o, tool, args).await, tool);
    }
    // still a member of the team: they see it
    let teams = ok(&app.mcp(o, "list_teams", json!({})).await, "teams");
    assert_eq!(teams[0]["role"], "owner");
    // their own cooks stay theirs: that never came from the team
    ok(
        &app.mcp(o, "rotate_token", json!({"name": "oc"})).await,
        "own cook",
    );
    // promoted again, the ownership is back
    ok(
        &app.mcp(
            root,
            "set_role",
            json!({"user": "orch", "role": "orchestrator"}),
        )
        .await,
        "promote",
    );
    ok(
        &app.mcp(
            o,
            "create_project",
            json!({"team": "kitchen", "key": "NEW", "name": "New"}),
        )
        .await,
        "owner again",
    );
    ok(
        &app.mcp(
            o,
            "remove_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        )
        .await,
        "owner again",
    );
}

/// An `admin` grant passes on the owner's role, never more: an orchestrator's cook is no super-admin.
#[tokio::test]
async fn an_admin_grant_cook_gets_its_owners_role_and_never_more() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    kitchen(&w).await;
    let (st, _) = app
        .post(
            o,
            "/agents/oc/grants",
            json!({"grants": ["manage", "admin"]}),
        )
        .await;
    assert_eq!(st, 200);
    let oc = ok(
        &app.mcp(o, "rotate_token", json!({"name": "oc"})).await,
        "token",
    )["token"]
        .as_str()
        .unwrap()
        .to_string();
    // it acts as an orchestrator: its owner's teams and cooks, and a team of its own
    ok(
        &app.mcp(
            &oc,
            "create_project",
            json!({"team": "kitchen", "key": "BYC", "name": "By cook"}),
        )
        .await,
        "owner's team",
    );
    ok(
        &app.mcp(&oc, "create_team", json!({"name": "bycook"})).await,
        "a team through its owner",
    );
    // ... but not as a super-admin
    refused(
        &app.mcp(
            &oc,
            "set_role",
            json!({"user": "mem", "role": "orchestrator"}),
        )
        .await,
        "set_role",
    );
    assert_eq!(app.get(&oc, "/forges").await.0, 403);
    let e = refused(
        &app.mcp(
            &oc,
            "add_wake_target",
            json!({"cook": "oc", "type": "webhook", "url": "https://example.com/h"}),
        )
        .await,
        "webhook",
    );
    assert!(e.contains("super-admin"), "{e}");
    refused(
        &app.mcp(&oc, "rotate_token", json!({"name": "pc"})).await,
        "another one's cook",
    );
    refused(
        &app.mcp(&oc, "list_forge_repos", json!({})).await,
        "forge repos without a pot",
    );
    // a `manage`-only cook of a super-admin acts as an orchestrator at most; with `admin` it acts as its owner
    ok(
        &app.mcp(&w.root, "create_cook", json!({"name": "rc"})).await,
        "root cook",
    );
    let (st, _) = app
        .post(&w.root, "/agents/rc/grants", json!({"grants": ["manage"]}))
        .await;
    assert_eq!(st, 200);
    let rc = ok(
        &app.mcp(&w.root, "rotate_token", json!({"name": "rc"}))
            .await,
        "token",
    )["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        app.get(&rc, "/forges/repos").await.0,
        403,
        "manage alone never reaches super-admin powers"
    );
    let (st, _) = app
        .post(
            &w.root,
            "/agents/rc/grants",
            json!({"grants": ["manage", "admin"]}),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(app.get(&rc, "/forges/repos").await.0, 200);
    // set_role itself is always a person's own act
    refused(
        &app.mcp(
            &rc,
            "set_role",
            json!({"user": "mem", "role": "orchestrator"}),
        )
        .await,
        "a cook never sets roles",
    );
}

#[tokio::test]
async fn the_people_directory_has_no_details() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    // a person who disabled themselves out is not offered
    let dir = ok(&app.mcp(o, "list_users", json!({})).await, "directory");
    let rows = dir.as_array().unwrap();
    assert!(rows.iter().any(|u| u["name"] == "mem"));
    for u in rows {
        // no sign-in state or anything else: the typed view leaves these empty
        assert!(u["linked"].is_null() && u["disabled"].is_null(), "{u}");
        let mut keys: Vec<&str> = u.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            ["disabled", "display_name", "linked", "name", "role"],
            "{u}"
        );
    }
    assert!(
        rows.iter()
            .any(|u| u["name"] == "orch" && u["role"] == "orchestrator")
    );
    // over REST too
    let (st, v) = app.get(o, "/users").await;
    assert_eq!(st, 200);
    assert!(
        v[0].get("linked").is_none() && v[0].get("disabled").is_none(),
        "{v}"
    );
    assert_eq!(v[0].as_object().unwrap().len(), 3, "{v}");
    // a super-admin sees everything about everyone
    let full = ok(
        &app.mcp(&w.root, "list_users", json!({})).await,
        "full list",
    );
    assert!(
        full.as_array()
            .unwrap()
            .iter()
            .all(|u| u.get("linked").is_some() && u.get("disabled").is_some())
    );
    // a member sees nobody
    refused(&app.mcp(&w.mem, "list_users", json!({})).await, "a member");
    assert_eq!(app.get(&w.mem, "/users").await.0, 403);
}

/// A team's owner may take someone else's cook out of the team, but not manage the cook itself.
#[tokio::test]
async fn an_owner_may_remove_but_not_manage_a_members_cook() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    kitchen(&w).await;
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        )
        .await,
        "member",
    );
    ok(
        &app.mcp(&w.mem, "create_cook", json!({"name": "mk"})).await,
        "cook",
    );
    ok(
        &app.mcp(
            &w.mem,
            "add_team_member",
            json!({"team": "kitchen", "principal": "mk"}),
        )
        .await,
        "the member brings the cook",
    );
    // the owner sees it among the chefs but may not manage it
    let chefs = ok(
        &app.mcp(o, "list_team_members", json!({"team": "kitchen"}))
            .await,
        "chefs",
    );
    assert!(chefs.as_array().unwrap().iter().any(|c| c["name"] == "mk"));
    let mine = ok(&app.mcp(o, "list_cooks", json!({})).await, "cooks");
    assert!(mine.as_array().unwrap().iter().all(|c| c["name"] != "mk"));
    for (tool, args) in [
        ("rotate_token", json!({"name": "mk"})),
        ("set_enabled", json!({"name": "mk", "enabled": false})),
        ("delete_cook", json!({"name": "mk"})),
        ("rename_cook", json!({"name": "mk", "newName": "x"})),
        ("set_grants", json!({"name": "mk", "grants": "manage"})),
        ("add_wake_target", json!({"cook": "mk", "type": "relay"})),
        ("list_wake_targets", json!({"cook": "mk"})),
        (
            "set_cook_repos",
            json!({"cook": "mk", "project": "KIT", "repos": ["x"]}),
        ),
    ] {
        refused(&app.mcp(o, tool, args).await, tool);
    }
    assert_eq!(app.post(o, "/agents/mk/rotate", json!({})).await.0, 404);
    // and cannot pull another one's cook into the team
    refused(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "pc"}),
        )
        .await,
        "someone else's cook",
    );
    // removing it from their team is theirs to do
    ok(
        &app.mcp(
            o,
            "remove_team_member",
            json!({"team": "kitchen", "principal": "mk"}),
        )
        .await,
        "remove",
    );
    let chefs = ok(
        &app.mcp(o, "list_team_members", json!({"team": "kitchen"}))
            .await,
        "chefs",
    );
    assert!(chefs.as_array().unwrap().iter().all(|c| c["name"] != "mk"));
    // the cook still exists, its owner's
    ok(
        &app.mcp(&w.mem, "rotate_token", json!({"name": "mk"})).await,
        "owner",
    );
}

/// Usage is a super-admin's for everyone, an orchestrator's for their cooks and the pots they own.
#[tokio::test]
async fn usage_follows_the_cooks_and_pots_you_own() {
    let w = world().await;
    let (app, o) = (&w.app, &w.orch);
    kitchen(&w).await;
    ok(
        &app.mcp(
            o,
            "add_team_member",
            json!({"team": "kitchen", "principal": "mem"}),
        )
        .await,
        "member",
    );
    ok(
        &app.mcp(&w.mem, "create_cook", json!({"name": "mk"})).await,
        "cook",
    );
    // oc (orch's), mk (mem's, works in orch's pot KIT), pc (orch2's, in no pot of orch)
    for (cook, pot) in [("oc", Some("KIT")), ("mk", Some("KIT")), ("pc", None)] {
        sqlx::query(
            "insert into usage_daily (day, principal_id, project_id, kind, name, calls, chars)
             select current_date, a.id, (select id from projects where key = $2), 'tool', 'whoami', 1, 40
             from principals a where a.name = $1",
        )
        .bind(cook)
        .bind(pot)
        .execute(&app.state.db)
        .await
        .unwrap();
    }
    let seen = |r: Value| -> Vec<String> {
        let mut v: Vec<String> = ok(&r, "usage")["by_cook"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["cook"].as_str().unwrap().to_string())
            .filter(|c| ["oc", "mk", "pc"].contains(&c.as_str()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        seen(app.mcp(o, "usage_report", json!({})).await),
        ["mk", "oc"]
    );
    assert_eq!(
        seen(app.mcp(&w.orch2, "usage_report", json!({})).await),
        ["pc"]
    );
    assert_eq!(
        seen(app.mcp(&w.mem, "usage_report", json!({})).await),
        ["mk"]
    );
    assert_eq!(
        seen(app.mcp(&w.root, "usage_report", json!({})).await),
        ["mk", "oc", "pc"]
    );
    // a demoted owner's pot visibility goes with the role
    app.set_role("orch", "member").await;
    assert_eq!(seen(app.mcp(o, "usage_report", json!({})).await), ["oc"]);
}

#[tokio::test]
async fn the_sole_super_admin_cannot_step_down_even_by_a_no_op_first() {
    let w = world().await;
    let (app, root) = (&w.app, &w.root);
    let source = || async {
        sqlx::query_scalar::<_, String>("select role_source from principals where name = 'octo'")
            .fetch_one(&app.state.db)
            .await
            .unwrap()
    };
    // setting the role they already have changes nothing: still `init`, still usable
    let v = ok(
        &app.mcp(
            root,
            "set_role",
            json!({"user": "octo", "role": "super_admin"}),
        )
        .await,
        "no-op",
    );
    assert_eq!(v["role_source"], "init");
    assert_eq!(source().await, "init");
    let usable: i64 = sqlx::query_scalar(
        "select count(*) from principals where kind = 'user' and role = 'super_admin'
         and disabled_at is null and (oidc_sub is not null or role_source = 'init')",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(usable, 1);
    // so the only super-admin still cannot demote themselves
    let e = refused(
        &app.mcp(root, "set_role", json!({"user": "octo", "role": "member"}))
            .await,
        "self-demotion",
    );
    assert!(e.contains("last super-admin"), "{e}");
    assert_eq!(role_of(app, "octo").await, "super_admin");
    assert_eq!(source().await, "init");
}

#[tokio::test]
async fn a_refused_disable_leaves_the_role_change_unapplied() {
    let w = world().await;
    let (app, root) = (&w.app, &w.root);
    let (st, _) = app
        .rest(
            root,
            reqwest::Method::PATCH,
            "/users/octo",
            Some(json!({"role": "orchestrator", "disabled": true})),
        )
        .await;
    assert_eq!(st, 409);
    assert_eq!(role_of(app, "octo").await, "super_admin");
    let off: bool =
        sqlx::query_scalar("select disabled_at is not null from principals where name = 'octo'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert!(!off);
}

#[tokio::test]
async fn a_member_owned_cook_with_manage_and_admin_cannot_create_a_team() {
    let w = world().await;
    let (app, m) = (&w.app, &w.mem);
    ok(
        &app.mcp(m, "create_cook", json!({"name": "mc"})).await,
        "member's cook",
    );
    let (st, _) = app
        .post(
            &w.root,
            "/agents/mc/grants",
            json!({"grants": ["manage", "admin"]}),
        )
        .await;
    assert_eq!(st, 200);
    let mc = ok(
        &app.mcp(m, "rotate_token", json!({"name": "mc"})).await,
        "token",
    )["token"]
        .as_str()
        .unwrap()
        .to_string();
    refused(
        &app.mcp(&mc, "create_team", json!({"name": "nope"})).await,
        "create_team",
    );
}
