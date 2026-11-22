//! The pantry (shared memory per project) over MCP, REST /api/v1, the generated /api/v2 model and
//! the change feed: save, search, append, revisions, conflicts, forget rules, visibility.
mod common;
use corntchap::admin;
use reqwest::Method;
use serde_json::{Value, json};

struct World {
    app: common::TestApp,
    admin: String,
    /// a1, a2: agents in team acme (project PAY). b1: agent in team other (project OPS).
    a1: String,
    a2: String,
    b1: String,
    relay: String,
}

async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let admin = tok.unwrap();
    for (team, key, name) in [("acme", "PAY", "Payments"), ("other", "OPS", "Platform")] {
        app.post(&admin, "/teams", json!({"name": team})).await;
        app.post(
            &admin,
            "/projects",
            json!({"team": team, "key": key, "name": name}),
        )
        .await;
    }
    let mut toks = vec![];
    for (name, team) in [("a1", "acme"), ("a2", "acme"), ("b1", "other")] {
        let (_, ag) = app
            .post(
                &admin,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "local"}),
            )
            .await;
        toks.push(ag["token"].as_str().unwrap().to_string());
        app.post(
            &admin,
            &format!("/teams/{team}/members"),
            json!({"principal": name}),
        )
        .await;
    }
    let (_, rl) = app.post(&admin, "/relays", json!({"name": "r1"})).await;
    app.post(&admin, "/projects/PAY/tasks", json!({"title": "Wire CI"}))
        .await;
    World {
        a1: toks[0].clone(),
        a2: toks[1].clone(),
        b1: toks[2].clone(),
        relay: rl["token"].as_str().unwrap().into(),
        admin,
        app,
    }
}

impl World {
    async fn tool(&self, token: &str, tool: &str, args: Value) -> Value {
        let r = self.app.mcp(token, tool, args).await;
        assert_ne!(r["isError"], true, "{tool}: {r}");
        common::TestApp::payload(&r)
    }
    async fn tool_err(&self, token: &str, tool: &str, args: Value) -> String {
        let r = self.app.mcp(token, tool, args).await;
        assert_eq!(r["isError"], true, "{tool} should fail: {r}");
        r["content"][0]["text"].as_str().unwrap_or("").to_string()
    }
}

