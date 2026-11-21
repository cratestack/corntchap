//! Stations (registered sessions) and the question pool: register, wake the idle station, claim
//! from a station, ask with wait (task parked), a cook relays the owner's answer from a chat, the
//! asker gets its task back with a fresh lease and its station is woken with the answer.
mod common;
use corntchap::{admin, stations};
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

/// The newest wake event's payload.
async fn last_wake(app: &common::TestApp) -> Value {
    sqlx::query_scalar("select payload from events where kind = 'wake' order by seq desc limit 1")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn stations_and_the_question_pool() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, "octo").await.unwrap();
    let root = tok.unwrap();
    app.post(&root, "/users", json!({"login": "alice"})).await;
    app.post(&root, "/users", json!({"login": "bob"})).await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/teams/acme/members",
        json!({"principal": "alice", "role": "owner"}),
    )
    .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    app.post(
        &alice,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let mut cooks = vec![];
    for (owner, name) in [(&alice, "c1"), (&alice, "c2"), (&bob, "b1")] {
        let (st, v) = app
            .post(
                owner,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local", "max_leases": 3}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        cooks.push(v["token"].as_str().unwrap().to_string());
    }
    let (c1, c2, b1) = (cooks[0].clone(), cooks[1].clone(), cooks[2].clone());
    for c in ["c1", "c2"] {
        app.post(&alice, "/teams/acme/members", json!({"principal": c}))
            .await;
    }
    // b1 is in the team, but its owner bob is not
    app.post(&bob, "/teams", json!({"name": "x"})).await; // bob is no admin: refused, harmless
    app.post(&root, "/teams/acme/members", json!({"principal": "b1"}))
        .await;
    for t in ["one", "two", "three"] {
        app.post(
            &alice,
            "/projects/PAY/tasks",
            json!({"title": t, "items": ["a"]}),
        )
        .await;
    }

    // --- stations: alice registers two c1 sessions (checkout + worktree) ---
    let mut st = vec![];
    for (sess, dir) in [("ses_main", "/home/a/pay"), ("ses_wt", "/home/a/pay-wt")] {
        let (code, v) = app
            .post(&alice, "/stations", json!({"cook": "c1", "project": "pay", "runtime": "opencode", "session": sess, "directory": dir, "label": dir}))
            .await;
        assert_eq!(code, 200, "{v}");
        assert!(v["id"].as_str().unwrap().starts_with("st_"));
        st.push(v["id"].as_str().unwrap().to_string());
    }
    // registering the same session again keeps its id
    let (_, again) = app
        .post(&alice, "/stations", json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": "ses_main", "label": "main"}))
        .await;
    assert_eq!(
        (again["id"].as_str(), &again["label"]),
        (Some(st[0].as_str()), &json!("main"))
    );
    // bob cannot register for alice's cook; c1 can register its own session
    let (code, _) = app
        .post(
            &bob,
            "/stations",
            json!({"cook": "c1", "project": "PAY", "runtime": "opencode", "session": "x"}),
        )
        .await;
    assert_eq!(code, 404);
    let own = ok(
        &app.mcp(&c2, "register_station", json!({"project": "PAY", "runtime": "claude-code", "session": "cc-1", "directory": "/home/a/pay"})).await,
        "self-register",
    );
    assert_eq!(own["cook"], "c2");
    let mine = ok(&app.mcp(&c1, "list_stations", json!({})).await, "list");
    assert_eq!(mine.as_array().unwrap().len(), 2);
    let all = ok(
        &app.mcp(&alice, "list_stations", json!({"project": "PAY"}))
            .await,
        "alice list",
    );
    assert_eq!(all.as_array().unwrap().len(), 3);

    // --- the dispatcher wakes idle stations, least recently woken first ---
    let ids = |app: &common::TestApp| {
        let db = app.state.db.clone();
        async move {
            let agent: uuid::Uuid =
                sqlx::query_scalar("select id from principals where name = 'c1'")
                    .fetch_one(&db)
                    .await
                    .unwrap();
            let project: uuid::Uuid =
                sqlx::query_scalar("select id from projects where key = 'PAY'")
                    .fetch_one(&db)
                    .await
                    .unwrap();
            (agent, project)
        }
    };
    let (agent, project) = ids(&app).await;
    let mut conn = app.state.db.acquire().await.unwrap();
    let (_, first) = stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    let (_, second) = stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    assert_ne!(
        first.as_ref().unwrap()["id"],
        second.as_ref().unwrap()["id"]
    );
    assert_eq!(
        first.as_ref().unwrap()["session"]
            .as_str()
            .map(|s| s.starts_with("ses_")),
        Some(true)
    );

    // --- claiming from a station records it ---
    let claim = ok(
        &app.mcp(
            &c1,
            "claim_next",
            json!({"project": "PAY", "station": st[0]}),
        )
        .await,
        "claim",
    );
    let task = claim["task"]["id"].as_str().unwrap().to_string();
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    let e = refused(
        &app.mcp(
            &c2,
            "claim_next",
            json!({"project": "PAY", "station": st[1]}),
        )
        .await,
        "foreign station",
    );
    assert!(e.contains("no station"), "{e}");
    let held = ok(&app.mcp(&c1, "list_stations", json!({})).await, "holding");
    let s0 = held
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == st[0].as_str())
        .unwrap();
    assert_eq!(s0["holding"], json!([task.clone()]));
    // with one station busy, the other one is picked; with both busy, nothing is woken
    let (_, pick) = stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    assert_eq!(pick.unwrap()["id"], st[1].as_str());
    let c2claim = ok(
        &app.mcp(
            &c1,
            "claim_next",
            json!({"project": "PAY", "station": st[1]}),
        )
        .await,
        "claim 2",
    );
    let (has, none) = stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    assert!(has && none.is_none());
    ok(
        &app.mcp(&c1, "release", json!({"id": c2claim["task"]["id"], "epoch": c2claim["lease"]["epoch"], "reason": "later"})).await,
        "release",
    );
    drop(conn);

    // --- ask with wait: the task is parked for c1 ---
    let q = ok(
        &app.mcp(&c1, "ask", json!({"task": task, "question": "Refund partial amounts: round down or up?", "options": "down\nup", "wait": true, "station": st[0]})).await,
        "ask",
    );
    assert_eq!(
        (&q["id"], &q["status"], &q["options"], &q["station"]),
        (
            &json!("PAY-Q1"),
            &json!("open"),
            &json!(["down", "up"]),
            &json!(st[0])
        )
    );
    let t = ok(
        &app.mcp(&c1, "get_task", json!({"id": task})).await,
        "parked task",
    );
    assert_eq!(t["status"], "blocked");
    assert!(t["result"].as_str().unwrap().contains("PAY-Q1"));
    refused(
        &app.mcp(
            &c1,
            "complete",
            json!({"id": task, "epoch": epoch, "result": "x"}),
        )
        .await,
        "complete while parked",
    );
    let parked = ok(&app.mcp(&c1, "list_stations", json!({})).await, "waiting");
    let s0 = parked
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == st[0].as_str())
        .unwrap();
    assert_eq!(s0["waiting"], json!([task.clone()]));
    let pool = ok(
        &app.mcp(&alice, "list_questions", json!({"project": "PAY"}))
            .await,
        "pool",
    );
    assert_eq!(pool.as_array().unwrap().len(), 1);
    // wait is only for a task you hold
    let e = refused(
        &app.mcp(
            &c2,
            "ask",
            json!({"task": "PAY-3", "question": "?", "wait": true}),
        )
        .await,
        "wait on unheld",
    );
    assert!(e.contains("parks a task you hold"), "{e}");

    // --- c2, in a chat with alice, picks the question; b1 cannot relay (bob is not in the team) ---
    let picked = ok(
        &app.mcp(&c2, "pick_question", json!({"project": "PAY"}))
            .await,
        "pick",
    );
    assert_eq!(
        (&picked["question"]["id"], &picked["question"]["picked_by"]),
        (&json!("PAY-Q1"), &json!("c2"))
    );
    let none = ok(
        &app.mcp(&c1, "pick_question", json!({"project": "PAY"}))
            .await,
        "held",
    );
    assert_eq!(none["none"], true, "held for c2: {none}");
    let e = refused(
        &app.mcp(
            &b1,
            "answer_question",
            json!({"id": "PAY-Q1", "answer": "up"}),
        )
        .await,
        "b1 relay",
    );
    assert!(
        e.contains("not in this pot's team") || e.contains("another chat"),
        "{e}"
    );

    // c2 relays alice's answer
    let a = ok(
        &app.mcp(
            &c2,
            "answer_question",
            json!({"id": "PAY-Q1", "answer": "Round down, to the cent."}),
        )
        .await,
        "answer",
    );
    assert_eq!(
        (&a["status"], &a["answered_by"], &a["relayed_by"]),
        (&json!("answered"), &json!("alice"), &json!("c2"))
    );
    // c1 has its task back with a fresh lease, and its station is woken with the answer
    let t = ok(
        &app.mcp(&c1, "get_task", json!({"id": task})).await,
        "resumed",
    );
    assert_eq!(
        (&t["status"], &t["claimed_by"]),
        (&json!("claimed"), &json!("c1"))
    );
    assert_eq!(t["lease_epoch"].as_i64().unwrap(), epoch + 1);
    let w = last_wake(&app).await;
    assert_eq!(
        (&w["agent"], &w["reason"], &w["station"]["id"]),
        (&json!("c1"), &json!("answer"), &json!(st[0]))
    );
    let text = w["text"].as_str().unwrap();
    assert!(
        text.contains("Round down")
            && text.contains("alice, via c2")
            && text.contains(&format!("epoch {}", epoch + 1)),
        "{text}"
    );
    refused(
        &app.mcp(
            &c1,
            "complete",
            json!({"id": task, "epoch": epoch, "result": "x"}),
        )
        .await,
        "old epoch",
    );
    ok(
        &app.mcp(
            &c1,
            "complete",
            json!({"id": task, "epoch": epoch + 1, "result": "rounded down"}),
        )
        .await,
        "complete",
    );
    refused(
        &app.mcp(
            &c2,
            "answer_question",
            json!({"id": "PAY-Q1", "answer": "again"}),
        )
        .await,
        "answered twice",
    );

    // --- the console path: alice answers directly; follow-ups wake the asker ---
    let q2 = ok(&app.mcp(&c1, "ask", json!({"project": "PAY", "question": "Which currency for test fixtures?", "station": st[1]})).await, "ask 2");
    let (code, n) = app
        .post(
            &alice,
            &format!("/questions/{}/note", q2["id"].as_str().unwrap()),
            json!({"note": "For which test suite?"}),
        )
        .await;
    assert_eq!(
        (code, n["notes"][0]["author"].as_str()),
        (200, Some("alice")),
        "{n}"
    );
    let w = last_wake(&app).await;
    assert_eq!(
        (&w["reason"], &w["station"]["id"]),
        (&json!("note"), &json!(st[1]))
    );
    ok(
        &app.mcp(
            &c1,
            "comment_question",
            json!({"id": "PAY-Q2", "note": "The ledger e2e tests."}),
        )
        .await,
        "reply",
    );
    let (code, a2) = app
        .post(&alice, "/questions/PAY-Q2/answer", json!({"answer": "XAF"}))
        .await;
    assert_eq!(
        (code, &a2["answered_by"], &a2["relayed_by"]),
        (200, &json!("alice"), &Value::Null),
        "{a2}"
    );
    assert_eq!(a2["notes"].as_array().unwrap().len(), 2);

    // --- addressed to a cook: that cook answers as itself; dismiss by the asker ---
    ok(&app.mcp(&c1, "ask", json!({"project": "PAY", "question": "Did you already migrate the ledger?", "to": "c2"})).await, "ask c2");
    let p = ok(
        &app.mcp(&c2, "pick_question", json!({})).await,
        "c2 picks its own first",
    );
    assert_eq!(p["question"]["to"], "c2");
    let a3 = ok(
        &app.mcp(
            &c2,
            "answer_question",
            json!({"id": "PAY-Q3", "answer": "Yes, in PAY-2."}),
        )
        .await,
        "c2 answers",
    );
    assert_eq!(
        (&a3["answered_by"], &a3["relayed_by"]),
        (&json!("c2"), &Value::Null)
    );
    ok(
        &app.mcp(
            &c1,
            "ask",
            json!({"project": "PAY", "question": "obsolete?"}),
        )
        .await,
        "ask 4",
    );
    let d = ok(
        &app.mcp(
            &c1,
            "dismiss_question",
            json!({"id": "PAY-Q4", "reason": "found it in the pantry"}),
        )
        .await,
        "dismiss",
    );
    assert_eq!(d["status"], "dismissed");
    let e = refused(
        &app.mcp(&c2, "dismiss_question", json!({"id": "PAY-Q2"}))
            .await,
        "dismiss answered",
    );
    assert!(e.contains("already"), "{e}");

    // --- release puts a picked question back; a person unblocking a parked task clears it ---
    let claim = ok(
        &app.mcp(&c1, "claim_task", json!({"id": "PAY-3", "station": st[0]}))
            .await,
        "claim PAY-3",
    );
    let e3 = claim["lease"]["epoch"].as_i64().unwrap();
    ok(
        &app.mcp(
            &c1,
            "ask",
            json!({"task": "PAY-3", "question": "Ship behind a flag?", "wait": true}),
        )
        .await,
        "ask 5",
    );
    ok(
        &app.mcp(&c2, "pick_question", json!({"project": "PAY"}))
            .await,
        "pick 5",
    );
    let r = ok(
        &app.mcp(&c2, "release_question", json!({"id": "PAY-Q5"}))
            .await,
        "release",
    );
    assert!(r["picked_by"].is_null());
    let (code, t) = app.post(&alice, "/tasks/PAY-3/unblock", json!({})).await;
    assert_eq!(
        (code, &t["status"], &t["claimed_by"]),
        (200, &json!("open"), &Value::Null),
        "{t}"
    );
    let late = ok(
        &app.mcp(
            &alice,
            "answer_question",
            json!({"id": "PAY-Q5", "answer": "yes"}),
        )
        .await,
        "late answer",
    );
    assert_eq!(late["status"], "answered");
    let t = ok(
        &app.mcp(&c1, "get_task", json!({"id": "PAY-3"})).await,
        "not resumed",
    );
    assert_eq!(
        t["status"], "open",
        "unblocked by a person: the answer does not take it back ({e3})"
    );

    // --- my_questions and the feed ---
    let mine = ok(&app.mcp(&c1, "my_questions", json!({})).await, "mine");
    assert_eq!(mine.as_array().unwrap().len(), 5);
    let feed = ok(
        &app.mcp(
            &alice,
            "changes_since",
            json!({"cursor": 0, "project": "PAY", "limit": 500}),
        )
        .await,
        "feed",
    );
    let kinds: Vec<&str> = feed["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .filter(|k| k.starts_with("question."))
        .collect();
    assert!(
        kinds.contains(&"question.asked")
            && kinds.contains(&"question.answered")
            && kinds.contains(&"question.noted")
            && kinds.contains(&"question.dismissed")
    );

    // REST mirrors: list, mine, get, remove a station
    let (_, list) = app
        .rest(
            &alice,
            Method::GET,
            "/projects/PAY/questions?status=all",
            None,
        )
        .await;
    assert_eq!(list.as_array().unwrap().len(), 5);
    let (code, _) = app
        .rest(
            &alice,
            Method::DELETE,
            &format!("/stations/{}", st[1]),
            None,
        )
        .await;
    assert_eq!(code, 200);
    let left = ok(
        &app.mcp(&c1, "list_stations", json!({})).await,
        "after remove",
    );
    assert_eq!(left.as_array().unwrap().len(), 1);
}
