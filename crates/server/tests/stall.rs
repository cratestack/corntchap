//! A lease ends when the work stops: no progress for STALL_MINUTES (default 120), or MAX_HOLD_HOURS
//! (default 8) held. Heartbeats and activity are not progress; the holder's comments are.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

struct World {
    app: common::TestApp,
    root: String,
    c1: String,
}

async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay"}),
    )
    .await;
    let (_, v) = app
        .post(
            &root,
            "/agents",
            json!({"name": "c1", "kind": "claude-code", "runtime": "local", "max_leases": 3}),
        )
        .await;
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    for t in ["one", "two"] {
        app.post(
            &root,
            "/projects/PAY/tasks",
            json!({"title": t, "items": ["a", "b"]}),
        )
        .await;
    }
    World { app, root, c1 }
}

impl World {
    async fn claim(&self, id: &str, extra: Value) -> Value {
        let mut a = json!({"id": id});
        for (k, v) in extra.as_object().into_iter().flatten() {
            a[k] = v.clone();
        }
        ok(&self.app.mcp(&self.c1, "claim_task", a).await, "claim")
    }

    async fn sql(&self, sql: &str) {
        sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
            .execute(&self.app.state.db)
            .await
            .unwrap();
    }

    async fn secs(&self, expr: &str, number: i64) -> f64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select extract(epoch from {expr} - now())::float8 from tasks where number = {number}"
        )))
        .fetch_one(&self.app.state.db)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_claim_never_outlasts_the_stall_limit() {
    let w = world().await;
    let c = w.claim("PAY-1", json!({"ttlS": 86400})).await;
    let left = w.secs("lease_expires_at", 1).await;
    assert!((7100.0..=7200.0).contains(&left), "capped to 2 h: {left}");
    // the lease view says when it stalls
    let stalls = w.secs("progress_at + interval '2 hours'", 1).await;
    assert!((7100.0..=7200.0).contains(&stalls), "{stalls}");
    assert!(c["lease"]["stalls_at"].is_string(), "{c}");
    assert!(c["lease"]["warning"].is_null(), "plenty of time left: {c}");
    // the same through claim_next
    let n = ok(
        &w.app
            .mcp(
                &w.c1,
                "claim_next",
                json!({"project": "PAY", "ttlS": 86400}),
            )
            .await,
        "next",
    );
    assert_eq!(n["task"]["id"], "PAY-2");
    let left = w.secs("lease_expires_at", 2).await;
    assert!(left <= 7200.0, "{left}");
    // a heartbeat asking for a day is capped too
    let hb = ok(
        &w.app
            .mcp(
                &w.c1,
                "heartbeat",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "ttlS": 86400}),
            )
            .await,
        "heartbeat",
    );
    let left = w.secs("lease_expires_at", 1).await;
    assert!((7100.0..=7200.0).contains(&left), "{left} {hb}");
}

