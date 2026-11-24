//! Asking a cook wakes it, and reviews: request, verdict (approve, changes, comment), rounds,
//! permissions, the task's `review`, cancel, and fire-and-forget cooks not being called for either.
mod common;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// The tool call failed; its message.
fn err(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r.to_string()
}

async fn wakes_after(app: &common::TestApp, from: i64) -> Vec<Value> {
    sqlx::query_scalar("select payload from events where kind = 'wake' and seq > $1 order by seq")
        .bind(from)
        .fetch_all(&app.state.db)
        .await
        .unwrap()
}

async fn cursor(app: &common::TestApp) -> i64 {
    sqlx::query_scalar("select coalesce(max(seq), 0) from events")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
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
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
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

async fn station(w: &World, cook: &str, session: &str) -> Value {
    let (_, s) = w
        .app
        .post(
            &w.alice,
            "/stations",
            json!({"cook": cook, "project": "PAY", "runtime": "opencode", "session": session}),
        )
        .await;
    s
}

#[tokio::test]
async fn asking_a_cook_wakes_it() {
    let w = world(false).await;
    let app = &w.app;
    let st = station(&w, "c2", "ses_c2").await;
    app.post(
        &w.alice,
        &format!("/stations/{}/main", st["id"].as_str().unwrap()),
        json!({}),
    )
    .await;

    let c = cursor(app).await;
    let q = ok(
        &app.mcp(
            &w.alice,
            "ask",
            json!({"project": "PAY", "to": "c2", "title": "Round up or down?",
                   "description": "Totals are in cents.",
                   "choices": "Up: always\nDown: never", "kind": "single"}),
        )
        .await,
        "ask",
    );
    let wakes = wakes_after(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    let w0 = &wakes[0];
    assert_eq!(
        (&w0["agent"], &w0["reason"], &w0["station"]["id"]),
        (&json!("c2"), &json!("question"), &st["id"])
    );
    let text = w0["text"].as_str().unwrap();
    for part in [
        "alice asks you",
        "Round up or down?",
        "Totals are in cents.",
        "up = Up: always",
        "down = Down: never",
        "answer_question",
        q["id"].as_str().unwrap(),
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }

    // the cook answers as itself
    let a = ok(
        &app.mcp(
            &w.c2,
            "answer_question",
            json!({"id": q["id"], "choices": "up"}),
        )
        .await,
        "answer",
    );
    assert_eq!(a["status"], "answered");

    // a question to a person, to nobody, or to oneself wakes nobody
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.alice,
            "ask",
            json!({"project": "PAY", "to": "bob", "title": "Lunch?"}),
        )
        .await,
        "ask bob",
    );
    ok(
        &app.mcp(
            &w.alice,
            "ask",
            json!({"project": "PAY", "title": "Anyone?"}),
        )
        .await,
        "ask all",
    );
    ok(
        &app.mcp(
            &w.c2,
            "ask",
            json!({"project": "PAY", "to": "c2", "title": "Me?"}),
        )
        .await,
        "ask self",
    );
    assert!(wakes_after(app, c).await.is_empty());
}

