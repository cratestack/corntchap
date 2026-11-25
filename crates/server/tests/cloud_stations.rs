//! One station per cloud run: a Claude routine run, a Devin session, a Coder run, a webhook or
//! GitHub-comment run opens a station (with the run's session URL when it has one), the wake text
//! names it, and everything the run does is attributed to it. Runs against one fake endpoint that
//! plays a routine, Devin, Coder, a webhook and the GitHub comments API.
mod common;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::IntoResponse,
};
use corntchap::admin;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Req>>>,
    /// Devin follow-ups answer 404 (the session is gone)
    gone: Arc<AtomicBool>,
    /// everything answers 500
    broken: Arc<AtomicBool>,
    /// everything answers 429 (nothing started)
    throttled: Arc<AtomicBool>,
    /// the session URL the routine answers with, instead of a claude.ai one
    routine_url: Arc<Mutex<Option<String>>>,
}

async fn handle(State(f): State<Fake>, method: Method, uri: Uri, body: Bytes) -> impl IntoResponse {
    let path = uri.path().to_string();
    let n = {
        let mut s = f.seen.lock().unwrap();
        s.push(Req {
            method: method.to_string(),
            path: path.clone(),
            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        });
        s.iter().filter(|r| r.method == "POST").count()
    };
    if f.broken.load(Ordering::SeqCst) {
        return (StatusCode::INTERNAL_SERVER_ERROR, "{}".to_string());
    }
    if f.throttled.load(Ordering::SeqCst) {
        return (StatusCode::TOO_MANY_REQUESTS, "{}".to_string());
    }
    let ok = |v: Value| (StatusCode::OK, v.to_string());
    if path.starts_with("/routine") {
        let url = f
            .routine_url
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| format!("https://claude.ai/code/session_{n}"));
        return ok(json!({"claude_code_session_url": url}));
    }
    if path.starts_with("/hook") {
        // a webhook's answer is not trusted with a session page
        return ok(
            json!({"claude_code_session_url": "https://claude.ai/code/from-a-webhook",
            "url": "https://example.com/x"}),
        );
    }
    if path.ends_with("/messages") && path.contains("/v3/") {
        if f.gone.load(Ordering::SeqCst) {
            return (StatusCode::NOT_FOUND, "{}".to_string());
        }
        return ok(json!({"status": "running", "url": "https://app.devin.ai/sessions/same"}));
    }
    if path.ends_with("/messages") {
        return ok(json!({"queued": false}));
    }
    if path.ends_with("/sessions") {
        return ok(json!({"session_id": format!("devin-{n}"), "status": "new",
            "url": format!("https://app.devin.ai/sessions/{n}")}));
    }
    if path == "/api/v2/chats" {
        return ok(json!({"id": format!("chat-{n}"), "status": "running"}));
    }
    ok(json!({}))
}

async fn fake() -> (String, Fake) {
    let f = Fake::default();
    let app = Router::new().fallback(handle).with_state(f.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(l, app).await.unwrap();
    });
    (base, f)
}

impl Fake {
    async fn wait(&self, part: &str, n: usize) -> Vec<Req> {
        for _ in 0..80 {
            // "/sessions" is Devin's create, not the messages under a session
            let got: Vec<Req> = self
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| {
                    r.method == "POST"
                        && if part.ends_with("/sessions") {
                            r.path.ends_with(part)
                        } else {
                            r.path.contains(part)
                        }
                })
                .cloned()
                .collect();
            if got.len() >= n {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("no POST *{part} x{n}: {:?}", self.seen.lock().unwrap());
    }
}

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

/// All the strings in a request body, to look for the station in whatever field carries the text.
fn text_of(r: &Req) -> String {
    r.body.to_string()
}

struct World {
    app: common::TestApp,
    root: String,
    base: String,
    fake: Fake,
    /// cook name -> token
    tokens: std::collections::HashMap<String, String>,
}

/// An admin, a team, a pot PAY, and one cook per (name, target type), each with its target.
async fn world(cooks: &[(&str, &str)]) -> World {
    let (base, fake) = fake().await;
    let app = common::start(true).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(&root, "/teams/acme/members", json!({"principal": "octo"}))
        .await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let mut tokens = std::collections::HashMap::new();
    for (name, typ) in cooks {
        let (_, v) = app
            .post(
                &root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "cloud"}),
            )
            .await;
        tokens.insert(name.to_string(), v["token"].as_str().unwrap().to_string());
        app.post(&root, "/teams/acme/members", json!({"principal": name}))
            .await;
        let config = match *typ {
            "claude-routine" => json!({"url": format!("{base}/routine/fire"), "token": "sk-x"}),
            "devin" => {
                json!({"url": format!("{base}/"), "token": "cog_x", "organization": "org-1"})
            }
            "coder" => {
                json!({"url": format!("{base}/"), "token": "ct", "organization": "11111111-2222-3333-4444-555555555555"})
            }
            "webhook" => json!({"url": format!("{base}/hook")}),
            "github-comment" => {
                json!({"repo": "acme/pay", "issue": 7, "token": "ghp_x", "api_url": base})
            }
            other => panic!("{other}"),
        };
        let (code, r) = app
            .post(
                &root,
                &format!("/agents/{name}/wake-targets"),
                json!({"type": typ, "config": config}),
            )
            .await;
        assert_eq!(code, 200, "{r}");
    }
    World {
        app,
        root,
        base,
        fake,
        tokens,
    }
}

