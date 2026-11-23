//! The cratestack-generated REST surface at /api/v2: route table, policies, procedure routes.
mod common;
use corntchap::admin;
use reqwest::Method;
use serde_json::{Value, json};

/// Prints the generated route table (`cargo test -p corntchap --test generated_api routes -- --nocapture`).
#[test]
fn routes_are_listed() {
    let routes = corntchap::schema::cratestack_schema::axum::ROUTE_TRANSPORTS;
    let mut lines: Vec<String> = routes
        .iter()
        .map(|r| format!("{:6} /api/v2{}", r.method, r.path))
        .collect();
    lines.sort();
    println!("{}", lines.join("\n"));
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/tasks" && r.method == "GET")
    );
    assert!(
        routes
            .iter()
            .any(|r| r.path == "/projects/{id}" && r.method == "GET")
    );
    for p in [
        "claimNext",
        "claimTask",
        "heartbeat",
        "checkItem",
        "complete",
        "release",
        "block",
    ] {
        let path = format!("/$procs/{p}");
        assert!(
            routes.iter().any(|r| r.path == path && r.method == "POST"),
            "missing procedure route {path}"
        );
    }
}

struct World {
    app: common::TestApp,
    admin: String,
    /// user alice, owner of team a; agent a1 in team a; user bob in team b; relay.
    alice: String,
    a1: String,
    bob: String,
    relay: String,
}