#[tokio::test]
async fn a_review_goes_round_by_round() {
    let w = world(false).await;
    let app = &w.app;
    let st = station(&w, "c2", "ses_c2").await;
    let claim = ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();

    // permissions: only who holds or held the task asks; never for oneself
    let e = err(
        &app.mcp(
            &w.c3,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2"}),
        )
        .await,
        "stranger",
    );
    assert!(e.contains("neither holds nor held"), "{e}");
    let e = err(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c1"}),
        )
        .await,
        "self",
    );
    assert!(e.contains("your own work"), "{e}");
    let e = err(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "nobody"}),
        )
        .await,
        "unknown",
    );
    assert!(e.contains("nobody named"), "{e}");

    // no review yet: the task has none
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-1"})).await,
        "get",
    );
    assert!(t["review"].is_null(), "{t}");
    assert_eq!(t["reviews"], json!([]));

    // round 1: c2 is woken with the task, the note and the call to make
    let c = cursor(app).await;
    let r1 = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2", "note": "https://github.com/acme/pay/pull/9"}),
        )
        .await,
        "request",
    );
    assert_eq!(
        (&r1["id"], &r1["state"], &r1["round"], &r1["reviewer"]),
        (
            &json!("PAY-R1"),
            &json!("requested"),
            &json!(1),
            &json!("c2")
        )
    );
    let wakes = wakes_after(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(
        (
            &wakes[0]["agent"],
            &wakes[0]["reason"],
            &wakes[0]["station"]["id"]
        ),
        (&json!("c2"), &json!("review"), &st["id"])
    );
    let text = wakes[0]["text"].as_str().unwrap();
    for part in [
        "c1 asks you to review PAY-1",
        "refunds",
        "https://github.com/acme/pay/pull/9",
        "PAY-R1",
        "`review`",
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }
    let kinds: Vec<String> = sqlx::query_scalar(
        "select kind from events where seq > $1 and kind like 'review.%' order by seq",
    )
    .bind(c)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(kinds, ["review.requested"]);

    // one open review per task and reviewer; a review is not a lane
    let e = err(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2"}),
        )
        .await,
        "twice",
    );
    assert!(e.contains("already has a review"), "{e}");
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-1"})).await,
        "get",
    );
    assert_eq!(t["status"], "claimed");
    assert_eq!(
        (
            &t["review"]["state"],
            &t["review"]["reviewer"],
            &t["review"]["requested_by"]
        ),
        (&json!("requested"), &json!("c2"), &json!("c1"))
    );
    assert_eq!(t["reviews"].as_array().unwrap().len(), 1);
    let list = ok(
        &app.mcp(&w.c1, "list_tasks", json!({"project": "PAY"}))
            .await,
        "list",
    );
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "PAY-1")
        .unwrap();
    assert_eq!(row["review"]["state"], "requested");

    // who may give the verdict: not the requester, not another cook
    let e = err(
        &app.mcp(
            &w.c1,
            "review",
            json!({"id": "PAY-R1", "verdict": "approve"}),
        )
        .await,
        "own",
    );
    assert!(e.contains("own request"), "{e}");
    let e = err(
        &app.mcp(
            &w.c3,
            "review",
            json!({"id": "PAY-R1", "verdict": "approve"}),
        )
        .await,
        "wrong reviewer",
    );
    assert!(e.contains("addressed to someone else"), "{e}");
    let e = err(
        &app.mcp(
            &w.alice,
            "review",
            json!({"id": "PAY-R1", "verdict": "approve"}),
        )
        .await,
        "a person is not the named reviewer either",
    );
    assert!(e.contains("addressed to someone else"), "{e}");
    let e = err(
        &app.mcp(
            &w.c2,
            "review",
            json!({"id": "PAY-R1", "verdict": "changes"}),
        )
        .await,
        "no body",
    );
    assert!(e.contains("say what to change"), "{e}");
    let e = err(
        &app.mcp(&w.c2, "review", json!({"id": "PAY-R1", "verdict": "maybe"}))
            .await,
        "bad verdict",
    );
    assert!(e.contains("approve, changes or comment"), "{e}");

    // changes: c1 is woken on the task with the next step; the task stays claimed
    let c = cursor(app).await;
    let done = ok(
        &app.mcp(
            &w.c2,
            "review",
            json!({"task": "PAY-1", "verdict": "changes", "body": "Handle the zero amount."}),
        )
        .await,
        "changes",
    );
    assert_eq!(
        (&done["state"], &done["reviewed_by"], &done["body"]),
        (
            &json!("changes_requested"),
            &json!("c2"),
            &json!("Handle the zero amount.")
        )
    );
    let wakes = wakes_after(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(
        (&wakes[0]["agent"], &wakes[0]["reason"]),
        (&json!("c1"), &json!("review-done"))
    );
    let text = wakes[0]["text"].as_str().unwrap();
    for part in [
        "c2 asked for changes on PAY-1",
        "Handle the zero amount.",
        "request_review",
        "round 2",
    ] {
        assert!(text.contains(part), "{part} missing from {text}");
    }
    let e = err(
        &app.mcp(
            &w.c2,
            "review",
            json!({"id": "PAY-R1", "verdict": "approve"}),
        )
        .await,
        "decided",
    );
    assert!(e.contains("already changes_requested"), "{e}");
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-1"})).await,
        "get",
    );
    assert_eq!(
        (&t["status"], &t["review"]["state"], &t["review"]["round"]),
        (&json!("claimed"), &json!("changes_requested"), &json!(1))
    );

    // round 2, approved; the task shows the latest round and both are kept
    let r2 = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2", "note": "zero handled"}),
        )
        .await,
        "again",
    );
    assert_eq!((&r2["id"], &r2["round"]), (&json!("PAY-R2"), &json!(2)));
    let c = cursor(app).await;
    ok(
        &app.mcp(
            &w.c2,
            "review",
            json!({"id": "PAY-R2", "verdict": "approve"}),
        )
        .await,
        "approve",
    );
    let wakes = wakes_after(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0]["text"]
            .as_str()
            .unwrap()
            .contains("c2 approved PAY-1"),
        "{wakes:?}"
    );
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-1"})).await,
        "get",
    );
    assert_eq!(
        (
            &t["review"]["state"],
            &t["review"]["round"],
            &t["review"]["id"]
        ),
        (&json!("approved"), &json!(2), &json!("PAY-R2"))
    );
    let rounds: Vec<(i64, String)> = t["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["round"].as_i64().unwrap(),
                r["state"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        rounds,
        [(1, "changes_requested".into()), (2, "approved".into())]
    );
    let all = ok(
        &app.mcp(&w.c1, "list_reviews", json!({"project": "PAY"}))
            .await,
        "list",
    );
    assert_eq!(all.as_array().unwrap().len(), 2);
    let open = ok(
        &app.mcp(
            &w.c1,
            "list_reviews",
            json!({"project": "PAY", "state": "requested"}),
        )
        .await,
        "list",
    );
    assert!(open.as_array().unwrap().is_empty());

    // served and still reviewable by a person: complete, then ask a person
    ok(
        &app.mcp(
            &w.c1,
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "done"}),
        )
        .await,
        "complete",
    );
    ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "bob", "note": "after the fact"}),
        )
        .await,
        "held once",
    );
}

