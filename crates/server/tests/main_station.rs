//! A cook's main station in a pot answers its messages and mentions (issue #23): chosen
//! deliberately, kept across re-registration, at most one per cook and pot, and the recency
//! fallback stays when none is set.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

async fn last_wake_station(app: &common::TestApp) -> Value {
    let p: Value = sqlx::query_scalar(
        "select payload from events where kind = 'wake' order by seq desc limit 1",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    p["station"]["id"].clone()
}

#[tokio::test]
async fn the_main_station_answers() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/users", json!({"login": "alice"})).await;
    app.post(&root, "/users", json!({"login": "eve"})).await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams/acme/members", json!({"principal": "alice"}))
        .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let eve = admin::operator_token(&app.state.db, "eve", None)
        .await
        .unwrap();
    for key in ["PAY", "OPS"] {
        app.post(
            &root,
            "/projects",
            json!({"team": "acme", "key": key, "name": key}),
        )
        .await;
    }
    let (_, cook) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "doug", "kind": "opencode", "runtime": "local"}),
        )
        .await;
    let doug = cook["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "doug"}))
        .await;
    let mut st = vec![];
    for sess in ["ses_a", "ses_b", "ses_c"] {
        let (code, v) = app
            .post(
                &alice,
                "/stations",
                json!({"cook": "doug", "project": "PAY", "runtime": "opencode", "session": sess, "label": sess}),
            )
            .await;
        assert_eq!(code, 200, "{v}");
        assert_eq!(v["main"], false);
        st.push(v["id"].as_str().unwrap().to_string());
    }
    // ses_c was woken most recently: without a main station it answers (unchanged behaviour)
    sqlx::query("update stations set last_woken_at = now() - (interval '1 minute' * (3 - (select count(*) from stations s2 where s2.created_at <= stations.created_at)))")
        .execute(&app.state.db)
        .await
        .unwrap();
    let dm = |who: String, text: &'static str| {
        let app = &app;
        async move {
            ok(
                &app.mcp(
                    &who,
                    "pass",
                    json!({"project": "PAY", "message": text, "to": "doug"}),
                )
                .await,
                text,
            )
        }
    };
    dm(alice.clone(), "first").await;
    assert_eq!(last_wake_station(&app).await, json!(st[2]), "recency");

    // alice makes ses_a the main station: it answers even though ses_c was woken later
    let (code, v) = app
        .post(&alice, &format!("/stations/{}/main", st[0]), json!({}))
        .await;
    assert_eq!((code, &v["main"]), (200, &json!(true)), "{v}");
    dm(alice.clone(), "second").await;
    assert_eq!(last_wake_station(&app).await, json!(st[0]), "main wins");
    // mentions too
    ok(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "@doug look"}),
        )
        .await,
        "mention",
    );
    assert_eq!(last_wake_station(&app).await, json!(st[0]));

    // at most one per cook and pot: the cook itself moves it to ses_b over MCP
    let moved = ok(
        &app.mcp(&doug, "set_main_station", json!({"id": st[1]}))
            .await,
        "move",
    );
    assert_eq!(moved["main"], true);
    let list = ok(
        &app.mcp(&doug, "list_stations", json!({"project": "PAY"}))
            .await,
        "list",
    );
    let mains: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["main"] == true)
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(mains, [st[1].as_str()]);

    // re-registration (station scan) keeps the flag
    let (_, again) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "doug", "project": "PAY", "runtime": "opencode", "session": "ses_b", "label": "renamed"}),
        )
        .await;
    assert_eq!(
        (&again["id"], &again["main"]),
        (&json!(st[1]), &json!(true))
    );

    // somebody else cannot choose doug's main station
    let (code, _) = app
        .post(&eve, &format!("/stations/{}/main", st[2]), json!({}))
        .await;
    assert_eq!(code, 404);

    // deleting the main station falls back cleanly to the recency order
    let (code, _) = app
        .rest(
            &alice,
            reqwest::Method::DELETE,
            &format!("/stations/{}", st[1]),
            None,
        )
        .await;
    assert_eq!(code, 200);
    dm(alice.clone(), "third").await;
    let w = last_wake_station(&app).await;
    assert!(w == json!(st[0]) || w == json!(st[2]), "{w}");

    // unset: back to recency
    ok(
        &app.mcp(
            &doug,
            "set_main_station",
            json!({"id": st[0], "main": false}),
        )
        .await,
        "unset",
    );
    sqlx::query("update stations set last_woken_at = now() where ref = $1")
        .bind(&st[2])
        .execute(&app.state.db)
        .await
        .unwrap();
    dm(alice.clone(), "fourth").await;
    assert_eq!(last_wake_station(&app).await, json!(st[2]));

    // a main station moved to another pot stops being the main one of the first
    app.post(&alice, &format!("/stations/{}/main", st[2]), json!({}))
        .await;
    let (_, moved) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "doug", "project": "OPS", "runtime": "opencode", "session": "ses_c"}),
        )
        .await;
    assert_eq!(
        (&moved["project"], &moved["main"]),
        (&json!("OPS"), &json!(false))
    );
}