struct Run {
    id: Uuid,
    reference: String,
    runtime: String,
    url: Option<String>,
    session: String,
    ended: bool,
    target: Option<Uuid>,
}

impl World {
    async fn runs(&self, cook: &str) -> Vec<Run> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(Uuid, String, String, Option<String>, String, bool, Option<Uuid>)> =
            sqlx::query_as(
                "select st.id, st.ref, st.runtime, st.session_url, st.session_ref, st.ended_at is not null, st.target_id
                 from stations st join principals a on a.id = st.agent_id
                 where a.name = $1 and st.kind = 'cloud-run' order by st.created_at",
            )
            .bind(cook)
            .fetch_all(&self.app.state.db)
            .await
            .unwrap();
        rows.into_iter()
            .map(
                |(id, reference, runtime, url, session, ended, target)| Run {
                    id,
                    reference,
                    runtime,
                    url,
                    session,
                    ended,
                    target,
                },
            )
            .collect()
    }

    /// Wait until the cook has `n` runs whose delivery was recorded (session settled).
    async fn settled_runs(&self, cook: &str, n: usize) -> Vec<Run> {
        for _ in 0..80 {
            let runs = self.runs(cook).await;
            let done: i64 = sqlx::query_scalar(
                "select count(*) from stations st join principals a on a.id = st.agent_id
                 where a.name = $1 and st.kind = 'cloud-run' and st.delivery_seq is not null",
            )
            .bind(cook)
            .fetch_one(&self.app.state.db)
            .await
            .unwrap();
            if runs.len() >= n && done as usize >= n {
                return runs;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!(
            "{cook} has no {n} settled runs: {:?}",
            self.runs(cook).await.len()
        );
    }

    async fn new_task(&self, title: &str) {
        let (code, r) = self
            .app
            .post(&self.root, "/projects/PAY/tasks", json!({"title": title}))
            .await;
        assert_eq!(code, 200, "{r}");
    }

    async fn dm(&self, cook: &str, text: &str) {
        ok(
            &self
                .app
                .mcp(
                    &self.root,
                    "pass",
                    json!({"project": "PAY", "message": text, "to": cook}),
                )
                .await,
            "dm",
        );
    }

    async fn ended(&self, id: Uuid) -> bool {
        sqlx::query_scalar("select ended_at is not null from stations where id = $1")
            .bind(id)
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }

    async fn task(&self, id: &str) -> Value {
        ok(
            &self
                .app
                .mcp(&self.root, "get_task", json!({"id": id}))
                .await,
            "get_task",
        )
    }
}

#[tokio::test]
async fn a_routine_run_gets_a_station_with_its_session_url() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    let fires = w.fake.wait("/routine/fire", 1).await;
    let runs = w.settled_runs("claude-web", 1).await;
    let run = &runs[0];
    assert_eq!(run.runtime, "claude-routine");
    assert_eq!(run.url.as_deref(), Some("https://claude.ai/code/session_1"));
    assert!(run.target.is_some());
    // the wake text names the station and says to pass it
    let text = text_of(&fires[0]);
    assert!(
        text.contains(&format!("Your station is {}", run.reference))
            && text.contains(&format!("pass station={}", run.reference))
            && text.contains("what_now, claim_next, claim_task, comment, pass and ask"),
        "{text}"
    );
    // wake.delivered carries the station and its session
    let payload: Value = sqlx::query_scalar(
        "select payload from events where kind = 'wake.delivered' and payload->>'type' = 'claude-routine'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(payload["station"], json!(run.reference), "{payload}");
    assert_eq!(payload["session_url"], json!(run.url), "{payload}");
    // the station as the API shows it
    let list = ok(
        &w.app
            .mcp(&w.root, "list_stations", json!({"cook": "claude-web"}))
            .await,
        "list_stations",
    )["items"]
        .clone();
    let st = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!(run.reference))
        .unwrap_or_else(|| panic!("{list}"));
    assert_eq!(
        (st["kind"].as_str(), st["target_type"].as_str()),
        (Some("cloud-run"), Some("claude-routine"))
    );
    assert_eq!(st["session_url"], json!(run.url));
    assert!(st["ended_at"].is_null());
    let live = ok(
        &w.app
            .mcp(
                &w.root,
                "list_stations",
                json!({"cook": "claude-web", "live": false}),
            )
            .await,
        "list ended",
    )["items"]
        .clone();
    assert!(live.as_array().unwrap().is_empty(), "{live}");

    // claim and comment with the station: attributed, with the URL
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": run.reference}),
            )
            .await,
        "claim",
    );
    assert_eq!(claim["task"]["station"], json!(run.reference), "{claim}");
    assert_eq!(claim["task"]["station_url"], json!(run.url), "{claim}");
    ok(
        &w.app
            .mcp(
                tok,
                "comment",
                json!({"id": "PAY-1", "body": "on it", "station": run.reference}),
            )
            .await,
        "comment",
    );
    let t = w.task("PAY-1").await;
    assert_eq!(t["comments"][0]["station"], json!(run.reference), "{t}");
    assert_eq!(t["comments"][0]["station_url"], json!(run.url), "{t}");
    assert_eq!(t["station_url"], json!(run.url), "{t}");
}

