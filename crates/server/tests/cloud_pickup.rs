//! Cloud cooks are fire and forget, so ready work must find them without an event: a failed
//! delivery is retried by the dispatcher's sweep, a cook that finishes one of two tasks is woken for
//! the other, a target added later hears of work already there, a 429's `Retry-After` is honoured,
//! a task the cook gave back is not offered again until someone touches it, a cook that never
//! claims is woken at most three times, and a cook's owner message is not followed by a second run from the sweep.
mod common;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use corntchap::admin;
use serde_json::{Value, json};

#[derive(Default)]
struct Fake {
    /// answers for the next requests: (status, Retry-After seconds); then 200
    plan: VecDeque<(u16, Option<u64>)>,
    hits: Vec<(Instant, String)>,
}
type Shared = Arc<Mutex<Fake>>;

async fn handle(State(fake): State<Shared>, body: Bytes) -> impl IntoResponse {
    let mut f = fake.lock().unwrap();
    let text = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["text"].as_str().map(String::from))
        .unwrap_or_default();
    f.hits.push((Instant::now(), text));
    let (status, wait) = f.plan.pop_front().unwrap_or((200, None));
    let mut headers = HeaderMap::new();
    if let Some(w) = wait {
        headers.insert(
            "retry-after",
            HeaderValue::from_str(&w.to_string()).unwrap(),
        );
    }
    (StatusCode::from_u16(status).unwrap(), headers, "{}")
}

async fn fake_endpoint(plan: &[(u16, Option<u64>)]) -> (String, Shared) {
    let fake: Shared = Arc::new(Mutex::new(Fake {
        plan: plan.iter().copied().collect(),
        hits: vec![],
    }));
    let app = Router::new().fallback(handle).with_state(fake.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/hook", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, fake)
}

fn hits(f: &Shared) -> Vec<(Instant, String)> {
    f.lock().unwrap().hits.clone()
}

async fn wait_hits(f: &Shared, n: usize, secs: u64) -> Vec<(Instant, String)> {
    for _ in 0..secs * 10 {
        let h = hits(f);
        if h.len() >= n {
            return h;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("expected {n} deliveries, got {:?}", hits(f));
}

struct World {
    app: common::TestApp,
    root: String,
    cook: String,
}

/// A fast sweep and a short coalescing window; one project (PAY) and one cloud cook.
async fn world() -> World {
    world_with(Duration::from_millis(200)).await
}

async fn world_with(coalesce: Duration) -> World {
    // a release must not be held back by the cooldown here: it is the given-back rule under test
    corntchap::service::RELEASE_COOLDOWN_SECS.store(0, std::sync::atomic::Ordering::Relaxed);
    let app = common::start_with(true, |c| {
        c.cloud_sweep = Duration::from_secs(1);
        c.cloud_coalesce = coalesce;
    })
    .await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (_, v) = app
        .post(
            &root,
            "/agents",
            json!({"name": "chef", "kind": "other", "runtime": "cloud"}),
        )
        .await;
    let cook = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "chef"}))
        .await;
    World { app, root, cook }
}

impl World {
    async fn target(&self, url: &str, labels: &[&str]) {
        let (code, r) = self
            .app
            .post(
                &self.root,
                "/agents/chef/wake-targets",
                json!({"type": "webhook", "labels": labels, "config": {"url": url}}),
            )
            .await;
        assert_eq!(code, 200, "{r}");
    }

    async fn task(&self, title: &str, labels: &[&str]) {
        let (code, r) = self
            .app
            .post(
                &self.root,
                "/projects/PAY/tasks",
                json!({"title": title, "labels": labels}),
            )
            .await;
        assert_eq!(code, 200, "{r}");
    }

    /// What the restaurant decided, in order: (decision, reason).
    async fn decisions(&self) -> Vec<(String, String)> {
        sqlx::query_as(
            "select e.payload->>'decision', e.payload->>'reason' from events e
               join principals a on a.id = e.actor_id and a.kind = 'restaurant'
              where e.kind = 'restaurant.decided' order by e.seq",
        )
        .fetch_all(&self.app.state.db)
        .await
        .unwrap()
    }

    async fn claim(&self) -> Value {
        let r = self
            .app
            .mcp(&self.cook, "claim_next", json!({"project": "PAY"}))
            .await;
        assert_ne!(r["isError"], true, "{r}");
        common::TestApp::payload(&r)
    }
}

