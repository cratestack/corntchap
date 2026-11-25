//! The console pages the people, station, activity, file and usage lists on their own (`teamPage`,
//! `teamMemberPage`, `userPage`, `cookPage`, `firePage`, `wakeTargetPage`, `forgeUnfollowedPage`,
//! `stationPage`, `activityPage`, `eventPage`, `filePage`, `usageCookPage`): keyset walks that stay whole
//! when rows arrive mid-walk, the filters, the visibility rules unchanged, and a cursor minted for another
//! sort a 400. The MCP list tools answer the same pages on both protocol paths.
mod common;
use corntchap::admin;
use serde_json::{Value, json};
use std::collections::HashSet;

struct W {
    app: common::TestApp,
    root: String,
    alice: String,
    bob: String,
    carol: String,
}

/// Teams acme (alice owns, carol is a chef) with pot PAY, and other (bob is a chef) with pot OPS.
async fn world() -> W {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob", "carol"] {
        app.add_user(u).await;
    }
    app.set_role("alice", "orchestrator").await;
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&root, "/teams", json!({"name": team})).await;
        app.post(
            &root,
            "/projects",
            json!({"team": team, "key": key, "name": key}),
        )
        .await;
    }
    for (team, who, role) in [
        ("acme", "alice", "owner"),
        ("acme", "carol", "member"),
        ("other", "bob", "member"),
    ] {
        let (st, v) = app
            .post(
                &root,
                &format!("/teams/{team}/members"),
                json!({"principal": who, "role": role}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let tok = |who: &'static str| {
        let db = app.state.db.clone();
        async move { admin::operator_token(&db, who, None).await.unwrap() }
    };
    let (alice, bob, carol) = (tok("alice").await, tok("bob").await, tok("carol").await);
    W {
        app,
        root,
        alice,
        bob,
        carol,
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
            cursor = v["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                return seen;
            }
        }
        panic!("the walk of {name} does not end");
    }

    async fn walk_plain(
        &self,
        token: &str,
        name: &str,
        args: Value,
        limit: i64,
        id_of: fn(&Value) -> String,
    ) -> Vec<String> {
        self.walk(token, name, args, limit, id_of, || async {})
            .await
    }

    /// A cook owned by `owner`, in team acme.
    async fn cook(&self, name: &str, owner: &str) {
        let (st, v) = self
            .app
            .post(
                &self.root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "local", "owner": owner}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        let (st, v) = self
            .app
            .post(
                &self.root,
                "/teams/acme/members",
                json!({"principal": name}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }

    async fn pid(&self, name: &str) -> uuid::Uuid {
        sqlx::query_scalar("select id from principals where name = $1")
            .bind(name)
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }

    async fn pot(&self, key: &str) -> uuid::Uuid {
        sqlx::query_scalar("select id from projects where key = $1")
            .bind(key)
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }
}

fn name(v: &Value) -> String {
    v["name"].as_str().unwrap().to_string()
}
fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}
fn seq(v: &Value) -> String {
    v["seq"].as_i64().unwrap().to_string()
}
fn cook(v: &Value) -> String {
    v["cook"].as_str().unwrap().to_string()
}
fn names(items: &Value) -> Vec<String> {
    items.as_array().unwrap().iter().map(name).collect()
}

fn unique(ids: &[String]) -> usize {
    ids.iter().collect::<HashSet<_>>().len()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test]
async fn teams_and_their_chefs() {
    let w = world().await;
    for n in 1..=5 {
        w.app
            .post(&w.root, "/teams", json!({"name": format!("t{n}")}))
            .await;
    }
    let all = ["acme", "other", "t1", "t2", "t3", "t4", "t5"];
    let add = || async {
        w.app.post(&w.root, "/teams", json!({"name": "t25"})).await;
    };
    let seen = w.walk(&w.root, "teamPage", json!({}), 2, name, add).await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    for t in all {
        assert!(seen.contains(&t.to_string()), "{t} missing from {seen:?}");
    }
    assert_eq!(seen, sorted(seen.clone()), "by name");

    let v = w.ok(&w.root, "teamPage", json!({"q": "T2"})).await;
    assert_eq!(names(&v["items"]), ["t2", "t25"], "any case, by name");
    assert!(v["nextCursor"].is_null());
    // pots and counts come with each team
    let v = w.ok(&w.root, "teamPage", json!({"q": "acme"})).await;
    assert_eq!(v["items"][0]["projects"], json!(["PAY"]));
    assert_eq!(v["items"][0]["members"], 3, "alice, carol and the creator");

    // a chef sees only their own teams
    let mine = w.walk_plain(&w.bob, "teamPage", json!({}), 1, name).await;
    assert_eq!(mine, ["other"]);

    // chefs of acme: people and cooks
    w.cook("pk-a1", "alice").await;
    w.cook("pk-a2", "alice").await;
    let args = json!({"team": "acme"});
    let seen = w
        .walk_plain(&w.alice, "teamMemberPage", args.clone(), 2, name)
        .await;
    assert_eq!(seen, ["alice", "carol", "octo", "pk-a1", "pk-a2"]);
    let v = w
        .ok(
            &w.alice,
            "teamMemberPage",
            json!({"team": "acme", "kind": "agent"}),
        )
        .await;
    assert_eq!(names(&v["items"]), ["pk-a1", "pk-a2"]);
    assert_eq!(v["items"][0]["fireAndForget"], false);
    let v = w
        .ok(
            &w.alice,
            "teamMemberPage",
            json!({"team": "acme", "kind": "user", "role": "owner"}),
        )
        .await;
    assert_eq!(names(&v["items"]), ["alice", "octo"]);
    let v = w
        .ok(
            &w.alice,
            "teamMemberPage",
            json!({"team": "acme", "role": "chef", "q": "car"}),
        )
        .await;
    assert_eq!(names(&v["items"]), ["carol"]);
    let (st, _) = w
        .proc(
            &w.alice,
            "teamMemberPage",
            json!({"team": "acme", "kind": "relay"}),
        )
        .await;
    assert_eq!(st, 400);
    // bob is not in acme
    let (st, _) = w.proc(&w.bob, "teamMemberPage", args).await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn people_cooks_and_fires() {
    let w = world().await;
    for n in 1..=4 {
        w.app.add_user(&format!("pp{n}")).await;
    }
    w.app.set_role("pp2", "orchestrator").await;
    let all = ["alice", "bob", "carol", "octo", "pp1", "pp2", "pp3", "pp4"];
    let add = || async { w.app.add_user("pp25").await };
    let seen = w.walk(&w.root, "userPage", json!({}), 3, name, add).await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    for u in all {
        assert!(seen.contains(&u.to_string()), "{u} missing from {seen:?}");
    }
    assert_eq!(seen, sorted(seen.clone()));
    let v = w.ok(&w.root, "userPage", json!({"q": "PP2"})).await;
    assert_eq!(names(&v["items"]), ["pp2", "pp25"]);
    let v = w
        .ok(&w.root, "userPage", json!({"role": "orchestrator"}))
        .await;
    assert_eq!(names(&v["items"]), ["alice", "pp2"]);
    sqlx::query("update principals set disabled_at = now() where name = 'pp3'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let v = w.ok(&w.root, "userPage", json!({"disabled": true})).await;
    assert_eq!(names(&v["items"]), ["pp3"]);
    assert_eq!(v["items"][0]["disabled"], true);
    let v = w
        .ok(&w.root, "userPage", json!({"disabled": false, "q": "pp"}))
        .await;
    assert!(!names(&v["items"]).contains(&"pp3".to_string()));
    // an orchestrator gets the directory, a member is refused
    let v = w.ok(&w.alice, "userPage", json!({"q": "pp3"})).await;
    assert!(
        v["items"].as_array().unwrap().is_empty(),
        "disabled people are not listed"
    );
    let v = w.ok(&w.alice, "userPage", json!({"q": "bob"})).await;
    assert!(v["items"][0]["linked"].is_null(), "no sign-in details: {v}");
    let (st, _) = w.proc(&w.bob, "userPage", json!({})).await;
    assert_eq!(st, 403);

    // cooks: alice owns five, bob one
    for n in 1..=5 {
        w.cook(&format!("ck-a{n}"), "alice").await;
    }
    w.cook("ck-b1", "bob").await;
    w.app
        .post(
            &w.root,
            "/teams/other/members",
            json!({"principal": "ck-b1"}),
        )
        .await;
    sqlx::query("update principals set disabled_at = now() where name = 'ck-a4'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let add = || async { w.cook("ck-a25", "alice").await };
    let seen = w.walk(&w.alice, "cookPage", json!({}), 2, name, add).await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert_eq!(seen, sorted(seen.clone()));
    assert!(!seen.contains(&"ck-b1".to_string()), "bob's cook is bob's");
    assert_eq!(seen.len(), 6);
    let v = w.ok(&w.alice, "cookPage", json!({"q": "A2"})).await;
    assert_eq!(names(&v["items"]), ["ck-a2", "ck-a25"]);
    let v = w.ok(&w.alice, "cookPage", json!({"disabled": true})).await;
    assert_eq!(names(&v["items"]), ["ck-a4"]);
    let v = w.ok(&w.root, "cookPage", json!({"owner": "bob"})).await;
    assert_eq!(names(&v["items"]), ["ck-b1"]);
    let v = w.ok(&w.root, "cookPage", json!({"team": "other"})).await;
    assert_eq!(names(&v["items"]), ["ck-b1"]);
    let v = w
        .ok(
            &w.root,
            "cookPage",
            json!({"team": "acme", "owner": "alice", "disabled": false, "q": "ck-a"}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 5);
    // a person with no cooks sees none
    let v = w.ok(&w.carol, "cookPage", json!({})).await;
    assert!(v["items"].as_array().unwrap().is_empty(), "{v}");

    // fires
    for n in 1..=4 {
        let (st, v) = w
            .app
            .post(&w.alice, "/relays", json!({"name": format!("fr-{n}")}))
            .await;
        assert_eq!(st, 200, "{v}");
    }
    w.app
        .post(&w.bob, "/relays", json!({"name": "fr-bob"}))
        .await;
    let seen = w.walk_plain(&w.alice, "firePage", json!({}), 3, name).await;
    assert_eq!(seen, ["fr-1", "fr-2", "fr-3", "fr-4"]);
    let v = w.ok(&w.alice, "firePage", json!({"q": "FR-3"})).await;
    assert_eq!(names(&v["items"]), ["fr-3"]);
    let v = w.ok(&w.root, "firePage", json!({"q": "bob"})).await;
    assert_eq!(names(&v["items"]), ["fr-bob"]);
}

#[tokio::test]
async fn wake_targets_and_unfollowed_repos() {
    let w = world().await;
    w.cook("wk-1", "alice").await;
    w.cook("wk-2", "alice").await;
    for n in 0..5 {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/agents/wk-1/wake-targets",
                json!({"type": "relay", "labels": [format!("l{n}")]}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let add = || async {
        w.app
            .post(
                &w.alice,
                "/agents/wk-1/wake-targets",
                json!({"type": "relay"}),
            )
            .await;
    };
    let seen = w
        .walk(
            &w.alice,
            "wakeTargetPage",
            json!({"cook": "wk-1"}),
            2,
            id,
            add,
        )
        .await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert!(seen.len() >= 5, "{seen:?}");
    assert_eq!(seen, sorted(seen.clone()));
    let v = w
        .ok(
            &w.alice,
            "wakeTargetPage",
            json!({"cook": "wk-1", "page": {"limit": 1}}),
        )
        .await;
    assert_eq!(v["items"][0]["type"], "relay");
    assert!(v["items"][0].get("config").is_none(), "no secrets: {v}");
    // somebody else's cook, and one that does not exist
    let (st, _) = w
        .proc(&w.bob, "wakeTargetPage", json!({"cook": "wk-1"}))
        .await;
    assert!(st == 403 || st == 404, "{st}");
    let (st, _) = w
        .proc(&w.alice, "wakeTargetPage", json!({"cook": "nobody"}))
        .await;
    assert!(st == 403 || st == 404, "{st}");
    let (st, _) = w
        .proc(
            &w.alice,
            "wakeTargetPage",
            json!({"cook": "wk-1", "page": {"cursor": "garbage"}}),
        )
        .await;
    assert_eq!(st, 400);

    // repositories no pot follows: most recent first, super-admins only
    let db = &w.app.state.db;
    for n in 1..=5 {
        sqlx::query(
            "insert into forge_unfollowed (provider, norm_url, repo, html_url, last_seen)
             values ('github', $1, $2, '', date_trunc('minute', now()) - make_interval(mins => $3::int))",
        )
        .bind(format!("github.com/acme/r{n}"))
        .bind(format!("acme/r{n}"))
        .bind(if n == 4 { 3 } else { n })
        .execute(db)
        .await
        .unwrap();
    }
    let repo = |v: &Value| v["repo"].as_str().unwrap().to_string();
    let add = || async {
        sqlx::query("insert into forge_unfollowed (provider, norm_url, repo) values ('github', 'github.com/zed/new', 'zed/new')")
            .execute(db)
            .await
            .unwrap();
    };
    let seen = w
        .walk(&w.root, "forgeUnfollowedPage", json!({}), 2, repo, add)
        .await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert_eq!(
        seen,
        ["acme/r1", "acme/r2", "acme/r3", "acme/r4", "acme/r5"],
        "ties (r3 and r4 share a minute) fall back to the url"
    );
    let v = w
        .ok(
            &w.root,
            "forgeUnfollowedPage",
            json!({"q": "ZED", "page": {}}),
        )
        .await;
    assert_eq!(v["items"][0]["repo"], "zed/new");
    assert!(v["items"][0]["installation"].is_null());
    let (st, _) = w
        .proc(&w.alice, "forgeUnfollowedPage", json!({"page": {}}))
        .await;
    assert_eq!(st, 403);
}

#[tokio::test]
async fn stations_activity_and_the_feed() {
    let w = world().await;
    w.cook("st-a", "alice").await;
    w.cook("st-b", "alice").await;
    let db = &w.app.state.db;
    let (pay, ops) = (w.pot("PAY").await, w.pot("OPS").await);
    let (ca, cb) = (w.pid("st-a").await, w.pid("st-b").await);
    let station = |n: i64, agent: uuid::Uuid, pot: uuid::Uuid, kind: &'static str| async move {
        sqlx::query("insert into stations (ref, agent_id, project_id, runtime, session_ref, kind, label, directory, created_at) values ($1, $2, $3, 'other', $1, $4, $5, $6, now() - make_interval(secs => 1000 - $7::int))")
            .bind(format!("st_{kind}{n}"))
            .bind(agent)
            .bind(pot)
            .bind(kind)
            .bind(format!("label {n}"))
            .bind(format!("/work/dir{n}"))
            .bind(n as i32)
            .execute(db)
            .await
            .unwrap();
    };
    for n in 1..=6 {
        station(n, if n % 2 == 0 { ca } else { cb }, pay, "session").await;
    }
    for n in 7..=9 {
        station(n, ca, pay, "cloud-run").await;
    }
    station(10, ca, ops, "session").await;
    let add = || async { station(99, cb, pay, "session").await };
    let seen = w
        .walk(
            &w.alice,
            "stationPage",
            json!({"project": "PAY", "kind": "session"}),
            2,
            id,
            add,
        )
        .await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert_eq!(
        seen,
        [
            "st_session6",
            "st_session5",
            "st_session4",
            "st_session3",
            "st_session2",
            "st_session1"
        ],
        "newest first; the one that arrived mid-walk is newer than the cursor"
    );
    // without a kind: sessions and the cloud runs still going
    let v = w
        .ok(
            &w.alice,
            "stationPage",
            json!({"project": "PAY", "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 10, "{v}");
    let v = w
        .ok(
            &w.alice,
            "stationPage",
            json!({"kind": "cloud-run", "live": true, "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    let v = w
        .ok(
            &w.alice,
            "stationPage",
            json!({"cook": "st-a", "kind": "session", "q": "DIR4"}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["items"][0]["id"], "st_session4");
    let v = w
        .ok(
            &w.alice,
            "stationPage",
            json!({"q": "/work/dir", "cook": "st-b", "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 4, "{v}");
    // a cursor minted for another kind is refused
    let v = w
        .ok(
            &w.alice,
            "stationPage",
            json!({"kind": "session", "page": {"limit": 1}}),
        )
        .await;
    let cur = v["nextCursor"].as_str().unwrap();
    let (st, _) = w
        .proc(
            &w.alice,
            "stationPage",
            json!({"kind": "cloud-run", "page": {"cursor": cur}}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w
        .proc(&w.alice, "stationPage", json!({"kind": "tmux", "page": {}}))
        .await;
    assert_eq!(st, 400);
    // bob sees none of acme's, a person without cooks none at all; cloudRunPage is the same list
    let v = w.ok(&w.bob, "stationPage", json!({"project": "PAY"})).await;
    assert!(v["items"].as_array().unwrap().is_empty(), "{v}");
    let v = w
        .ok(&w.root, "cloudRunPage", json!({"page": {"limit": 100}}))
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 3);

    // activity: events of kind activity, newest first
    for n in 1..=7 {
        sqlx::query(
            "insert into events (project_id, kind, payload) values ($1, 'activity', $2)",
        )
        .bind(pay)
        .bind(json!({"station": format!("st_{}", n % 2), "cook": if n % 3 == 0 { "st-b" } else { "st-a" },
                     "tool": "Edit", "files": [format!("f{n}.rs")], "count": 1}))
        .execute(db)
        .await
        .unwrap();
    }
    let add = || async {
        sqlx::query("insert into events (project_id, kind, payload) values ($1, 'activity', '{\"station\":\"st_0\",\"cook\":\"st-a\",\"tool\":\"Bash\"}')")
            .bind(pay)
            .execute(db)
            .await
            .unwrap();
    };
    let seen = w
        .walk(
            &w.alice,
            "activityPage",
            json!({"project": "PAY"}),
            3,
            seq,
            add,
        )
        .await;
    assert_eq!(seen.len(), 7, "{seen:?}");
    assert_eq!(unique(&seen), 7);
    let nums: Vec<i64> = seen.iter().map(|s| s.parse().unwrap()).collect();
    assert!(
        nums.windows(2).all(|p| p[0] > p[1]),
        "newest first: {nums:?}"
    );
    let v = w
        .ok(
            &w.alice,
            "activityPage",
            json!({"project": "PAY", "cook": "ST-B", "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 2, "{v}");
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["cook"] == "st-b")
    );
    let v = w
        .ok(
            &w.alice,
            "activityPage",
            json!({"project": "PAY", "station": "st_0", "cook": "st-a", "page": {"limit": 100}}),
        )
        .await;
    assert!(v["items"].as_array().unwrap().len() >= 2, "{v}");
    assert!(v["items"][0]["files"].is_array());
    let (st, _) = w
        .proc(
            &w.bob,
            "activityPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    assert_eq!(st, 404);
    let (st, _) = w
        .proc(
            &w.alice,
            "activityPage",
            json!({"project": "PAY", "page": {"cursor": "x"}}),
        )
        .await;
    assert_eq!(st, 400);

    // the change feed of a pot: kinds, prefixes, actor, task, and who may read an event
    for i in 1..=4 {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/projects/PAY/tasks",
                json!({"title": format!("t{i}")}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let alice_id = w.pid("alice").await;
    sqlx::query("insert into events (project_id, kind, actor_id, payload, audience) values ($1, 'pass.posted', $2, '{\"to\":\"alice\"}', $3)")
        .bind(pay)
        .bind(alice_id)
        .bind(vec![alice_id])
        .execute(db)
        .await
        .unwrap();
    let all = w
        .walk_plain(&w.alice, "eventPage", json!({"project": "PAY"}), 3, seq)
        .await;
    assert_eq!(unique(&all), all.len(), "{all:?}");
    let nums: Vec<i64> = all.iter().map(|s| s.parse().unwrap()).collect();
    assert!(
        nums.windows(2).all(|p| p[0] > p[1]),
        "newest first: {nums:?}"
    );
    assert!(!all.is_empty());
    let kinds = |v: &Value| -> Vec<String> {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].as_str().unwrap().to_string())
            .collect()
    };
    let v = w
        .ok(
            &w.alice,
            "eventPage",
            json!({"project": "PAY", "kinds": "task.*", "page": {"limit": 100}}),
        )
        .await;
    let k = kinds(&v);
    assert_eq!(k.len(), 4, "{k:?}");
    assert!(k.iter().all(|k| k.starts_with("task.")));
    let v = w
        .ok(&w.alice, "eventPage", json!({"project": "PAY", "kinds": "pass.posted, task.created", "actor": "Alice", "page": {"limit": 100}}))
        .await;
    assert_eq!(kinds(&v).len(), 5, "{v}");
    assert!(v["items"][0]["payload"].is_object());
    let v = w
        .ok(
            &w.alice,
            "eventPage",
            json!({"project": "PAY", "task": "PAY-2", "page": {"limit": 100}}),
        )
        .await;
    assert!(!kinds(&v).is_empty());
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["task"] == "PAY-2")
    );
    let v = w
        .ok(
            &w.alice,
            "eventPage",
            json!({"project": "PAY", "actor": "nobody", "page": {}}),
        )
        .await;
    assert!(v["items"].as_array().unwrap().is_empty());
    // the direct event is for alice alone: carol, a chef of the pot, never sees it
    let dm = |v: &Value| kinds(v).contains(&"pass.posted".to_string());
    let a = w
        .ok(
            &w.alice,
            "eventPage",
            json!({"project": "PAY", "page": {"limit": 200}}),
        )
        .await;
    let c = w
        .ok(
            &w.carol,
            "eventPage",
            json!({"project": "PAY", "page": {"limit": 200}}),
        )
        .await;
    assert!(dm(&a), "{a}");
    assert!(!dm(&c), "{c}");
    assert!(!kinds(&c).is_empty());
    let (st, _) = w
        .proc(&w.bob, "eventPage", json!({"project": "PAY", "page": {}}))
        .await;
    assert_eq!(st, 404);
    let (st, _) = w
        .proc(
            &w.alice,
            "eventPage",
            json!({"project": "PAY", "page": {"cursor": "x"}}),
        )
        .await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn files_and_usage() {
    let w = world().await;
    let db = &w.app.state.db;
    let pay = w.pot("PAY").await;
    for n in 1..=7i64 {
        sqlx::query("insert into files (id, project_id, number, name, content_type, storage_key, status, upload_expires_at, created_at) values (gen_random_uuid(), $1, $2, $3, 'image/png', 'k/' || $2, $4, now(), now() - make_interval(secs => 100 - ($2::int / 2)))")
            .bind(pay)
            .bind(n)
            .bind(format!("shot-{n}.png"))
            .bind(if n % 3 == 0 { "failed" } else { "ready" })
            .execute(db)
            .await
            .unwrap();
    }
    let number = |v: &Value| v["id"].as_str().unwrap().to_string();
    let add = || async {
        sqlx::query("insert into files (id, project_id, number, name, content_type, storage_key, status, upload_expires_at) values (gen_random_uuid(), $1, 8, 'late.png', 'image/png', 'k/8', 'ready', now())")
            .bind(pay)
            .execute(db)
            .await
            .unwrap();
    };
    let seen = w
        .walk(
            &w.alice,
            "filePage",
            json!({"project": "PAY"}),
            3,
            number,
            add,
        )
        .await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert_eq!(
        seen,
        [
            "PAY-F7", "PAY-F6", "PAY-F5", "PAY-F4", "PAY-F3", "PAY-F2", "PAY-F1"
        ],
        "newest first; ties on the second fall back to the number"
    );
    let v = w
        .ok(
            &w.alice,
            "filePage",
            json!({"project": "PAY", "status": "failed", "page": {"limit": 100}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 2);
    let v = w
        .ok(
            &w.alice,
            "filePage",
            json!({"project": "PAY", "q": "SHOT-5", "page": {}}),
        )
        .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert!(v["items"][0]["url"].as_str().unwrap().contains("PAY-F5"));
    let (st, _) = w
        .proc(
            &w.alice,
            "filePage",
            json!({"project": "PAY", "status": "deleted"}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w.proc(&w.bob, "filePage", json!({"project": "PAY"})).await;
    assert_eq!(st, 404);
    let (st, _) = w
        .proc(
            &w.alice,
            "filePage",
            json!({"project": "PAY", "page": {"cursor": "x"}}),
        )
        .await;
    assert_eq!(st, 400);

    // usage: the biggest senders first, across pages
    for (n, chars) in [
        ("us-a", 500),
        ("us-b", 900),
        ("us-c", 500),
        ("us-d", 100),
        ("us-e", 700),
    ] {
        w.cook(n, "alice").await;
        sqlx::query("insert into usage_daily (day, principal_id, project_id, kind, name, calls, chars) values ((now() at time zone 'utc')::date, $1, $2, 'tool', 'claim_next', 2, $3)")
            .bind(w.pid(n).await)
            .bind(pay)
            .bind(chars as i64)
            .execute(db)
            .await
            .unwrap();
    }
    w.cook("us-bob", "bob").await;
    sqlx::query("insert into usage_daily (day, principal_id, kind, name, calls, chars) values ((now() at time zone 'utc')::date, $1, 'list', 'tools/list', 1, 5000)")
        .bind(w.pid("us-bob").await)
        .execute(db)
        .await
        .unwrap();
    let mid = || async {
        sqlx::query("insert into usage_daily (day, principal_id, project_id, kind, name, calls, chars) values ((now() at time zone 'utc')::date, $1, $2, 'tool', 'x', 1, 800)")
            .bind(w.pid("us-d").await)
            .bind(pay)
            .execute(db)
            .await
            .unwrap();
    };
    let seen = w
        .walk(
            &w.alice,
            "usageCookPage",
            json!({"project": "PAY", "page": {}}),
            2,
            cook,
            mid,
        )
        .await;
    assert_eq!(unique(&seen), seen.len(), "{seen:?}");
    assert_eq!(
        &seen[..3],
        ["us-b", "us-e", "us-a"],
        "chars desc, then name; a total that grew past the cursor is not repeated: {seen:?}"
    );
    let v = w
        .ok(
            &w.alice,
            "usageCookPage",
            json!({"days": 1, "q": "US-C", "page": {}}),
        )
        .await;
    assert_eq!(
        v["items"][0],
        json!({"cook": "us-c", "calls": 2, "chars": 500, "tokens": 125})
    );
    // the owner sees their cooks only; a super-admin everyone's
    let v = w
        .ok(&w.alice, "usageCookPage", json!({"page": {"limit": 100}}))
        .await;
    assert!(
        !v["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["cook"] == "us-bob")
    );
    let v = w
        .ok(&w.root, "usageCookPage", json!({"page": {"limit": 1}}))
        .await;
    assert_eq!(v["items"][0]["cook"], "us-bob");
    assert!(v["nextCursor"].is_string());
    let (st, _) = w
        .proc(&w.alice, "usageCookPage", json!({"page": {"cursor": "x"}}))
        .await;
    assert_eq!(st, 400);
    // a cursor of another procedure's sort is refused
    let teams = w
        .ok(&w.root, "teamPage", json!({"page": {"limit": 1}}))
        .await;
    let (st, _) = w
        .proc(
            &w.alice,
            "usageCookPage",
            json!({"page": {"cursor": teams["nextCursor"]}}),
        )
        .await;
    assert_eq!(st, 400);
    // a forged key that does not fit its column is a 400, never a 500
    let first = chrono::Utc::now().date_naive() - chrono::Duration::days(6);
    let window = format!("-chars,cook:{first}");
    let forged = cratestack::encode_cursor(&window, &[Some("seven".into()), Some("x".into())]);
    let (st, _) = w
        .proc(
            &w.alice,
            "usageCookPage",
            json!({"page": {"cursor": forged}}),
        )
        .await;
    assert_eq!(st, 400);
    // a cursor minted for another window (the day rolled over, or another `days`) is refused; this window's is read
    let other = format!("-chars,cook:{}", first - chrono::Duration::days(1));
    let stale = cratestack::encode_cursor(&other, &[Some("100".into()), Some("x".into())]);
    let (st, _) = w
        .proc(
            &w.alice,
            "usageCookPage",
            json!({"page": {"cursor": stale}}),
        )
        .await;
    assert_eq!(st, 400);
    let fresh = cratestack::encode_cursor(&window, &[Some("100".into()), Some("x".into())]);
    let (st, v) = w
        .proc(
            &w.alice,
            "usageCookPage",
            json!({"page": {"cursor": fresh}}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, _) = w
        .proc(
            &w.alice,
            "usageCookPage",
            json!({"days": 3, "page": {"cursor": cratestack::encode_cursor(&window, &[Some("100".into()), Some("x".into())])}}),
        )
        .await;
    assert_eq!(
        st, 400,
        "a cursor of the 7-day window is not the 3-day window's"
    );
}

/// Keys Postgres would refuse (out-of-range timestamps, NUL in text) are a 400, never a 500.
#[tokio::test]
async fn forged_cursor_keys_are_a_400() {
    let w = world().await;
    let bad_ts = [
        "2026-01-01 00:00:00+16",
        "-0001-01-01 00:00:00+00",
        "+10000-01-01 00:00:00+00",
        "0001-01-01 00:00:00+05",
    ];
    let mut cases: Vec<(&str, Value, String, Vec<Option<String>>)> = vec![];
    for ts in bad_ts {
        let k = |a: &str, b: &str| vec![Some(a.to_string()), Some(b.to_string())];
        cases.push((
            "filePage",
            json!({"project": "PAY"}),
            "-created_at,-number".into(),
            k(ts, "3"),
        ));
        cases.push((
            "stationPage",
            json!({"project": "PAY"}),
            "-created_at,ref:live".into(),
            k(ts, "x"),
        ));
    }
    cases.push((
        "filePage",
        json!({"project": "PAY"}),
        "-created_at,-number".into(),
        vec![
            Some("2026-01-01 00:00:00+00".into()),
            Some("9223372036854775808".into()),
        ],
    ));
    cases.push((
        "stationPage",
        json!({"project": "PAY"}),
        "-created_at,ref:live".into(),
        vec![Some("2026-01-01 00:00:00+00".into()), Some("a\0b".into())],
    ));
    for (proc, args, order, keys) in cases {
        let mut args = args;
        args["page"] = json!({"cursor": cratestack::encode_cursor(&order, &keys)});
        let (st, v) = w.proc(&w.alice, proc, args).await;
        assert_eq!(st, 400, "{proc} {keys:?}: {v}");
    }
}

#[tokio::test]
async fn the_list_tools_answer_pages_on_both_protocols() {
    let w = world().await;
    for n in 1..=5 {
        w.app
            .post(&w.root, "/teams", json!({"name": format!("t{n}")}))
            .await;
    }
    // the 2026-07-28 path
    let mut got = vec![];
    let mut cursor = Value::Null;
    for _ in 0..20 {
        let r = w
            .app
            .mcp(
                &w.root,
                "list_teams",
                json!({"page": {"limit": 3, "cursor": cursor}}),
            )
            .await;
        assert_ne!(r["isError"], true, "{r}");
        let p = common::TestApp::payload(&r);
        got.extend(names(&p["items"]));
        cursor = p["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(got, ["acme", "other", "t1", "t2", "t3", "t4", "t5"]);
    // without a page: the first page of 25
    let r = w.app.mcp(&w.root, "list_users", json!({})).await;
    let p = common::TestApp::payload(&r);
    assert!(p["items"].as_array().unwrap().len() >= 4, "{p}");
    assert!(p["nextCursor"].is_null());
    for (tool, args) in [
        ("list_cooks", json!({})),
        ("list_fires", json!({})),
        ("list_stations", json!({})),
        ("list_files", json!({"project": "PAY"})),
        ("list_activity", json!({"project": "PAY"})),
        ("list_team_members", json!({"team": "acme"})),
    ] {
        let r = w.app.mcp(&w.root, tool, args).await;
        assert_ne!(r["isError"], true, "{tool}: {r}");
        assert!(
            common::TestApp::payload(&r)["items"].is_array(),
            "{tool}: {r}"
        );
    }
    // a cursor for another sort is a tool error
    let r = w
        .app
        .mcp(
            &w.root,
            "list_teams",
            json!({"page": {"cursor": "garbage"}}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");

    // the handshake-era path
    let post = |version: Option<&'static str>, method: &'static str, params: Value| {
        let (http, base, token) = (w.app.http.clone(), w.app.base.clone(), w.root.clone());
        async move {
            let mut r = http
                .post(format!("{base}/mcp"))
                .bearer_auth(token)
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}));
            if let Some(v) = version {
                r = r.header("MCP-Protocol-Version", v);
            }
            r.send().await.unwrap().json::<Value>().await.unwrap()
        }
    };
    let mut got = vec![];
    let mut cursor = Value::Null;
    for _ in 0..20 {
        let v = post(
            Some("2025-06-18"),
            "tools/call",
            json!({"name": "list_teams", "arguments": {"page": {"limit": 3, "cursor": cursor}}}),
        )
        .await;
        let p = common::TestApp::payload(&v["result"]);
        got.extend(names(&p["items"]));
        cursor = p["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(got, ["acme", "other", "t1", "t2", "t3", "t4", "t5"]);

    // tools/list on the handshake-era path: whole when it fits a page, a bad cursor is -32602
    let v = post(Some("2025-06-18"), "tools/list", json!({})).await;
    let tools = v["result"]["tools"].as_array().unwrap();
    assert!(tools.len() > 100, "{}", tools.len());
    assert!(
        v["result"]["nextCursor"].is_null() == (tools.len() < 200),
        "{}",
        tools.len()
    );
    for bad in [json!("garbage"), json!(5)] {
        let v = post(Some("2025-06-18"), "tools/list", json!({"cursor": bad})).await;
        assert_eq!(v["error"]["code"], -32602, "{v}");
    }
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in [
        "list_teams",
        "list_users",
        "list_stations",
        "list_files",
        "list_activity",
    ] {
        assert!(names.contains(&t), "{t}");
    }
    let t = tools.iter().find(|t| t["name"] == "list_teams").unwrap();
    assert!(
        t["inputSchema"]["properties"]["page"].is_object(),
        "{}",
        t["inputSchema"]
    );
}