#[tokio::test]
async fn calls_without_a_station_go_to_the_latest_run() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let first = w.settled_runs("claude-web", 1).await.remove(0);
    // a newer run (a second wake) takes over attribution
    let (agent, project): (Uuid, Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'claude-web'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    let mut conn = w.app.state.db.acquire().await.unwrap();
    let newer =
        corntchap::stations::new_run(&mut conn, agent, project, first.target, "claude-routine")
            .await
            .unwrap();
    corntchap::stations::settle_run(
        &mut conn,
        newer.id,
        None,
        Some("https://claude.ai/code/newer"),
        None,
    )
    .await
    .unwrap();
    drop(conn);
    let claim = ok(
        &w.app
            .mcp(tok, "claim_next", json!({"project": "PAY"}))
            .await,
        "claim_next",
    );
    assert_eq!(claim["task"]["station"], json!(newer.reference), "{claim}");
    assert_eq!(
        claim["task"]["station_url"], "https://claude.ai/code/newer",
        "{claim}"
    );
    ok(
        &w.app
            .mcp(
                tok,
                "comment",
                json!({"id": "PAY-1", "body": "no station named"}),
            )
            .await,
        "comment",
    );
    let t = w.task("PAY-1").await;
    assert_eq!(t["comments"][0]["station"], json!(newer.reference), "{t}");
    assert_eq!(
        t["comments"][0]["station_url"], "https://claude.ai/code/newer",
        "{t}"
    );
    // a person's comment is not attributed to any run
    ok(
        &w.app
            .mcp(&w.root, "comment", json!({"id": "PAY-1", "body": "thanks"}))
            .await,
        "person comment",
    );
    let t = w.task("PAY-1").await;
    assert!(t["comments"][1]["station"].is_null(), "{t}");
}