#[tokio::test]
async fn a_failed_delivery_is_retried_by_the_sweep() {
    let (url, fake) = fake_endpoint(&[(500, None)]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Refund screen", &[]).await;
    let h = wait_hits(&fake, 2, 15).await;
    assert!(
        h[1].1.contains("new work is ready for chef in PAY"),
        "{h:?}"
    );
}

#[tokio::test]
async fn a_cook_that_finishes_one_of_two_tasks_is_woken_for_the_other() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("One", &[]).await;
    w.task("Two", &[]).await;
    wait_hits(&fake, 1, 10).await;
    let c = w.claim().await;
    let id = c["task"]["id"].as_str().unwrap().to_string();
    let epoch = c["lease"]["epoch"].as_i64().unwrap();
    let done = w
        .app
        .mcp(
            &w.cook,
            "complete",
            json!({"id": id, "epoch": epoch, "result": "done"}),
        )
        .await;
    assert_ne!(done["isError"], true, "{done}");
    let before = hits(&fake).len();
    // its own completion sends no event to it; the sweep finds the second task
    wait_hits(&fake, before + 1, 10).await;
    let last = hits(&fake).pop().unwrap().1;
    assert!(last.contains("new work is ready"), "{last}");
}

#[tokio::test]
async fn a_released_task_is_not_offered_again_until_someone_touches_it() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Too big", &[]).await;
    wait_hits(&fake, 1, 10).await;
    let c = w.claim().await;
    let epoch = c["lease"]["epoch"].as_i64().unwrap();
    let r = w
        .app
        .mcp(
            &w.cook,
            "release",
            json!({"id": "PAY-1", "epoch": epoch, "reason": "needs a person"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = hits(&fake).len();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(hits(&fake).len(), before, "{:?}", hits(&fake));
    // someone comments: it is worth another look
    let r = w
        .app
        .mcp(
            &w.root,
            "comment",
            json!({"id": "PAY-1", "body": "split it in two"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    for _ in 0..100 {
        if hits(&fake)[before..]
            .iter()
            .any(|h| h.1.contains("new work is ready"))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("not offered again: {:?}", hits(&fake));
}

#[tokio::test]
async fn the_wake_names_the_targets_labels() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &["backend", "api"]).await;
    w.task("Endpoint", &["backend"]).await;
    let h = wait_hits(&fake, 1, 10).await;
    assert!(
        h[0].1
            .contains("claim_next with project PAY and labels [backend, api]"),
        "{}",
        h[0].1
    );
}

#[tokio::test]
async fn a_429_waits_for_retry_after() {
    let (url, fake) = fake_endpoint(&[(429, Some(3))]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Refund screen", &[]).await;
    let first = wait_hits(&fake, 1, 10).await[0].0;
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert_eq!(hits(&fake).len(), 1, "delivered before Retry-After");
    // the wait is the restaurant's decision: told once, with the time it ends
    let held: Vec<(String, String)> = w.decisions().await;
    assert_eq!(
        held,
        [("wake.held".to_string(), "retry_after".to_string())],
        "{held:?}"
    );
    let (until, text): (String, String) = sqlx::query_as(
        "select payload->>'until', payload->>'text' from events where kind = 'restaurant.decided'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        until.starts_with("20") && text.contains("429"),
        "{until} {text}"
    );
    let h = wait_hits(&fake, 2, 10).await;
    assert!(
        h[1].0.duration_since(first) >= Duration::from_secs(3),
        "second delivery after {:?}",
        h[1].0.duration_since(first)
    );
}

#[tokio::test]
async fn a_target_added_after_the_task_is_woken_by_the_sweep() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.task("Waiting already", &[]).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(hits(&fake).is_empty());
    w.target(&url, &[]).await;
    wait_hits(&fake, 1, 10).await;
}

#[tokio::test]
async fn a_cook_that_never_claims_is_woken_three_times() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Nobody takes it", &[]).await;
    wait_hits(&fake, 3, 15).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(hits(&fake).len(), 3, "{:?}", hits(&fake));
    // the cap is the restaurant's decision, told once, with the count
    let capped: Vec<(String, String)> = w
        .decisions()
        .await
        .into_iter()
        .filter(|d| d.0 == "wake.capped")
        .collect();
    assert_eq!(capped, [("wake.capped".to_string(), "sweep".to_string())]);
    let (misses, text, actor): (i32, String, String) = sqlx::query_as(
        "select (e.payload->>'misses')::int, e.payload->>'text', a.name from events e join principals a on a.id = e.actor_id
          where e.kind = 'restaurant.decided' and e.payload->>'decision' = 'wake.capped'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!((misses, actor.as_str()), (3, "restaurant"));
    assert!(
        text.contains("chef") && text.contains("3 times in a row"),
        "{text}"
    );
    // more sweeps with nothing new to decide write nothing at all
    let events: i64 = sqlx::query_scalar("select count(*) from events")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    let after: i64 = sqlx::query_scalar("select count(*) from events")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(after, events, "an idle sweep tick writes no event");
    // what_now tells the cook the restaurant stopped waking it
    let now = w
        .app
        .mcp(&w.cook, "what_now", json!({"project": "PAY"}))
        .await;
    let now = common::TestApp::payload(&now);
    assert!(
        now["restaurant"]
            .as_str()
            .unwrap_or("")
            .contains("3 times in a row"),
        "{now}"
    );
    // a claim ends it
    w.claim().await;
    let now = common::TestApp::payload(
        &w.app
            .mcp(&w.cook, "what_now", json!({"project": "PAY"}))
            .await,
    );
    assert!(now["restaurant"].is_null(), "{now}");
}

#[tokio::test]
async fn a_lapsed_lease_is_not_offered_again_until_someone_touches_it() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Run dies", &[]).await;
    wait_hits(&fake, 1, 10).await;
    w.claim().await;
    // the run dies: the lease runs out and the reaper gives the task back
    sqlx::query(
        "update tasks set lease_expires_at = now() - interval '1 second' where status = 'claimed'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 1);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = hits(&fake).len();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(hits(&fake).len(), before, "{:?}", hits(&fake));
    let r = w
        .app
        .mcp(
            &w.root,
            "comment",
            json!({"id": "PAY-1", "body": "try again"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    for _ in 0..100 {
        if hits(&fake)[before..]
            .iter()
            .any(|h| h.1.contains("new work is ready"))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("not offered again: {:?}", hits(&fake));
}

#[tokio::test]
async fn a_cook_holding_a_live_lease_is_not_swept() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("One", &[]).await;
    w.task("Two", &[]).await;
    wait_hits(&fake, 1, 10).await;
    w.claim().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = hits(&fake).len();
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(hits(&fake).len(), before, "{:?}", hits(&fake));
}

#[tokio::test]
async fn an_owner_message_is_not_followed_by_a_sweep_run() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world_with(Duration::from_secs(10)).await;
    w.target(&url, &[]).await;
    let r = w
        .app
        .mcp(
            &w.root,
            "pass",
            json!({"project": "PAY", "message": "ping", "to": "chef"}),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    wait_hits(&fake, 1, 10).await;
    // ready work appears right after: the run just started is the wake, the sweep adds none
    w.task("Refund screen", &[]).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let h = hits(&fake);
    assert_eq!(h.len(), 1, "{h:?}");
}

#[tokio::test]
async fn a_failing_target_gets_at_most_three_work_deliveries() {
    let plan: Vec<(u16, Option<u64>)> = vec![(500, None); 50];
    let (url, fake) = fake_endpoint(&plan).await;
    let w = world().await;
    w.target(&url, &[]).await;
    w.task("Refund screen", &[]).await;
    wait_hits(&fake, 3, 20).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(hits(&fake).len(), 3, "{:?}", hits(&fake));
}

#[tokio::test]
async fn a_wake_held_back_by_the_backoff_is_said_once() {
    let (url, fake) = fake_endpoint(&[]).await;
    let w = world_with(Duration::from_secs(30)).await;
    w.target(&url, &[]).await;
    w.task("One", &[]).await;
    wait_hits(&fake, 1, 10).await;
    // more ready work while the cook was just woken: the next wake waits, and the restaurant says so
    w.task("Two", &[]).await;
    for _ in 0..50 {
        if !w.decisions().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        w.decisions().await,
        [("wake.held".to_string(), "backoff".to_string())]
    );
    let (misses, until, text): (i32, String, String) = sqlx::query_as(
        "select (payload->>'misses')::int, payload->>'until', payload->>'text' from events where kind = 'restaurant.decided'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(misses, 1);
    assert!(
        until.starts_with("20") && text.contains("chef"),
        "{until} {text}"
    );
    // the same decision again (more work, more sweeps) writes nothing
    w.task("Three", &[]).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(w.decisions().await.len(), 1, "{:?}", w.decisions().await);
    assert_eq!(hits(&fake).len(), 1, "no second wake inside the window");
    // restaurant.decided never wakes anybody: it is not a dispatcher trigger
    assert!(
        !corntchap::jobs::wake_triggers().contains(&"restaurant.decided"),
        "a decision must not trigger a wake"
    );
}