fn keys(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|m| m["key"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn agents_share_a_pantry_per_project() {
    let w = world().await;

    // a1 saves a how-to; the key comes from the title.
    let m = w
        .tool(
            &w.a1,
            "remember",
            json!({"project": "pay", "title": "How to run the e2e tests",
                   "body": "`just e2e` needs Postgres: run `just setup` first. Tests create a database per test.",
                   "kind": "howto", "tags": "testing, CI", "task": "PAY-1"}),
        )
        .await;
    assert_eq!(m["key"], "how-to-run-the-e2e-tests", "{m}");
    assert_eq!(
        (&m["revision"], &m["kind"], &m["task"]),
        (&json!(1), &json!("howto"), &json!("PAY-1"))
    );
    assert_eq!(m["tags"], json!(["ci", "testing"]));
    assert_eq!(m["created_by"], "a1");
    w.tool(
        &w.a1,
        "remember",
        json!({"project": "PAY", "key": "money", "title": "Amounts are integer cents",
               "body": "Never use floats for money: amounts are i64 cents everywhere (see ledger/amount.rs).",
               "kind": "convention"}),
    )
    .await;

    // a2 (same team) finds them; ranking puts the best match first.
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "query": "postgres e2e"}),
        )
        .await;
    assert_eq!(keys(&r), ["how-to-run-the-e2e-tests"], "{r}");
    assert!(r[0]["rank"].as_f64().unwrap() > 0.0);
    // stemming: "floating" matches "floats"
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "query": "floating amounts"}),
        )
        .await;
    assert_eq!(keys(&r), ["money"], "{r}");
    // no memory has every word: fall back to any word
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "query": "e2e kubernetes"}),
        )
        .await;
    assert_eq!(keys(&r), ["how-to-run-the-e2e-tests"], "{r}");
    let r = w
        .tool(&w.a2, "recall", json!({"project": "PAY", "query": "zebra"}))
        .await;
    assert!(keys(&r).is_empty());
    // filters; without a query: most recent first, no rank
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "tags": "testing"}),
        )
        .await;
    assert_eq!(keys(&r), ["how-to-run-the-e2e-tests"]);
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "kind": "convention"}),
        )
        .await;
    assert_eq!(keys(&r), ["money"]);
    let r = w.tool(&w.a2, "recall", json!({"project": "PAY"})).await;
    assert_eq!(keys(&r), ["money", "how-to-run-the-e2e-tests"]);
    assert!(r[0]["rank"].is_null(), "{r}");
    let err = w
        .tool_err(&w.a2, "recall", json!({"project": "PAY", "kind": "rumour"}))
        .await;
    assert!(err.contains("unknown kind"), "{err}");

    // a2 appends to a1's memory, passing the revision it read.
    let m = w
        .tool(
            &w.a2,
            "remember",
            json!({"project": "PAY", "key": "how-to-run-the-e2e-tests", "append": true, "revision": 1,
                   "body": "On CI the database is the `postgres` service container."}),
        )
        .await;
    assert_eq!(m["revision"], 2);
    assert_eq!(m["title"], "How to run the e2e tests");
    assert!(m["body"].as_str().unwrap().ends_with("service container."));
    assert!(m["body"].as_str().unwrap().starts_with("`just e2e`"));
    assert_eq!(
        (&m["created_by"], &m["updated_by"]),
        (&json!("a1"), &json!("a2"))
    );
    // a1 still holds revision 1: its overwrite is refused instead of losing a2's addition.
    let err = w
        .tool_err(
            &w.a1,
            "remember",
            json!({"project": "PAY", "key": "how-to-run-the-e2e-tests", "revision": 1, "body": "stale"}),
        )
        .await;
    assert!(err.contains("revision 2"), "{err}");
    // revision 0 = must be new
    let err = w
        .tool_err(
            &w.a1,
            "remember",
            json!({"project": "PAY", "key": "money", "revision": 0, "title": "x", "body": "y"}),
        )
        .await;
    assert!(err.contains("revision"), "{err}");
    // the overwritten version is kept
    let kept: i64 = sqlx::query_scalar("select count(*) from memory_revisions")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(kept, 1);
    // saving the same content again changes nothing
    let same = w
        .tool(
            &w.a1,
            "remember",
            json!({"project": "PAY", "key": "money", "title": "Amounts are integer cents",
                   "body": "Never use floats for money: amounts are i64 cents everywhere (see ledger/amount.rs)."}),
        )
        .await;
    assert_eq!(same["revision"], 1);

    // long bodies come back as excerpts from recall, in full from get_memory
    let long = "word ".repeat(400);
    w.tool(
        &w.a1,
        "remember",
        json!({"project": "PAY", "title": "Long notes", "body": long}),
    )
    .await;
    let r = w
        .tool(
            &w.a2,
            "recall",
            json!({"project": "PAY", "query": "long notes"}),
        )
        .await;
    assert_eq!(r[0]["truncated"], true, "{r}");
    let full = w
        .tool(
            &w.a2,
            "get_memory",
            json!({"project": "PAY", "key": "long-notes"}),
        )
        .await;
    assert_eq!(full["body"].as_str().unwrap(), long.trim());
    assert!(full["truncated"].is_null());

    // the digest: grouped by kind, conventions first
    let d = w
        .tool(&w.a2, "read_pantry", json!({"project": "PAY"}))
        .await;
    let md = d["markdown"].as_str().unwrap();
    assert!(md.starts_with("# Payments pantry"), "{md}");
    let (c, h) = (
        md.find("## Conventions").unwrap(),
        md.find("## How-tos").unwrap(),
    );
    assert!(c < h, "{md}");
    assert!(md.contains("`how-to-run-the-e2e-tests` · ci, testing"));

    // visibility: another team's agent cannot see the project, relays see nothing
    let err = w.tool_err(&w.b1, "recall", json!({"project": "PAY"})).await;
    assert!(err.contains("not found"), "{err}");
    let err = w
        .tool_err(
            &w.b1,
            "remember",
            json!({"project": "PAY", "title": "x", "body": "y"}),
        )
        .await;
    assert!(err.contains("not found"), "{err}");
    let (st, _) = w.app.get(&w.relay, "/projects/PAY/pantry").await;
    assert_eq!(st, 403);
    let r = w.tool(&w.b1, "recall", json!({"project": "OPS"})).await;
    assert!(keys(&r).is_empty());

    // forget: an agent only forgets its own memories
    let err = w
        .tool_err(&w.a2, "forget", json!({"project": "PAY", "key": "money"}))
        .await;
    assert!(err.contains("someone else"), "{err}");
    w.tool(&w.a1, "forget", json!({"project": "PAY", "key": "money"}))
        .await;
    let err = w
        .tool_err(
            &w.a1,
            "get_memory",
            json!({"project": "PAY", "key": "money"}),
        )
        .await;
    assert!(err.contains("no memory"), "{err}");

    // the change feed carries pantry events
    let feed = w
        .tool(
            &w.a2,
            "changes_since",
            json!({"cursor": 0, "project": "PAY"}),
        )
        .await;
    let kinds: Vec<(String, Option<String>)> = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].as_str().unwrap().starts_with("memory."))
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_string(),
                e["payload"]["key"].as_str().map(String::from),
            )
        })
        .collect();
    assert_eq!(
        kinds.first(),
        Some(&(
            "memory.saved".into(),
            Some("how-to-run-the-e2e-tests".into())
        ))
    );
    assert_eq!(
        kinds.last(),
        Some(&("memory.forgotten".into(), Some("money".into())))
    );
}

