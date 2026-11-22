//! Starting a new Claude Code session from the server (`start_station`): a new station with a
//! fresh session id, in a folder where the cook already has one, woken at once with its task.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

async fn last_wake(app: &common::TestApp) -> Value {
    sqlx::query_scalar("select payload from events where kind = 'wake' order by seq desc limit 1")
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

fn err_text(r: &Value) -> String {
    assert_eq!(r["isError"], true, "expected an error: {r}");
    r.to_string()
}

#[tokio::test]
async fn a_started_session_is_a_new_station_woken_with_its_task() {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "eve"] {
        app.add_user(u).await;
    }
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams/acme/members", json!({"principal": "alice"}))
        .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let eve = admin::operator_token(&app.state.db, "eve", None)
        .await
        .unwrap();
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay"}),
    )
    .await;
    let (_, c) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "clara", "kind": "claude-code", "runtime": "local"}),
        )
        .await;
    let clara = c["token"].as_str().unwrap().to_string();
    app.post(
        &alice,
        "/agents",
        json!({"name": "doug", "kind": "opencode", "runtime": "local"}),
    )
    .await;
    for a in ["clara", "doug"] {
        app.post(&root, "/teams/acme/members", json!({"principal": a}))
            .await;
    }
    let (code, _) = app
        .post(
            &alice,
            "/projects/PAY/tasks",
            json!({"title": "Fix the refund rounding"}),
        )
        .await;
    assert_eq!(code, 200);

    // no folder known yet
    let r = app
        .mcp(
            &alice,
            "start_station",
            json!({"project": "PAY", "cook": "clara"}),
        )
        .await;
    assert!(err_text(&r).contains("no station with a folder"), "{r}");

    let (code, first) = app
        .post(
            &alice,
            "/stations",
            json!({"cook": "clara", "project": "PAY", "runtime": "claude-code", "session": "old-1",
                   "directory": "/w/pay"}),
        )
        .await;
    assert_eq!(code, 200, "{first}");

    let r = app
        .mcp(
            &alice,
            "start_station",
            json!({"project": "PAY", "cook": "clara", "task": "PAY-1"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let st = common::TestApp::payload(&r);
    assert_eq!(st["runtime"], "claude-code");
    assert_eq!(st["directory"], "/w/pay");
    assert_eq!(st["label"], "pay (PAY-1)");
    assert_ne!(st["id"], first["id"]);
    let session = st["session"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(session).is_ok(), "{session}");

    let w = last_wake(&app).await;
    assert_eq!(w["reason"], "start");
    assert_eq!(w["agent"], "clara");
    assert_eq!(w["task"], "PAY-1");
    assert_eq!(w["station"]["id"], st["id"]);
    assert_eq!(w["station"]["session"], session);
    let text = w["text"].as_str().unwrap();
    assert!(text.contains("alice started this session"), "{text}");
    assert!(text.contains("Your task is PAY-1"), "{text}");
    assert!(text.contains(st["id"].as_str().unwrap()), "{text}");

    // only folders where the cook already has a station
    let r = app
        .mcp(
            &alice,
            "start_station",
            json!({"project": "PAY", "cook": "clara", "directory": "/etc"}),
        )
        .await;
    assert!(err_text(&r).contains("no station in /etc"), "{r}");
    // only Claude Code cooks
    let r = app
        .mcp(
            &alice,
            "start_station",
            json!({"project": "PAY", "cook": "doug"}),
        )
        .await;
    assert!(err_text(&r).contains("only Claude Code"), "{r}");
    // only cooks you manage
    let r = app
        .mcp(
            &eve,
            "start_station",
            json!({"project": "PAY", "cook": "clara"}),
        )
        .await;
    assert!(err_text(&r).contains("no cook named clara"), "{r}");
    // a task of another pot is refused, an unknown one too
    let r = app
        .mcp(
            &alice,
            "start_station",
            json!({"project": "PAY", "cook": "clara", "task": "PAY-99"}),
        )
        .await;
    err_text(&r);

    // a cook can start a sibling session of its own, with a prompt and no task
    let r = app
        .mcp(
            &clara,
            "start_station",
            json!({"project": "PAY", "prompt": "Review the open PRs", "label": "reviewer"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let sib = common::TestApp::payload(&r);
    assert_eq!(sib["cook"], "clara");
    assert_eq!(sib["label"], "reviewer");
    let w = last_wake(&app).await;
    let text = w["text"].as_str().unwrap();
    assert!(text.contains("Review the open PRs"), "{text}");
    assert!(
        !text.contains("claim_next with your station), work it and complete it"),
        "{text}"
    );

    let (_, all) = app.get(&alice, "/stations?project=PAY").await;
    assert_eq!(all.as_array().unwrap().len(), 3, "{all}");
}
