//! The console board pages each column on its own (`taskPage` with bucket ready, claimed, waiting, done,
//! cancelled): keyset walks that stay whole when tasks arrive mid-walk, totals under the holder, status and
//! label filters, big priorities, and the pot-access rule unchanged.
mod common;
use corntchap::admin;
use serde_json::{Value, json};
use std::collections::HashSet;

#[allow(dead_code)]
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

/// 40 tasks: 1-6 served, 7-8 tossed, 9-11 blocked, 12-14 on the fire (alice, live lease), 15-16 claimed with a
/// lapsed lease (ready again), 17 waiting on 18; the rest ready. Priorities cycle 0..4, labels even or odd.
async fn board() -> W {
    let w = world().await;
    for i in 1..=40 {
        w.task(
            &format!("task {i}"),
            i % 5,
            if i % 2 == 0 { "even" } else { "odd" },
        )
        .await;
    }
    let db = &w.app.state.db;
    for n in 1..=16 {
        let (status, ago): (&str, i32) = match n {
            1..=6 => ("done", n),
            7..=8 => ("cancelled", n),
            9..=11 => ("blocked", 0),
            _ => ("claimed", 0),
        };
        let lease = if n >= 15 { -60 } else { 60 };
        sqlx::query(
            "update tasks set status = $2, updated_at = now() - make_interval(mins => $3),
               claimed_by = case when $2 = 'claimed' then (select id from principals where name = 'alice') end,
               lease_expires_at = case when $2 = 'claimed' then now() + make_interval(mins => $4) end
             where number = $1 and project_id = (select id from projects where key = 'PAY')",
        )
        .bind(n)
        .bind(status)
        .bind(ago)
        .bind(lease)
        .execute(db)
        .await
        .unwrap();
    }
    sqlx::query(
        "insert into task_deps (task_id, depends_on)
         select a.id, b.id from tasks a, tasks b where a.number = 17 and b.number = 18
           and a.project_id = b.project_id",
    )
    .execute(db)
    .await
    .unwrap();
    w
}

fn page(bucket: &str, extra: Value, p: Value) -> Value {
    let mut a = json!({"project": "PAY", "bucket": bucket, "page": p});
    for (k, v) in extra.as_object().unwrap() {
        a[k] = v.clone();
    }
    a
}

impl W {
    async fn total(&self, bucket: &str, extra: Value) -> i64 {
        let v = self
            .ok(
                &self.alice,
                "taskPage",
                page(bucket, extra, json!({"limit": 1})),
            )
            .await;
        v["total"].as_i64().unwrap()
    }
}