#[tokio::test]
async fn people_review_and_anyone_on_the_team_may() {
    let w = world(false).await;
    let app = &w.app;
    ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-2"})).await,
        "claim",
    );

    // a person named as reviewer: no wake, it waits in their reviews and the feed
    let c = cursor(app).await;
    let r = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-2", "reviewer": "bob", "note": "branch pay-2"}),
        )
        .await,
        "to bob",
    );
    assert!(wakes_after(app, c).await.is_empty());
    let mine = ok(
        &app.mcp(
            &w.bob,
            "list_reviews",
            json!({"project": "PAY", "mine": true}),
        )
        .await,
        "bob's",
    );
    assert_eq!(mine.as_array().unwrap().len(), 1, "{mine}");
    let alices = ok(
        &app.mcp(
            &w.alice,
            "list_reviews",
            json!({"project": "PAY", "mine": true}),
        )
        .await,
        "alice's",
    );
    assert!(alices.as_array().unwrap().is_empty());
    let projects = ok(
        &app.mcp(&w.bob, "list_projects", json!({})).await,
        "projects",
    );
    assert_eq!(projects[0]["waiting_reviews"], 1, "{projects}");
    // the activity feed (the pot's events) has it
    let feed: Vec<Value> = sqlx::query_scalar(
        "select payload from events where kind = 'review.requested' and seq > $1",
    )
    .bind(c)
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    assert_eq!(feed.len(), 1);
    assert_eq!(
        (&feed[0]["reviewer"], &feed[0]["review_id"]),
        (&json!("bob"), &r["id"])
    );
    ok(
        &app.mcp(
            &w.bob,
            "review",
            json!({"id": r["id"], "verdict": "comment", "body": "looks fine, one nit"}),
        )
        .await,
        "comment",
    );
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-2"})).await,
        "get",
    );
    assert_eq!(
        (&t["review"]["state"], &t["review"]["reviewed_by"]),
        (&json!("commented"), &json!("bob"))
    );

    // no reviewer named: anyone of the team but the requester, and the first verdict closes it
    let r = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-2", "note": "again"}),
        )
        .await,
        "anyone",
    );
    assert!(r["reviewer"].is_null(), "{r}");
    assert_eq!(r["round"], 2);
    let e = err(
        &app.mcp(&w.c1, "request_review", json!({"task": "PAY-2"}))
            .await,
        "second open to anyone",
    );
    assert!(e.contains("already has a review"), "{e}");
    let e = err(
        &app.mcp(
            &w.c1,
            "review",
            json!({"task": "PAY-2", "verdict": "approve"}),
        )
        .await,
        "requester",
    );
    assert!(e.contains("has no review waiting"), "{e}");
    let c = cursor(app).await;
    let done = ok(
        &app.mcp(
            &w.alice,
            "review",
            json!({"task": "PAY-2", "verdict": "approve"}),
        )
        .await,
        "alice takes it",
    );
    assert_eq!(
        (&done["state"], &done["reviewed_by"]),
        (&json!("approved"), &json!("alice"))
    );
    let wakes = wakes_after(app, c).await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert_eq!(wakes[0]["agent"], "c1");

    // a person asks too (a person can plan), a cook outside the team's task history cannot
    ok(
        &app.mcp(
            &w.alice,
            "request_review",
            json!({"task": "PAY-3", "reviewer": "c3", "note": "please"}),
        )
        .await,
        "person asks",
    );
    let e = err(
        &app.mcp(
            &w.c2,
            "request_review",
            json!({"task": "PAY-3", "reviewer": "bob"}),
        )
        .await,
        "c2",
    );
    assert!(e.contains("neither holds nor held"), "{e}");
}

