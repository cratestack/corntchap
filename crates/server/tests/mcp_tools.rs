//! Every MCP tool answers with data that matches its declared (typed) output. A mismatch between
//! what the service returns and the schema's output type only shows up as an `internal error` at
//! call time, so each tool is called here with a populated board, over the native transport.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

struct W {
    app: common::TestApp,
    agent: String,
}

impl W {
    async fn ok(&self, tool: &str, args: Value) -> Value {
        let r = self.app.mcp(&self.agent, tool, args.clone()).await;
        assert_ne!(r["isError"], true, "{tool}({args}) failed: {r}");
        assert!(r.is_object(), "{tool}: no result: {r}");
        common::TestApp::payload(&r)
    }
}

#[tokio::test]
async fn every_tool_returns_its_typed_output() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    app.post(&admin, "/teams", json!({"name": "acme"})).await;
    app.post(
        &admin,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &admin,
            "/agents",
            json!({"name": "a1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    app.post(&admin, "/teams/acme/members", json!({"principal": "a1"}))
        .await;
    let w = W {
        agent: ag["token"].as_str().unwrap().to_string(),
        app,
    };

    // a board with a checklist, a dependency and a subtask
    let base = w
        .ok(
            "create_task",
            json!({"project": "PAY", "title": "Base", "body": "b", "items": "one\ntwo", "labels": "api, db", "priority": 5}),
        )
        .await;
    assert_eq!(base["id"], "PAY-1");
    w.ok(
        "create_task",
        json!({"project": "PAY", "title": "After base", "dependsOn": "PAY-1"}),
    )
    .await;
    w.ok(
        "create_task",
        json!({"project": "PAY", "title": "Child", "parent": "PAY-1"}),
    )
    .await;

    // reads with data in them
    let me = w.ok("whoami", json!({})).await;
    assert_eq!(me["name"], "a1");
    let projects = w.ok("list_projects", json!({})).await;
    assert_eq!(projects["items"][0]["key"], "PAY");
    assert_eq!(
        w.ok("list_tasks", json!({"project": "PAY"})).await["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    w.ok(
        "list_tasks",
        json!({"project": "PAY", "bucket": "ready", "label": "api"}),
    )
    .await;
    let t = w.ok("get_task", json!({"id": "PAY-1"})).await;
    assert_eq!(t["subtasks"][0]["id"], "PAY-3");
    w.ok("get_board", json!({"project": "PAY"})).await;
    w.ok("get_task_markdown", json!({"id": "PAY-1"})).await;

    // wait_for_work with ready tasks (it used to fail with an internal error here)
    let ready = w
        .ok("wait_for_work", json!({"project": "PAY", "timeoutS": 1}))
        .await;
    let ids: Vec<&str> = ready["ready"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["PAY-3"], "{ready}");
    w.ok("wait_for_work", json!({})).await;

    // the lease protocol
    let claim = w.ok("claim_task", json!({"id": "PAY-3"})).await;
    let e3 = claim["lease"]["epoch"].as_i64().unwrap();
    w.ok(
        "heartbeat",
        json!({"id": "PAY-3", "epoch": e3, "ttlS": 120}),
    )
    .await;
    w.ok(
        "update_task",
        json!({"id": "PAY-3", "title": "Child (edited)", "labels": "x"}),
    )
    .await;
    w.ok(
        "comment",
        json!({"id": "PAY-3", "body": "note", "epoch": e3}),
    )
    .await;
    let done = w
        .ok(
            "complete",
            json!({"id": "PAY-3", "epoch": e3, "result": "ok"}),
        )
        .await;
    assert_eq!(done["status"], "done");

    let next = w.ok("claim_next", json!({"project": "PAY"})).await;
    assert_eq!(next["task"]["id"], "PAY-1", "{next}");
    let e1 = next["lease"]["epoch"].as_i64().unwrap();
    w.ok(
        "check_item",
        json!({"id": "PAY-1", "epoch": e1, "position": 1, "done": true}),
    )
    .await;
    w.ok(
        "release",
        json!({"id": "PAY-1", "epoch": e1, "reason": "later"}),
    )
    .await;
    let again = w.ok("claim_task", json!({"id": "PAY-1"})).await;
    let e1 = again["lease"]["epoch"].as_i64().unwrap();
    w.ok(
        "block",
        json!({"id": "PAY-1", "epoch": e1, "reason": "need a human"}),
    )
    .await;
    let none = w
        .ok(
            "claim_next",
            json!({"project": "PAY", "labels": "nothing-has-this"}),
        )
        .await;
    assert_eq!(none["none"], true);

    let feed = w
        .ok(
            "changes_since",
            json!({"cursor": 0, "project": "PAY", "limit": 500}),
        )
        .await;
    assert!(feed["events"].as_array().unwrap().len() > 8);
    assert!(feed["next_cursor"].as_i64().unwrap() > 0);

    // the pantry tools (more in tests/pantry.rs)
    w.ok(
        "remember",
        json!({"project": "PAY", "title": "Note", "body": "b", "task": "PAY-1"}),
    )
    .await;
    w.ok("recall", json!({"project": "PAY", "query": "note"}))
        .await;
    w.ok("get_memory", json!({"project": "PAY", "key": "note"}))
        .await;
    w.ok("read_pantry", json!({"project": "PAY"})).await;
    w.ok("forget", json!({"project": "PAY", "key": "note"}))
        .await;

    // the pass (more in tests/pass.rs)
    w.ok(
        "pass",
        json!({"project": "PAY", "message": "taking PAY-1", "task": "PAY-1"}),
    )
    .await;
    let page = w.ok("read_pass", json!({"project": "PAY"})).await;
    assert_eq!(page["messages"][0]["body"], "taking PAY-1");

    // REST: /changes takes `since` as well as `cursor`
    let (_, by_cursor) = w.app.get(&w.agent, "/changes?cursor=3&project=PAY").await;
    let (_, by_since) = w.app.get(&w.agent, "/changes?since=3&project=PAY").await;
    assert_eq!(by_cursor["events"][0]["seq"], by_since["events"][0]["seq"]);
    assert_eq!(by_since["events"][0]["seq"], 4);
}

/// A relay token used as an agent's MCP credential says what to do instead.
#[tokio::test]
async fn relay_token_on_mcp_explains_itself() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    let (_, rl) = app.post(&admin, "/relays", json!({"name": "r1"})).await;
    let r = app
        .mcp(rl["token"].as_str().unwrap(), "whoami", json!({}))
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("cook token"), "{text}");
}

/// The `*Page` procedures are not MCP tools under their own names (the list tools carry `list_*`
/// names), and the agents' other tools keep their default shapes: `list_tasks` is a page
/// (`items`, `total`, `next_cursor`), `read_pass` and `get_task` carry only the additive `older` and
/// `comments_total`.
#[tokio::test]
async fn page_procedures_are_not_tools_and_default_shapes_hold() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    app.post(&admin, "/teams", json!({"name": "acme"})).await;
    app.post(
        &admin,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &admin,
            "/agents",
            json!({"name": "a1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    app.post(&admin, "/teams/acme/members", json!({"principal": "a1"}))
        .await;
    let w = W {
        agent: ag["token"].as_str().unwrap().to_string(),
        app,
    };
    w.ok("create_task", json!({"project": "PAY", "title": "Base"}))
        .await;
    for i in 0..3 {
        w.ok("comment", json!({"id": "PAY-1", "body": format!("c{i}")}))
            .await;
    }
    w.ok("pass", json!({"project": "PAY", "message": "hello"}))
        .await;

    // tools/list holds no page procedure
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}});
    let r: Value = w
        .app
        .http
        .post(format!("{}/mcp", w.app.base))
        .bearer_auth(&w.agent)
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
    let names: Vec<&str> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for n in names.iter() {
        assert!(
            !n.ends_with("_page") && !n.contains("page_"),
            "{n} is a page tool"
        );
    }
    for n in [
        "task_page",
        "question_page",
        "review_page",
        "memory_page",
        "cloud_run_page",
        "view_page",
    ] {
        let r = w.app.mcp(&w.agent, n, json!({})).await;
        assert!(
            r["isError"] == true || r.is_null(),
            "{n} must not be callable as a tool: {r}"
        );
    }
    // they are procedures on the REST surface
    let r = w
        .app
        .http
        .post(format!("{}/api/v2/$procs/taskPage", w.app.base))
        .bearer_auth(&w.agent)
        .json(&json!({"project": "PAY", "bucket": "active", "page": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // list_tasks: a page of tasks with the total and no cursor on the last page
    let page = w.ok("list_tasks", json!({"project": "PAY"})).await;
    assert!(page["next_cursor"].is_null(), "{page}");
    assert_eq!(page["total"], 1);
    let tasks = page["items"].clone();
    assert!(tasks.is_array(), "{tasks}");
    let keys = |v: &Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert!(
        !keys(&tasks[0])
            .iter()
            .any(|k| k.contains("cursor") || k == "total")
    );
    // read_pass: the same keys as ever plus `older`
    let pass = w.ok("read_pass", json!({"project": "PAY"})).await;
    assert_eq!(keys(&pass), ["cursor", "messages", "older", "unread"]);
    assert_eq!(pass["older"], false);
    // get_task: every comment by default, with the count beside them
    let t = w.ok("get_task", json!({"id": "PAY-1"})).await;
    assert_eq!(t["comments"].as_array().unwrap().len(), 3);
    assert_eq!(t["comments_total"], 3);
    assert_eq!(t["comments"][0]["body"], "c0", "oldest first");
}