#[tokio::test]
async fn devin_reuses_its_station_and_a_gone_session_gets_a_new_one() {
    let w = world(&[("devin-cook", "devin")]).await;
    w.new_task("Refund screen").await;
    let created = w.fake.wait("/sessions", 1).await;
    let runs = w.settled_runs("devin-cook", 1).await;
    let s1 = &runs[0];
    assert_eq!(s1.runtime, "devin");
    assert_eq!(s1.url.as_deref(), Some("https://app.devin.ai/sessions/1"));
    assert!(s1.session.starts_with("devin-"), "{}", s1.session);
    assert!(text_of(&created[0]).contains(&format!("Your station is {}", s1.reference)));

    // the next wake is a follow-up to the same session: the same station
    w.dm("devin-cook", "carry on").await;
    let msgs = w.fake.wait("/messages", 1).await;
    assert!(
        text_of(&msgs[0]).contains(&format!("Your station is {}", s1.reference)),
        "{:?}",
        msgs[0]
    );
    let runs = w.runs("devin-cook").await;
    assert_eq!(runs.len(), 1, "a follow-up opens no new station");
    assert!(!runs[0].ended);

    // the session is gone: a new session, a new station, its ref in the text
    w.fake.gone.store(true, Ordering::SeqCst);
    w.dm("devin-cook", "are you there").await;
    let created = w.fake.wait("/sessions", 2).await;
    let runs = w.settled_runs("devin-cook", 2).await;
    let s2 = runs.iter().find(|r| r.id != s1.id).unwrap();
    let text = text_of(&created[1]);
    assert!(
        text.contains(&format!("Your station is {}", s2.reference))
            && !text.contains(&s1.reference),
        "{text}"
    );
    assert!(
        s2.url
            .as_deref()
            .unwrap()
            .starts_with("https://app.devin.ai/sessions/")
    );
    let stored: Option<Uuid> =
        sqlx::query_scalar("select station_id from coder_runs where project = 'PAY'")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(stored, Some(s2.id));
    assert!(
        w.ended(s1.id).await,
        "the station of the gone session is over"
    );
    assert!(!w.ended(s2.id).await);
    let delivered: Vec<Value> = sqlx::query_scalar(
        "select payload from events where kind = 'wake.delivered' and payload->>'type' = 'devin' order by seq",
    )
    .fetch_all(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        delivered
            .iter()
            .any(|p| p["station"] == json!(s2.reference)),
        "{delivered:?}"
    );
}

#[tokio::test]
async fn coder_webhook_and_github_comment_runs_get_stations_without_a_url() {
    let w = world(&[
        ("coder-cook", "coder"),
        ("hook-cook", "webhook"),
        ("gh-cook", "github-comment"),
    ])
    .await;
    w.new_task("Refund screen").await;
    let chat = w.fake.wait("/api/v2/chats", 1).await;
    let hook = w.fake.wait("/hook", 1).await;
    let comment = w.fake.wait("/issues/7/comments", 1).await;
    for (cook, req, runtime) in [
        ("coder-cook", &chat[0], "coder"),
        ("hook-cook", &hook[0], "webhook"),
        ("gh-cook", &comment[0], "github-comment"),
    ] {
        let runs = w.settled_runs(cook, 1).await;
        let run = &runs[0];
        assert_eq!(run.runtime, runtime);
        assert!(run.url.is_none(), "{cook}");
        assert!(
            text_of(req).contains(&format!("Your station is {}", run.reference)),
            "{cook}: {:?}",
            req.body
        );
    }
    // Coder keeps its run's station on the run
    let coder = w.runs("coder-cook").await;
    let stored: Option<Uuid> =
        sqlx::query_scalar("select station_id from coder_runs where project = 'PAY'")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(stored, Some(coder[0].id));
    let _ = &w.base;
}