#[tokio::test]
async fn pantry_over_rest_and_the_generated_api() {
    let w = world().await;
    let rest = |t: &str, m: Method, p: &str, b: Option<Value>| {
        let (t, p) = (t.to_string(), p.to_string());
        let app = &w.app;
        async move { app.rest(&t, m, &p, b).await }
    };

    // PUT under an explicit key, POST derives it
    let (st, m) = rest(
        &w.a1,
        Method::PUT,
        "/projects/PAY/pantry/Build",
        Some(json!({"title": "Build", "body": "cargo build --workspace", "kind": "howto", "tags": ["rust"]})),
    )
    .await;
    assert_eq!((st, m["key"].as_str()), (200, Some("build")), "{m}");
    let (st, m) = rest(
        &w.a1,
        Method::POST,
        "/projects/PAY/pantry",
        Some(json!({"title": "Deploys go through Argo", "body": "Never kubectl apply by hand.", "kind": "gotcha"})),
    )
    .await;
    assert_eq!(
        (st, m["key"].as_str()),
        (200, Some("deploys-go-through-argo"))
    );
    let (st, _) = rest(
        &w.a1,
        Method::POST,
        "/projects/PAY/pantry",
        Some(json!({"title": "empty", "body": "  "})),
    )
    .await;
    assert_eq!(st, 400);

    let (_, r) = w.app.get(&w.a2, "/projects/pay/pantry?q=argo").await;
    assert_eq!(keys(&r), ["deploys-go-through-argo"]);
    let (_, r) = w.app.get(&w.a2, "/projects/PAY/pantry?tags=rust").await;
    assert_eq!(keys(&r), ["build"]);
    let (_, m) = w.app.get(&w.a2, "/projects/PAY/pantry/build").await;
    assert_eq!(m["body"], "cargo build --workspace");
    let (_, d) = w
        .app
        .get(&w.a2, "/projects/PAY/pantry?format=markdown")
        .await;
    assert!(d["markdown"].as_str().unwrap().contains("## Gotchas"));

    // the human (admin) can forget anyone's memory
    let (st, _) = rest(&w.admin, Method::DELETE, "/projects/PAY/pantry/build", None).await;
    assert_eq!(st, 200);
    let (st, _) = w.app.get(&w.a2, "/projects/PAY/pantry/build").await;
    assert_eq!(st, 404);

    // the Memory model under /api/v2 follows the project's policy
    let v2 = |t: &str| {
        let t = t.to_string();
        let app = &w.app;
        async move {
            let r = app
                .http
                .get(format!("{}/api/v2/memories", app.base))
                .bearer_auth(&t)
                .send()
                .await
                .unwrap();
            let st = r.status().as_u16();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            let n = match &v {
                Value::Array(a) => a.len(),
                _ => v["data"]
                    .as_array()
                    .or(v["items"].as_array())
                    .map_or(0, |a| a.len()),
            };
            (st, n)
        }
    };
    assert_eq!(v2(&w.a2).await, (200, 1));
    assert_eq!(v2(&w.b1).await, (200, 0));
}

#[tokio::test]
async fn pantry_tools_describe_their_arguments() {
    let w = world().await;
    // the legacy front door reuses the generated table, so it shows what every client sees
    let r: Value = w
        .app
        .http
        .post(format!("{}/mcp", w.app.base))
        .bearer_auth(&w.a1)
        .header("MCP-Protocol-Version", "2025-06-18")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let tool = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
    for n in ["remember", "recall", "get_memory", "read_pantry", "forget"] {
        assert!(tool(n)["description"].as_str().unwrap().len() > 20, "{n}");
    }
    let remember = tool("remember");
    let props = &remember["inputSchema"]["properties"];
    for arg in [
        "project", "title", "body", "key", "kind", "append", "revision",
    ] {
        assert!(
            props[arg]["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "remember.{arg} has no description: {props}"
        );
    }
    assert_eq!(
        remember["inputSchema"]["required"],
        json!(["project", "body"])
    );
    // `///` docs on the output type's fields reach the output schema (array outputs have none)
    let out = &tool("get_memory")["outputSchema"];
    assert!(
        out.to_string().contains("pass it back to remember"),
        "{out}"
    );
}
