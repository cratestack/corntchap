mod common;
use corntchap::admin;
use serde_json::{Value, json};

/// Handshake-era MCP (2025-06-18 here): initialize, notifications, tools/list, tools/call.
struct Legacy<'a> {
    app: &'a common::TestApp,
    token: String,
}

impl Legacy<'_> {
    async fn send(&self, version: Option<&str>, body: Value) -> reqwest::Response {
        let mut r = self
            .app
            .http
            .post(format!("{}/mcp", self.app.base))
            .bearer_auth(&self.token)
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(v) = version {
            r = r.header("MCP-Protocol-Version", v);
        }
        r.send().await.unwrap()
    }
    async fn rpc(&self, version: Option<&str>, method: &str, params: Value) -> Value {
        let resp = self
            .send(
                version,
                json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}),
            )
            .await;
        assert_eq!(resp.status().as_u16(), 200, "{method}");
        let v: Value = resp.json().await.unwrap();
        assert_eq!(v["id"], 7);
        v
    }
    async fn call(&self, tool: &str, args: Value) -> Value {
        self.rpc(
            Some("2025-06-18"),
            "tools/call",
            json!({"name": tool, "arguments": args}),
        )
        .await["result"]
            .clone()
    }
}

#[tokio::test]
async fn legacy_handshake_list_and_call() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let admin_tok = tok.unwrap();
    app.post(&admin_tok, "/teams", json!({"name": "acme"}))
        .await;
    app.post(
        &admin_tok,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, ag) = app
        .post(
            &admin_tok,
            "/agents",
            json!({"name": "a1", "kind": "other", "runtime": "local"}),
        )
        .await;
    let agent = ag["token"].as_str().unwrap().to_string();
    app.post(
        &admin_tok,
        "/teams/acme/members",
        json!({"principal": "a1"}),
    )
    .await;
    app.post(
        &admin_tok,
        "/projects/PAY/tasks",
        json!({"title": "Do it", "items": ["one", "two"]}),
    )
    .await;

    let c = Legacy {
        app: &app,
        token: agent.clone(),
    };

    // initialize negotiates: echo a supported version, else fall back to 2025-11-25
    let init = c
        .rpc(
            None,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        )
        .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18", "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "corntchap");
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("claim_next")
    );
    let newer = c
        .rpc(None, "initialize", json!({"protocolVersion": "2099-01-01"}))
        .await;
    assert_eq!(newer["result"]["protocolVersion"], "2025-11-25");

    // notifications get 202 and no body
    let n = c
        .send(
            Some("2025-06-18"),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await;
    assert_eq!(n.status().as_u16(), 202);
    assert!(n.text().await.unwrap().is_empty());

    // tools/list reuses the generated table
    let list = c.rpc(Some("2025-06-18"), "tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in ["claim_next", "check_item", "complete", "get_task_markdown"] {
        assert!(names.contains(&want), "{want} missing in {names:?}");
    }
    let claim = tools.iter().find(|t| t["name"] == "claim_next").unwrap();
    assert_eq!(claim["inputSchema"]["type"], "object");
    assert_eq!(claim["annotations"]["readOnlyHint"], false);
    let get = tools.iter().find(|t| t["name"] == "get_task").unwrap();
    assert_eq!(get["annotations"]["readOnlyHint"], true);

    // tools/call: claim, tick, stale complete, markdown, complete
    let r = c.call("claim_next", json!({"project": "PAY"})).await;
    assert_eq!(r["isError"], false, "{r}");
    let payload = common::TestApp::payload(&r);
    assert_eq!(payload["task"]["id"], "PAY-1");
    assert_eq!(r["structuredContent"], payload);
    let epoch = payload["lease"]["epoch"].as_i64().unwrap();
    let r = c
        .call(
            "check_item",
            json!({"id": "PAY-1", "epoch": epoch, "position": 1, "done": true}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    let r = c
        .call(
            "complete",
            json!({"id": "PAY-1", "epoch": epoch + 5, "result": "x"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("lease_lost"),
        "{r}"
    );
    let md = c.call("get_task_markdown", json!({"id": "PAY-1"})).await;
    assert!(
        common::TestApp::payload(&md)["markdown"]
            .as_str()
            .unwrap()
            .contains("PAY-1"),
        "{md}"
    );
    let r = c
        .call(
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "done"}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");

    // bad arguments: isError naming the argument; unknown tool: -32602; unknown method: -32601
    let r = c.call("get_task", json!({})).await;
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"].as_str().unwrap().contains("id"),
        "{r}"
    );
    let e = c
        .rpc(Some("2025-03-26"), "tools/call", json!({"name": "nope"}))
        .await;
    assert_eq!(e["error"]["code"], -32602);
    let e = c.rpc(None, "wat/ever", json!({})).await;
    assert_eq!(e["error"]["code"], -32601);
    let e = c.rpc(None, "prompts/list", json!({})).await;
    assert_eq!(e["result"]["prompts"], json!([]));
    let e = c.rpc(None, "ping", json!({})).await;
    assert_eq!(e["result"], json!({}));

    // JSON-RPC batch (2025-03-26)
    let resp = c
        .send(
            Some("2025-03-26"),
            json!([
                {"jsonrpc": "2.0", "id": 1, "method": "ping"},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {"jsonrpc": "2.0", "id": 2, "method": "resources/list"}
            ]),
        )
        .await;
    let b: Value = resp.json().await.unwrap();
    assert_eq!(b.as_array().unwrap().len(), 2, "{b}");

    // GET/DELETE are not supported in stateless mode
    for m in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let r = app
            .http
            .request(m, format!("{}/mcp", app.base))
            .bearer_auth(&agent)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 405);
    }

    // missing / bad token: 401 with the protected-resource challenge
    for bearer in [None, Some("ct_garbage")] {
        let mut r = app
            .http
            .post(format!("{}/mcp", app.base))
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
        if let Some(b) = bearer {
            r = r.bearer_auth(b);
        }
        let r = r.send().await.unwrap();
        assert_eq!(r.status().as_u16(), 401);
        let h = r.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            h.contains(&format!(
                "resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
                app.base
            )),
            "{h}"
        );
    }

    // the 2026-07-28 path still reaches cratestack on the same URL
    let who = common::TestApp::payload(&app.mcp(&agent, "whoami", json!({})).await);
    assert_eq!(who["name"], "a1");
}