async fn world() -> World {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,cratestack=debug")
        .with_test_writer()
        .try_init();
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    for (team, key) in [("ta", "AAA"), ("tb", "BBB")] {
        app.post(&admin, "/teams", json!({"name": team})).await;
        app.post(
            &admin,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    for u in ["alice", "bob"] {
        app.add_user(u).await;
    }
    let mut toks = vec![];
    for u in ["alice", "bob"] {
        let t = admin::operator_token(&app.state.db, u, None).await.unwrap();
        toks.push(t);
    }
    app.post(&admin, "/teams/ta/members", json!({"principal": "alice"}))
        .await;
    app.post(&admin, "/teams/tb/members", json!({"principal": "bob"}))
        .await;
    let (_, ag) = app
        .post(
            &admin,
            "/agents",
            json!({"name": "a1", "kind": "other", "runtime": "local"}),
        )
        .await;
    app.post(&admin, "/teams/ta/members", json!({"principal": "a1"}))
        .await;
    let (_, rl) = app.post(&admin, "/relays", json!({"name": "r1"})).await;
    app.post(
        &admin,
        "/projects/AAA/tasks",
        json!({"title": "A task", "items": ["one", "two"]}),
    )
    .await;
    app.post(&admin, "/projects/BBB/tasks", json!({"title": "B task"}))
        .await;
    World {
        alice: toks[0].clone(),
        bob: toks[1].clone(),
        a1: ag["token"].as_str().unwrap().into(),
        relay: rl["token"].as_str().unwrap().into(),
        admin,
        app,
    }
}

impl World {
    async fn v2(&self, token: &str, m: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut r = self
            .app
            .http
            .request(m, format!("{}/api/v2{path}", self.app.base))
            .bearer_auth(token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        let text = resp.text().await.unwrap();
        (
            st,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }
    async fn titles(&self, token: &str, path: &str) -> Vec<String> {
        let (st, v) = self.v2(token, Method::GET, path, None).await;
        assert_eq!(st, 200, "{path}: {v}");
        rows(&v)
            .iter()
            .filter_map(|t| t["title"].as_str().or(t["name"].as_str()).map(String::from))
            .collect()
    }
}

fn rows(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        _ => v["data"]
            .as_array()
            .or(v["items"].as_array())
            .cloned()
            .unwrap_or_default(),
    }
}

#[tokio::test]
async fn members_only_see_their_teams() {
    let w = world().await;
    assert_eq!(w.titles(&w.alice, "/tasks").await, vec!["A task"]);
    assert_eq!(w.titles(&w.bob, "/tasks").await, vec!["B task"]);
    let mut all = w.titles(&w.admin, "/tasks").await;
    all.sort();
    assert_eq!(all, vec!["A task", "B task"]);
    assert_eq!(w.titles(&w.alice, "/projects").await, vec!["AAA"]);
    assert_eq!(w.titles(&w.alice, "/teams").await, vec!["ta"]);

    // team B's task by id is not found for alice
    let (_, v) = w.v2(&w.admin, Method::GET, "/tasks", None).await;
    let b = rows(&v)
        .into_iter()
        .find(|t| t["title"] == "B task")
        .unwrap();
    let (st, _) = w
        .v2(
            &w.alice,
            Method::GET,
            &format!("/tasks/{}", b["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(st, 404);
    // items, comments and events are scoped the same way
    let (_, items) = w.v2(&w.alice, Method::GET, "/items", None).await;
    assert_eq!(rows(&items).len(), 2);
    let (_, items) = w.v2(&w.bob, Method::GET, "/items", None).await;
    assert_eq!(rows(&items).len(), 0);
    let projects = |ev: &Value| -> std::collections::BTreeSet<String> {
        rows(ev)
            .iter()
            .filter_map(|e| e["projectId"].as_str().map(String::from))
            .collect()
    };
    let (_, ev_b) = w.v2(&w.bob, Method::GET, "/events", None).await;
    let (_, ev_a) = w.v2(&w.alice, Method::GET, "/events", None).await;
    let (_, ev_all) = w.v2(&w.admin, Method::GET, "/events", None).await;
    assert_eq!(projects(&ev_a).len(), 1, "{ev_a}");
    assert_eq!(projects(&ev_b).len(), 1, "{ev_b}");
    assert!(projects(&ev_a).is_disjoint(&projects(&ev_b)));
    assert!(projects(&ev_all).len() >= 2);
}

#[tokio::test]
async fn secrets_are_never_exposed() {
    let w = world().await;
    let (st, v) = w.v2(&w.admin, Method::GET, "/principals", None).await;
    assert_eq!(st, 200);
    let text = v.to_string();
    for forbidden in [
        "oidcSub",
        "oidc_sub",
        "adminSource",
        "token_hash",
        "tokenHash",
        "idRef",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
    }
    assert!(!rows(&v).is_empty());
    // no model for tokens at all
    let (st, _) = w.v2(&w.admin, Method::GET, "/tokens", None).await;
    assert_eq!(st, 404);
    // alice sees herself and her team (alice, a1), never bob
    let names = w.titles(&w.alice, "/principals").await;
    assert!(names.contains(&"alice".to_string()) && !names.contains(&"bob".to_string()));
}

#[tokio::test]
async fn relays_get_nothing() {
    let w = world().await;
    for path in [
        "/tasks",
        "/projects",
        "/teams",
        "/events",
        "/principals",
        "/items",
    ] {
        let (st, v) = w.v2(&w.relay, Method::GET, path, None).await;
        assert!(
            st == 403 || (st == 200 && rows(&v).is_empty()),
            "{path}: relay got {st} {v}"
        );
    }
    let (st, _) = w
        .v2(
            &w.relay,
            Method::POST,
            "/$procs/projectOverview",
            Some(json!({})),
        )
        .await;
    assert_eq!(st, 403);
}

#[tokio::test]
async fn agent_edits_only_the_task_it_holds() {
    let w = world().await;
    // a second task in the agent's project
    w.app
        .post(&w.admin, "/projects/AAA/tasks", json!({"title": "Other"}))
        .await;
    let (_, v) = w.v2(&w.a1, Method::GET, "/tasks", None).await;
    let list = rows(&v);
    assert_eq!(list.len(), 2, "{v}");
    let id_of = |title: &str| {
        list.iter().find(|t| t["title"] == title).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (mine, other) = (id_of("A task"), id_of("Other"));

    // not held: refused
    let (st, v) = w
        .v2(
            &w.a1,
            Method::PATCH,
            &format!("/tasks/{mine}"),
            Some(json!({"title": "hijack"})),
        )
        .await;
    assert!(
        st == 403 || st == 404,
        "agent edited an unheld task: {st} {v}"
    );

    // claim over the generated procedure route, then edit
    let (st, v) = w
        .v2(
            &w.a1,
            Method::POST,
            "/$procs/claimTask",
            Some(json!({"id": "AAA-1"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = w
        .v2(
            &w.a1,
            Method::PATCH,
            &format!("/tasks/{mine}"),
            Some(json!({"title": "mine now"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    // still not the other one
    let (st, _) = w
        .v2(
            &w.a1,
            Method::PATCH,
            &format!("/tasks/{other}"),
            Some(json!({"title": "no"})),
        )
        .await;
    assert!(st == 403 || st == 404);
    // lease and status fields are not writable through the model route
    let (st, _) = w
        .v2(
            &w.a1,
            Method::PATCH,
            &format!("/tasks/{mine}"),
            Some(json!({"status": "done"})),
        )
        .await;
    let (_, v) = w
        .v2(&w.a1, Method::GET, &format!("/tasks/{mine}"), None)
        .await;
    assert_eq!(
        v["status"], "claimed",
        "status changed via PATCH (st {st}): {v}"
    );
    assert_eq!(v["title"], "mine now");
}

#[tokio::test]
async fn other_teams_cannot_write_and_non_admins_cannot_manage() {
    let w = world().await;
    let (_, v) = w.v2(&w.admin, Method::GET, "/tasks", None).await;
    let a = rows(&v)
        .into_iter()
        .find(|t| t["title"] == "A task")
        .unwrap();
    let id = a["id"].as_str().unwrap();
    // bob (team b, a user) cannot edit team a's task
    let (st, _) = w
        .v2(
            &w.bob,
            Method::PATCH,
            &format!("/tasks/{id}"),
            Some(json!({"title": "x"})),
        )
        .await;
    assert!(st == 403 || st == 404, "{st}");
    // alice (team a user) can
    let (st, v) = w
        .v2(
            &w.alice,
            Method::PATCH,
            &format!("/tasks/{id}"),
            Some(json!({"title": "renamed"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    // only admins create teams
    let (st, _) = w
        .v2(
            &w.alice,
            Method::POST,
            "/teams",
            Some(json!({"name": "mine"})),
        )
        .await;
    assert_eq!(st, 403);
    let (st, v) = w
        .v2(
            &w.admin,
            Method::POST,
            "/teams",
            Some(json!({"name": "tc"})),
        )
        .await;
    assert!(st == 200 || st == 201, "{st} {v}");
    // tasks are created through the procedure, not the model route
    let (st, _) = w
        .v2(
            &w.alice,
            Method::POST,
            "/tasks",
            Some(json!({"title": "x", "body": "", "priority": 0})),
        )
        .await;
    assert!(st == 403 || st == 404 || st == 405, "{st}");
    let (st, v) = w
        .v2(
            &w.alice,
            Method::POST,
            "/$procs/taskCreate",
            Some(json!({"project": "AAA", "title": "via proc"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
}

#[tokio::test]
async fn human_transitions_are_procedures_and_mcp_is_typed() {
    let w = world().await;
    let (st, v) = w
        .v2(
            &w.alice,
            Method::POST,
            "/$procs/cancelTask",
            Some(json!({"id": "AAA-1"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["status"], "cancelled");
    // agents may not use the human-only transitions
    let (st, _) = w
        .v2(
            &w.a1,
            Method::POST,
            "/$procs/unblockTask",
            Some(json!({"id": "AAA-1"})),
        )
        .await;
    assert_eq!(st, 403);

    // tools/list: the tool names are unchanged and task tools carry an outputSchema
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}});
    let r: Value = w
        .app
        .http
        .post(format!("{}/mcp", w.app.base))
        .bearer_auth(&w.a1)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/list")
        .header("accept", "application/json, text/event-stream")
        .json(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {"_meta": meta}}),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "add_team_member",
            "add_wake_target",
            "answer_question",
            "ask",
            "block",
            "changes_since",
            "check_item",
            "claim_next",
            "claim_task",
            "comment",
            "comment_question",
            "complete",
            "connect_cloud",
            "connect_cloud_save",
            "create_cook",
            "create_fire",
            "create_menu",
            "create_project",
            "create_task",
            "create_team",
            "create_upload",
            "delete_cook",
            "delete_file",
            "delete_fire",
            "delete_project",
            "delete_team",
            "delete_view",
            "delete_wake_target",
            "dismiss_question",
            "forget",
            "get_board",
            "get_file",
            "get_memory",
            "get_question",
            "get_task",
            "get_task_markdown",
            "heartbeat",
            "list_activity",
            "list_cooks",
            "list_files",
            "list_fires",
            "list_menus",
            "list_projects",
            "list_questions",
            "list_stations",
            "list_tasks",
            "list_team_members",
            "list_teams",
            "list_users",
            "list_views",
            "list_wake_targets",
            "my_questions",
            "pass",
            "pick_question",
            "read_pantry",
            "read_pass",
            "recall",
            "register_station",
            "release",
            "release_question",
            "remember",
            "remove_station",
            "remove_team_member",
            "rename_cook",
            "rename_fire",
            "rename_project",
            "rename_team",
            "restore_task",
            "rotate_token",
            "run_view",
            "save_view",
            "set_enabled",
            "set_grants",
            "set_main_station",
            "start_station",
            "toss_task",
            "unblock_task",
            "update_menu",
            "update_task",
            "upload_file",
            "usage_report",
            "wait_for_work",
            "whoami",
        ]
    );
    let get = tools.iter().find(|t| t["name"] == "get_task").unwrap();
    assert!(get["outputSchema"].is_object(), "{get}");
}

#[tokio::test]
async fn cook_lease_limits_are_changed_through_the_model() {
    let w = world().await;
    let (_, v) = w.v2(&w.admin, Method::GET, "/principals", None).await;
    let id_of = |name: &str| {
        rows(&v)
            .into_iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (a1, alice) = (id_of("a1"), id_of("alice"));
    // an admin raises a cook's limit
    let (st, v) = w
        .v2(
            &w.admin,
            Method::PATCH,
            &format!("/principals/{a1}"),
            Some(json!({"maxLeases": 2})),
        )
        .await;
    assert_eq!((st, &v["maxLeases"]), (200, &json!(2)), "{v}");
    // out of range, or any other field: refused
    let (st, _) = w
        .v2(
            &w.admin,
            Method::PATCH,
            &format!("/principals/{a1}"),
            Some(json!({"maxLeases": 0})),
        )
        .await;
    assert!((400..500).contains(&st), "{st}");
    let (st, _) = w
        .v2(
            &w.admin,
            Method::PATCH,
            &format!("/principals/{a1}"),
            Some(json!({"name": "renamed"})),
        )
        .await;
    assert!((400..500).contains(&st), "{st}");
    // a teammate who does not own it, the cook itself, and people's own limits: refused
    for (who, id) in [(&w.alice, &a1), (&w.a1, &a1), (&w.admin, &alice)] {
        let (st, _) = w
            .v2(
                who,
                Method::PATCH,
                &format!("/principals/{}", id),
                Some(json!({"maxLeases": 3})),
            )
            .await;
        assert!(st == 403 || st == 404, "{st}");
    }
    // the owner may
    let (st, made) = w
        .app
        .post(
            &w.alice,
            "/agents",
            json!({"name": "a2", "kind": "other", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200, "{made}");
    let (_, v2) = w.v2(&w.alice, Method::GET, "/principals", None).await;
    let a2 = rows(&v2).into_iter().find(|p| p["name"] == "a2").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, v) = w
        .v2(
            &w.alice,
            Method::PATCH,
            &format!("/principals/{a2}"),
            Some(json!({"maxLeases": 4})),
        )
        .await;
    assert_eq!((st, &v["maxLeases"]), (200, &json!(4)), "{v}");
}

#[tokio::test]
async fn files_and_questions_models_follow_the_pot() {
    let w = world().await;
    let db = &w.app.state.db;
    let pid: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'AAA'")
        .fetch_one(db)
        .await
        .unwrap();
    for (n, status) in [(1i64, "ready"), (2, "deleted")] {
        sqlx::query(
            "insert into files (id, project_id, number, name, content_type, storage_key, status, upload_expires_at)
             values (gen_random_uuid(), $1, $2, 'shot.png', 'image/png', 'pots/x/' || $2, $3, now())",
        )
        .bind(pid)
        .bind(n)
        .bind(status)
        .execute(db)
        .await
        .unwrap();
    }
    sqlx::query("insert into questions (project_id, number, body) values ($1, 1, 'which logo?')")
        .bind(pid)
        .execute(db)
        .await
        .unwrap();
    let (st, v) = w
        .v2(&w.alice, Method::GET, "/files?project.key=AAA", None)
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(rows(&v).len(), 1, "deleted files are not listed: {v}");
    assert!(
        !v.to_string().contains("storage"),
        "storage keys stay on the server: {v}"
    );
    let (_, v) = w.v2(&w.bob, Method::GET, "/files", None).await;
    assert!(rows(&v).is_empty(), "another team's files: {v}");
    let (_, v) = w.v2(&w.alice, Method::GET, "/questions", None).await;
    assert_eq!(rows(&v).len(), 1);
    let (_, v) = w.v2(&w.bob, Method::GET, "/questions", None).await;
    assert!(rows(&v).is_empty());
}
