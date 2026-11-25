//! The console pages the pot, task, menu, view, label, tag, repository, pass, comment, question,
//! review and "my work" lists on their own (`potPage`, `taskPage`, `menuPage`, `savedViewPage`,
//! `viewPage`, `labelPage`, `tagPage`, `repoPage`, `passConversationPage`, `commentPage`,
//! `questionPage`, `questionMinePage`, `reviewPage`, `myWorkPage`): keyset walks with no gaps or
//! repeats, the filters, the visibility rules unchanged, and a cursor minted for another sort or a
//! forged one a 400. The MCP list tools answer the same pages.
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
    dave: String,
}

/// Teams acme (alice owns, carol and dave are chefs) with pot PAY, and other (bob is a chef) with pot OPS.
async fn world() -> W {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob", "carol", "dave"] {
        app.add_user(u).await;
    }
    app.set_role("alice", "orchestrator").await;
    for (team, key) in [("acme", "PAY"), ("other", "OPS")] {
        app.post(&root, "/teams", json!({"name": team})).await;
        app.post(
            &root,
            "/projects",
            json!({"team": team, "key": key, "name": format!("{key} pot")}),
        )
        .await;
    }
    for (team, who, role) in [
        ("acme", "alice", "owner"),
        ("acme", "carol", "member"),
        ("acme", "dave", "member"),
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
    let (alice, bob, carol, dave) = (
        tok("alice").await,
        tok("bob").await,
        tok("carol").await,
        tok("dave").await,
    );
    W {
        app,
        root,
        alice,
        bob,
        carol,
        dave,
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

    /// An MCP tool result's payload, which must not be an error.
    async fn tool(&self, token: &str, name: &str, args: Value) -> Value {
        let r = self.app.mcp(token, name, args.clone()).await;
        assert_ne!(r["isError"], true, "{name}({args}): {r}");
        common::TestApp::payload(&r)
    }

    /// Walk a `*Page` procedure with `limit` per page (the next-page key is `cursor_key`); `mid`
    /// runs after the first page.
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
        let key = if name == "taskPage" {
            "next_cursor"
        } else {
            "nextCursor"
        };
        for _ in 0..100 {
            args["page"] = json!({"limit": limit, "cursor": cursor});
            let v = self.ok(token, name, args.clone()).await;
            let items = v["items"].as_array().unwrap();
            assert!(items.len() as i64 <= limit, "{v}");
            seen.extend(items.iter().map(id_of));
            if let Some(m) = mid.take() {
                m().await;
            }
            cursor = v[key].as_str().map(String::from);
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

    /// Walk an MCP list tool (`page: {limit, cursor}`) to its end; the items it saw.
    async fn walk_tool(&self, token: &str, tool: &str, mut args: Value, limit: i64) -> Vec<Value> {
        let mut out = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            args["page"] = json!({"limit": limit, "cursor": cursor});
            let v = self.tool(token, tool, args.clone()).await;
            out.extend(v["items"].as_array().unwrap().iter().cloned());
            cursor = v["nextCursor"]
                .as_str()
                .or(v["next_cursor"].as_str())
                .map(String::from);
            if cursor.is_none() {
                return out;
            }
        }
        panic!("the walk of {tool} does not end");
    }

    async fn task(&self, token: &str, pot: &str, title: &str, extra: Value) -> String {
        let mut body = json!({"title": title});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        let (st, v) = self
            .app
            .post(token, &format!("/projects/{pot}/tasks"), body)
            .await;
        assert_eq!(st, 200, "{v}");
        v["id"].as_str().unwrap().to_string()
    }
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap().to_string()
}
fn key(v: &Value) -> String {
    s(v, "key")
}
fn id(v: &Value) -> String {
    s(v, "id")
}
fn label(v: &Value) -> String {
    s(v, "label")
}
fn tag(v: &Value) -> String {
    s(v, "tag")
}
fn name(v: &Value) -> String {
    s(v, "name")
}
fn chef(v: &Value) -> String {
    s(v, "chef")
}
fn title(v: &Value) -> String {
    s(v, "title")
}
fn unique(ids: &[String]) -> usize {
    ids.iter().collect::<HashSet<_>>().len()
}

/// A forged cursor: the right sort, keys that do not fit it.
fn forged(order: &str, keys: &[&str]) -> String {
    cratestack::encode_cursor(
        order,
        &keys.iter().map(|k| Some(k.to_string())).collect::<Vec<_>>(),
    )
}

#[tokio::test]
async fn pots_page_by_key_with_filters() {
    let w = world().await;
    for (k, n) in [("PAB", "Billing"), ("PAC", "Cards"), ("PAD", "Data")] {
        let (st, v) = w
            .app
            .post(
                &w.root,
                "/projects",
                json!({"team": "acme", "key": k, "name": n}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let all = w.walk_plain(&w.alice, "potPage", json!({}), 2, key).await;
    assert_eq!(all, ["PAB", "PAC", "PAD", "PAY"]);
    // a pot appearing mid-walk leaves the walk whole
    let seen = w
        .walk(&w.alice, "potPage", json!({}), 2, key, || async {
            w.app
                .post(
                    &w.root,
                    "/projects",
                    json!({"team": "acme", "key": "PAZ", "name": "Zed"}),
                )
                .await;
        })
        .await;
    assert_eq!(unique(&seen), seen.len());
    assert!(seen.starts_with(&["PAB".into(), "PAC".into(), "PAD".into(), "PAY".into()]));
    // filters
    let q = w
        .walk_plain(&w.alice, "potPage", json!({"q": "card"}), 5, key)
        .await;
    assert_eq!(q, ["PAC"]);
    let q = w
        .walk_plain(&w.alice, "potPage", json!({"team": "other"}), 5, key)
        .await;
    assert!(q.is_empty(), "alice is not in other: {q:?}");
    // visibility: bob sees his own team's pot only
    let bob = w.walk_plain(&w.bob, "potPage", json!({}), 2, key).await;
    assert_eq!(bob, ["OPS"]);
    // a bad or foreign cursor is a 400
    let (st, _) = w
        .proc(&w.alice, "potPage", json!({"page": {"cursor": "nonsense"}}))
        .await;
    assert_eq!(st, 400);
    let other = forged("name", &["PAB"]);
    let (st, _) = w
        .proc(&w.alice, "potPage", json!({"page": {"cursor": other}}))
        .await;
    assert_eq!(st, 400);
    // list_projects answers the same pages over MCP
    let items = w.walk_tool(&w.alice, "list_projects", json!({}), 2).await;
    assert_eq!(
        items.iter().map(key).collect::<Vec<_>>(),
        ["PAB", "PAC", "PAD", "PAY", "PAZ"]
    );
    let bob = w.tool(&w.bob, "list_projects", json!({})).await;
    assert_eq!(bob["items"].as_array().unwrap().len(), 1);
    assert!(bob.get("nextCursor").is_none() || bob["nextCursor"].is_null());
}

#[tokio::test]
async fn tasks_page_by_bucket_all_label_and_the_list_tasks_tool() {
    let w = world().await;
    let mut ids = vec![];
    for i in 0..5 {
        let labels = if i % 2 == 0 {
            json!(["api", "urgent"])
        } else {
            json!(["api"])
        };
        ids.push(
            w.task(
                &w.alice,
                "PAY",
                &format!("task {i}"),
                json!({"priority": i, "labels": labels}),
            )
            .await,
        );
    }
    // one served: out of the default list, in `all`
    let (st, v) = w
        .app
        .post(&w.alice, &format!("/tasks/{}/claim", ids[0]), json!({}))
        .await;
    assert_eq!(st, 200, "{v}");
    let (st, v) = w
        .app
        .post(
            &w.alice,
            &format!("/tasks/{}/complete", ids[0]),
            json!({"result": "done", "learned": "nothing new", "epoch": v["lease"]["epoch"]}),
        )
        .await;
    assert_eq!(st, 200, "{v}");

    let default = w
        .walk_plain(&w.alice, "taskPage", json!({"project": "PAY"}), 2, id)
        .await;
    assert_eq!(default.len(), 4, "bucket left out is active: {default:?}");
    assert_eq!(default, ["PAY-5", "PAY-4", "PAY-3", "PAY-2"]);
    let all = w
        .walk_plain(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "all"}),
            2,
            id,
        )
        .await;
    assert_eq!(all.len(), 5);
    assert_eq!(unique(&all), 5);
    assert!(all.contains(&ids[0]), "the served task is in `all`");
    let urgent = w
        .walk_plain(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "all", "label": "urgent"}),
            2,
            id,
        )
        .await;
    assert_eq!(urgent.len(), 3, "{urgent:?}");
    // the total counts every match; a cursor of one bucket does not fit another
    let first = w
        .ok(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "page": {"limit": 1}}),
        )
        .await;
    assert_eq!(first["total"], 4);
    let (st, _) = w
        .proc(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "all", "page": {"cursor": first["next_cursor"]}}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w
        .proc(
            &w.alice,
            "taskPage",
            json!({"project": "PAY", "bucket": "nonsense"}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w.proc(&w.bob, "taskPage", json!({"project": "PAY"})).await;
    assert_eq!(st, 404, "bob does not see PAY");
    // list_tasks is taskPage
    let tool = w
        .tool(&w.alice, "list_tasks", json!({"project": "PAY"}))
        .await;
    assert_eq!(tool["total"], 4);
    let items = w
        .walk_tool(
            &w.alice,
            "list_tasks",
            json!({"project": "PAY", "bucket": "all"}),
            2,
        )
        .await;
    assert_eq!(items.len(), 5);
}

#[tokio::test]
async fn menus_page_open_first_then_served() {
    let w = world().await;
    for (n, k) in [
        ("Alpha", "alpha"),
        ("Beta", "beta"),
        ("Gamma", "gamma"),
        ("Delta", "delta"),
    ] {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/projects/PAY/menus",
                json!({"name": n, "key": k}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    // serve the first: it goes behind the open ones
    let (st, v) = w
        .app
        .rest(
            &w.alice,
            reqwest::Method::PATCH,
            "/projects/PAY/menus/alpha",
            Some(json!({"status": "served"})),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let open = w
        .walk_plain(&w.alice, "menuPage", json!({"project": "PAY"}), 2, key)
        .await;
    assert_eq!(open, ["beta", "gamma", "delta"]);
    let all = w
        .walk_plain(
            &w.alice,
            "menuPage",
            json!({"project": "PAY", "status": "all"}),
            1,
            key,
        )
        .await;
    assert_eq!(all, ["beta", "gamma", "delta", "alpha"]);
    let served = w
        .walk_plain(
            &w.alice,
            "menuPage",
            json!({"project": "PAY", "status": "served"}),
            1,
            key,
        )
        .await;
    assert_eq!(served, ["alpha"]);
    let q = w
        .walk_plain(
            &w.alice,
            "menuPage",
            json!({"project": "PAY", "q": "ELT"}),
            5,
            key,
        )
        .await;
    assert_eq!(q, ["delta"]);
    // rows carry their progress
    let p = w.ok(&w.alice, "menuPage", json!({"project": "PAY"})).await;
    assert_eq!(p["items"][0]["progress"], 0.0);
    // bad status, other team, forged cursor
    for (who, args, want) in [
        (&w.alice, json!({"project": "PAY", "status": "x"}), 400),
        (&w.bob, json!({"project": "PAY"}), 404),
        (
            &w.alice,
            json!({"project": "PAY", "page": {"cursor": forged("served,position,key", &["x", "1", "k"])}}),
            400,
        ),
        (
            &w.alice,
            json!({"project": "PAY", "page": {"cursor": forged("served,position,key", &["0", "99999999999999999999", "k"])}}),
            400,
        ),
    ] {
        let (st, v) = w.proc(who, "menuPage", args.clone()).await;
        assert_eq!(st, want, "{args}: {v}");
    }
    // list_menus is menuPage
    let items = w
        .walk_tool(
            &w.alice,
            "list_menus",
            json!({"project": "PAY", "status": "all"}),
            2,
        )
        .await;
    assert_eq!(items.len(), 4);
}

#[tokio::test]
async fn views_page_saved_and_run() {
    let w = world().await;
    for (i, n) in ["one", "two", "three", "four"].iter().enumerate() {
        w.tool(
            &w.alice,
            "save_view",
            json!({"project": "PAY", "name": n, "where": format!("priority >= {i}")}),
        )
        .await;
    }
    for i in 0..5 {
        w.task(&w.alice, "PAY", &format!("t{i}"), json!({"priority": i}))
            .await;
    }
    let saved = w
        .walk_plain(&w.alice, "savedViewPage", json!({"project": "PAY"}), 3, key)
        .await;
    assert_eq!(saved, ["one", "two", "three", "four"]);
    let q = w
        .walk_plain(
            &w.alice,
            "savedViewPage",
            json!({"project": "PAY", "q": "THR"}),
            3,
            key,
        )
        .await;
    assert_eq!(q, ["three"]);
    let (st, _) = w
        .proc(&w.bob, "savedViewPage", json!({"project": "PAY"}))
        .await;
    assert_eq!(st, 404);
    let items = w
        .walk_tool(&w.alice, "list_views", json!({"project": "PAY"}), 2)
        .await;
    assert_eq!(items.len(), 4);

    // run_view pages the view's tasks; a cursor belongs to the condition that made it
    let run = w
        .walk_tool(
            &w.alice,
            "run_view",
            json!({"project": "PAY", "where": "priority >= 1", "orderBy": "priority"}),
            3,
        )
        .await;
    assert_eq!(
        run.iter().map(id).collect::<Vec<_>>(),
        ["PAY-2", "PAY-3", "PAY-4", "PAY-5"]
    );
    let first = w
        .tool(
            &w.alice,
            "run_view",
            json!({"project": "PAY", "where": "true", "page": {"limit": 1}}),
        )
        .await;
    let (st, _) = w
        .proc(
            &w.alice,
            "viewPage",
            json!({"project": "PAY", "where": "priority >= 1", "page": {"cursor": first["nextCursor"]}}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w.proc(&w.bob, "viewPage", json!({"project": "PAY"})).await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn labels_tags_and_repos_page_by_name() {
    let w = world().await;
    for (t, labels) in [
        ("a", json!(["api", "urgent"])),
        ("b", json!(["api", "ui"])),
        ("c", json!(["api", "docs"])),
        ("d", json!(["ui"])),
    ] {
        w.task(&w.alice, "PAY", t, json!({"labels": labels})).await;
    }
    // a done task: counted by bucket `all` only
    let id = w
        .task(&w.alice, "PAY", "e", json!({"labels": ["old"]}))
        .await;
    w.app
        .post(&w.alice, &format!("/tasks/{id}/cancel"), json!({}))
        .await;
    let labels = w
        .walk_plain(&w.alice, "labelPage", json!({"project": "PAY"}), 2, label)
        .await;
    assert_eq!(labels, ["api", "docs", "ui", "urgent"]);
    let all = w
        .walk_plain(
            &w.alice,
            "labelPage",
            json!({"project": "PAY", "bucket": "all"}),
            2,
            label,
        )
        .await;
    assert_eq!(all, ["api", "docs", "old", "ui", "urgent"]);
    let counted = w
        .ok(
            &w.alice,
            "labelPage",
            json!({"project": "PAY", "q": "AP", "page": {}}),
        )
        .await;
    assert_eq!(counted["items"], json!([{"label": "api", "count": 3}]));
    let ui = w
        .ok(
            &w.alice,
            "labelPage",
            json!({"project": "PAY", "q": "ui", "page": {}}),
        )
        .await;
    assert_eq!(ui["items"], json!([{"label": "ui", "count": 2}]));
    for (who, args, want) in [
        (
            &w.alice,
            json!({"project": "PAY", "bucket": "x", "page": {}}),
            400,
        ),
        (&w.bob, json!({"project": "PAY", "page": {}}), 404),
        (&w.alice, json!({"project": "PAY"}), 400), // the page is required
    ] {
        let (st, v) = w.proc(who, "labelPage", args.clone()).await;
        assert_eq!(st, want, "{args}: {v}");
    }

    // tags of the pantry
    for (k, kind, tags) in [
        ("m1", "note", json!(["db", "ops"])),
        ("m2", "gotcha", json!(["db"])),
        ("m3", "note", json!(["api"])),
    ] {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/projects/PAY/pantry",
                json!({"key": k, "title": k, "body": "b", "kind": kind, "tags": tags}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let tags = w
        .walk_plain(&w.alice, "tagPage", json!({"project": "PAY"}), 2, tag)
        .await;
    assert_eq!(tags, ["api", "db", "ops"]);
    let notes = w
        .ok(
            &w.alice,
            "tagPage",
            json!({"project": "PAY", "kind": "note", "page": {}}),
        )
        .await;
    assert_eq!(
        notes["items"],
        json!([{"tag": "api", "count": 1}, {"tag": "db", "count": 1}, {"tag": "ops", "count": 1}])
    );
    let q = w
        .ok(
            &w.alice,
            "tagPage",
            json!({"project": "PAY", "q": "D", "page": {}}),
        )
        .await;
    assert_eq!(q["items"], json!([{"tag": "db", "count": 2}]));
    let (st, _) = w
        .proc(
            &w.alice,
            "tagPage",
            json!({"project": "PAY", "kind": "x", "page": {}}),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = w
        .proc(&w.bob, "tagPage", json!({"project": "PAY", "page": {}}))
        .await;
    assert_eq!(st, 404);

    // repositories
    for n in ["web", "api", "docs"] {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/projects/PAY/repos",
                json!({"url": format!("https://github.com/acme/{n}.git"), "name": n}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let repos = w
        .walk_plain(&w.alice, "repoPage", json!({"project": "PAY"}), 2, name)
        .await;
    assert_eq!(repos, ["api", "docs", "web"]);
    let q = w
        .ok(
            &w.alice,
            "repoPage",
            json!({"project": "PAY", "q": "GITHUB.COM/ACME/DO", "page": {}}),
        )
        .await;
    assert_eq!(q["items"][0]["name"], "docs");
    assert_eq!(q["items"].as_array().unwrap().len(), 1);
    let (st, _) = w
        .proc(&w.bob, "repoPage", json!({"project": "PAY", "page": {}}))
        .await;
    assert_eq!(st, 404);
    let (st, _) = w
        .proc(
            &w.alice,
            "repoPage",
            json!({"project": "PAY", "page": {"cursor": forged("label", &["a"])}}),
        )
        .await;
    assert_eq!(st, 400);
}

#[tokio::test]
async fn pass_conversations_newest_first_with_unread_counts() {
    let w = world().await;
    for (from, to, msg) in [
        (&w.carol, "alice", "carol to alice 1"),
        (&w.dave, "alice", "dave to alice"),
        (&w.carol, "alice", "carol to alice 2"),
        (&w.alice, "dave", "alice to dave"),
        (&w.carol, "dave", "carol to dave"),
        (&w.carol, "pot", "on the pass"),
    ] {
        let mut args = json!({"project": "PAY", "message": msg});
        if to != "pot" {
            args["to"] = json!(to);
        }
        w.tool(from, "pass", args).await;
    }
    // alice: dave (her own message is the newest) then carol; the pass itself is no conversation
    let v = w
        .ok(
            &w.alice,
            "passConversationPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    let rows: Vec<(String, i64, String)> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (chef(c), c["unread"].as_i64().unwrap(), s(c, "preview")))
        .collect();
    assert_eq!(
        rows,
        [
            ("dave".to_string(), 0, "alice to dave".to_string()),
            ("carol".to_string(), 2, "carol to alice 2".to_string())
        ]
    );
    assert_eq!(v["items"][0]["kind"], "user");
    assert!(
        v["items"][0]["last_seq"].as_i64().unwrap() > v["items"][1]["last_seq"].as_i64().unwrap()
    );
    // dave never sees what carol said to alice
    let d = w
        .ok(
            &w.dave,
            "passConversationPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    let rows: Vec<(String, String)> = d["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (chef(c), s(c, "preview")))
        .collect();
    assert_eq!(
        rows,
        [
            ("carol".to_string(), "carol to dave".to_string()),
            ("alice".to_string(), "alice to dave".to_string())
        ]
    );
    // a walk one by one, q and unread
    let seen = w
        .walk_plain(
            &w.alice,
            "passConversationPage",
            json!({"project": "PAY"}),
            1,
            chef,
        )
        .await;
    assert_eq!(seen, ["dave", "carol"]);
    let q = w
        .walk_plain(
            &w.alice,
            "passConversationPage",
            json!({"project": "PAY", "q": "CAR"}),
            5,
            chef,
        )
        .await;
    assert_eq!(q, ["carol"]);
    // answering a conversation reads it; reading one clears its unread count
    let unread_args = json!({"project": "PAY", "unread": true});
    let unread = w
        .walk_plain(
            &w.alice,
            "passConversationPage",
            unread_args.clone(),
            5,
            chef,
        )
        .await;
    assert_eq!(unread, ["carol"]);
    w.tool(
        &w.alice,
        "read_pass",
        json!({"project": "PAY", "chef": "carol"}),
    )
    .await;
    let unread = w
        .walk_plain(&w.alice, "passConversationPage", unread_args, 5, chef)
        .await;
    assert!(unread.is_empty());
    // other team, forged and foreign cursors
    let (st, _) = w
        .proc(
            &w.bob,
            "passConversationPage",
            json!({"project": "PAY", "page": {}}),
        )
        .await;
    assert_eq!(st, 404);
    for cursor in [
        forged("-last_seq,chef", &["x", "carol"]),
        forged("name", &["carol"]),
    ] {
        let (st, _) = w
            .proc(
                &w.alice,
                "passConversationPage",
                json!({"project": "PAY", "page": {"cursor": cursor}}),
            )
            .await;
        assert_eq!(st, 400);
    }
}

#[tokio::test]
async fn comments_page_newest_first() {
    let w = world().await;
    let t = w.task(&w.alice, "PAY", "talk", json!({})).await;
    for i in 0..5 {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                &format!("/tasks/{t}/comment"),
                json!({"body": format!("c{i}")}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let body = |v: &Value| s(v, "body");
    let seen = w
        .walk_plain(&w.alice, "commentPage", json!({"task": t}), 2, body)
        .await;
    assert_eq!(seen, ["c4", "c3", "c2", "c1", "c0"]);
    // a comment arriving mid-walk does not repeat or skip one
    let seen = w
        .walk(
            &w.alice,
            "commentPage",
            json!({"task": t}),
            2,
            body,
            || async {
                w.app
                    .post(
                        &w.alice,
                        &format!("/tasks/{t}/comment"),
                        json!({"body": "late"}),
                    )
                    .await;
            },
        )
        .await;
    assert_eq!(unique(&seen), seen.len());
    assert_eq!(&seen[..2], ["c4", "c3"]);
    let first = w
        .ok(&w.alice, "commentPage", json!({"task": t, "page": {}}))
        .await;
    assert_eq!(first["items"][0]["author"], "alice");
    assert_eq!(first["items"][0]["author_kind"], "user");
    let (st, _) = w
        .proc(&w.bob, "commentPage", json!({"task": t, "page": {}}))
        .await;
    assert_eq!(st, 404);
    for cursor in [
        forged("-created_at,id", &["yesterday", "x"]),
        forged("number", &["1"]),
    ] {
        let (st, _) = w
            .proc(
                &w.alice,
                "commentPage",
                json!({"task": t, "page": {"cursor": cursor}}),
            )
            .await;
        assert_eq!(st, 400);
    }
    // an id that is not a uuid is a mismatch, not a failed cast
    let odd = forged("-created_at,id", &["2026-05-01 10:11:12+00", "not a uuid"]);
    let (st, _) = w
        .proc(
            &w.alice,
            "commentPage",
            json!({"task": t, "page": {"cursor": odd}}),
        )
        .await;
    assert_eq!(st, 200);
}

#[tokio::test]
async fn questions_and_reviews_page_over_mcp_too() {
    let w = world().await;
    let t = w.task(&w.alice, "PAY", "work", json!({})).await;
    for i in 0..5 {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                "/projects/PAY/questions",
                json!({"title": format!("q{i}?")}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    // the pool: open, oldest first
    let pool = w
        .walk_tool(&w.carol, "list_questions", json!({"project": "PAY"}), 2)
        .await;
    assert_eq!(
        pool.iter().map(title).collect::<Vec<_>>(),
        ["q0?", "q1?", "q2?", "q3?", "q4?"]
    );
    // the ones alice asked, newest first, also through procedures
    let mine = w.walk_tool(&w.alice, "my_questions", json!({}), 2).await;
    assert_eq!(
        mine.iter().map(title).collect::<Vec<_>>(),
        ["q4?", "q3?", "q2?", "q1?", "q0?"]
    );
    let mine = w
        .walk_plain(
            &w.alice,
            "questionMinePage",
            json!({"status": "all"}),
            2,
            id,
        )
        .await;
    assert_eq!(mine.len(), 5);
    assert_eq!(unique(&mine), 5);
    let none = w
        .ok(&w.carol, "questionMinePage", json!({"page": {}}))
        .await;
    assert!(none["items"].as_array().unwrap().is_empty());
    let (st, _) = w
        .proc(
            &w.alice,
            "questionMinePage",
            json!({"status": "x", "page": {}}),
        )
        .await;
    assert_eq!(st, 400);
    for cursor in [
        forged("-created_at,id", &["yesterday", "x"]),
        forged("created_at,number", &["2026-05-01 10:11:12+00", "1"]),
    ] {
        let (st, _) = w
            .proc(
                &w.alice,
                "questionMinePage",
                json!({"page": {"cursor": cursor}}),
            )
            .await;
        assert_eq!(st, 400);
    }
    let (st, _) = w
        .proc(&w.bob, "questionPage", json!({"project": "PAY"}))
        .await;
    assert_eq!(st, 404, "the page is optional, the pot is not bob's");

    // reviews: the task filter and the page
    for who in ["carol", "dave"] {
        let (st, v) = w
            .app
            .post(
                &w.alice,
                &format!("/tasks/{t}/review-request"),
                json!({"reviewer": who, "note": "look"}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
    }
    let other = w.task(&w.alice, "PAY", "other", json!({})).await;
    let (st, v) = w
        .app
        .post(
            &w.alice,
            &format!("/tasks/{other}/review-request"),
            json!({"reviewer": "carol"}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let all = w
        .walk_tool(&w.alice, "list_reviews", json!({"project": "PAY"}), 2)
        .await;
    assert_eq!(all.len(), 3);
    let of_task = w
        .walk_tool(
            &w.alice,
            "list_reviews",
            json!({"project": "PAY", "task": t}),
            1,
        )
        .await;
    assert_eq!(of_task.len(), 2, "{of_task:?}");
    let mine = w
        .walk_tool(
            &w.carol,
            "list_reviews",
            json!({"project": "PAY", "mine": true}),
            1,
        )
        .await;
    assert_eq!(mine.len(), 2);
    let (st, _) = w
        .proc(
            &w.alice,
            "reviewPage",
            json!({"project": "PAY", "task": "PAY-99"}),
        )
        .await;
    assert_eq!(st, 404);
}

#[tokio::test]
async fn my_work_pages_by_section() {
    let w = world().await;
    let (st, _) = w
        .app
        .post(
            &w.root,
            "/projects",
            json!({"team": "acme", "key": "PAB", "name": "B"}),
        )
        .await;
    assert_eq!(st, 200);
    // owned: priority first, then pot and number
    let a = w
        .task(
            &w.alice,
            "PAY",
            "a",
            json!({"owner": "alice", "priority": 1}),
        )
        .await;
    let b = w
        .task(
            &w.alice,
            "PAB",
            "b",
            json!({"owner": "alice", "priority": 5}),
        )
        .await;
    let c = w
        .task(
            &w.alice,
            "PAY",
            "c",
            json!({"owner": "alice", "priority": 5}),
        )
        .await;
    let d = w
        .task(
            &w.alice,
            "PAB",
            "d",
            json!({"owner": "alice", "priority": 1}),
        )
        .await;
    w.task(&w.alice, "PAY", "not mine", json!({"owner": "carol"}))
        .await;
    let owned = w
        .walk_plain(&w.alice, "myWorkPage", json!({"section": "owned"}), 2, id)
        .await;
    assert_eq!(owned, [b.clone(), c.clone(), d.clone(), a.clone()]);
    let pot = w
        .walk_plain(
            &w.alice,
            "myWorkPage",
            json!({"section": "owned", "project": "PAY"}),
            1,
            id,
        )
        .await;
    assert_eq!(pot, [c.clone(), a.clone()]);
    // holding: a live lease
    let (st, v) = w
        .app
        .post(&w.alice, &format!("/tasks/{c}/claim"), json!({}))
        .await;
    assert_eq!(st, 200, "{v}");
    let held = w
        .walk_plain(&w.alice, "myWorkPage", json!({"section": "holding"}), 5, id)
        .await;
    assert_eq!(held, std::slice::from_ref(&c));
    // blocked: tasks that wait on a task alice owns (once, whoever owns them)
    let dep = w
        .task(
            &w.carol,
            "PAY",
            "waits",
            json!({"depends_on": [a.clone()], "priority": 3}),
        )
        .await;
    let blocked = w
        .walk_plain(&w.alice, "myWorkPage", json!({"section": "blocked"}), 1, id)
        .await;
    assert_eq!(blocked, std::slice::from_ref(&dep));
    let row = w
        .ok(
            &w.alice,
            "myWorkPage",
            json!({"section": "blocked", "page": {}}),
        )
        .await;
    assert_eq!(row["items"][0]["via"], "dependency");
    assert_eq!(row["items"][0]["because"], json!(a));
    assert!(row["items"][0].get("pot_name").is_none());
    // carol owns nothing alice's; she holds nothing
    let none = w
        .ok(
            &w.dave,
            "myWorkPage",
            json!({"section": "owned", "page": {}}),
        )
        .await;
    assert!(none["items"].as_array().unwrap().is_empty());
    // bad section, unknown pot, forged and foreign cursors
    for (args, want) in [
        (json!({"section": "x", "page": {}}), 400),
        (
            json!({"section": "owned", "project": "NOPE", "page": {}}),
            404,
        ),
        (
            json!({"section": "owned", "page": {"cursor": forged("-priority,key,number:owned", &["x", "PAY", "1"])}}),
            400,
        ),
        (
            json!({"section": "holding", "page": {"cursor": forged("-priority,key,number:owned", &["1", "PAY", "1"])}}),
            400,
        ),
        (json!({"section": "owned"}), 400),
    ] {
        let (st, v) = w.proc(&w.alice, "myWorkPage", args.clone()).await;
        assert_eq!(st, want, "{args}: {v}");
    }
}
