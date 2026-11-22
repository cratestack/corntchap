//! Saved views: a checked SQL condition and order over a pot's tasks, written by people or cooks,
//! run read-only and only over that pot.
mod common;
use corntchap::admin;
use reqwest::Method;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}
fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn sql_views() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "eve"] {
        app.add_user(u).await;
    }
    for (team, who) in [("acme", "alice"), ("other", "eve")] {
        app.post(&root, "/teams", json!({"name": team})).await;
        app.post(
            &root,
            &format!("/teams/{team}/members"),
            json!({"principal": who}),
        )
        .await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let eve = admin::operator_token(&app.state.db, "eve", None)
        .await
        .unwrap();
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(
            &root,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    let (st, cook) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200, "{cook}");
    let c1 = cook["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    app.post(&alice, "/projects/PAY/menus", json!({"name": "MVP"}))
        .await;
    for (title, labels, prio, menu) in [
        ("refund screen", vec!["urgent"], 5, Some("mvp")),
        ("receipt logo", vec![], 1, Some("mvp")),
        ("billing export", vec!["urgent"], 3, None),
        ("tidy readme", vec![], 0, None),
    ] {
        app.post(
            &alice,
            "/projects/PAY/tasks",
            json!({"title": title, "labels": labels, "priority": prio, "menu": menu}),
        )
        .await;
    }
    app.post(
        &eve,
        "/projects/OPS/tasks",
        json!({"title": "urgent ops thing", "labels": ["urgent"]}),
    )
    .await;

    // --- try a condition, then save it (a cook builds it for its people) ---
    let tried = ok(
        &app.mcp(&c1, "run_view", json!({"project": "PAY", "where": "'urgent' = any(labels)", "orderBy": "priority desc"}))
            .await,
        "try",
    );
    assert_eq!(
        ids(&tried),
        ["PAY-1", "PAY-3"],
        "only this pot's urgent tasks"
    );
    let v = ok(
        &app.mcp(
            &c1,
            "save_view",
            json!({"project": "PAY", "name": "Urgent on the MVP", "where": "menu = 'mvp' and 'urgent' = any(labels) and status <> 'done'", "layout": "board"}),
        )
        .await,
        "save",
    );
    assert_eq!(
        (&v["key"], &v["layout"], &v["created_by"]),
        (&json!("urgent-on-the-mvp"), &json!("board"), &json!("c1"))
    );
    let run = ok(
        &app.mcp(
            &alice,
            "run_view",
            json!({"project": "PAY", "key": "urgent-on-the-mvp"}),
        )
        .await,
        "run saved",
    );
    assert_eq!(ids(&run), ["PAY-1"]);
    assert_eq!(run[0]["title"], "refund screen");
    // same key: updated, not duplicated
    ok(
        &app.mcp(&alice, "save_view", json!({"project": "PAY", "name": "Urgent on the MVP", "where": "'urgent' = any(labels)"}))
            .await,
        "update",
    );
    let list = ok(
        &app.mcp(&alice, "list_views", json!({"project": "PAY"}))
            .await,
        "list",
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["where"], "'urgent' = ANY(labels)");

    // --- what a view's SQL cannot do ---
    for (bad, why) in [
        ("true) union select token_hash from tokens --", "escape"),
        ("exists (select 1 from principals)", "subquery"),
        ("id in (select key from projects)", "IN subquery"),
        ("pg_sleep(5) is null", "function"),
        ("token_hash is not null", "unknown column"),
        ("status = $1", "placeholder"),
        ("status = 'open'; delete from tasks", "second statement"),
    ] {
        refused(
            &app.mcp(
                &c1,
                "save_view",
                json!({"project": "PAY", "name": "bad", "where": bad}),
            )
            .await,
            why,
        );
        refused(
            &app.mcp(&c1, "run_view", json!({"project": "PAY", "where": bad}))
                .await,
            why,
        );
    }
    // a condition that parses but does not run (a type error) is refused too
    let e = refused(
        &app.mcp(
            &c1,
            "save_view",
            json!({"project": "PAY", "name": "typo", "where": "priority = 'high'"}),
        )
        .await,
        "type error",
    );
    assert!(e.contains("does not run"), "{e}");
    let (_, n) = app.get(&root, "/projects/PAY/tasks").await;
    assert_eq!(
        n.as_array().map(|a| a.len()),
        Some(4),
        "nothing was touched"
    );

    // --- other teams see neither the views nor the tasks ---
    refused(
        &app.mcp(&eve, "list_views", json!({"project": "PAY"})).await,
        "other team",
    );
    refused(
        &app.mcp(&eve, "run_view", json!({"project": "PAY", "where": "true"}))
            .await,
        "other team runs",
    );
    let model = |t: String| {
        let app = &app;
        async move {
            let r = app
                .http
                .request(Method::GET, format!("{}/api/v2/pot_views", app.base))
                .bearer_auth(&t)
                .send()
                .await
                .unwrap();
            let v: Value = r.json().await.unwrap();
            v["items"]
                .as_array()
                .or_else(|| v["data"].as_array())
                .or_else(|| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        }
    };
    assert_eq!(model(alice.clone()).await, 1);
    assert_eq!(model(eve.clone()).await, 0);

    // --- deleting: people, or the cook that made it ---
    ok(
        &app.mcp(
            &alice,
            "save_view",
            json!({"project": "PAY", "name": "Mine", "where": "true"}),
        )
        .await,
        "alice's view",
    );
    let e = refused(
        &app.mcp(&c1, "delete_view", json!({"project": "PAY", "key": "mine"}))
            .await,
        "a cook deletes a person's view",
    );
    assert!(e.contains("only the views it made"), "{e}");
    ok(
        &app.mcp(
            &c1,
            "delete_view",
            json!({"project": "PAY", "key": "urgent-on-the-mvp"}),
        )
        .await,
        "its own",
    );
    ok(
        &app.mcp(
            &alice,
            "delete_view",
            json!({"project": "PAY", "key": "mine"}),
        )
        .await,
        "a person deletes",
    );
    assert_eq!(model(alice.clone()).await, 0);
}
