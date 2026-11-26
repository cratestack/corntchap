//! Questions linked to tasks: who may link, what a blocking link does to readiness (claim_next,
//! wait_for_work, the dispatcher, the task and project counts), who is woken when the question is
//! answered or dismissed, and what get_task and get_question show.
mod common;
use std::time::Duration;

use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// The tool call failed; its message.
fn err(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

async fn cursor(app: &common::TestApp) -> i64 {
    sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

/// The wakes after `from` with reason `answer`, as (cook, text).
async fn answer_wakes(app: &common::TestApp, from: i64) -> Vec<(String, String)> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "select payload from events where kind = 'wake' and seq > $1 order by seq",
    )
    .bind(from)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    rows.into_iter()
        .filter(|p| p["reason"] == "answer")
        .map(|p| {
            (
                p["agent"].as_str().unwrap().to_string(),
                p["text"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

struct World {
    app: common::TestApp,
    root: String,
    alice: String,
    bob: String,
    c1: String,
    c2: String,
    c3: String,
}

/// alice and bob (people) and the cooks c1, c2, c3 of a team with the pot PAY (tasks PAY-1 to
/// PAY-3) and a second pot OPS (task OPS-1).
async fn world(background: bool) -> World {
    let app = common::start(background).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob"] {
        app.add_user(u).await;
    }
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    for u in ["alice", "bob"] {
        app.post(&root, "/teams/acme/members", json!({"principal": u}))
            .await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    for (k, n) in [("PAY", "Payments"), ("OPS", "Operations")] {
        app.post(
            &root,
            "/projects",
            json!({"team": "acme", "key": k, "name": n}),
        )
        .await;
    }
    let mut toks = vec![];
    for name in ["c1", "c2", "c3"] {
        let (_, v) = app
            .post(
                &alice,
                "/agents",
                json!({"name": name, "kind": "opencode", "runtime": "local", "max_leases": 3}),
            )
            .await;
        toks.push(v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
    }
    for t in ["refunds", "receipts", "invoices"] {
        app.post(&alice, "/projects/PAY/tasks", json!({"title": t}))
            .await;
    }
    app.post(&alice, "/projects/OPS/tasks", json!({"title": "deploy"}))
        .await;
    let mut it = toks.into_iter();
    World {
        app,
        root,
        alice,
        bob,
        c1: it.next().unwrap(),
        c2: it.next().unwrap(),
        c3: it.next().unwrap(),
    }
}

/// alice asks a free question in PAY (optionally on a task); its id.
async fn ask_as(w: &World, token: &str, title: &str, task: Option<&str>) -> String {
    let mut a = json!({"project": "PAY", "title": title});
    if let Some(t) = task {
        a["task"] = json!(t);
    }
    ok(&w.app.mcp(token, "ask", a).await, "ask")["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn claim(w: &World, token: &str, task: &str) -> Value {
    ok(
        &w.app.mcp(token, "claim_task", json!({"id": task})).await,
        "claim_task",
    )
}

async fn release(w: &World, token: &str, claimed: &Value) {
    ok(
        &w.app
            .mcp(
                token,
                "release",
                json!({"id": claimed["task"]["id"], "epoch": claimed["lease"]["epoch"], "reason": "later"}),
            )
            .await,
        "release",
    );
}

async fn task_json(w: &World, id: &str) -> Value {
    ok(
        &w.app.mcp(&w.alice, "get_task", json!({"id": id})).await,
        "get_task",
    )
}

#[tokio::test]
async fn who_may_link_and_unlink() {
    let w = world(false).await;
    let app = &w.app;
    let q = ask_as(&w, &w.alice, "Which currency?", None).await;
    assert_eq!(q, "PAY-Q1");

    // a cook that neither holds the task nor may plan: refused
    let e = err(
        &app.mcp(
            &w.c1,
            "link_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "link by a stranger",
    );
    assert!(e.contains("only to a task it holds"), "{e}");

    // the holder links (without blocks: it only wants to be told)
    claim(&w, &w.c1, "PAY-1").await;
    let linked = ok(
        &app.mcp(
            &w.c1,
            "link_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "link by the holder",
    );
    assert_eq!(linked["tasks"][0]["id"], "PAY-1", "{linked}");
    assert_eq!(linked["tasks"][0]["blocks"], false);
    assert_eq!(linked["tasks"][0]["implicit"], false);
    assert_eq!(linked["tasks"][0]["linked_by"], "c1");

    // a person may link to any task, and change blocks by linking again
    let linked = ok(
        &app.mcp(
            &w.bob,
            "link_question",
            json!({"question": &q, "task": "PAY-2", "blocks": true}),
        )
        .await,
        "link by a person",
    );
    assert_eq!(linked["tasks"].as_array().unwrap().len(), 2, "{linked}");
    let again = ok(
        &app.mcp(
            &w.bob,
            "link_question",
            json!({"question": &q, "task": "PAY-2", "blocks": false}),
        )
        .await,
        "link again",
    );
    assert_eq!(again["tasks"][1]["blocks"], false, "{again}");

    // a cook granted plan may link to any task
    let (st, _) = app
        .post(&w.alice, "/agents/c3/grants", json!({"grants": ["plan"]}))
        .await;
    assert_eq!(st, 200);
    ok(
        &app.mcp(
            &w.c3,
            "link_question",
            json!({"question": &q, "task": "PAY-3"}),
        )
        .await,
        "link by a planner",
    );

    // another pot's task: refused; an unknown task: not found
    let e = err(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q, "task": "OPS-1"}),
        )
        .await,
        "link across pots",
    );
    assert!(e.contains("different pots"), "{e}");

    // unlink: a stranger (c2) may not; the one who linked, or the holder, may
    let e = err(
        &app.mcp(
            &w.c2,
            "unlink_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "unlink by a stranger",
    );
    assert!(e.contains("may unlink"), "{e}");
    let left = ok(
        &app.mcp(
            &w.c1,
            "unlink_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "unlink by the linker",
    );
    assert_eq!(left["tasks"].as_array().unwrap().len(), 2, "{left}");
    let e = err(
        &app.mcp(
            &w.c1,
            "unlink_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "unlink twice",
    );
    assert!(e.contains("not linked"), "{e}");

    // the task a question was asked on cannot be unlinked, and a question that is answered cannot
    // be linked
    let asked = ask_as(&w, &w.alice, "On refunds", Some("PAY-1")).await;
    let e = err(
        &app.mcp(
            &w.alice,
            "unlink_question",
            json!({"question": &asked, "task": "PAY-1"}),
        )
        .await,
        "unlink the implicit link",
    );
    assert!(e.contains("was asked on"), "{e}");
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q, "text": "EUR"}),
        )
        .await,
        "answer",
    );
    let e = err(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q, "task": "PAY-1"}),
        )
        .await,
        "link an answered question",
    );
    assert!(e.contains("already answered"), "{e}");

    // the REST routes do the same
    let q2 = ask_as(&w, &w.alice, "Rounding?", None).await;
    let (st, v) = app
        .post(
            &w.alice,
            &format!("/questions/{q2}/link"),
            json!({"task": "PAY-3", "blocks": true}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["tasks"][0]["blocks"], true, "{v}");
}

#[tokio::test]
async fn a_blocking_question_makes_the_task_not_ready() {
    let w = world(false).await;
    let app = &w.app;
    let q = ask_as(&w, &w.alice, "Which currency?", None).await;
    for t in ["PAY-1", "PAY-2"] {
        ok(
            &app.mcp(
                &w.alice,
                "link_question",
                json!({"question": &q, "task": t, "blocks": true}),
            )
            .await,
            "link",
        );
    }
    // the task says so, and the pot counts it as waiting
    let t1 = task_json(&w, "PAY-1").await;
    assert_eq!(t1["ready"], false, "{t1}");
    assert_eq!(t1["question_blocked"], true, "{t1}");
    assert_eq!(t1["questions"][0]["id"], "PAY-Q1");
    assert_eq!(t1["questions"][0]["blocks"], true);
    assert_eq!(t1["questions"][0]["status"], "open");
    let (_, projects) = app.get(&w.alice, "/projects").await;
    let pay = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "PAY")
        .unwrap();
    assert_eq!(pay["ready"], 1, "{pay}");
    let (_, ready) = app
        .get(&w.alice, "/projects/PAY/tasks?ready_only=true")
        .await;
    let ids: Vec<&str> = ready
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["PAY-3"], "{ready}");

    // wait_for_work and claim_next hand out only PAY-3
    let waited = ok(
        &app.mcp(
            &w.c2,
            "wait_for_work",
            json!({"project": "PAY", "timeoutS": 1}),
        )
        .await,
        "wait_for_work",
    );
    let ids: Vec<&str> = waited["ready"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["PAY-3"], "{waited}");
    let c = ok(
        &app.mcp(&w.c2, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(c["task"]["id"], "PAY-3", "{c}");
    let c = ok(
        &app.mcp(&w.c2, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next 2",
    );
    assert_eq!(c["none"], true, "{c}");

    // claim_task by name still works, with a warning that names the question
    let c = claim(&w, &w.c1, "PAY-1").await;
    let warning = c["warning"].as_str().unwrap_or_default();
    assert!(
        warning.contains("PAY-Q1") && warning.contains("waits on"),
        "{c}"
    );
    release(&w, &w.c1, &c).await;

    // answering unblocks both
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q, "text": "EUR"}),
        )
        .await,
        "answer",
    );
    let t1 = task_json(&w, "PAY-1").await;
    assert_eq!(t1["ready"], true, "{t1}");
    assert_eq!(t1["question_blocked"], false);
    assert_eq!(t1["questions"][0]["status"], "answered");
    assert_eq!(t1["questions"][0]["answer"], "EUR");
    let c = ok(
        &app.mcp(&w.c2, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next 3",
    );
    assert_ne!(c["none"], true, "{c}");

    // unlinking a blocking link frees the task too; so does dismissing
    let q2 = ask_as(&w, &w.alice, "Another?", None).await;
    ok(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q2, "task": "PAY-2", "blocks": true}),
        )
        .await,
        "link",
    );
    assert_eq!(task_json(&w, "PAY-2").await["ready"], false);
    ok(
        &app.mcp(
            &w.alice,
            "unlink_question",
            json!({"question": &q2, "task": "PAY-2"}),
        )
        .await,
        "unlink",
    );
    assert_eq!(task_json(&w, "PAY-2").await["ready"], true);
    ok(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q2, "task": "PAY-2", "blocks": true}),
        )
        .await,
        "link again",
    );
    assert_eq!(task_json(&w, "PAY-2").await["ready"], false);
    ok(
        &app.mcp(
            &w.alice,
            "dismiss_question",
            json!({"id": &q2, "reason": "moot"}),
        )
        .await,
        "dismiss",
    );
    assert_eq!(task_json(&w, "PAY-2").await["ready"], true);
}

