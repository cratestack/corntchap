//! What corntchap sends to agents is counted per cook, pot and day (`usage_daily`), on both MCP
//! paths (handshake-era and 2026-07-28), for tool listings and for wake prompts, and
//! `usage_report` shows each person only their own cooks.
mod common;
use corntchap::{admin, questions};
use serde_json::{Value, json};
use uuid::Uuid;

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

fn text_chars(result: &Value) -> i64 {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no result text: {result}"))
        .chars()
        .count() as i64
}

/// The cooks (and people) whose usage `token` may see, by name: `usageCookPage` with these `args`.
async fn cooks_seen(app: &common::TestApp, token: &str, mut args: Value) -> Vec<String> {
    args["page"] = json!({"limit": 200});
    let r = app
        .http
        .post(format!("{}/api/v2/$procs/usageCookPage", app.base))
        .bearer_auth(token)
        .json(&args)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let mut names: Vec<String> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["cook"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// A handshake-era tools/call or tools/list; `url` is `/mcp` or `/mcp/<POT>`.
async fn legacy(
    app: &common::TestApp,
    token: &str,
    url: &str,
    method: &str,
    params: Value,
) -> Value {
    let r: Value = app
        .http
        .post(format!("{}{url}", app.base))
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r["result"].clone()
}

/// A 2026-07-28 request, as cratestack serves it.
async fn modern(
    app: &common::TestApp,
    token: &str,
    url: &str,
    method: &str,
    params: Value,
) -> Value {
    let mut params = params;
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                             "io.modelcontextprotocol/clientCapabilities": {}});
    let r: Value = app
        .http
        .post(format!("{}{url}", app.base))
        .bearer_auth(token)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .header("Mcp-Name", params["name"].as_str().unwrap_or_default())
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    r["result"].clone()
}