#[tokio::test]
async fn a_request_can_be_cancelled() {
    let w = world(false).await;
    let app = &w.app;
    ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    let r = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2"}),
        )
        .await,
        "ask",
    );
    let e = err(
        &app.mcp(&w.c2, "cancel_review", json!({"id": r["id"]}))
            .await,
        "not yours",
    );
    assert!(e.contains("only the one who asked"), "{e}");
    let c = ok(
        &app.mcp(&w.c1, "cancel_review", json!({"id": r["id"]}))
            .await,
        "cancel",
    );
    assert_eq!(c["state"], "cancelled");
    let t = ok(
        &app.mcp(&w.c1, "get_task", json!({"id": "PAY-1"})).await,
        "get",
    );
    assert!(
        t["review"].is_null(),
        "a cancelled request leaves no chip: {t}"
    );
    let e = err(
        &app.mcp(
            &w.c2,
            "review",
            json!({"id": r["id"], "verdict": "approve"}),
        )
        .await,
        "gone",
    );
    assert!(e.contains("already cancelled"), "{e}");
    // asking again is possible, as a new round
    let again = ok(
        &app.mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2"}),
        )
        .await,
        "again",
    );
    assert_eq!(again["round"], 1);
}

/// A tiny endpoint answering 200 and keeping each request's body.
async fn fake_endpoint() -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<(String, String)>>> = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = l.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push((path, body.to_string()));
                let resp = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, seen)
}

#[tokio::test]
async fn a_fire_and_forget_cook_is_not_called_for_a_question_a_review_or_a_verdict() {
    let (base, seen) = fake_endpoint().await;
    let w = world(true).await;
    let app = &w.app;
    // c2 is a cloud cook (fire and forget) through a webhook
    app.post(
        &w.root,
        "/agents/c2/wake-targets",
        json!({"type": "webhook", "config": {"url": format!("{base}/hook"), "secret": "s3cret"}}),
    )
    .await;
    ok(
        &app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim",
    );
    // a question to it and a review request for it are refused
    let r = app
        .mcp(
            &w.alice,
            "ask",
            json!({"project": "PAY", "to": "c2", "title": "Which currency?"}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("c2 is fire-and-forget"), "{r}");
    let r = app
        .mcp(
            &w.c1,
            "request_review",
            json!({"task": "PAY-1", "reviewer": "c2", "note": "see the branch"}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(r.to_string().contains("c2 is fire-and-forget"), "{r}");
    // the verdict on its own work does not start it either
    ok(
        &app.mcp(&w.c3, "claim_task", json!({"id": "PAY-3"})).await,
        "claim 3",
    );
    app.post(
        &w.root,
        "/agents/c3/wake-targets",
        json!({"type": "webhook", "config": {"url": format!("{base}/hook3"), "secret": "s3cret"}}),
    )
    .await;
    ok(
        &app.mcp(
            &w.c3,
            "request_review",
            json!({"task": "PAY-3", "reviewer": "bob"}),
        )
        .await,
        "ask bob",
    );
    ok(
        &app.mcp(
            &w.bob,
            "review",
            json!({"task": "PAY-3", "verdict": "changes", "body": "rename it"}),
        )
        .await,
        "changes",
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    for needle in ["Which currency?", "see the branch", "rename it"] {
        assert!(
            !seen.lock().unwrap().iter().any(|(_, b)| b.contains(needle)),
            "{needle} started a run: {:?}",
            seen.lock().unwrap()
        );
    }
}