#[tokio::test]
async fn ask_with_blocks_records_a_blocking_link() {
    let w = world(false).await;
    let app = &w.app;
    let held = claim(&w, &w.c1, "PAY-1").await;
    let asked = ok(
        &app.mcp(
            &w.c1,
            "ask",
            json!({"project": "PAY", "task": "PAY-1", "title": "Which currency?", "blocks": true}),
        )
        .await,
        "ask blocks",
    );
    assert_eq!(asked["tasks"][0]["blocks"], true, "{asked}");
    assert_eq!(asked["tasks"][0]["implicit"], false);
    // no wait: the task is still held by c1; once given back it is not handed out
    let t = task_json(&w, "PAY-1").await;
    assert_eq!(t["status"], "claimed", "{t}");
    release(&w, &w.c1, &held).await;
    let c = ok(
        &app.mcp(&w.c2, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(c["task"]["id"], "PAY-2", "{c}");

    // blocks without a task is refused
    let e = err(
        &app.mcp(
            &w.c1,
            "ask",
            json!({"project": "PAY", "title": "x?", "blocks": true}),
        )
        .await,
        "blocks without task",
    );
    assert!(e.contains("needs the task"), "{e}");

    // the question shows its task, the task shows the question
    let q = ok(
        &app.mcp(&w.alice, "get_question", json!({"id": "PAY-Q1"}))
            .await,
        "get_question",
    );
    assert_eq!(q["tasks"][0]["id"], "PAY-1", "{q}");
    assert_eq!(q["tasks"][0]["blocks"], true);
}

#[tokio::test]
async fn answering_wakes_the_linker_the_holder_and_the_owner_once() {
    let w = world(false).await;
    let app = &w.app;
    // c1 held PAY-1 and gave it back (it last held it); c2 holds PAY-2 and linked it; c3 owns PAY-3
    let c1 = claim(&w, &w.c1, "PAY-1").await;
    release(&w, &w.c1, &c1).await;
    claim(&w, &w.c2, "PAY-2").await;
    ok(
        &app.mcp(
            &w.alice,
            "update_task",
            json!({"id": "PAY-3", "owner": "c3"}),
        )
        .await,
        "owner",
    );
    let q = ask_as(&w, &w.alice, "Which currency?", None).await;
    ok(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q, "task": "PAY-1", "blocks": true}),
        )
        .await,
        "link 1",
    );
    ok(
        &app.mcp(
            &w.c2,
            "link_question",
            json!({"question": &q, "task": "PAY-2", "blocks": true}),
        )
        .await,
        "link 2",
    );
    ok(
        &app.mcp(
            &w.bob,
            "link_question",
            json!({"question": &q, "task": "PAY-3"}),
        )
        .await,
        "link 3",
    );
    // c2 also links PAY-1 (it will be told once, for both tasks)
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q, "text": "EUR"}),
        )
        .await,
        "answer",
    );
    let wakes = answer_wakes(app, c).await;
    let mut cooks: Vec<&str> = wakes.iter().map(|w| w.0.as_str()).collect();
    cooks.sort();
    assert_eq!(
        cooks,
        ["c1", "c2", "c3"],
        "each cook once, never alice: {wakes:?}"
    );
    let text = |cook: &str| wakes.iter().find(|w| w.0 == cook).unwrap().1.clone();
    let t1 = text("c1");
    assert!(
        t1.contains("PAY-Q1") && t1.contains("EUR") && t1.contains("PAY-1"),
        "{t1}"
    );
    assert!(t1.contains("unblocked now"), "{t1}");
    assert!(t1.contains("answered by alice"), "{t1}");
    let t2 = text("c2");
    assert!(t2.contains("PAY-2") && t2.contains("unblocked now"), "{t2}");
    let t3 = text("c3");
    assert!(t3.contains("PAY-3") && t3.contains("did not wait"), "{t3}");
}