/// (calls, chars) of the rows matching, waiting for the (asynchronous) recording to land.
async fn counted(
    app: &common::TestApp,
    cook: &str,
    pot: Option<&str>,
    kind: &str,
    name: &str,
    calls: i64,
) -> (i64, i64) {
    for _ in 0..100 {
        let row: (i64, i64) = sqlx::query_as(
            "select coalesce(sum(u.calls), 0)::bigint, coalesce(sum(u.chars), 0)::bigint
             from usage_daily u join principals a on a.id = u.principal_id
             left join projects pr on pr.id = u.project_id
             where a.name = $1 and u.kind = $3 and u.name = $4
               and (($2::text is null and u.project_id is null) or pr.key = $2)",
        )
        .bind(cook)
        .bind(pot)
        .bind(kind)
        .bind(name)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
        if row.0 >= calls {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("{cook} {pot:?} {kind} {name}: expected {calls} calls");
}

#[tokio::test]
async fn usage_is_counted_per_cook_pot_and_kind_and_shown_to_its_owner_only() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    let call = |tok: &str, tool: &str, args: Value| {
        let (tok, tool, app) = (tok.to_string(), tool.to_string(), &app);
        async move { app.mcp(&tok, &tool, args).await }
    };
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    for (key, name) in [("PAY", "Payments"), ("OPS", "Operations")] {
        app.post(
            &root,
            "/projects",
            json!({"team": "acme", "key": key, "name": name}),
        )
        .await;
    }
    app.post(
        &root,
        "/projects/PAY/tasks",
        json!({"title": "Do it", "items": ["one"]}),
    )
    .await;
    app.add_user("alice").await;
    app.add_user("bob").await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    let mut cooks = vec![];
    for (owner_tok, cook) in [(&alice, "c1"), (&bob, "b1")] {
        let made = ok(
            &call(
                owner_tok,
                "create_cook",
                json!({"name": cook, "agentKind": "other", "runtime": "local"}),
            )
            .await,
            "create_cook",
        );
        cooks.push(made["token"].as_str().unwrap().to_string());
        ok(
            &call(
                &root,
                "add_team_member",
                json!({"team": "acme", "principal": cook}),
            )
            .await,
            "member",
        );
    }
    let (c1, b1) = (cooks[0].clone(), cooks[1].clone());

    // --- handshake-era path: the pot from `project`, from a task ref, from the pot's URL ---
    let board = legacy(
        &app,
        &c1,
        "/mcp",
        "tools/call",
        json!({"name": "get_board", "arguments": {"project": "PAY"}}),
    )
    .await;
    let by_ref = legacy(
        &app,
        &c1,
        "/mcp",
        "tools/call",
        json!({"name": "get_task", "arguments": {"id": "PAY-1"}}),
    )
    .await;
    let on_url = legacy(
        &app,
        &c1,
        "/mcp/OPS",
        "tools/call",
        json!({"name": "list_tasks", "arguments": {}}),
    )
    .await;
    let list = legacy(&app, &c1, "/mcp", "tools/list", json!({})).await;
    // output schemas still go out (clients validate results with them) but are not counted: a listing
    // is metered as models see it (name, description, input schema)
    assert!(
        list["tools"][0].get("outputSchema").is_some()
            || list["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t.get("outputSchema").is_some())
    );
    let mut seen = list.clone();
    for t in seen["tools"].as_array_mut().unwrap() {
        let o = t.as_object_mut().unwrap();
        o.remove("outputSchema");
        o.remove("_meta");
        o.remove("annotations");
    }
    let list_chars = seen.to_string().chars().count() as i64;
    assert!(
        list_chars * 2 < list.to_string().chars().count() as i64,
        "the catalog is well under the wire size"
    );

    // --- 2026-07-28 path: every tools/call response is measured, and still arrives intact ---
    let m_board = modern(
        &app,
        &c1,
        "/mcp",
        "tools/call",
        json!({"name": "get_board", "arguments": {"project": "PAY"}}),
    )
    .await;
    assert_eq!(text_chars(&m_board), text_chars(&board), "{m_board}");
    let m_task = modern(
        &app,
        &c1,
        "/mcp",
        "tools/call",
        json!({"name": "get_task", "arguments": {"id": "PAY-1"}}),
    )
    .await;
    let m_list = modern(&app, &c1, "/mcp/OPS", "tools/list", json!({})).await;
    assert!(m_list["tools"].as_array().unwrap().len() > 10, "{m_list}");

    // --- bob's cook, another pot's call ---
    let b_board = legacy(
        &app,
        &b1,
        "/mcp",
        "tools/call",
        json!({"name": "get_board", "arguments": {"project": "OPS"}}),
    )
    .await;

    // legacy get_board + modern get_board, both PAY
    let (calls, chars) = counted(&app, "c1", Some("PAY"), "tool", "get_board", 2).await;
    assert_eq!(
        (calls, chars),
        (2, text_chars(&board) + text_chars(&m_board))
    );
    // the pot of a task ref
    let (calls, chars) = counted(&app, "c1", Some("PAY"), "tool", "get_task", 2).await;
    assert_eq!(
        (calls, chars),
        (2, text_chars(&by_ref) + text_chars(&m_task))
    );
    // the pot of the URL
    let (calls, chars) = counted(&app, "c1", Some("OPS"), "tool", "list_tasks", 1).await;
    assert_eq!((calls, chars), (1, text_chars(&on_url)));
    // listings: no pot on /mcp, the pot on its URL
    let (calls, chars) = counted(&app, "c1", None, "list", "tools/list", 1).await;
    assert_eq!((calls, chars), (1, list_chars));
    let (calls, chars) = counted(&app, "c1", Some("OPS"), "list", "tools/list", 1).await;
    assert_eq!(calls, 1);
    assert!(chars > 1000, "{chars}");
    counted(&app, "b1", Some("OPS"), "tool", "get_board", 1).await;
    let _ = b_board;

    // --- a wake prompt: counted by reason, the text as the relay will receive it ---
    let ids: (Uuid, Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'c1'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    let mut conn = app.state.db.acquire().await.unwrap();
    questions::wake(
        &mut conn,
        ids.1,
        "PAY",
        ids.0,
        None,
        None,
        "Your answer is in.",
        "answer",
    )
    .await
    .unwrap();
    let sent: Value = sqlx::query_scalar(
        "select payload from events where kind = 'wake' order by seq desc limit 1",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    let wake_chars = sent["text"].as_str().unwrap().chars().count() as i64;
    let (calls, chars) = counted(&app, "c1", Some("PAY"), "wake", "answer", 1).await;
    assert_eq!((calls, chars), (1, wake_chars));

    // --- usage_report: c1 sees itself ---
    let me = ok(&call(&c1, "usage_report", json!({})).await, "c1 report");
    assert_eq!(me["days"], 7);
    assert!(me["note"].as_str().unwrap().contains("estimate"));
    // 5 tool calls + 2 listings (+ the report call itself is recorded after it answers) + 1 wake
    assert!(me["totals"]["calls"].as_i64().unwrap() >= 8, "{me}");
    let total = me["totals"]["chars"].as_i64().unwrap();
    assert_eq!(me["totals"]["tokens"].as_i64().unwrap(), (total + 3) / 4);
    assert_eq!(me["by_day"].as_array().unwrap().len(), 7);
    assert_eq!(cooks_seen(&app, &c1, json!({})).await[0], "c1");
    let top: Vec<(&str, &str)> = me["top"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["kind"].as_str().unwrap(), t["name"].as_str().unwrap()))
        .collect();
    assert!(top.contains(&("list", "tools/list")), "{top:?}");
    assert!(top.contains(&("wake", "answer")), "{top:?}");
    assert!(top.contains(&("tool", "get_board")), "{top:?}");
    // one pot only: the OPS pot has a listing and a list_tasks call
    let ops = ok(
        &call(&c1, "usage_report", json!({"project": "ops", "days": 1})).await,
        "ops",
    );
    assert_eq!(ops["by_day"].as_array().unwrap().len(), 1);
    assert_eq!(ops["totals"]["calls"], 2, "{ops}");

    // --- alice owns c1 (and her own calls count under her name) ---
    assert_eq!(cooks_seen(&app, &alice, json!({})).await, ["alice", "c1"]);
    // bob sees himself and b1, never alice's cook: asking for c1 by name finds nothing
    assert_eq!(cooks_seen(&app, &bob, json!({})).await, ["b1", "bob"]);
    let peek = ok(
        &call(&bob, "usage_report", json!({"cook": "c1"})).await,
        "bob peeks",
    );
    assert_eq!(peek["totals"]["calls"], 0, "{peek}");
    assert!(cooks_seen(&app, &bob, json!({"q": "c1"})).await.is_empty());
    let peek = ok(
        &call(&bob, "usage_report", json!({"project": "PAY"})).await,
        "bob peeks at the pot",
    );
    assert_eq!(peek["totals"]["calls"], 0, "{peek}");
    // and b1 (a cook) sees only itself
    assert_eq!(cooks_seen(&app, &b1, json!({})).await, ["b1"]);
    // an admin sees everyone
    let everyone = cooks_seen(&app, &root, json!({})).await;
    for who in ["alice", "b1", "bob", "c1"] {
        assert!(
            everyone.iter().any(|c| c == who),
            "{who} missing: {everyone:?}"
        );
    }

    // --- retention: usage older than 90 days is pruned ---
    sqlx::query(
        "insert into usage_daily (day, principal_id, kind, name, calls, chars)
         select current_date - 100, id, 'tool', 'old', 1, 10 from principals where name = 'c1'",
    )
    .execute(&app.state.db)
    .await
    .unwrap();
    corntchap::jobs::retain(&app.state).await.unwrap();
    let old: i64 = sqlx::query_scalar("select count(*) from usage_daily where name = 'old'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(old, 0);
}