#[tokio::test]
async fn every_column_keyset_walks_once_with_inserts_mid_walk() {
    let w = board().await;
    // 40 - 6 served - 2 tossed = 32 in play: 3 blocked + 1 on a dependency, 3 on the fire, 25 ready
    assert_eq!(w.total("ready", json!({})).await, 25);
    assert_eq!(w.total("claimed", json!({})).await, 3);
    assert_eq!(w.total("waiting", json!({})).await, 4);
    assert_eq!(w.total("active", json!({})).await, 32);
    assert_eq!(w.total("done", json!({})).await, 6);
    assert_eq!(w.total("cancelled", json!({})).await, 2);

    let w2 = &w;
    let ready = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "ready"}),
            4,
            id,
            // one task that sorts before the cursor, one after it
            || async move {
                w2.task("front", 9, "").await;
                w2.task("back", -9, "").await;
            },
        )
        .await;
    assert_eq!(unique(&ready), ready.len(), "a task twice: {ready:?}");
    assert_eq!(
        ready.len(),
        26,
        "the 25 and the one that sorts last: {ready:?}"
    );
    assert_eq!(ready.last().map(String::as_str), Some("PAY-42"));
    assert!(!ready.contains(&"PAY-41".to_string()), "behind the cursor");
    assert!(
        ready.contains(&"PAY-15".to_string()),
        "a lapsed lease is ready"
    );
    assert!(!ready.contains(&"PAY-17".to_string()), "waits on PAY-18");

    for (bucket, want) in [
        ("claimed", vec!["PAY-14", "PAY-13", "PAY-12"]),
        ("waiting", vec!["PAY-9", "PAY-10", "PAY-11", "PAY-17"]),
        (
            "done",
            vec!["PAY-1", "PAY-2", "PAY-3", "PAY-4", "PAY-5", "PAY-6"],
        ),
        ("cancelled", vec!["PAY-7", "PAY-8"]),
    ] {
        let seen = w
            .walk(
                &w.alice,
                "taskPage",
                json!({"project": "PAY", "bucket": bucket}),
                1,
                id,
                || async {},
            )
            .await;
        let mut sorted = seen.clone();
        sorted.sort();
        let mut want_sorted: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        want_sorted.sort();
        assert_eq!(sorted, want_sorted, "{bucket}: {seen:?}");
    }
    // served: newest change first (a trigger stamps updated_at, so PAY-6, changed last, is the newest)
    let done = w
        .ok(&w.alice, "taskPage", page("done", json!({}), json!({})))
        .await;
    assert_eq!(id(&done["items"][0]), "PAY-6");
    // on the fire: priority desc (PAY-14 is 4, PAY-13 is 3, PAY-12 is 2)
    let fire = w
        .ok(&w.alice, "taskPage", page("claimed", json!({}), json!({})))
        .await;
    assert_eq!(id(&fire["items"][0]), "PAY-14");
    // the default page is 25, the cap 200
    let v = w
        .ok(&w.alice, "taskPage", page("ready", json!({}), json!({})))
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 25);
    assert!(v["nextCursor"].is_string());
    // a cursor of one column does not fit another
    let (st, _) = w
        .proc(
            &w.alice,
            "taskPage",
            page(
                "claimed",
                json!({}),
                json!({"cursor": v["nextCursor"], "limit": 2}),
            ),
        )
        .await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn totals_follow_the_holder_status_and_label_filters() {
    let w = board().await;
    // alice holds the three on the fire and the two lapsed ones
    assert_eq!(w.total("claimed", json!({"holder": "alice"})).await, 3);
    assert_eq!(w.total("ready", json!({"holder": "alice"})).await, 2);
    assert_eq!(w.total("waiting", json!({"holder": "alice"})).await, 0);
    assert_eq!(w.total("active", json!({"holder": "alice"})).await, 5);
    assert_eq!(w.total("active", json!({"holder": "nobody"})).await, 0);
    assert_eq!(w.total("done", json!({"holder": "alice"})).await, 0);
    // status is the raw task status and narrows whatever list is asked for (a lapsed lease is still `claimed`)
    assert_eq!(w.total("active", json!({"status": "claimed"})).await, 5);
    assert_eq!(w.total("active", json!({"status": "blocked"})).await, 3);
    assert_eq!(w.total("ready", json!({"status": "open"})).await, 23);
    assert_eq!(w.total("ready", json!({"status": "blocked"})).await, 0);
    assert_eq!(w.total("done", json!({"status": "claimed"})).await, 0);
    assert_eq!(
        w.total("active", json!({"status": " Claimed ", "holder": "alice"}))
            .await,
        5
    );
    // a status with no bucket looks at every bucket: `done` finds the done tasks
    for (status, n) in [("done", 6), ("cancelled", 2), ("blocked", 3), ("open", 24)] {
        let (st, v) = w
            .proc(
                &w.alice,
                "taskPage",
                json!({"project": "PAY", "status": status, "page": {"limit": 100}}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["total"], n, "{status}");
        assert_eq!(v["items"].as_array().unwrap().len(), n as usize, "{status}");
        assert!(
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["status"] == status)
        );
    }
    // labels: odd numbers 19..39 are ready (11 of them); 12..16 on the fire or lapsed: 13, 15 odd
    assert_eq!(w.total("ready", json!({"label": "odd"})).await, 12);
    assert_eq!(w.total("ready", json!({"label": "even"})).await, 13);
    assert_eq!(w.total("claimed", json!({"label": "odd"})).await, 1);
    assert_eq!(
        w.total("ready", json!({"label": "odd", "holder": "alice"}))
            .await,
        1
    );
    // the page walk of a filtered column counts the same rows
    let seen = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "ready", "label": "even"}),
            5,
            id,
            || async {},
        )
        .await;
    assert_eq!(seen.len(), 13);
    // a status that is no column is a 400
    let (st, _) = w
        .proc(
            &w.alice,
            "taskPage",
            page("active", json!({"status": "nonsense"}), json!({})),
        )
        .await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn another_teams_pot_answers_as_the_unpaged_list_in_every_column() {
    let w = board().await;
    let (listed, _) = w
        .proc(&w.bob, "projectTasks", json!({"project": "PAY"}))
        .await;
    assert_eq!(listed, 404);
    for bucket in ["ready", "claimed", "waiting", "done", "cancelled", "active"] {
        let (st, _) = w
            .proc(&w.bob, "taskPage", page(bucket, json!({}), json!({})))
            .await;
        assert_eq!(st, listed, "{bucket}");
    }
    let (st, _) = w
        .proc(
            "not-a-token",
            "taskPage",
            page("ready", json!({}), json!({})),
        )
        .await;
    assert_ne!(st, 200);
}

#[tokio::test]
async fn columns_take_big_priorities() {
    let w = world().await;
    w.task("huge", 3_000_000_000, "").await;
    w.task("tiny", -3_000_000_000, "").await;
    w.task("plain", 0, "").await;
    sqlx::query("update tasks set status = 'blocked' where number = 3")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    for (bucket, want) in [
        ("ready", vec!["PAY-1", "PAY-2"]),
        ("waiting", vec!["PAY-3"]),
    ] {
        let seen = w
            .walk(
                &w.alice,
                "taskPage",
                json!({"project": "PAY", "bucket": bucket}),
                1,
                id,
                || async {},
            )
            .await;
        assert_eq!(seen, want, "{bucket}");
    }
}

/// `bucket: all` walks by number, which never changes: heartbeats and edits mid-walk skip and repeat nothing.
#[tokio::test]
async fn the_all_bucket_walks_by_number_whatever_changes_mid_walk() {
    let w = board().await;
    let db = &w.app.state.db;
    let seen = w
        .walk(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "all"}),
            7,
            id,
            || async {
                sqlx::query("update tasks set updated_at = now() + make_interval(secs => number)")
                    .execute(db)
                    .await
                    .unwrap();
            },
        )
        .await;
    assert_eq!(seen.len(), 40, "{seen:?}");
    assert_eq!(unique(&seen), 40);
    assert_eq!(seen[0], "PAY-40");
}