#[tokio::test]
async fn a_still_blocked_task_says_so_and_dismissing_notifies() {
    let w = world(false).await;
    let app = &w.app;
    claim(&w, &w.c1, "PAY-1").await;
    let q1 = ask_as(&w, &w.alice, "First?", None).await;
    let q2 = ask_as(&w, &w.alice, "Second?", None).await;
    for q in [&q1, &q2] {
        ok(
            &app.mcp(
                &w.c1,
                "link_question",
                json!({"question": q, "task": "PAY-1", "blocks": true}),
            )
            .await,
            "link",
        );
    }
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q1, "text": "yes"}),
        )
        .await,
        "answer",
    );
    let wakes = answer_wakes(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(wakes[0].1.contains("still waits on PAY-Q2"), "{wakes:?}");
    assert_eq!(task_json(&w, "PAY-1").await["ready"], false);

    // dismissing the other tells the holder too, and frees the task
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.bob,
            "dismiss_question",
            json!({"id": &q2, "reason": "no longer needed"}),
        )
        .await,
        "dismiss",
    );
    let wakes = answer_wakes(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0].1.contains("dismissed by bob") && wakes[0].1.contains("no longer needed"),
        "{wakes:?}"
    );
    assert!(wakes[0].1.contains("unblocked now"), "{wakes:?}");
}

