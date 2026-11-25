//! The console's paging: `*Page` procedures (keyset cursors), `/v1/users` and `/v1/agents` with
//! `?q` and `limit`, and the File model's `@@paged` list. Every walk visits each row exactly once,
//! even when rows are added in the middle; a cursor minted for another sort is a 400; another team's
//! pot answers as it does for the unpaged list.
mod common;
use corntchap::admin;
use serde_json::{Value, json};
use std::collections::HashSet;

struct W {
    app: common::TestApp,
    root: String,
    alice: String,
    bob: String,
}

async fn world() -> W {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob"] {
        app.add_user(u).await;
    }
    for (team, who, key) in [("acme", "alice", "PAY"), ("other", "bob", "OTH")] {
        app.post(&root, "/teams", json!({"name": team})).await;
        app.post(
            &root,
            &format!("/teams/{team}/members"),
            json!({"principal": who}),
        )
        .await;
        app.post(
            &root,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    W {
        app,
        root,
        alice,
        bob,
    }
}

impl W {
    async fn proc(&self, token: &str, name: &str, args: Value) -> (u16, Value) {
        let r = self
            .app
            .http
            .post(format!("{}/api/v2/$procs/{name}", self.app.base))
            .bearer_auth(token)
            .json(&args)
            .send()
            .await
            .unwrap();
        let st = r.status().as_u16();
        (st, r.json().await.unwrap_or(Value::Null))
    }

    async fn ok(&self, token: &str, name: &str, args: Value) -> Value {
        let (st, v) = self.proc(token, name, args.clone()).await;
        assert_eq!(st, 200, "{name}({args}): {v}");
        v
    }

    /// Walk a `*Page` procedure with `limit` per page; `mid` runs after the first page.
    async fn walk<F, Fut>(
        &self,
        token: &str,
        name: &str,
        mut args: Value,
        limit: i64,
        id_of: fn(&Value) -> String,
        mid: F,
    ) -> Vec<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut mid = Some(mid);
        let mut seen = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            args["page"] = json!({"limit": limit, "cursor": cursor});
            let v = self.ok(token, name, args.clone()).await;
            let items = v["items"].as_array().unwrap();
            assert!(items.len() as i64 <= limit, "{v}");
            seen.extend(items.iter().map(id_of));
            if let Some(m) = mid.take() {
                m().await;
            }
            cursor = v["nextCursor"]
                .as_str()
                .or(v["nextCursor"].as_str())
                .map(String::from);
            if cursor.is_none() {
                return seen;
            }
        }
        panic!("the walk of {name} does not end");
    }

    async fn task(&self, title: &str, priority: i64, labels: &str) {
        let (st, v) = self
            .app
            .post(
                &self.alice,
                "/projects/PAY/tasks",
                json!({"title": title, "priority": priority, "labels": labels.split(',').collect::<Vec<_>>()}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

fn unique(ids: &[String]) -> usize {
    ids.iter().collect::<HashSet<_>>().len()
}

#[tokio::test]
async fn task_page_walks_every_bucket_once() {
    let w = world().await;
    for i in 0..14 {
        w.task(
            &format!("task {i}"),
            i % 4,
            if i % 2 == 0 { "even" } else { "odd" },
        )
        .await;
    }
    // PAY-1..PAY-4 served (updated a minute apart, PAY-4 newest), PAY-5..PAY-6 tossed, one blocked
    for (n, status, ago) in [
        (1, "done", 4),
        (2, "done", 3),
        (3, "done", 2),
        (4, "done", 1),
        (5, "cancelled", 2),
        (6, "cancelled", 1),
        (7, "blocked", 0),
    ] {
        sqlx::query("update tasks set status = $2, updated_at = now() - make_interval(mins => $3) where number = $1 and project_id = (select id from projects where key = 'PAY')")
            .bind(n)
            .bind(status)
            .bind(ago)
            .execute(&w.app.state.db)
            .await
            .unwrap();
    }
    let page = |bucket: &str, extra: Value, p: Value| {
        let mut a = json!({"project": "PAY", "bucket": bucket, "page": p});
        for (k, v) in extra.as_object().unwrap() {
            a[k] = v.clone();
        }
        a
    };

    // active: priority desc, number; the walk survives a task created after the first page
    let w2 = &w;
    let seen = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "active"}),
            4,
            id,
            || async move { w2.task("late arrival", 0, "").await },
        )
        .await;
    assert_eq!(unique(&seen), seen.len(), "a task twice: {seen:?}");
    // 14 - 4 done - 2 tossed = 8 originals, plus the late one (priority 0, the last number)
    assert_eq!(seen.len(), 9, "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("PAY-15"));
    let first = w
        .ok(
            &w.alice,
            "taskPage",
            page("active", json!({}), json!({"limit": 3})),
        )
        .await;
    let prios: Vec<i64> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["priority"].as_i64().unwrap())
        .collect();
    assert_eq!(prios, vec![3, 3, 2], "highest priority first: {first}");
    assert_eq!(first["total"], 9);
    assert!(first["nextCursor"].is_string());

    // done: newest change first, exactly once, total counts all
    let done = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "done"}),
            3,
            id,
            || async {},
        )
        .await;
    assert_eq!(done, vec!["PAY-4", "PAY-3", "PAY-2", "PAY-1"]);
    let v = w
        .ok(
            &w.alice,
            "taskPage",
            page("done", json!({}), json!({"limit": 3})),
        )
        .await;
    assert_eq!(v["total"], 4);
    let tossed = w
        .ok(
            &w.alice,
            "taskPage",
            page("cancelled", json!({}), json!({})),
        )
        .await;
    assert_eq!(
        tossed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(id)
            .collect::<Vec<_>>(),
        vec!["PAY-6", "PAY-5"]
    );
    assert_eq!(tossed["total"], 2);
    assert!(tossed["nextCursor"].is_null());

    // filters count too
    let v = w
        .ok(
            &w.alice,
            "taskPage",
            page("active", json!({"label": "odd"}), json!({"limit": 2})),
        )
        .await;
    assert_eq!(v["total"], 4, "odd-numbered tasks still active: {v}");
    let v = w
        .ok(
            &w.alice,
            "taskPage",
            page("active", json!({"q": "LATE"}), json!({})),
        )
        .await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["title"], "late arrival");

    // limits are clamped: 0 and below ask for one, a huge one is the cap
    for limit in [0, -5] {
        let v = w
            .ok(
                &w.alice,
                "taskPage",
                page("active", json!({}), json!({"limit": limit})),
            )
            .await;
        assert_eq!(
            v["items"].as_array().unwrap().len(),
            1,
            "limit {limit}: {v}"
        );
    }
    let v = w
        .ok(
            &w.alice,
            "taskPage",
            page("active", json!({}), json!({"limit": 100000})),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 9);
    assert!(v["nextCursor"].is_null());
    let v = w
        .ok(
            &w.alice,
            "taskPage",
            page("done", json!({}), json!({"limit": 100000})),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 4);

    // a cursor minted for another bucket (another sort) or by nobody is a 400
    let active = w
        .ok(
            &w.alice,
            "taskPage",
            page("active", json!({}), json!({"limit": 2})),
        )
        .await;
    let cursor = active["nextCursor"].as_str().unwrap();
    for (bucket, c) in [("done", cursor), ("active", "not-a-cursor")] {
        let (st, v) = w
            .proc(
                &w.alice,
                "taskPage",
                page(bucket, json!({}), json!({"cursor": c})),
            )
            .await;
        assert_eq!(st, 400, "{bucket}: {v}");
    }
    let (st, _) = w
        .proc(&w.alice, "taskPage", page("nonsense", json!({}), json!({})))
        .await;
    assert_eq!(st, 400);

    // a forged cursor with the right sort but values that do not parse (number, timestamp) is a 400, not a 500
    for (bucket, order, keys) in [
        ("active", "-priority,number:active", vec!["high", "3"]),
        ("active", "-priority,number:active", vec!["2", "3.5"]),
        ("done", "-updated_at,-number:done", vec!["yesterday", "3"]),
        (
            "done",
            "-updated_at,-number:done",
            vec!["2026-01-01 00:00:00+00", "x"],
        ),
    ] {
        let keys: Vec<Option<String>> = keys.into_iter().map(|k| Some(k.to_string())).collect();
        let forged = cratestack::encode_cursor(order, &keys);
        let (st, v) = w
            .proc(
                &w.alice,
                "taskPage",
                page(bucket, json!({}), json!({"cursor": forged})),
            )
            .await;
        assert_eq!(st, 400, "{bucket} {keys:?}: {v}");
    }

    // another team's pot answers as the unpaged list does
    let (listed, _) = w
        .proc(&w.bob, "projectTasks", json!({"project": "PAY"}))
        .await;
    let (paged, _) = w
        .proc(&w.bob, "taskPage", page("active", json!({}), json!({})))
        .await;
    assert_eq!(listed, 404);
    assert_eq!(paged, listed);
}

