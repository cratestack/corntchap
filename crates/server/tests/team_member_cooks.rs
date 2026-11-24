//! A plain team member (a chef) may add the cooks they own to the team, and remove them again;
//! nothing else changes: no users, no roles, nobody else's cooks.
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
    alice: String,
    carol: String,
    dave: String,
}

/// octo (admin); alice owns team acme; carol is a plain chef of acme and owns cook ck;
/// alice owns cook ak; dave (in no team) owns cook dk; erin is another user.
async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "carol", "dave", "erin"] {
        app.add_user(u).await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let carol = admin::operator_token(&app.state.db, "carol", None)
        .await
        .unwrap();
    let dave = admin::operator_token(&app.state.db, "dave", None)
        .await
        .unwrap();
    ok(
        &app.mcp(&root, "create_team", json!({"name": "acme"})).await,
        "team",
    );
    for (who, role) in [("alice", "owner"), ("carol", "chef")] {
        ok(
            &app.mcp(
                &root,
                "add_team_member",
                json!({"team": "acme", "principal": who, "role": role}),
            )
            .await,
            "seat",
        );
    }
    for (t, n) in [(&alice, "ak"), (&carol, "ck"), (&dave, "dk")] {
        ok(&app.mcp(t, "create_cook", json!({"name": n})).await, "cook");
    }
    World {
        app,
        alice,
        carol,
        dave,
    }
}

async fn role_of(app: &TestApp, who: &str) -> Option<String> {
    sqlx::query_scalar(
        "select m.role from team_members m join principals p on p.id = m.principal_id
         join teams t on t.id = m.team_id where t.name = 'acme' and p.name = $1",
    )
    .bind(who)
    .fetch_optional(&app.state.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_member_adds_their_own_cook() {
    let w = world().await;
    ok(
        &w.app
            .mcp(
                &w.carol,
                "add_team_member",
                json!({"team": "acme", "principal": "ck"}),
            )
            .await,
        "member adds own cook",
    );
    assert_eq!(role_of(&w.app, "ck").await.as_deref(), Some("member"));
    // over REST too
    let (st, _) = w
        .app
        .post(&w.carol, "/teams/acme/members", json!({"principal": "ck"}))
        .await;
    assert_eq!(st, 200);
}

#[tokio::test]
async fn a_member_cannot_add_someone_elses_cook() {
    let w = world().await;
    for cook in ["ak", "dk"] {
        refused(
            &w.app
                .mcp(
                    &w.carol,
                    "add_team_member",
                    json!({"team": "acme", "principal": cook}),
                )
                .await,
            "someone else's cook",
        );
        assert_eq!(role_of(&w.app, cook).await, None);
    }
}

#[tokio::test]
async fn a_member_cannot_add_users_or_change_roles() {
    let w = world().await;
    refused(
        &w.app
            .mcp(
                &w.carol,
                "add_team_member",
                json!({"team": "acme", "principal": "erin"}),
            )
            .await,
        "add a user",
    );
    assert_eq!(role_of(&w.app, "erin").await, None);
    // promoting herself, re-roling the owner or herself
    for (who, role) in [("carol", "owner"), ("alice", "chef"), ("carol", "chef")] {
        refused(
            &w.app
                .mcp(
                    &w.carol,
                    "add_team_member",
                    json!({"team": "acme", "principal": who, "role": role}),
                )
                .await,
            "change a role",
        );
    }
    assert_eq!(role_of(&w.app, "carol").await.as_deref(), Some("member"));
    assert_eq!(role_of(&w.app, "alice").await.as_deref(), Some("owner"));
    // removing a user is refused too
    refused(
        &w.app
            .mcp(
                &w.carol,
                "remove_team_member",
                json!({"team": "acme", "principal": "alice"}),
            )
            .await,
        "remove a user",
    );
    assert_eq!(role_of(&w.app, "alice").await.as_deref(), Some("owner"));
}

#[tokio::test]
async fn a_member_removes_their_own_cook_only() {
    let w = world().await;
    ok(
        &w.app
            .mcp(
                &w.carol,
                "add_team_member",
                json!({"team": "acme", "principal": "ck"}),
            )
            .await,
        "add",
    );
    ok(
        &w.app
            .mcp(
                &w.alice,
                "add_team_member",
                json!({"team": "acme", "principal": "ak"}),
            )
            .await,
        "owner adds own cook",
    );
    refused(
        &w.app
            .mcp(
                &w.carol,
                "remove_team_member",
                json!({"team": "acme", "principal": "ak"}),
            )
            .await,
        "remove another's cook",
    );
    assert_eq!(role_of(&w.app, "ak").await.as_deref(), Some("member"));
    ok(
        &w.app
            .mcp(
                &w.carol,
                "remove_team_member",
                json!({"team": "acme", "principal": "ck"}),
            )
            .await,
        "remove own cook",
    );
    assert_eq!(role_of(&w.app, "ck").await, None);
}

#[tokio::test]
async fn a_non_member_still_sees_no_team() {
    let w = world().await;
    let e = refused(
        &w.app
            .mcp(
                &w.dave,
                "add_team_member",
                json!({"team": "acme", "principal": "dk"}),
            )
            .await,
        "outsider",
    );
    assert!(e.contains("not found"), "{e}");
}

#[tokio::test]
async fn owners_keep_every_right() {
    let w = world().await;
    for (who, role) in [("erin", "chef"), ("dave", "owner")] {
        ok(
            &w.app
                .mcp(
                    &w.alice,
                    "add_team_member",
                    json!({"team": "acme", "principal": who, "role": role}),
                )
                .await,
            "owner adds a user",
        );
    }
    assert_eq!(role_of(&w.app, "dave").await.as_deref(), Some("owner"));
    // an owner still cannot add someone else's cook
    refused(
        &w.app
            .mcp(
                &w.alice,
                "add_team_member",
                json!({"team": "acme", "principal": "ck"}),
            )
            .await,
        "foreign cook",
    );
    ok(
        &w.app
            .mcp(
                &w.alice,
                "remove_team_member",
                json!({"team": "acme", "principal": "erin"}),
            )
            .await,
        "owner removes",
    );
    assert_eq!(role_of(&w.app, "erin").await, None);
}