#[tokio::test]
async fn a_question_asked_on_a_task_tells_its_holder_and_the_asker_once() {
    let w = world(false).await;
    let app = &w.app;
    claim(&w, &w.c2, "PAY-2").await;
    // alice asks on a task c2 holds: c2 is told when it is answered (implicit link)
    let q = ask_as(&w, &w.alice, "On receipts?", Some("PAY-2")).await;
    let c = cursor(app).await;
    ok(
        &app.mcp(&w.bob, "answer_question", json!({"id": &q, "text": "go"}))
            .await,
        "answer",
    );
    let wakes = answer_wakes(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(wakes[0].0, "c2");
    assert!(
        wakes[0].1.contains("PAY-2") && wakes[0].1.contains("go"),
        "{wakes:?}"
    );

    // c2 asks on its own task: it is woken once (as the asker), not twice
    let q = ask_as(&w, &w.c2, "More?", Some("PAY-2")).await;
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q, "text": "yes"}),
        )
        .await,
        "answer",
    );
    let wakes = answer_wakes(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0].1.starts_with("Your question PAY-Q2 was answered"),
        "{wakes:?}"
    );

    // the question lists the task as an implicit link
    let q = ok(
        &app.mcp(&w.alice, "get_question", json!({"id": "PAY-Q1"}))
            .await,
        "get_question",
    );
    assert_eq!(q["tasks"][0]["implicit"], true, "{q}");
    assert_eq!(q["tasks"][0]["blocks"], false);
    assert_eq!(
        task_json(&w, "PAY-2").await["questions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn a_fire_and_forget_cook_is_not_started_for_an_answer() {
    let w = world(false).await;
    let app = &w.app;
    let (_, v) = app
        .post(
            &w.alice,
            "/agents",
            json!({"name": "rita", "kind": "claude-code", "runtime": "cloud"}),
        )
        .await;
    let rita = v["token"].as_str().unwrap().to_string();
    app.post(&w.root, "/teams/acme/members", json!({"principal": "rita"}))
        .await;
    sqlx::query("insert into wake_targets (agent_id, type, projects, labels, config) select id, 'claude-routine', '{}', '{}', '{}' from principals where name = 'rita'")
        .execute(&app.state.db)
        .await
        .unwrap();
    claim(&w, &rita, "PAY-1").await;
    let q = ask_as(&w, &w.alice, "Which?", None).await;
    ok(
        &app.mcp(
            &rita,
            "link_question",
            json!({"question": &q, "task": "PAY-1", "blocks": true}),
        )
        .await,
        "link",
    );
    let c = cursor(app).await;
    ok(
        &app.mcp(&w.alice, "answer_question", json!({"id": &q, "text": "A"}))
            .await,
        "answer",
    );
    assert!(answer_wakes(app, c).await.is_empty());
    let withheld: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'restaurant.decided' and seq > $1 and payload::text like '%rita%'",
    )
    .bind(c)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert!(withheld >= 1, "the restaurant says it did not wake rita");
}