#[tokio::test]
async fn a_run_ends_when_it_holds_nothing_and_a_later_call_reopens_it() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let run = w.settled_runs("claude-web", 1).await.remove(0);
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": run.reference}),
            )
            .await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    assert!(!w.ended(run.id).await, "it holds a task");
    ok(
        &w.app
            .mcp(
                tok,
                "release",
                json!({"id": "PAY-1", "epoch": epoch, "reason": "PR open: https://github.com/acme/pay/pull/1"}),
            )
            .await,
        "release",
    );
    assert!(w.ended(run.id).await, "the last task was released");
    // the release reason is the run's comment
    let t = w.task("PAY-1").await;
    assert_eq!(t["comments"][0]["station"], json!(run.reference), "{t}");

    // naming the station again reopens it
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": run.reference}),
            )
            .await,
        "claim again",
    );
    assert!(!w.ended(run.id).await, "a claim reopens the run");
    // its lease lapses and is reaped: the run is over again
    sqlx::query("update tasks set lease_expires_at = now() - interval '1 minute' where claimed_by is not null")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    assert!(claim["lease"]["epoch"].is_number());
    corntchap::jobs::reap(&w.app.state).await.unwrap();
    assert!(w.ended(run.id).await, "the reaped lease ended the run");

    // a quiet run that holds nothing ends on its own after the stall time
    sqlx::query("update stations set ended_at = null, created_at = now() - interval '3 hours', last_woken_at = now() - interval '3 hours' where id = $1")
        .bind(run.id)
        .execute(&w.app.state.db)
        .await
        .unwrap();
    // (a task the run once held was touched just now: that is not the run's activity)
    sqlx::query("update comments set created_at = now() - interval '3 hours'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    assert!(
        corntchap::stations::end_stale_runs(&w.app.state.db)
            .await
            .unwrap()
            >= 1
    );
    assert!(w.ended(run.id).await);
    // the restaurant ended it, and says which run and why
    let (actor, reason, station, agent): (String, String, String, String) = sqlx::query_as(
        "select a.name, e.payload->>'reason', e.payload->>'station', e.payload->>'agent'
         from events e join principals a on a.id = e.actor_id
         where e.kind = 'restaurant.decided' and e.payload->>'decision' = 'run.ended'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(
        (
            actor.as_str(),
            reason.as_str(),
            station.as_str(),
            agent.as_str()
        ),
        ("restaurant", "stale", run.reference.as_str(), "claude-web")
    );
    // a run that is not quiet stays
    sqlx::query("update stations set ended_at = null, last_woken_at = now() where id = $1")
        .bind(run.id)
        .execute(&w.app.state.db)
        .await
        .unwrap();
    corntchap::stations::end_stale_runs(&w.app.state.db)
        .await
        .unwrap();
    assert!(!w.ended(run.id).await);
}

#[tokio::test]
async fn cloud_runs_are_not_sessions_to_wake_or_make_main() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let run = w.settled_runs("claude-web", 1).await.remove(0);
    let (agent, project): (Uuid, Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'claude-web'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    let mut conn = w.app.state.db.acquire().await.unwrap();
    let (has_any, picked) = corntchap::stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    assert!(
        !has_any && picked.is_none(),
        "a cloud run is never woken as a session"
    );
    drop(conn);
    let (code, r) = w
        .app
        .post(
            &w.root,
            &format!("/stations/{}/main", run.reference),
            json!({"main": true}),
        )
        .await;
    assert_eq!(code, 400, "{r}");
    assert!(r.to_string().contains("cloud run"), "{r}");
    // and one cannot address a pass to it
    let r = w
        .app
        .mcp(
            &w.root,
            "pass",
            json!({"project": "PAY", "message": "hi", "to": run.reference}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
}

async fn wait_failed(w: &World, n: i64) {
    for _ in 0..80 {
        let failed: i64 =
            sqlx::query_scalar("select count(*) from events where kind = 'wake.failed'")
                .fetch_one(&w.app.state.db)
                .await
                .unwrap();
        if failed >= n {
            // record_delivery settles the station right after emitting
            tokio::time::sleep(Duration::from_millis(300)).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("no {n} failed wakes");
}

#[tokio::test]
async fn a_throttled_delivery_leaves_no_station() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    w.fake.throttled.store(true, Ordering::SeqCst);
    w.new_task("Refund screen").await;
    wait_failed(&w, 1).await;
    assert!(
        w.runs("claude-web").await.is_empty(),
        "429: nothing was sent, so no station"
    );
}

#[tokio::test]
async fn a_failed_delivery_keeps_the_station_of_a_run_that_may_have_started() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.fake.broken.store(true, Ordering::SeqCst);
    w.new_task("Refund screen").await;
    wait_failed(&w, 1).await;
    // a 500 may come after the run started: the station stays, ended, and the run can use it
    let runs = w.runs("claude-web").await;
    assert_eq!(runs.len(), 1, "the station was kept");
    let run = &runs[0];
    assert!(run.ended);
    // not in the default listing, but in the cloud runs
    let list = |args: Value| async {
        ok(
            &w.app.mcp(&w.root, "list_stations", args).await,
            "list_stations",
        )["items"]
            .clone()
    };
    let default = list(json!({"cook": "claude-web"})).await;
    assert!(default.as_array().unwrap().is_empty(), "{default}");
    let runs_listed = list(json!({"cook": "claude-web", "kind": "cloud-run"})).await;
    assert_eq!(runs_listed.as_array().unwrap().len(), 1, "{runs_listed}");
    assert_eq!(runs_listed[0]["id"], json!(run.reference));
    let capped =
        list(json!({"cook": "claude-web", "kind": "cloud-run", "page": {"limit": 1}})).await;
    assert_eq!(capped.as_array().unwrap().len(), 1);

    // the run's later calls name that station: they are attributed to it, and it reopens
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": run.reference}),
            )
            .await,
        "claim",
    );
    assert_eq!(claim["task"]["station"], json!(run.reference), "{claim}");
    assert!(!w.ended(run.id).await, "a call from the run reopens it");
    let live = list(json!({"cook": "claude-web"})).await;
    assert_eq!(live.as_array().unwrap().len(), 1, "{live}");
}