/// Models are MCP resources on both protocol doors, read under the same @@allow policies.
#[tokio::test]
async fn models_are_resources_under_policies() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let admin_tok = tok.unwrap();
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&admin_tok, "/teams", json!({"name": team})).await;
        app.post(
            &admin_tok,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
        for n in 0..3 {
            app.post(
                &admin_tok,
                &format!("/projects/{key}/tasks"),
                json!({"title": format!("{key} task {n}")}),
            )
            .await;
        }
    }
    let (_, ag) = app
        .post(
            &admin_tok,
            "/agents",
            json!({"name": "a1", "kind": "other", "runtime": "local"}),
        )
        .await;
    app.post(
        &admin_tok,
        "/teams/acme/members",
        json!({"principal": "a1"}),
    )
    .await;
    let mcp = Legacy {
        app: &app,
        token: ag["token"].as_str().unwrap().to_string(),
    };

    let init = mcp
        .rpc(Some("2025-06-18"), "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}))
        .await;
    assert!(init["result"]["capabilities"]["resources"].is_object());

    let list = mcp
        .rpc(Some("2025-06-18"), "resources/list", json!({}))
        .await;
    let uris: Vec<String> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap().to_string())
        .collect();
    for seg in ["projects", "tasks", "items", "comments", "events", "teams"] {
        assert!(
            uris.contains(&format!("cratestack://corntchap/{seg}")),
            "{seg} missing in {uris:?}"
        );
    }
    let templates = mcp
        .rpc(Some("2025-06-18"), "resources/templates/list", json!({}))
        .await;
    assert!(
        templates["result"]["resourceTemplates"]
            .as_array()
            .unwrap()
            .len()
            >= 2 * uris.len()
    );

    // Page through tasks two at a time: only team acme's three tasks are visible.
    let mut seen = vec![];
    let mut uri = "cratestack://corntchap/tasks?limit=2".to_string();
    loop {
        let r = mcp
            .rpc(Some("2025-06-18"), "resources/read", json!({"uri": uri}))
            .await;
        let page: Value =
            serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
        for t in page["items"].as_array().unwrap() {
            seen.push(t["title"].as_str().unwrap().to_string());
        }
        match page["nextCursor"].as_str() {
            Some(c) => uri = format!("cratestack://corntchap/tasks?limit=2&cursor={c}"),
            None => break,
        }
    }
    seen.sort();
    assert_eq!(seen, vec!["PAY task 0", "PAY task 1", "PAY task 2"]);

    // A record from the other team reads as not found.
    let other_id: String = sqlx::query_scalar("select t.id::text from tasks t join projects p on p.id = t.project_id where p.key = 'OPS' limit 1")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let r = mcp
        .rpc(
            Some("2025-06-18"),
            "resources/read",
            json!({"uri": format!("cratestack://corntchap/tasks/{other_id}")}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let mine: String = sqlx::query_scalar("select t.id::text from tasks t join projects p on p.id = t.project_id where p.key = 'PAY' limit 1")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let r = mcp
        .rpc(
            Some("2025-06-18"),
            "resources/read",
            json!({"uri": format!("cratestack://corntchap/tasks/{mine}")}),
        )
        .await;
    assert!(
        r["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("PAY task"),
        "{r}"
    );
}