#[tokio::test]
async fn the_dispatcher_skips_a_blocked_task_and_looks_again_when_it_is_answered() {
    let w = world(true).await;
    let app = &w.app;
    // the only task there is waits on a question
    for t in ["PAY-2", "PAY-3"] {
        ok(
            &app.mcp(&w.alice, "toss_task", json!({"id": t})).await,
            "toss",
        );
    }
    let q = ask_as(&w, &w.alice, "Which currency?", None).await;
    ok(
        &app.mcp(
            &w.alice,
            "link_question",
            json!({"question": &q, "task": "PAY-1", "blocks": true}),
        )
        .await,
        "link",
    );
    // doug is a cook with a relay target: it is woken for ready work
    let (_, ag) = app
        .post(
            &w.alice,
            "/agents",
            json!({"name": "doug", "kind": "opencode", "runtime": "local", "max_leases": 3}),
        )
        .await;
    assert!(ag["token"].is_string());
    app.post(&w.root, "/teams/acme/members", json!({"principal": "doug"}))
        .await;
    let (st, _) = app
        .post(
            &w.alice,
            "/agents/doug/wake-targets",
            json!({"type": "relay"}),
        )
        .await;
    assert_eq!(st, 200);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let work_wakes = |app: &common::TestApp| {
        let db = app.state.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from events where kind = 'wake' and payload->>'agent' = 'doug' and payload->>'reason' is distinct from 'answer'",
            )
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    // an event that makes the dispatcher look at the pot finds nothing ready
    ok(
        &app.mcp(
            &w.alice,
            "update_task",
            json!({"id": "PAY-1", "priority": 2}),
        )
        .await,
        "update",
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(work_wakes(app).await, 0, "a blocked task is not work");
    // the answer makes it ready: the dispatcher looks at the pot and wakes doug
    ok(
        &app.mcp(
            &w.alice,
            "answer_question",
            json!({"id": &q, "text": "EUR"}),
        )
        .await,
        "answer",
    );
    let mut n = 0;
    for _ in 0..40 {
        n = work_wakes(app).await;
        if n > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(n, 1, "doug is woken for the task the answer freed");
}