#[tokio::test]
async fn a_station_that_does_not_exist_falls_back_for_a_fire_and_forget_agent() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let run = w.settled_runs("claude-web", 1).await.remove(0);
    ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": "st_0123456789"}),
            )
            .await,
        "claim with a station that is gone",
    );
    ok(
        &w.app
            .mcp(
                tok,
                "comment",
                json!({"id": "PAY-1", "body": "hi", "station": "st_0123456789"}),
            )
            .await,
        "comment",
    );
    let t = w.task("PAY-1").await;
    assert_eq!(t["comments"][0]["station"], json!(run.reference), "{t}");
    assert_eq!(t["station"], json!(run.reference), "{t}");
}

#[tokio::test]
async fn session_urls_are_kept_only_for_known_hosts_over_https() {
    use corntchap::stations::safe_session_url as safe;
    assert_eq!(
        safe("claude-routine", "https://claude.ai/code/session_1").as_deref(),
        Some("https://claude.ai/code/session_1")
    );
    assert!(safe("claude-routine", "https://sub.claude.ai/x").is_some());
    assert!(safe("devin", "https://app.devin.ai/sessions/1").is_some());
    for bad in [
        "http://claude.ai/x",
        "javascript:alert(1)",
        "https://evil.com/claude.ai",
        "https://claude.ai.evil.com/x",
        "https://notclaude.ai/x",
        "https://user:pw@claude.ai/x",
        "https://app.devin.ai/x",
        "not a url",
    ] {
        assert!(safe("claude-routine", bad).is_none(), "{bad}");
    }
    assert!(safe("devin", "https://claude.ai/x").is_none());
    for typ in ["webhook", "github-comment", "coder"] {
        assert!(safe(typ, "https://claude.ai/x").is_none(), "{typ}");
    }

    // a routine that answers with a page elsewhere: no link, and not copied into the label
    let w = world(&[("claude-web", "claude-routine")]).await;
    *w.fake.routine_url.lock().unwrap() = Some("https://evil.example/phish".into());
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let run = w.settled_runs("claude-web", 1).await.remove(0);
    assert!(run.url.is_none());
    let label: Option<String> = sqlx::query_scalar("select label from stations where id = $1")
        .bind(run.id)
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert!(!label.unwrap_or_default().contains("evil"), "label");
}

#[tokio::test]
async fn a_webhook_answer_does_not_set_a_session_url() {
    let w = world(&[("hook-cook", "webhook")]).await;
    w.new_task("Refund screen").await;
    w.fake.wait("/hook", 1).await;
    let run = w.settled_runs("hook-cook", 1).await.remove(0);
    assert!(run.url.is_none(), "{:?}", run.url);
}

#[tokio::test]
async fn a_reused_station_ends_again_when_the_follow_up_fails() {
    let w = world(&[("devin-cook", "devin")]).await;
    w.new_task("Refund screen").await;
    w.fake.wait("/sessions", 1).await;
    let s1 = w.settled_runs("devin-cook", 1).await.remove(0);
    sqlx::query("update stations set ended_at = now() where id = $1")
        .bind(s1.id)
        .execute(&w.app.state.db)
        .await
        .unwrap();
    w.fake.broken.store(true, Ordering::SeqCst);
    w.dm("devin-cook", "carry on").await;
    wait_failed(&w, 1).await;
    let runs = w.runs("devin-cook").await;
    assert_eq!(runs.len(), 1, "the follow-up reuses the station");
    assert!(w.ended(s1.id).await, "a failed follow-up leaves it ended");
}

