//! The pass: the chefs of a pot talk. Cooks post; people speak through their own cook (fromOwner).
//! The pot's pass is read by every chef of the pot; a direct message only by its author, the cook
//! that relayed it and its recipient (admins included), in the tools, the model, the feed and the
//! live stream alike. A direct message or an @mention wakes the cook it is for.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

async fn wakes(app: &common::TestApp) -> Vec<Value> {
    sqlx::query_scalar("select payload from events where kind = 'wake' order by seq")
        .fetch_all(&app.state.db)
        .await
        .unwrap()
}

/// Bodies of the messages a token reads through the generated model route.
async fn model_bodies(app: &common::TestApp, token: &str) -> Vec<String> {
    let r = app
        .http
        .get(format!("{}/api/v2/pass_messages?limit=100", app.base))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let rows = v["items"]
        .as_array()
        .or_else(|| v["data"].as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<String> = rows
        .iter()
        .filter_map(|m| m["body"].as_str().map(String::from))
        .collect();
    out.sort();
    out
}

fn bodies(page: &Value) -> Vec<&str> {
    page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_pass() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob", "eve"] {
        app.post(&root, "/users", json!({"login": u})).await;
    }
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams", json!({"name": "other"})).await;
    for (team, who) in [("acme", "alice"), ("acme", "bob"), ("other", "eve")] {
        app.post(
            &root,
            &format!("/teams/{team}/members"),
            json!({"principal": who}),
        )
        .await;
    }
    let tok = |n: &'static str| {
        let db = app.state.db.clone();
        async move { admin::operator_token(&db, n, None).await.unwrap() }
    };
    let (alice, bob, eve) = (tok("alice").await, tok("bob").await, tok("eve").await);
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    app.post(&alice, "/projects/PAY/tasks", json!({"title": "refunds"}))
        .await;
    let mut cooks = vec![];
    for name in ["c1", "c2"] {
        let (st, v) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local"}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        cooks.push(v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let (c1, c2) = (cooks[0].clone(), cooks[1].clone());
    // b1 speaks for bob (a chef of PAY); x1 is in the team but its owner eve is not
    let mut other = vec![];
    for (owner, name) in [(&bob, "b1"), (&eve, "x1")] {
        let (st, v) = app
            .post(
                owner,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local"}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        other.push(v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    let (b1, x1) = (other[0].clone(), other[1].clone());

    // --- people do not write on the pass themselves ---
    let e = refused(
        &app.mcp(
            &alice,
            "pass",
            json!({"project": "PAY", "message": "hello"}),
        )
        .await,
        "a person posts",
    );
    assert!(e.contains("tell your cook"), "{e}");
    let e = refused(
        &app.mcp(
            &x1,
            "pass",
            json!({"project": "PAY", "message": "hi", "fromOwner": true}),
        )
        .await,
        "owner not a chef of the pot",
    );
    assert!(e.contains("not a chef of PAY"), "{e}");

    // --- on the pot's pass: every chef reads it; nobody else ---
    let m = ok(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "pay", "message": "I take the refunds, PAY-1", "task": "PAY-1"}),
        )
        .await,
        "post",
    );
    assert_eq!(
        (&m["from"], &m["from_kind"], &m["to"], &m["task"]),
        (&json!("c1"), &json!("agent"), &Value::Null, &json!("PAY-1"))
    );
    assert!(wakes(&app).await.is_empty(), "a plain post wakes nobody");
    let page = ok(
        &app.mcp(&c2, "read_pass", json!({"project": "PAY"})).await,
        "c2 reads",
    );
    assert_eq!(bodies(&page), ["I take the refunds, PAY-1"]);
    refused(
        &app.mcp(&eve, "read_pass", json!({"project": "PAY"})).await,
        "another team",
    );
    refused(
        &app.mcp(&eve, "pass", json!({"project": "PAY", "message": "hi"}))
            .await,
        "another team posts",
    );

    // --- unread: counted per conversation, cleared by reading ---
    let (_, pots) = app.get(&bob, "/projects").await;
    let _ = pots;
    let unread = |t: String| {
        let app = &app;
        async move {
            let r = app
                .http
                .post(format!("{}/api/v2/$procs/projectOverview", app.base))
                .bearer_auth(&t)
                .json(&json!({}))
                .send()
                .await
                .unwrap();
            let v: Value = r.json().await.unwrap();
            v.as_array().unwrap()[0]["unread_pass"].as_i64().unwrap()
        }
    };
    assert_eq!(unread(bob.clone()).await, 1);
    assert_eq!(unread(c1.clone()).await, 0, "your own message is read");
    ok(
        &app.mcp(&bob, "read_pass", json!({"project": "PAY", "chef": "pot"}))
            .await,
        "bob reads",
    );
    assert_eq!(unread(bob.clone()).await, 0);

    // --- alice, through her cook c1: a direct message to c2 in her name; c2 is woken ---
    let dm = ok(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "PAY", "message": "c2, the ledger secret is in vault", "to": "c2", "fromOwner": true}),
        )
        .await,
        "dm",
    );
    assert_eq!(
        (&dm["from"], &dm["from_kind"], &dm["via"], &dm["to"]),
        (&json!("alice"), &json!("user"), &json!("c1"), &json!("c2"))
    );
    let w = wakes(&app).await;
    assert_eq!(w.len(), 1);
    assert_eq!(
        (&w[0]["agent"], &w[0]["reason"]),
        (&json!("c2"), &json!("pass"))
    );
    assert!(
        w[0]["text"].as_str().unwrap().contains("ledger secret")
            && w[0]["text"]
                .as_str()
                .unwrap()
                .contains("alice (via c1) wrote to you")
            && w[0]["text"].as_str().unwrap().contains("to alice"),
        "{}",
        w[0]
    );
    let c2page = ok(
        &app.mcp(&c2, "read_pass", json!({"project": "PAY"})).await,
        "c2",
    );
    assert_eq!(
        c2page["unread"]["direct"],
        json!([{"chef": "alice", "count": 0}]),
        "read conversations are listed, with nothing unread"
    );
    assert!(bodies(&c2page).contains(&"c2, the ledger secret is in vault"));
    for (who, t) in [("b1", &b1), ("bob", &bob), ("octo (admin)", &root)] {
        let page = ok(
            &app.mcp(t, "read_pass", json!({"project": "PAY"})).await,
            who,
        );
        assert!(
            !bodies(&page).iter().any(|b| b.contains("ledger")),
            "{who} must not read the direct message"
        );
    }
    // the conversation view, and the unread count of the recipient before reading it
    let reply = ok(
        &app.mcp(
            &c2,
            "pass",
            json!({"project": "PAY", "message": "found it, thanks", "to": "alice"}),
        )
        .await,
        "reply",
    );
    assert_eq!(reply["to"], "alice");
    assert_eq!(
        unread(alice.clone()).await,
        2,
        "c1's message on the pass, and c2's reply"
    );
    let convo = ok(
        &app.mcp(&alice, "read_pass", json!({"project": "PAY", "chef": "c2"}))
            .await,
        "conversation",
    );
    assert_eq!(
        bodies(&convo),
        ["c2, the ledger secret is in vault", "found it, thanks"]
    );
    assert_eq!(
        unread(alice.clone()).await,
        1,
        "the pot's pass is still unread"
    );

    // --- the model applies the same privacy ---
    assert_eq!(
        model_bodies(&app, &bob).await,
        ["I take the refunds, PAY-1"]
    );
    assert_eq!(
        model_bodies(&app, &root).await,
        ["I take the refunds, PAY-1"],
        "admins do not read direct messages"
    );
    assert_eq!(
        model_bodies(&app, &alice).await,
        [
            "I take the refunds, PAY-1",
            "c2, the ledger secret is in vault",
            "found it, thanks"
        ]
    );
    assert!(model_bodies(&app, &eve).await.is_empty());
    assert_eq!(
        model_bodies(&app, &c1).await,
        [
            "I take the refunds, PAY-1",
            "c2, the ledger secret is in vault"
        ],
        "the relaying cook reads what it relayed, not the reply to its owner"
    );

    // --- the feed: a direct message's event only for its two chefs ---
    let feed_kinds = |t: String| {
        let app = &app;
        async move {
            let (_, f) = app.get(&t, "/changes?cursor=0&project=PAY&limit=500").await;
            f["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "pass.posted")
                .count()
        }
    };
    assert_eq!(feed_kinds(bob.clone()).await, 1);
    assert_eq!(feed_kinds(root.clone()).await, 1);
    assert_eq!(feed_kinds(alice.clone()).await, 3);
    assert_eq!(feed_kinds(c2.clone()).await, 3);
    assert_eq!(feed_kinds(c1.clone()).await, 2);

    // --- @mentions wake the cooks named (not the author or its cook, not people) ---
    let before = wakes(&app).await.len();
    ok(
        &app.mcp(
            &b1,
            "pass",
            json!({"project": "PAY", "message": "@c1 @C2 @alice @b1 can someone review PAY-1?", "fromOwner": true}),
        )
        .await,
        "mention",
    );
    let w = wakes(&app).await;
    let woken: Vec<&str> = w[before..]
        .iter()
        .map(|x| x["agent"].as_str().unwrap())
        .collect();
    assert_eq!(woken, ["c1", "c2"]);
    assert!(
        w[before]["text"]
            .as_str()
            .unwrap()
            .contains("bob (via b1) mentioned you")
    );

    // --- paging with after, and the mistakes ---
    let all = ok(
        &app.mcp(&c1, "read_pass", json!({"project": "PAY", "limit": 1}))
            .await,
        "newest",
    );
    assert_eq!(
        bodies(&all),
        ["@c1 @C2 @alice @b1 can someone review PAY-1?"]
    );
    let first_seq = m["seq"].as_i64().unwrap();
    let after = ok(
        &app.mcp(
            &c1,
            "read_pass",
            json!({"project": "PAY", "after": first_seq}),
        )
        .await,
        "after",
    );
    assert_eq!(
        bodies(&after),
        [
            "c2, the ledger secret is in vault",
            "@c1 @C2 @alice @b1 can someone review PAY-1?"
        ]
    );
    let e = refused(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "PAY", "message": "x", "to": "eve"}),
        )
        .await,
        "not a chef of the pot",
    );
    assert!(e.contains("no chef named eve"), "{e}");
    refused(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "PAY", "message": "x", "to": "c1"}),
        )
        .await,
        "to yourself",
    );
    refused(
        &app.mcp(
            &c1,
            "pass",
            json!({"project": "PAY", "message": "x", "to": "alice", "fromOwner": true}),
        )
        .await,
        "to your owner in their name",
    );
    refused(
        &app.mcp(&c1, "pass", json!({"project": "PAY", "message": "   "}))
            .await,
        "empty",
    );
}
