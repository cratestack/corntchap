//! Menus (milestones): create, put tasks on them, progress, focus claim_next, serve, reopen.
mod common;
use corntchap::admin;
use reqwest::Method;
use serde_json::{Value, json};

async fn tool(app: &common::TestApp, tok: &str, name: &str, args: Value) -> Value {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_ne!(r["isError"], true, "{name}({args}): {r}");
    common::TestApp::payload(&r)
}
async fn tool_err(app: &common::TestApp, tok: &str, name: &str, args: Value) -> String {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_eq!(r["isError"], true, "{name}({args}) should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

#[tokio::test]
async fn menus_group_tasks_and_track_progress() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let admin = tok.unwrap();
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&admin, "/teams", json!({"name": team})).await;
        app.post(
            &admin,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    let mut toks = vec![];
    for (name, team) in [("a1", "acme"), ("b1", "other")] {
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
    let (a1, b1) = (&toks[0], &toks[1]);

    // create: key from the name, positions in order, due date checked
    let mvp = tool(
        &app,
        a1,
        "create_menu",
        json!({"project": "pay", "name": "MVP", "description": "first slice", "dueOn": "2026-12-15"}),
    )
    .await;
    assert_eq!(
        (
            &mvp["key"],
            &mvp["status"],
            &mvp["due_on"],
            &mvp["position"]
        ),
        (
            &json!("mvp"),
            &json!("open"),
            &json!("2026-12-15"),
            &json!(1)
        )
    );
    let v12 = tool(
        &app,
        a1,
        "create_menu",
        json!({"project": "PAY", "name": "v1.2 launch"}),
    )
    .await;
    assert_eq!(
        (&v12["key"], &v12["position"]),
        (&json!("v1.2-launch"), &json!(2))
    );
    let err = tool_err(
        &app,
        a1,
        "create_menu",
        json!({"project": "PAY", "name": "mvp"}),
    )
    .await;
    assert!(err.contains("already has a menu mvp"), "{err}");
    let err = tool_err(
        &app,
        a1,
        "create_menu",
        json!({"project": "PAY", "name": "x", "dueOn": "next week"}),
    )
    .await;
    assert!(err.contains("YYYY-MM-DD"), "{err}");

    // tasks on menus
    for (title, menu, prio) in [
        ("a", Some("MVP"), 0),
        ("b", Some("mvp"), 0),
        ("c", Some("v1.2-launch"), 9),
        ("d", None, 5),
    ] {
        let mut args = json!({"project": "PAY", "title": title, "priority": prio});
        if let Some(m) = menu {
            args["menu"] = json!(m);
        }
        let t = tool(&app, a1, "create_task", args).await;
        assert_eq!(
            t["menu"],
            menu.map(|m| m.to_lowercase())
                .map_or(Value::Null, Value::from),
            "{t}"
        );
    }
    let err = tool_err(
        &app,
        a1,
        "create_task",
        json!({"project": "PAY", "title": "e", "menu": "nope"}),
    )
    .await;
    assert!(err.contains("no menu nope"), "{err}");
    let ids = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    let on_mvp = tool(
        &app,
        a1,
        "list_tasks",
        json!({"project": "PAY", "menu": "mvp"}),
    )
    .await;
    assert_eq!(ids(&on_mvp), ["PAY-1", "PAY-2"]);
    let none = tool(
        &app,
        a1,
        "list_tasks",
        json!({"project": "PAY", "menu": "-"}),
    )
    .await;
    assert_eq!(ids(&none), ["PAY-4"]);

    // claim_next focused on a menu skips the higher-priority tasks elsewhere
    let c = tool(
        &app,
        a1,
        "claim_next",
        json!({"project": "PAY", "menu": "MVP"}),
    )
    .await;
    assert_eq!(c["task"]["id"], "PAY-1", "{c}");
    assert_eq!(c["task"]["menu"], "mvp");
    let e = c["lease"]["epoch"].as_i64().unwrap();
    tool(
        &app,
        a1,
        "complete",
        json!({"id": "PAY-1", "epoch": e, "result": "ok"}),
    )
    .await;

    // progress
    let menus = tool(&app, a1, "list_menus", json!({"project": "PAY"})).await;
    let m = &menus[0];
    assert_eq!(
        (&m["key"], &m["total"], &m["done"], &m["open"]),
        (&json!("mvp"), &json!(2), &json!(1), &json!(1)),
        "{menus}"
    );
    assert_eq!(m["progress"].as_f64().unwrap(), 0.5);

    // move a task between menus; off a menu with ""
    let t = tool(
        &app,
        &admin,
        "update_task",
        json!({"id": "PAY-4", "menu": "mvp"}),
    )
    .await;
    assert_eq!(t["menu"], "mvp");
    let t = tool(
        &app,
        &admin,
        "update_task",
        json!({"id": "PAY-4", "menu": ""}),
    )
    .await;
    assert!(t["menu"].is_null(), "{t}");

    // serve: hidden from the default list, no new tasks on it, reopen brings it back
    let served = tool(
        &app,
        a1,
        "update_menu",
        json!({"project": "PAY", "key": "v1.2-launch", "status": "served", "dueOn": ""}),
    )
    .await;
    assert_eq!(served["status"], "served");
    assert!(
        served["served_at"].is_string() && served["due_on"].is_null(),
        "{served}"
    );
    let open: Vec<String> = tool(&app, a1, "list_menus", json!({"project": "PAY"}))
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(open, ["mvp"]);
    let all = tool(
        &app,
        a1,
        "list_menus",
        json!({"project": "PAY", "includeServed": true}),
    )
    .await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    let err = tool_err(
        &app,
        a1,
        "create_task",
        json!({"project": "PAY", "title": "late", "menu": "v1.2-launch"}),
    )
    .await;
    assert!(err.contains("already served"), "{err}");
    let reopened = tool(
        &app,
        a1,
        "update_menu",
        json!({"project": "PAY", "key": "v1.2-launch", "status": "open"}),
    )
    .await;
    assert!(reopened["served_at"].is_null());
    let err = tool_err(
        &app,
        a1,
        "update_menu",
        json!({"project": "PAY", "key": "mvp", "status": "eaten"}),
    )
    .await;
    assert!(err.contains("open or served"), "{err}");

    // events
    let feed = tool(
        &app,
        a1,
        "changes_since",
        json!({"cursor": 0, "project": "PAY", "limit": 500}),
    )
    .await;
    let kinds: Vec<&str> = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .filter(|k| k.starts_with("menu."))
        .collect();
    assert_eq!(
        kinds,
        [
            "menu.created",
            "menu.created",
            "menu.served",
            "menu.reopened"
        ]
    );

    // visibility: another team sees nothing; REST and /api/v2 agree
    let err = tool_err(&app, b1, "list_menus", json!({"project": "PAY"})).await;
    assert!(err.contains("not found"), "{err}");
    let (st, v) = app.get(a1, "/projects/PAY/menus?all=true").await;
    assert_eq!((st, v.as_array().unwrap().len()), (200, 2));
    let (st, m) = app
        .rest(
            a1,
            Method::PATCH,
            "/projects/PAY/menus/mvp",
            Some(json!({"name": "MVP (beta)", "position": 7})),
        )
        .await;
    assert_eq!(
        (st, &m["name"], &m["position"]),
        (200, &json!("MVP (beta)"), &json!(7))
    );
    let (st, _) = app
        .post(
            a1,
            "/projects/PAY/menus",
            json!({"name": "Hardening", "due_on": "2027-01-31"}),
        )
        .await;
    assert_eq!(st, 200);
    let count = |tok: &str| {
        let (tok, app) = (tok.to_string(), &app);
        async move {
            let v: Value = app
                .http
                .get(format!("{}/api/v2/menus", app.base))
                .bearer_auth(&tok)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            match &v {
                Value::Array(a) => a.len(),
                _ => v["data"]
                    .as_array()
                    .or(v["items"].as_array())
                    .map_or(0, |a| a.len()),
            }
        }
    };
    assert_eq!(count(a1).await, 3);
    assert_eq!(count(b1).await, 0);
}