#[tokio::test]
async fn a_gone_session_opens_one_station_not_two() {
    let w = world(&[("devin-cook", "devin")]).await;
    w.new_task("Refund screen").await;
    w.fake.wait("/sessions", 1).await;
    w.settled_runs("devin-cook", 1).await;
    // a session from before stations: the run row has none, so the wake opens a station of its own
    sqlx::query("update coder_runs set station_id = null")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    w.fake.gone.store(true, Ordering::SeqCst);
    w.dm("devin-cook", "are you there").await;
    w.fake.wait("/sessions", 2).await;
    let runs = w.settled_runs("devin-cook", 2).await;
    assert_eq!(
        runs.len(),
        2,
        "one for the first session, one for the second: {}",
        runs.len()
    );
}

#[tokio::test]
async fn a_read_never_reopens_and_a_call_about_a_task_goes_to_its_run() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let a = w.settled_runs("claude-web", 1).await.remove(0);
    // an ended run: what_now (a read) leaves it ended
    sqlx::query("update stations set ended_at = now() where id = $1")
        .bind(a.id)
        .execute(&w.app.state.db)
        .await
        .unwrap();
    ok(
        &w.app.mcp(tok, "what_now", json!({"project": "PAY"})).await,
        "what_now",
    );
    assert!(w.ended(a.id).await, "a read does not reopen a run");
    ok(
        &w.app
            .mcp(
                tok,
                "what_now",
                json!({"project": "PAY", "station": a.reference}),
            )
            .await,
        "what_now naming it",
    );
    assert!(w.ended(a.id).await, "nor does naming it on a read");

    // two live runs: the one that holds the task gets the call about it, not the newest
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": a.reference}),
            )
            .await,
        "claim",
    );
    assert!(claim["lease"]["epoch"].is_number());
    let (agent, project): (Uuid, Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'claude-web'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    let mut conn = w.app.state.db.acquire().await.unwrap();
    let b = corntchap::stations::new_run(&mut conn, agent, project, a.target, "claude-routine")
        .await
        .unwrap();
    drop(conn);
    ok(
        &w.app
            .mcp(tok, "comment", json!({"id": "PAY-1", "body": "no station"}))
            .await,
        "comment",
    );
    let t = w.task("PAY-1").await;
    assert_eq!(t["comments"][0]["station"], json!(a.reference), "{t}");
    assert_ne!(b.reference, a.reference);
}

#[tokio::test]
async fn an_owner_message_about_a_task_names_only_the_new_run() {
    let w = world(&[("claude-web", "claude-routine")]).await;
    let tok = &w.tokens["claude-web"];
    w.new_task("Refund screen").await;
    w.fake.wait("/routine/fire", 1).await;
    let old = w.settled_runs("claude-web", 1).await.remove(0);
    let claim = ok(
        &w.app
            .mcp(
                tok,
                "claim_task",
                json!({"id": "PAY-1", "station": old.reference}),
            )
            .await,
        "claim",
    );
    let epoch = claim["lease"]["epoch"].as_i64().unwrap();
    ok(
        &w.app
            .mcp(
                tok,
                "complete",
                json!({"id": "PAY-1", "epoch": epoch, "result": "done", "learned": "nothing new"}),
            )
            .await,
        "complete",
    );
    // the owner asks about that task: a new run, and only it is named
    ok(
        &w.app
            .mcp(
                &w.root,
                "pass",
                json!({"project": "PAY", "message": "what happened here?", "to": "claude-web", "task": "PAY-1"}),
            )
            .await,
        "dm about the task",
    );
    let fires = w.fake.wait("/routine/fire", 2).await;
    let runs = w.settled_runs("claude-web", 2).await;
    let new = runs.iter().find(|r| r.id != old.id).unwrap();
    let text = text_of(&fires[1]);
    assert!(text.contains(&new.reference), "{text}");
    assert!(!text.contains(&old.reference), "{text}");
    // the station comes first, so a cut text keeps it
    let body = fires[1].body["text"].as_str().unwrap();
    assert!(
        body.starts_with(&format!("Your station is {}", new.reference)),
        "{body}"
    );
}