/// Priorities are 64-bit: a cursor after a task with a priority past 2^31 still reads the next page.
#[tokio::test]
async fn task_page_cursor_takes_big_priorities() {
    let w = world().await;
    w.task("huge", 3_000_000_000, "").await;
    w.task("tiny", -3_000_000_000, "").await;
    w.task("plain", 0, "").await;
    let seen = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "active"}),
            1,
            id,
            || async {},
        )
        .await;
    assert_eq!(seen, ["PAY-1", "PAY-3", "PAY-2"]);
}

#[tokio::test]
async fn question_review_and_memory_pages() {
    let w = world().await;
    let db = &w.app.state.db;
    let pid: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(db)
        .await
        .unwrap();
    // 9 questions a second apart (Q1 oldest); Q8 and Q9 answered
    for n in 1..=9i64 {
        sqlx::query("insert into questions (project_id, number, body, title, status, created_at) values ($1, $2, 'b', 'q', $3, now() - make_interval(secs => 100 - $2::int))")
            .bind(pid)
            .bind(n)
            .bind(if n >= 8 { "answered" } else { "open" })
            .execute(db)
            .await
            .unwrap();
    }
    let add_question = || async {
        sqlx::query(
            "insert into questions (project_id, number, body, title) values ($1, 20, 'b', 'late')",
        )
        .bind(pid)
        .execute(db)
        .await
        .unwrap();
    };
    let open = w
        .walk(
            &w.alice,
            "questionPage",
            json!({"project": "PAY"}),
            3,
            id,
            add_question,
        )
        .await;
    // open ones oldest first, once each; the late one comes last
    assert_eq!(
        open,
        [
            "PAY-Q1", "PAY-Q2", "PAY-Q3", "PAY-Q4", "PAY-Q5", "PAY-Q6", "PAY-Q7", "PAY-Q20"
        ]
    );
    let answered = w
        .walk(
            &w.alice,
            "questionPage",
            json!({"project": "PAY", "status": "answered"}),
            1,
            id,
            || async {},
        )
        .await;
    assert_eq!(answered, ["PAY-Q9", "PAY-Q8"], "the others newest first");
    let v = w
        .ok(
            &w.alice,
            "questionPage",
            json!({"project": "PAY", "status": "all", "page": {"limit": 0}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let v = w
        .ok(
            &w.alice,
            "questionPage",
            json!({"project": "PAY", "page": {"limit": 3}}),
        )
        .await;
    let (st, _) = w
        .proc(
            &w.alice,
            "questionPage",
            json!({"project": "PAY", "status": "answered", "page": {"cursor": v["nextCursor"]}}),
        )
        .await;
    assert_eq!(st, 400, "an open cursor does not fit the answered list");
    let (st, _) = w
        .proc(
            &w.bob,
            "questionPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    assert_eq!(st, 404, "another team's pot");

    // reviews: waiting ones first, then newest first
    let alice: uuid::Uuid = sqlx::query_scalar("select id from principals where name = 'alice'")
        .fetch_one(db)
        .await
        .unwrap();
    for i in 0..7 {
        w.task(&format!("rev {i}"), 0, "").await;
    }
    let tasks: Vec<uuid::Uuid> =
        sqlx::query_scalar("select id from tasks where project_id = $1 order by number")
            .bind(pid)
            .fetch_all(db)
            .await
            .unwrap();
    for (i, t) in tasks.iter().enumerate() {
        let state = if i % 3 == 0 { "requested" } else { "approved" };
        sqlx::query("insert into reviews (project_id, number, task_id, requested_by, state, created_at) values ($1, $2, $3, $4, $5, now() - make_interval(secs => 50 - $2::int))")
            .bind(pid)
            .bind(i as i64 + 1)
            .bind(t)
            .bind(alice)
            .bind(state)
            .execute(db)
            .await
            .unwrap();
    }
    let reviews = w
        .walk(
            &w.alice,
            "reviewPage",
            json!({"project": "PAY"}),
            3,
            id,
            || async {},
        )
        .await;
    // requested ones (R1, R4, R7) newest first, then the approved ones newest first
    assert_eq!(
        reviews,
        [
            "PAY-R7", "PAY-R4", "PAY-R1", "PAY-R6", "PAY-R5", "PAY-R3", "PAY-R2"
        ]
    );
    let v = w
        .ok(
            &w.alice,
            "reviewPage",
            json!({"project": "PAY", "state": "requested", "page": {"limit": 2}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    assert!(v["nextCursor"].is_string());

    // memories: most recently changed first, once each, a new one mid-walk does not repeat
    for i in 0..9 {
        let (st, v) = w
            .app
            .post(&w.alice, "/projects/PAY/pantry", json!({"title": format!("fact {i}"), "body": "b", "kind": if i % 2 == 0 { "fact" } else { "note" }}))
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let w2 = &w;
    let mems = w
        .walk(
            &w.alice,
            "memoryPage",
            json!({"project": "PAY"}),
            4,
            |m| m["key"].as_str().unwrap().into(),
            || async move {
                let (st, v) = w2
                    .app
                    .post(
                        &w2.alice,
                        "/projects/PAY/pantry",
                        json!({"title": "fresh", "body": "b"}),
                    )
                    .await;
                assert_eq!(st, 200, "{v}");
            },
        )
        .await;
    assert_eq!(unique(&mems), mems.len(), "{mems:?}");
    assert_eq!(
        mems.len(),
        9,
        "the memory saved mid-walk sorts before the cursor: {mems:?}"
    );
    assert_eq!(mems[0], "fact-8");
    let facts = w
        .ok(
            &w.alice,
            "memoryPage",
            json!({"project": "PAY", "kind": "fact", "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(facts["items"].as_array().unwrap().len(), 5);
    let (st, _) = w
        .proc(&w.bob, "memoryPage", json!({"project": "PAY", "page": {}}))
        .await;
    let (st2, _) = w
        .proc(&w.bob, "memoryRecall", json!({"project": "PAY"}))
        .await;
    assert_eq!((st, st2), (404, 404));
}

#[tokio::test]
async fn cloud_run_and_view_pages() {
    let w = world().await;
    let db = &w.app.state.db;
    let (st, ag) = w
        .app
        .post(
            &w.root,
            "/agents",
            json!({"name": "cloudy", "kind": "other", "runtime": "local"}),
        )
        .await;
    assert_eq!(st, 200);
    let _ = ag;
    w.app
        .post(
            &w.root,
            "/teams/acme/members",
            json!({"principal": "cloudy"}),
        )
        .await;
    let (agent, pid): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "select (select id from principals where name = 'cloudy'), (select id from projects where key = 'PAY')",
    )
    .fetch_one(db)
    .await
    .unwrap();
    for n in 1..=8 {
        sqlx::query("insert into stations (ref, agent_id, project_id, runtime, session_ref, kind, created_at, ended_at) values ($1, $2, $3, 'other', $1, 'cloud-run', now() - make_interval(secs => 100 - $4::int), case when $4 <= 5 then now() end)")
            .bind(format!("st_run{n}"))
            .bind(agent)
            .bind(pid)
            .bind(n)
            .execute(db)
            .await
            .unwrap();
    }
    let add = || async {
        sqlx::query("insert into stations (ref, agent_id, project_id, runtime, session_ref, kind) values ('st_late', $1, $2, 'other', 'late', 'cloud-run')")
            .bind(agent)
            .bind(pid)
            .execute(db)
            .await
            .unwrap();
    };
    let runs = w
        .walk(
            &w.root,
            "cloudRunPage",
            json!({"project": "PAY"}),
            3,
            id,
            add,
        )
        .await;
    // newest first; the run that started mid-walk is newer than the cursor, so it is not repeated
    assert_eq!(unique(&runs), runs.len(), "{runs:?}");
    assert_eq!(
        runs,
        [
            "st_run8", "st_run7", "st_run6", "st_run5", "st_run4", "st_run3", "st_run2", "st_run1"
        ]
    );
    let live = w
        .ok(
            &w.root,
            "cloudRunPage",
            json!({"live": true, "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(live["items"].as_array().unwrap().len(), 4, "{live}");
    let ended = w
        .ok(
            &w.root,
            "cloudRunPage",
            json!({"cook": "cloudy", "live": false, "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(ended["items"].as_array().unwrap().len(), 5);
    let none = w
        .ok(
            &w.bob,
            "cloudRunPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    assert!(none["items"].as_array().unwrap().is_empty(), "{none}");

    // views: a condition tried out, the page cursor is bound to that condition and order
    for i in 0..9 {
        w.task(&format!("v {i}"), i, "").await;
    }
    let args = json!({"project": "PAY", "where": "priority >= 2", "orderBy": "priority desc"});
    let seen = w
        .walk(&w.alice, "viewPage", args.clone(), 3, id, || async {})
        .await;
    assert_eq!(seen.len(), 7);
    assert_eq!(unique(&seen), 7);
    assert_eq!(seen[0], "PAY-9", "highest priority first: {seen:?}");
    let first = w
        .ok(&w.alice, "viewPage", json!({"project": "PAY", "where": "priority >= 2", "orderBy": "priority desc", "page": {"limit": 2}}))
        .await;
    let (st, _) = w
        .proc(&w.alice, "viewPage", json!({"project": "PAY", "where": "priority >= 3", "orderBy": "priority desc", "page": {"cursor": first["nextCursor"]}}))
        .await;
    assert_eq!(st, 400, "another condition, another sort");
    let (st, _) = w
        .proc(
            &w.alice,
            "viewPage",
            json!({"project": "PAY", "where": "drop table x", "page": {}}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w
        .proc(&w.bob, "viewPage", json!({"project": "PAY", "page": {}}))
        .await;
    assert_eq!(st, 404, "another team's pot");
}

#[tokio::test]
async fn directory_lists_page_and_filter_by_q() {
    let w = world().await;
    for u in ["carol", "carl", "dave", "erin"] {
        w.app.add_user(u).await;
    }
    sqlx::query("update principals set display_name = 'Caroline Smith' where name = 'dave'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    // without limit: the bare array, as before
    let (st, all) = w.app.get(&w.root, "/users").await;
    assert_eq!(st, 200);
    assert!(all.is_array(), "{all}");
    let handles = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|u| u["name"].as_str().unwrap().to_string())
            .collect()
    };
    let everyone = handles(&all);
    assert!(everyone.len() >= 6);
    // with limit: {items, nextCursor}, by handle, each once, a person added mid-walk is not repeated
    let mut seen = vec![];
    let mut cursor = String::new();
    for round in 0..20 {
        let (st, v) = w
            .app
            .get(&w.root, &format!("/users?limit=2&cursor={cursor}"))
            .await;
        assert_eq!(st, 200, "{v}");
        seen.extend(handles(&v["items"]));
        if round == 0 {
            w.app.add_user("zed").await;
        }
        match v["nextCursor"].as_str() {
            Some(c) => cursor = c.to_string(),
            None => break,
        }
    }
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert!(seen.windows(2).all(|p| p[0] < p[1]), "by handle: {seen:?}");
    for h in &everyone {
        assert!(seen.contains(h), "{h} missing from {seen:?}");
    }
    // q: handle or display name, any case
    let (_, v) = w.app.get(&w.root, "/users?q=CAR&limit=10").await;
    assert_eq!(handles(&v["items"]), ["carl", "carol", "dave"], "{v}");
    let (_, v) = w.app.get(&w.root, "/users?q=erin").await;
    assert_eq!(handles(&v), ["erin"], "no limit: the bare array");
    let (st, _) = w.app.get(&w.root, "/users?limit=2&cursor=bogus").await;
    assert_eq!(st, 400);
    let (_, v) = w.app.get(&w.root, "/users?limit=100000").await;
    assert!(v["items"].as_array().unwrap().len() <= 200);

    // orchestrators get the directory (no sign-in details); members get nothing; disabled people stay hidden
    w.app.set_role("alice", "orchestrator").await;
    sqlx::query("update principals set disabled_at = now() where name = 'carl'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let (st, v) = w.app.get(&w.alice, "/users?q=car&limit=10").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(handles(&v["items"]), ["carol", "dave"], "{v}");
    assert!(v["items"][0].get("linked").is_none(), "{v}");
    let (_, v) = w.app.get(&w.root, "/users?q=carl&limit=10").await;
    assert_eq!(
        handles(&v["items"]),
        ["carl"],
        "super-admins see disabled people too"
    );
    assert!(v["items"][0].get("linked").is_some());
    let (st, _) = w.app.get(&w.bob, "/users?q=car&limit=10").await;
    assert_eq!(st, 403);

    // agents: the same
    for n in ["bot-a", "bot-b", "bot-c", "helper"] {
        let (st, v) = w
            .app
            .post(
                &w.root,
                "/agents",
                json!({"name": n, "kind": "other", "runtime": "local"}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let (_, v) = w.app.get(&w.root, "/agents?q=bot&limit=2").await;
    assert_eq!(handles(&v["items"]), ["bot-a", "bot-b"]);
    let (_, v) = w
        .app
        .get(
            &w.root,
            &format!(
                "/agents?q=bot&limit=2&cursor={}",
                v["nextCursor"].as_str().unwrap()
            ),
        )
        .await;
    assert_eq!(handles(&v["items"]), ["bot-c"]);
    assert!(v["nextCursor"].is_null());
    let (_, v) = w.app.get(&w.root, "/agents").await;
    assert_eq!(handles(&v).len(), 4, "bare array without limit");
    // an owner sees only their own cooks
    let (_, v) = w.app.get(&w.alice, "/agents?limit=10").await;
    assert!(v["items"].as_array().unwrap().is_empty(), "{v}");
}

#[tokio::test]
async fn the_file_model_is_paged() {
    let w = world().await;
    let db = &w.app.state.db;
    let pid: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(db)
        .await
        .unwrap();
    for n in 1..=7i64 {
        sqlx::query("insert into files (id, project_id, number, name, content_type, storage_key, status, upload_expires_at, created_at) values (gen_random_uuid(), $1, $2, 'f.png', 'image/png', 'k/' || $2, 'ready', now(), now() - make_interval(secs => 100 - $2::int))")
            .bind(pid)
            .bind(n)
            .execute(db)
            .await
            .unwrap();
    }
    let get = |token: String, q: String| {
        let (http, base) = (w.app.http.clone(), w.app.base.clone());
        async move {
            let r = http
                .get(format!("{base}/api/v2/files?{q}"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            (
                r.status().as_u16(),
                r.json::<Value>().await.unwrap_or(Value::Null),
            )
        }
    };
    let (st, v) = get(w.alice.clone(), "limit=3&sort=-createdAt&cursor=".into()).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    assert_eq!(v["totalCount"], 7, "{v}");
    assert!(
        v["pageInfo"]["nextCursor"].is_string() || v["pageInfo"]["hasNextPage"] == true,
        "{v}"
    );
    let mut seen = vec![];
    let mut cursor = None::<String>;
    loop {
        let q = match &cursor {
            Some(c) => format!("limit=3&sort=-createdAt&cursor={c}"),
            None => "limit=3&sort=-createdAt&cursor=".into(),
        };
        let (st, v) = get(w.alice.clone(), q).await;
        assert_eq!(st, 200, "{v}");
        seen.extend(v["items"].as_array().unwrap().iter().map(|f| {
            f["number"]
                .as_i64()
                .or_else(|| f["number"].as_str().and_then(|n| n.parse().ok()))
                .unwrap()
        }));
        match v["pageInfo"]["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(seen, [7, 6, 5, 4, 3, 2, 1]);
    // members only
    let (_, v) = get(w.bob.clone(), "limit=3".into()).await;
    assert!(v["items"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(v["totalCount"], 0);
}