#[tokio::test]
async fn no_progress_ends_the_lease_with_a_reason() {
    let w = world().await;
    let c = w.claim("PAY-1", json!({})).await;
    let epoch = c["lease"]["epoch"].clone();
    // 3 hours since the last progress, the lease itself not yet run out (as before this rule)
    w.sql("update tasks set progress_at = now() - interval '3 hours' where number = 1")
        .await;
    let r = w
        .app
        .mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("lease_lost") && text.contains("no progress"),
        "{text}"
    );
    // ticking an item is a lease call too: it fails the same way
    let r = w
        .app
        .mcp(
            &w.c1,
            "check_item",
            json!({"id": "PAY-1", "epoch": epoch, "position": 1, "done": true}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
}

#[tokio::test]
async fn the_reaper_says_why_and_leaves_a_note() {
    let w = world().await;
    w.claim("PAY-1", json!({})).await;
    w.sql(
        "update tasks set progress_at = now() - interval '3 hours', lease_expires_at = now() - interval '1 second' where number = 1",
    )
    .await;
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 1);
    let (status, reason, agent): (String, String, String) = sqlx::query_as(
        "select t.status, e.payload->>'reason', e.payload->>'agent' from tasks t
           join events e on e.task_id = t.id and e.kind = 'task.lease_expired' where t.number = 1",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), reason.as_str(), agent.as_str()),
        ("open", "stalled", "c1")
    );
    let note: String = sqlx::query_scalar(
        "select c.body from comments c join tasks t on t.id = c.task_id where t.number = 1 and c.author_id is null",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        note.contains("released by the server")
            && note.contains("no progress for 2h")
            && note.contains("c1"),
        "{note}"
    );
    // the holder is told, with the reason
    let wake: String = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' and payload->>'agent' = 'c1' order by seq desc limit 1",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        wake.contains("no progress for 2h") && wake.contains("PAY-1"),
        "{wake}"
    );

    // a lease that merely ran out is `lapsed`, with no note
    w.claim("PAY-2", json!({})).await;
    w.sql("update tasks set lease_expires_at = now() - interval '1 second' where number = 2")
        .await;
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 1);
    let reason: String = sqlx::query_scalar(
        "select e.payload->>'reason' from events e join tasks t on t.id = e.task_id
          where t.number = 2 and e.kind = 'task.lease_expired'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(reason, "lapsed");
    let notes: i64 = sqlx::query_scalar(
        "select count(*) from comments c join tasks t on t.id = c.task_id where t.number = 2",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(notes, 0);
}

#[tokio::test]
async fn the_holders_comment_pushes_the_stall_out_again() {
    let w = world().await;
    let c = w.claim("PAY-1", json!({})).await;
    let epoch = c["lease"]["epoch"].clone();
    // 105 minutes without progress: 15 left, so the lease view warns
    w.sql(
        "update tasks set progress_at = now() - interval '105 minutes',
           lease_expires_at = now() + interval '15 minutes' where number = 1",
    )
    .await;
    let hb = ok(
        &w.app
            .mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
            .await,
        "heartbeat",
    );
    let warn = hb["warning"].as_str().unwrap_or_else(|| panic!("{hb}"));
    assert!(
        warn.contains("no progress since") && warn.contains("comment or tick"),
        "{warn}"
    );
    let left = w.secs("lease_expires_at", 1).await;
    assert!(left <= 15.0 * 60.0 + 5.0, "capped at the stall: {left}");

    ok(
        &w.app
            .mcp(
                &w.c1,
                "comment",
                json!({"id": "PAY-1", "body": "found it", "epoch": epoch}),
            )
            .await,
        "comment",
    );
    let hb = ok(
        &w.app
            .mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
            .await,
        "heartbeat",
    );
    assert!(hb["warning"].is_null(), "{hb}");
    let progress = w.secs("progress_at", 1).await;
    assert!(progress > -30.0, "progress is now: {progress}");
    let left = w.secs("lease_expires_at", 1).await;
    assert!(left > 500.0, "extended by the pot's ttl again: {left}");

    // ticking an item, saving a memory on the task and a question are progress too
    for (what, call) in [
        (
            "item",
            (
                "check_item",
                json!({"id": "PAY-1", "epoch": epoch, "position": 1, "done": true}),
            ),
        ),
        (
            "memory",
            (
                "remember",
                json!({"project": "PAY", "title": "Ledger", "body": "x", "task": "PAY-1"}),
            ),
        ),
        (
            "question",
            (
                "ask",
                json!({"project": "PAY", "title": "Which?", "task": "PAY-1"}),
            ),
        ),
    ] {
        w.sql("update tasks set progress_at = now() - interval '1 hour' where number = 1")
            .await;
        ok(&w.app.mcp(&w.c1, call.0, call.1).await, what);
        let p = w.secs("progress_at", 1).await;
        assert!(p > -30.0, "{what} is progress: {p}");
    }
}

#[tokio::test]
async fn a_heartbeat_alone_is_not_progress() {
    let w = world().await;
    let c = w.claim("PAY-1", json!({})).await;
    let epoch = c["lease"]["epoch"].clone();
    w.sql("update tasks set progress_at = now() - interval '90 minutes' where number = 1")
        .await;
    let before: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("select progress_at from tasks where number = 1")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    for _ in 0..3 {
        ok(
            &w.app
                .mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
                .await,
            "heartbeat",
        );
    }
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("select progress_at from tasks where number = 1")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(before, after, "heartbeats do not move progress_at");
    // nor does a person's comment on the task (it is not the holder's progress)
    app_comment_by_a_person(&w).await;
    let still: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("select progress_at from tasks where number = 1")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(before, still);
}

async fn app_comment_by_a_person(w: &World) {
    ok(
        &w.app
            .mcp(
                &w.root,
                "comment",
                json!({"id": "PAY-1", "body": "how is it going?"}),
            )
            .await,
        "comment",
    );
}

#[tokio::test]
async fn held_too_long_is_released_whatever_the_progress() {
    let w = world().await;
    let c = w.claim("PAY-1", json!({})).await;
    let epoch = c["lease"]["epoch"].clone();
    // held 9 hours, but it commented a minute ago
    w.sql("update tasks set held_since = now() - interval '9 hours', progress_at = now() where number = 1")
        .await;
    let r = w
        .app
        .mcp(&w.c1, "heartbeat", json!({"id": "PAY-1", "epoch": epoch}))
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("lease_lost") && text.contains("maximum"),
        "{text}"
    );
    w.sql("update tasks set lease_expires_at = now() - interval '1 second' where number = 1")
        .await;
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 1);
    let reason: String = sqlx::query_scalar(
        "select e.payload->>'reason' from events e join tasks t on t.id = e.task_id
          where t.number = 1 and e.kind = 'task.lease_expired'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(reason, "max_hold");
    let note: String = sqlx::query_scalar(
        "select c.body from comments c join tasks t on t.id = c.task_id where t.number = 1 and c.author_id is null",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        note.contains("released by the server") && note.contains("8h"),
        "{note}"
    );

    // the warning in the last 20 minutes before the maximum
    let c = w.claim("PAY-2", json!({})).await;
    w.sql("update tasks set held_since = now() - interval '7 hours 50 minutes' where number = 2")
        .await;
    let hb = ok(
        &w.app
            .mcp(
                &w.c1,
                "heartbeat",
                json!({"id": "PAY-2", "epoch": c["lease"]["epoch"]}),
            )
            .await,
        "heartbeat",
    );
    assert!(
        hb["warning"].as_str().unwrap().contains("held since"),
        "{hb}"
    );
    let left = w.secs("lease_expires_at", 2).await;
    assert!(left <= 10.0 * 60.0 + 5.0, "{left}");
}

#[tokio::test]
async fn a_late_comment_does_not_turn_a_stall_into_a_lapse() {
    let w = world().await;
    w.claim("PAY-1", json!({})).await;
    // the cap passed (stalled for 3 h, lease over); the holder comments only now
    w.sql(
        "update tasks set progress_at = now() - interval '3 hours', lease_expires_at = now() - interval '1 second' where number = 1",
    )
    .await;
    let _ = w
        .app
        .mcp(
            &w.c1,
            "comment",
            json!({"id": "PAY-1", "body": "still here"}),
        )
        .await;
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 1);
    let reason: String = sqlx::query_scalar(
        "select e.payload->>'reason' from events e join tasks t on t.id = e.task_id
          where t.number = 1 and e.kind = 'task.lease_expired'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(reason, "stalled");
}
