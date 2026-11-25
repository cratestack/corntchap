//! Forge events (src/forge): signed deliveries, deduplication, which repositories are followed,
//! how a PR finds its task, what each event does to `forge_prs`, who is woken (and who is not), that
//! a merge completes only a task nobody holds, the cloud rules, and the GitHub App manifest flow against a fake
//! GitHub.
mod common;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use common::TestApp;
use corntchap::{admin, forge::github::signature};
use serde_json::{Value, json};

const SECRET: &str = "whsec_test";

async fn tool(app: &TestApp, tok: &str, name: &str, args: Value) -> Value {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_ne!(r["isError"], true, "{name}({args}): {r}");
    TestApp::payload(&r)
}

// ---------- GitHub-shaped fixtures (small) ----------

fn repo(name: &str) -> Value {
    json!({"full_name": format!("Acme/{name}"), "html_url": format!("https://github.com/Acme/{name}"),
           "clone_url": format!("https://github.com/Acme/{name}.git")})
}

fn pr(repo_name: &str, n: i64, branch: &str, sha: &str, title: &str) -> Value {
    json!({"number": n, "html_url": format!("https://github.com/Acme/{repo_name}/pull/{n}"),
           "title": title, "body": "", "state": "open", "draft": false, "merged": false,
           "head": {"ref": branch, "sha": sha}, "base": {"ref": "main"}, "user": {"login": "ann"}})
}

fn pr_event(action: &str, repo_name: &str, p: Value) -> Value {
    json!({"action": action, "number": p["number"], "pull_request": p, "repository": repo(repo_name),
           "installation": {"id": 77}, "sender": {"login": "ann"}})
}

fn suite(repo_name: &str, n: i64, sha: &str, conclusion: &str) -> Value {
    json!({"action": "completed", "repository": repo(repo_name), "installation": {"id": 77},
           "check_suite": {"conclusion": conclusion, "head_sha": sha, "head_branch": "x", "app": {"name": "GitHub Actions"},
                           "pull_requests": [{"number": n}]}})
}

fn check_run(repo_name: &str, n: i64, sha: &str, name: &str, conclusion: &str) -> Value {
    json!({"action": "completed", "repository": repo(repo_name),
           "check_run": {"name": name, "conclusion": conclusion, "head_sha": sha, "html_url": format!("https://github.com/Acme/{repo_name}/runs/1"),
                         "output": {"title": "failed", "summary": ""}, "pull_requests": [{"number": n}]}})
}

fn workflow(repo_name: &str, n: i64, sha: &str, name: &str, conclusion: &str) -> Value {
    json!({"action": "completed", "repository": repo(repo_name),
           "workflow_run": {"name": name, "conclusion": conclusion, "head_sha": sha,
                            "html_url": format!("https://github.com/Acme/{repo_name}/actions/runs/9"), "pull_requests": [{"number": n}]}})
}

fn review(repo_name: &str, p: Value, state: &str, who: &str, body: &str) -> Value {
    json!({"action": "submitted", "repository": repo(repo_name), "pull_request": p,
           "review": {"state": state, "body": body, "user": {"login": who}, "html_url": "https://github.com/Acme/shop/pull/1#r1"}})
}

fn comment(repo_name: &str, n: i64, who: &str, body: &str) -> Value {
    json!({"action": "created", "repository": repo(repo_name),
           "issue": {"number": n, "title": "t", "pull_request": {"html_url": format!("https://github.com/Acme/{repo_name}/pull/{n}")}, "user": {"login": "ann"}},
           "comment": {"body": body, "user": {"login": who}, "html_url": format!("https://github.com/Acme/{repo_name}/pull/{n}#c1")}})
}

// ---------- the world ----------

struct World {
    app: TestApp,
    root: String,
    alice: String,
    n: std::sync::atomic::AtomicU32,
}

/// octo (admin), alice (owner of team acme and its pot SHOP, which has the repository acme/shop), a forge
/// connection whose webhook secret is `SECRET`.
async fn world(background: bool) -> World {
    let app = common::start(background).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
    app.set_role("alice", "orchestrator").await; // owning a team needs the role
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    tool(&app, &root, "create_team", json!({"name": "acme"})).await;
    tool(
        &app,
        &root,
        "add_team_member",
        json!({"team": "acme", "principal": "alice", "role": "owner"}),
    )
    .await;
    tool(
        &app,
        &alice,
        "create_project",
        json!({"team": "acme", "key": "SHOP", "name": "Shop"}),
    )
    .await;
    tool(
        &app,
        &alice,
        "add_repo",
        json!({"project": "SHOP", "url": "https://github.com/acme/shop.git"}),
    )
    .await;
    sqlx::query(
        "insert into forge_connections (provider, name, api_url, web_url, webhook_secret) values ('github','test','http://x','https://github.com',$1)",
    )
    .bind(SECRET)
    .execute(&app.state.db)
    .await
    .unwrap();
    World {
        app,
        root,
        alice,
        n: Default::default(),
    }
}

impl World {
    /// The restaurant decided `decision` for `reason` about `task`: its event, actor and sentence.
    async fn decided(&self, decision: &str, reason: &str, task: &str) {
        let (actor, text, tref): (String, String, String) = sqlx::query_as(
            "select a.name, e.payload->>'text', e.payload->>'task'
             from events e join principals a on a.id = e.actor_id
             where e.kind = 'restaurant.decided' and e.payload->>'decision' = $1 and e.payload->>'reason' = $2",
        )
        .bind(decision)
        .bind(reason)
        .fetch_one(&self.app.state.db)
        .await
        .unwrap_or_else(|e| panic!("{decision}/{reason}: {e}"));
        assert_eq!((actor.as_str(), tref.as_str()), ("restaurant", task));
        assert!(text.contains(task), "{text}");
    }

    async fn cook(&self, name: &str, runtime: &str) -> String {
        let (_, ag) = self
            .app
            .post(
                &self.root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": runtime, "max_leases": 10}),
            )
            .await;
        self.app
            .post(
                &self.root,
                "/teams/acme/members",
                json!({"principal": name}),
            )
            .await;
        ag["token"].as_str().unwrap().to_string()
    }

    async fn task(&self, title: &str) -> String {
        tool(
            &self.app,
            &self.alice,
            "create_task",
            json!({"project": "SHOP", "title": title}),
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// The cook takes the task and keeps it.
    async fn hold(&self, tok: &str, id: &str) {
        tool(&self.app, tok, "claim_task", json!({"id": id})).await;
    }

    fn delivery(&self) -> String {
        format!(
            "d-{}",
            self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        )
    }

    /// POST a signed delivery; returns the HTTP status and the delivery id.
    async fn post(&self, event: &str, body: &Value) -> (u16, String) {
        let id = self.delivery();
        (self.post_as(event, &id, body, SECRET).await, id)
    }

    async fn post_as(&self, event: &str, id: &str, body: &Value, secret: &str) -> u16 {
        let raw = serde_json::to_vec(body).unwrap();
        self.app
            .http
            .post(format!("{}/webhooks/github", self.app.base))
            .header("x-github-event", event)
            .header("x-github-delivery", id)
            .header("x-hub-signature-256", signature(secret, &raw))
            .header("content-type", "application/json")
            .body(raw)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    /// Deliver and wait until it has been processed: (status, detail).
    async fn send(&self, event: &str, body: Value) -> (String, String) {
        let (code, id) = self.post(event, &body).await;
        assert_eq!(code, 202, "{event}: {body}");
        self.settled(&id).await
    }

    async fn settled(&self, id: &str) -> (String, String) {
        for _ in 0..100 {
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "select status, detail from forge_deliveries where delivery_id = $1",
            )
            .bind(id)
            .fetch_optional(&self.app.state.db)
            .await
            .unwrap();
            if let Some((s, d)) = row
                && s != "received"
            {
                return (s, d.unwrap_or_default());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("delivery {id} was never processed");
    }

    async fn wakes(&self, agent: &str, reason: &str) -> Vec<String> {
        sqlx::query_scalar(
            "select payload->>'text' from events where kind = 'wake' and payload->>'reason' = $2 and payload->>'agent' = $1 order by seq",
        )
        .bind(agent)
        .bind(reason)
        .fetch_all(&self.app.state.db)
        .await
        .unwrap()
    }

    async fn pr_row(
        &self,
        repo_name: &str,
        n: i64,
    ) -> Option<(
        String,
        String,
        String,
        Vec<String>,
        String,
        String,
        Option<String>,
    )> {
        sqlx::query_as(
            "select state, checks, review, failing, mergeable, head_sha, (select pr.key || '-' || t.number from tasks t join projects pr on pr.id = t.project_id where t.id = forge_prs.task_id)
             from forge_prs where lower(repo) = lower($1) and number = $2",
        )
        .bind(format!("acme/{repo_name}"))
        .bind(n)
        .fetch_optional(&self.app.state.db)
        .await
        .unwrap()
    }

    async fn status(&self, id: &str) -> String {
        tool(&self.app, &self.alice, "get_task", json!({"id": id})).await["status"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn events(&self, kind: &str) -> i64 {
        sqlx::query_scalar("select count(*) from events where kind = $1")
            .bind(kind)
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }
}

// ---------- tests ----------

#[tokio::test]
async fn bad_signatures_are_refused() {
    let w = world(false).await;
    let body = pr_event("opened", "shop", pr("shop", 1, "shop-1-x", "aaa", "t"));
    // signed with another secret
    assert_eq!(w.post_as("pull_request", "x1", &body, "nope").await, 401);
    // no signature at all
    let r = w
        .app
        .http
        .post(format!("{}/webhooks/github", w.app.base))
        .header("x-github-event", "pull_request")
        .header("x-github-delivery", "x2")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // the body changed after signing
    let raw = serde_json::to_vec(&body).unwrap();
    let r = w
        .app
        .http
        .post(format!("{}/webhooks/github", w.app.base))
        .header("x-github-event", "pull_request")
        .header("x-github-delivery", "x3")
        .header("x-hub-signature-256", signature(SECRET, &raw))
        .body([raw.clone(), b" ".to_vec()].concat())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // an unknown provider
    let r = w
        .app
        .http
        .post(format!("{}/webhooks/nowhere", w.app.base))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    let n: i64 = sqlx::query_scalar("select count(*) from forge_deliveries")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0, "refused deliveries leave no trace");
    // and a good one is accepted
    assert_eq!(
        w.post_as("ping", "ok1", &json!({"zen": "x"}), SECRET).await,
        202
    );
}

#[tokio::test]
async fn a_redelivery_is_processed_once() {
    let w = world(false).await;
    w.task("Cart").await;
    let body = pr_event(
        "opened",
        "shop",
        pr("shop", 1, "shop-1-cart", "aaa", "Cart"),
    );
    assert_eq!(w.post_as("pull_request", "same", &body, SECRET).await, 202);
    let (st, _) = w.settled("same").await;
    assert_eq!(st, "ok");
    assert_eq!(w.post_as("pull_request", "same", &body, SECRET).await, 200);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(w.events("forge.pr").await, 1);
    let n: i64 =
        sqlx::query_scalar("select count(*) from forge_deliveries where delivery_id = 'same'")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(n, 1);
    // a delivery that failed may be redelivered
    sqlx::query("update forge_deliveries set status = 'error' where delivery_id = 'same'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(w.post_as("pull_request", "same", &body, SECRET).await, 202);
    // old deliveries go with the retention job
    sqlx::query(
        "update forge_deliveries set at = now() - interval '8 days' where delivery_id = 'same'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    w.settled("same").await;
    sqlx::query(
        "update forge_deliveries set at = now() - interval '8 days' where delivery_id = 'same'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    corntchap::jobs::retain(&w.app.state).await.unwrap();
    let n: i64 = sqlx::query_scalar("select count(*) from forge_deliveries")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn only_repositories_of_a_pot_are_followed() {
    let w = world(false).await;
    w.task("Cart").await;
    // not in any pot: acknowledged, nothing kept
    let (st, d) = w
        .send(
            "pull_request",
            pr_event(
                "opened",
                "elsewhere",
                pr("elsewhere", 5, "shop-1-x", "a", "SHOP-1"),
            ),
        )
        .await;
    assert_eq!(st, "ignored", "{d}");
    assert!(w.pr_row("elsewhere", 5).await.is_none());
    // a pot's repository, whatever the case or the spelling of its url
    let (st, _) = w
        .send(
            "pull_request",
            pr_event(
                "opened",
                "Shop",
                pr("Shop", 6, "plain", "a", "no task here"),
            ),
        )
        .await;
    assert_eq!(st, "ok");
    let row = w.pr_row("shop", 6).await.unwrap();
    assert_eq!(row.0, "open");
    assert_eq!(row.6, None, "tracked, linked to nothing");
    // the legacy repo_url of a pot with no project_repos row
    sqlx::query("insert into projects (team_id, key, name, repo_url) select team_id, 'OLD', 'Old', 'git@github.com:Acme/legacy.git' from projects where key = 'SHOP'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    sqlx::query(
        "delete from project_repos where project_id = (select id from projects where key = 'OLD')",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    let (st, _) = w
        .send(
            "pull_request",
            pr_event("opened", "legacy", pr("legacy", 1, "b", "a", "t")),
        )
        .await;
    assert_eq!(st, "ok");
    assert!(w.pr_row("legacy", 1).await.is_some());
    // nothing was imported: no task was made by any of it
    let tasks: i64 = sqlx::query_scalar("select count(*) from tasks")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(tasks, 1);
}

#[tokio::test]
async fn the_link_the_server_finds_is_the_restaurants_and_a_persons_stays_theirs() {
    let w = world(false).await;
    for t in ["One", "Two"] {
        w.task(t).await;
    }
    w.send(
        "pull_request",
        pr_event(
            "opened",
            "shop",
            pr("shop", 1, "feat/shop-1", "a", "unrelated"),
        ),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-2", "url": "https://github.com/acme/shop/pull/9"}),
    )
    .await;
    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "select a.name, a.kind, e.payload->>'reason', e.payload->>'why' from events e
           join principals a on a.id = e.actor_id where e.kind = 'forge.linked' order by e.seq",
    )
    .fetch_all(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str(), rows[0].2.as_deref()),
        ("restaurant", "restaurant", Some("auto_link"))
    );
    assert!(rows[0].3.as_deref().unwrap().contains("branch"), "{rows:?}");
    assert_eq!(rows[1].0, "alice", "a link by hand is the person's");
    // the other forge events have no actor: GitHub did them
    let github: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'forge.pr' and actor_id is null",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(github >= 1);
}

#[tokio::test]
async fn a_pr_finds_its_task_by_branch_title_or_by_hand() {
    let w = world(false).await;
    for t in ["One", "Two", "Three", "Four"] {
        w.task(t).await;
    }
    // by branch (any case)
    w.send(
        "pull_request",
        pr_event(
            "opened",
            "shop",
            pr("shop", 1, "feat/Shop-1", "a", "unrelated"),
        ),
    )
    .await;
    assert_eq!(
        w.pr_row("shop", 1).await.unwrap().6.as_deref(),
        Some("SHOP-1")
    );
    // by title
    w.send(
        "pull_request",
        pr_event(
            "opened",
            "shop",
            pr("shop", 2, "plain", "b", "Fix the cart (shop-2)"),
        ),
    )
    .await;
    assert_eq!(
        w.pr_row("shop", 2).await.unwrap().6.as_deref(),
        Some("SHOP-2")
    );
    // by body
    let mut p = pr("shop", 3, "plain", "c", "nothing");
    p["body"] = json!("closes SHOP-3.");
    w.send("pull_request", pr_event("opened", "shop", p)).await;
    assert_eq!(
        w.pr_row("shop", 3).await.unwrap().6.as_deref(),
        Some("SHOP-3")
    );
    // a ref to a task that does not exist links nothing and creates nothing
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 4, "shop-99", "d", "t")),
    )
    .await;
    assert_eq!(w.pr_row("shop", 4).await.unwrap().6, None);
    // a word that only contains the key is not a ref
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 5, "workshop-1", "e", "t")),
    )
    .await;
    assert_eq!(w.pr_row("shop", 5).await.unwrap().6, None);

    // by hand: wins over the branch, and shows in get_task
    let t = tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-4", "url": "https://github.com/acme/shop/pull/1"}),
    )
    .await;
    assert_eq!(t["prs"][0]["number"], 1, "{t}");
    assert_eq!(t["prs"][0]["explicit"], true);
    assert_eq!(
        w.pr_row("shop", 1).await.unwrap().6.as_deref(),
        Some("SHOP-4")
    );
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "feat/Shop-1", "a2", "unrelated"),
        ),
    )
    .await;
    assert_eq!(
        w.pr_row("shop", 1).await.unwrap().6.as_deref(),
        Some("SHOP-4"),
        "a person's link stays"
    );
    // a PR nobody has seen yet can be linked too
    let t = tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-4", "url": "https://github.com/Acme/shop/pull/44"}),
    )
    .await;
    assert_eq!(t["prs"].as_array().unwrap().len(), 2);
    assert_eq!(t["pr"]["count"], 2, "{t}");
    // the board JSON has the compact summary
    let list = tool(&w.app, &w.alice, "list_tasks", json!({"project": "SHOP"})).await;
    let four = list
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "SHOP-4")
        .unwrap();
    assert_eq!(
        four["pr"]["number"], 44,
        "the latest open one first: {four}"
    );
    // not a PR address, and an unknown host
    let r = w
        .app
        .mcp(
            &w.alice,
            "link_pr",
            json!({"id": "SHOP-4", "url": "https://github.com/acme/shop/issues/3"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    let r = w
        .app
        .mcp(
            &w.alice,
            "link_pr",
            json!({"id": "SHOP-4", "url": "https://example.com/a/b/pull/3"}),
        )
        .await;
    assert_eq!(r["isError"], true);
    // unlink: automatic linking leaves it alone from then on
    let t = tool(
        &w.app,
        &w.alice,
        "unlink_pr",
        json!({"id": "SHOP-4", "url": "https://github.com/acme/shop/pull/1"}),
    )
    .await;
    assert_eq!(t["prs"].as_array().unwrap().len(), 1);
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "feat/Shop-1", "a3", "unrelated"),
        ),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().6, None);
    // a person who cannot see the task cannot link to it
    w.app.add_user("mallory").await;
    let m = admin::operator_token(&w.app.state.db, "mallory", None)
        .await
        .unwrap();
    let r = w
        .app
        .mcp(
            &m,
            "link_pr",
            json!({"id": "SHOP-1", "url": "https://github.com/acme/shop/pull/2"}),
        )
        .await;
    assert_eq!(r["isError"], true);
}

#[tokio::test]
async fn events_update_the_pr_and_the_task_feed() {
    let w = world(false).await;
    w.task("Cart").await;
    let cook = w.cook("cookie", "local").await;
    w.hold(&cook, "SHOP-1").await;
    w.send(
        "pull_request",
        pr_event(
            "opened",
            "shop",
            pr("shop", 1, "shop-1-cart", "aaa", "Cart"),
        ),
    )
    .await;
    let r = w.pr_row("shop", 1).await.unwrap();
    assert_eq!(
        (r.0.as_str(), r.1.as_str(), r.2.as_str(), r.4.as_str()),
        ("open", "none", "none", "unknown")
    );
    // a run fails (names it), the suite finishes
    w.send("check_run", check_run("shop", 1, "aaa", "build", "failure"))
        .await;
    w.send("check_suite", suite("shop", 1, "aaa", "failure"))
        .await;
    let r = w.pr_row("shop", 1).await.unwrap();
    assert_eq!(
        (r.1.as_str(), r.3.clone()),
        ("failure", vec!["build".to_string()])
    );
    // a check of an older commit says nothing
    let (st, _) = w
        .send("check_suite", suite("shop", 1, "old", "success"))
        .await;
    assert_eq!(st, "ignored");
    assert_eq!(w.pr_row("shop", 1).await.unwrap().1, "failure");
    // a new head starts the checks over
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "shop-1-cart", "bbb", "Cart"),
        ),
    )
    .await;
    let r = w.pr_row("shop", 1).await.unwrap();
    assert_eq!(
        (r.1.as_str(), r.3.len(), r.5.as_str()),
        ("pending", 0, "bbb")
    );
    w.send("check_suite", suite("shop", 1, "bbb", "success"))
        .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().1, "success");
    // reviews: approved, then changes requested replaces it, a bare comment does not
    let p = pr("shop", 1, "shop-1-cart", "bbb", "Cart");
    w.send(
        "pull_request_review",
        review("shop", p.clone(), "approved", "bob", ""),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().2, "approved");
    w.send(
        "pull_request_review",
        review("shop", p.clone(), "commented", "bob", ""),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().2, "approved");
    w.send(
        "pull_request_review",
        review("shop", p, "changes_requested", "bob", "rename it"),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().2, "changes_requested");
    // a conflict
    let mut c = pr("shop", 1, "shop-1-cart", "bbb", "Cart");
    c["mergeable"] = json!(false);
    c["mergeable_state"] = json!("dirty");
    w.send("pull_request", pr_event("edited", "shop", c)).await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().4, "false");
    // the feed and get_task
    for k in [
        "forge.linked",
        "forge.pr",
        "forge.checks",
        "forge.review",
        "forge.conflict",
    ] {
        assert!(w.events(k).await >= 1, "no {k} event");
    }
    let t = tool(&w.app, &w.alice, "get_task", json!({"id": "SHOP-1"})).await;
    let p = &t["prs"][0];
    assert_eq!(
        (
            p["state"].as_str(),
            p["checks"].as_str(),
            p["review"].as_str(),
            p["mergeable"].as_str()
        ),
        (
            Some("open"),
            Some("success"),
            Some("changes_requested"),
            Some("false")
        ),
        "{t}"
    );
    assert_eq!(t["pr"]["conflict"], true);
    // closed, reopened, then merged
    w.send(
        "pull_request",
        pr_event("closed", "shop", {
            let mut p = pr("shop", 1, "shop-1-cart", "bbb", "Cart");
            p["state"] = json!("closed");
            p
        }),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().0, "closed");
    w.send(
        "pull_request",
        pr_event(
            "reopened",
            "shop",
            pr("shop", 1, "shop-1-cart", "bbb", "Cart"),
        ),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().0, "open");
    // a push to the branch moves the head
    let push = json!({"ref": "refs/heads/shop-1-cart", "after": "ccc", "repository": repo("shop")});
    let (st, _) = w.send("push", push).await;
    assert_eq!(st, "ok");
    assert_eq!(w.pr_row("shop", 1).await.unwrap().5, "ccc");
    // the app installed
    let inst = json!({"action": "created", "installation": {"id": 77, "account": {"login": "acme", "type": "Organization"}, "repository_selection": "all"}});
    w.send("installation", inst).await;
    let acct: Option<String> =
        sqlx::query_scalar("select account from forge_installations where installation_id = 77")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(acct.as_deref(), Some("acme"));
    w.send(
        "installation",
        json!({"action": "deleted", "installation": {"id": 77}}),
    )
    .await;
    let n: i64 = sqlx::query_scalar("select count(*) from forge_installations")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_failed_ci_wakes_the_holder_once_per_commit() {
    let w = world(false).await;
    w.task("Cart").await;
    let cook = w.cook("cookie", "local").await;
    w.hold(&cook, "SHOP-1").await;
    w.send(
        "pull_request",
        pr_event(
            "opened",
            "shop",
            pr("shop", 1, "shop-1-cart", "aaa", "Cart"),
        ),
    )
    .await;
    // a burst: two runs, the suite, the workflow, and the suite again
    w.send("check_run", check_run("shop", 1, "aaa", "build", "failure"))
        .await;
    w.send("check_run", check_run("shop", 1, "aaa", "lint", "failure"))
        .await;
    w.send("workflow_run", workflow("shop", 1, "aaa", "ci", "failure"))
        .await;
    w.send("check_suite", suite("shop", 1, "aaa", "failure"))
        .await;
    w.send("check_suite", suite("shop", 1, "aaa", "failure"))
        .await;
    let wakes = w.wakes("cookie", "forge").await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0].contains("CI failed on Acme/shop#1"),
        "{}",
        wakes[0]
    );
    assert!(
        wakes[0].contains("build, lint, ci") && wakes[0].contains("/actions/runs/9"),
        "{}",
        wakes[0]
    );
    assert_eq!(
        w.events("forge.checks").await,
        1,
        "once per (commit, conclusion): the workflow and the suite share it"
    );
    // success changes nothing here; a new commit failing is a new wake
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "shop-1-cart", "bbb", "Cart"),
        ),
    )
    .await;
    w.send("check_suite", suite("shop", 1, "bbb", "success"))
        .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 1);
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "shop-1-cart", "ccc", "Cart"),
        ),
    )
    .await;
    w.send("check_suite", suite("shop", 1, "ccc", "timed_out"))
        .await;
    w.send("check_suite", suite("shop", 1, "ccc", "failure"))
        .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 2);
    // cancelled is not a failure
    w.send(
        "pull_request",
        pr_event(
            "synchronize",
            "shop",
            pr("shop", 1, "shop-1-cart", "ddd", "Cart"),
        ),
    )
    .await;
    w.send("check_suite", suite("shop", 1, "ddd", "cancelled"))
        .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 2);
    // the status never moved
    assert_eq!(w.status("SHOP-1").await, "claimed");
}

#[tokio::test]
async fn reviews_and_comments_wake_the_holder_but_not_for_its_own_words() {
    let w = world(false).await;
    w.task("Cart").await;
    w.task("Unheld").await;
    let cook = w.cook("cookie", "local").await;
    w.hold(&cook, "SHOP-1").await;
    let p = pr("shop", 1, "shop-1-cart", "aaa", "Cart");
    w.send("pull_request", pr_event("opened", "shop", p.clone()))
        .await;
    // approved: shown, nobody woken
    w.send(
        "pull_request_review",
        review("shop", p.clone(), "approved", "bob", "lgtm"),
    )
    .await;
    assert!(w.wakes("cookie", "forge").await.is_empty());
    // changes requested
    w.send(
        "pull_request_review",
        review(
            "shop",
            p.clone(),
            "changes_requested",
            "bob",
            "please rename the helper",
        ),
    )
    .await;
    let wakes = w.wakes("cookie", "forge").await;
    assert_eq!(wakes.len(), 1);
    assert!(
        wakes[0].contains("bob")
            && wakes[0].contains("please rename the helper")
            && wakes[0].contains("SHOP-1"),
        "{}",
        wakes[0]
    );
    // an inline comment, with path and line
    let inline = json!({"action": "created", "repository": repo("shop"), "pull_request": p,
        "comment": {"body": "nit: spacing", "path": "src/cart.rs", "line": 12, "user": {"login": "bob"}, "html_url": "https://github.com/Acme/shop/pull/1#c9"}});
    w.send("pull_request_review_comment", inline).await;
    let wakes = w.wakes("cookie", "forge").await;
    assert_eq!(wakes.len(), 2);
    assert!(
        wakes[1].contains("src/cart.rs:12")
            && wakes[1].contains("nit: spacing")
            && wakes[1].contains("#c9"),
        "{}",
        wakes[1]
    );
    // a conversation comment by a person, then by the cook's own account (and its bot form)
    w.send("issue_comment", comment("shop", 1, "carol", "any news?"))
        .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 3);
    w.send(
        "issue_comment",
        comment("shop", 1, "cookie", "pushed a fix"),
    )
    .await;
    w.send(
        "issue_comment",
        comment("shop", 1, "Cookie[bot]", "pushed a fix"),
    )
    .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 3);
    assert_eq!(w.events("forge.comment").await, 4, "all of them are shown");
    // a conflict, once per commit
    let mut c = p.clone();
    c["mergeable"] = json!(false);
    for _ in 0..2 {
        w.send("pull_request", pr_event("edited", "shop", c.clone()))
            .await;
    }
    assert_eq!(w.wakes("cookie", "forge").await.len(), 4);
    // a PR whose task nobody holds: shown, no one to wake
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 2, "shop-2-x", "z", "Unheld")),
    )
    .await;
    w.send("issue_comment", comment("shop", 2, "carol", "hello"))
        .await;
    assert_eq!(w.wakes("cookie", "forge").await.len(), 4);
}

#[tokio::test]
async fn a_merge_notifies_the_holder_and_leaves_a_held_task_alone() {
    let w = world(false).await;
    w.task("Cart").await;
    w.task("Served").await;
    let cook = w.cook("cookie", "local").await;
    w.hold(&cook, "SHOP-1").await;
    // SHOP-2 was served by the cook: it still is the one to tell
    let c = tool(&w.app, &cook, "claim_task", json!({"id": "SHOP-2"})).await;
    tool(
        &w.app,
        &cook,
        "complete",
        json!({"id": "SHOP-2", "epoch": c["lease"]["epoch"], "result": "done"}),
    )
    .await;
    for (n, task, branch) in [(1, "SHOP-1", "shop-1-cart"), (2, "SHOP-2", "shop-2-served")] {
        let mut p = pr("shop", n, branch, "aaa", "t");
        w.send("pull_request", pr_event("opened", "shop", p.clone()))
            .await;
        p["state"] = json!("closed");
        p["merged"] = json!(true);
        w.send("pull_request", pr_event("closed", "shop", p.clone()))
            .await;
        assert_eq!(w.pr_row("shop", n).await.unwrap().0, "merged");
        let _ = task;
        // the same event again (a redelivery with another id) tells nobody twice
        w.send("pull_request", pr_event("closed", "shop", p)).await;
    }
    // SHOP-2 was already done: its merge says nothing to anyone
    let wakes = w.wakes("cookie", "forge-merged").await;
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    assert!(
        wakes[0].contains("Acme/shop#1 merged")
            && wakes[0].contains("SHOP-1's status is unchanged"),
        "{}",
        wakes[0]
    );
    assert_eq!(
        w.status("SHOP-1").await,
        "claimed",
        "a merge does not serve a held task"
    );
    assert_eq!(w.status("SHOP-2").await, "done");
    assert_eq!(w.events("forge.merged").await, 2);
    // a closed (not merged) PR changes nothing either
    w.task("Third").await;
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 3, "shop-3-x", "q", "t")),
    )
    .await;
    w.send(
        "pull_request",
        pr_event("closed", "shop", {
            let mut p = pr("shop", 3, "shop-3-x", "q", "t");
            p["state"] = json!("closed");
            p
        }),
    )
    .await;
    assert_eq!(w.status("SHOP-3").await, "open");
    assert_eq!(w.wakes("cookie", "forge-merged").await.len(), 1);
}

#[tokio::test]
async fn a_merge_reaches_the_repositories_that_depend_on_it() {
    let w = world(false).await;
    tool(
        &w.app,
        &w.alice,
        "add_repo",
        json!({"project": "SHOP", "url": "https://github.com/acme/web.git"}),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "SHOP", "repo": "web", "on": "shop"}),
    )
    .await;
    w.task("Cart").await;
    let holder = w.cook("holder", "local").await;
    let web = w.cook("webcook", "local").await;
    let other = w.cook("bystander", "local").await;
    w.hold(&holder, "SHOP-1").await;
    tool(
        &w.app,
        &web,
        "set_cook_repos",
        json!({"project": "SHOP", "repos": "web"}),
    )
    .await;
    tool(
        &w.app,
        &other,
        "set_cook_repos",
        json!({"project": "SHOP", "repos": "shop"}),
    )
    .await;
    let mut p = pr("shop", 1, "shop-1-cart", "aaa", "Cart endpoint");
    w.send("pull_request", pr_event("opened", "shop", p.clone()))
        .await;
    p["state"] = json!("closed");
    p["merged"] = json!(true);
    w.send("pull_request", pr_event("closed", "shop", p.clone()))
        .await;
    let up = w.wakes("webcook", "upstream").await;
    assert_eq!(up.len(), 1, "{up:?}");
    assert!(
        up[0].contains("Acme/shop#1")
            && up[0].contains("was merged")
            && up[0].contains("`web`")
            && up[0].contains("/pull/1"),
        "{}",
        up[0]
    );
    assert!(
        w.wakes("bystander", "upstream").await.is_empty(),
        "not scoped to a dependent repository"
    );
    assert_eq!(w.wakes("holder", "forge-merged").await.len(), 1);
    assert_eq!(w.events("task.propagated").await, 1);
    // mode task: the follow-up is made (as it is for a served task), once
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "SHOP", "repo": "web", "on": "shop", "mode": "task"}),
    )
    .await;
    let before: i64 = sqlx::query_scalar("select count(*) from tasks")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    w.task("Second").await;
    let mut p2 = pr("shop", 2, "shop-2-x", "bbb", "Second");
    w.send("pull_request", pr_event("opened", "shop", p2.clone()))
        .await;
    p2["state"] = json!("closed");
    p2["merged"] = json!(true);
    w.send("pull_request", pr_event("closed", "shop", p2.clone()))
        .await;
    w.send("pull_request", pr_event("closed", "shop", p2)).await;
    let titles: Vec<String> =
        sqlx::query_scalar("select title from tasks where from_upstream order by number")
            .fetch_all(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(titles.len(), 1, "{titles:?}");
    assert!(
        titles[0].starts_with("[web] Follow up on Acme/shop#2"),
        "{}",
        titles[0]
    );
    let after: i64 = sqlx::query_scalar("select count(*) from tasks")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(after, before + 2, "SHOP-2 and its follow-up only");
}

#[tokio::test]
async fn a_merge_that_completes_a_task_propagates_its_repos_once() {
    let w = world(false).await;
    tool(
        &w.app,
        &w.alice,
        "add_repo",
        json!({"project": "SHOP", "url": "https://github.com/acme/web.git"}),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "SHOP", "repo": "web", "on": "shop", "mode": "task"}),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "Cart", "repos": "shop"}),
    )
    .await;
    w.send("pull_request", merged(1, "shop-1-cart", "Cart", ""))
        .await;
    assert_eq!(w.status("SHOP-1").await, "done");
    let titles: Vec<String> =
        sqlx::query_scalar("select title from tasks where from_upstream order by number")
            .fetch_all(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(
        titles.len(),
        1,
        "one follow-up for the task's repos: {titles:?}"
    );
    assert!(
        titles[0].starts_with("[web] Follow up on SHOP-1"),
        "{}",
        titles[0]
    );
    assert_eq!(w.events("task.propagated").await, 1);
}

// ---------- cloud ----------

type Seen = Arc<Mutex<Vec<String>>>;

/// A tiny endpoint that records the path of every request and answers 200.
async fn fake_hooks() -> (String, Seen) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen: Seen = Default::default();
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = l.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let mut got = 0;
                loop {
                    let n = sock.read(&mut buf[got..]).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    got += n;
                    let t = String::from_utf8_lossy(&buf[..got]).to_string();
                    if let Some(i) = t.find("\r\n\r\n") {
                        let len = t[..i]
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if got >= i + 4 + len {
                            log.lock().unwrap().push(
                                t.lines()
                                    .next()
                                    .unwrap_or("")
                                    .split(' ')
                                    .nth(1)
                                    .unwrap_or("")
                                    .to_string(),
                            );
                            break;
                        }
                    }
                }
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                    )
                    .await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn hits(seen: &Seen, path: &str) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|p| p.as_str() == path)
        .count()
}

async fn until(seen: &Seen, path: &str, n: usize) {
    for _ in 0..80 {
        if hits(seen, path) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "{path} was not called {n} times: {:?}",
        seen.lock().unwrap()
    );
}

#[tokio::test]
async fn forge_events_never_start_a_cloud_cook() {
    let (base, seen) = fake_hooks().await;
    let w = world(true).await;
    let a = w.cook("alpha", "cloud").await;
    let b = w.cook("beta", "cloud").await;
    for name in ["alpha", "beta"] {
        let (code, r) = w
            .app
            .post(
                &w.root,
                &format!("/agents/{name}/wake-targets"),
                json!({"type": "webhook", "config": {"url": format!("{base}/hook/{name}")}}),
            )
            .await;
        assert_eq!(code, 200, "{r}");
    }
    // ready work starts them (the only thing that does); then they take a task each
    w.task("One").await;
    w.task("Two").await;
    until(&seen, "/hook/alpha", 1).await;
    until(&seen, "/hook/beta", 1).await;
    w.hold(&a, "SHOP-1").await;
    w.hold(&b, "SHOP-2").await;
    seen.lock().unwrap().clear();
    for (n, branch) in [(1, "shop-1-x"), (2, "shop-2-x")] {
        w.send(
            "pull_request",
            pr_event("opened", "shop", pr("shop", n, branch, "aaa", "t")),
        )
        .await;
        w.send("check_suite", suite("shop", n, "aaa", "failure"))
            .await;
    }
    let p = pr("shop", 1, "shop-1-x", "aaa", "t");
    w.send(
        "pull_request_review",
        review("shop", p, "changes_requested", "bob", "no"),
    )
    .await;
    w.send("issue_comment", comment("shop", 1, "carol", "hello"))
        .await;
    for n in [1, 2] {
        let mut p = pr("shop", n, "x", "aaa", "t");
        p["state"] = json!("closed");
        p["merged"] = json!(true);
        w.send("pull_request", pr_event("closed", "shop", p)).await;
    }
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "forge events start no cloud run: {:?}",
        seen.lock().unwrap()
    );
    let n: i64 = sqlx::query_scalar(
        "select count(*) from events where kind = 'wake' and payload->>'agent' in ('alpha', 'beta')",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(n, 0, "nothing was even emitted for them");
    assert_eq!(w.status("SHOP-1").await, "claimed", "held: unchanged");
}

#[tokio::test]
async fn a_merge_completes_a_task_nobody_holds() {
    let w = world(false).await;
    w.task("Cart").await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "After cart", "dependsOn": "SHOP-1"}),
    )
    .await;
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 1, "shop-1-cart", "aaa", "t")),
    )
    .await;
    // a second open PR for the same task keeps it open when the first merges
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 2, "shop-1-more", "bbb", "t")),
    )
    .await;
    let merge = |n: i64, branch: &'static str| {
        let mut p = pr("shop", n, branch, "x", "t");
        p["state"] = json!("closed");
        p["merged"] = json!(true);
        pr_event("closed", "shop", p)
    };
    w.send("pull_request", merge(1, "shop-1-cart")).await;
    assert_eq!(w.status("SHOP-1").await, "open", "another PR is still open");
    assert_eq!(w.events("task.completed").await, 0);
    w.send("pull_request", merge(2, "shop-1-more")).await;
    assert_eq!(w.status("SHOP-1").await, "done");
    let t = tool(&w.app, &w.alice, "get_task", json!({"id": "SHOP-1"})).await;
    assert!(
        t["result"]
            .as_str()
            .unwrap()
            .starts_with("Merged https://github.com/Acme/shop/pull/2"),
        "{t}"
    );
    let (n, actors): (i64, i64) = sqlx::query_as(
        "select count(*), count(*) filter (where actor_id = '00000000-0000-4000-8000-524553545552') from events where kind = 'task.completed'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!((n, actors), (1, 1), "one completion, by the restaurant");
    let (reason, why): (String, String) = sqlx::query_as(
        "select payload->>'reason', payload->>'why' from events where kind = 'task.completed'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(reason, "merged");
    assert!(why.contains("merged"), "{why}");
    // the first merge left the task open, and the restaurant said why
    w.decided("merge.kept_open", "open_pr", "SHOP-1").await;
    // what depends on it is ready now
    let next = tool(&w.app, &w.alice, "get_task", json!({"id": "SHOP-2"})).await;
    assert_eq!(next["ready"], true, "{next}");
    // a redelivery completes nothing twice
    w.send("pull_request", merge(2, "shop-1-more")).await;
    assert_eq!(w.events("task.completed").await, 1);
}

fn merged(n: i64, branch: &str, title: &str, body: &str) -> Value {
    let mut p = pr("shop", n, branch, "x", title);
    p["body"] = json!(body);
    p["state"] = json!("closed");
    p["merged"] = json!(true);
    pr_event("closed", "shop", p)
}

#[tokio::test]
async fn only_an_explicit_branch_or_title_link_completes_on_merge() {
    let w = world(false).await;
    for t in ["Body", "Title", "Branch", "Hand"] {
        w.task(t).await;
    }
    // a body that merely mentions the task links it but never completes it
    w.send(
        "pull_request",
        merged(1, "plain", "nothing", "see SHOP-1 for context"),
    )
    .await;
    assert_eq!(
        w.pr_row("shop", 1).await.unwrap().6.as_deref(),
        Some("SHOP-1")
    );
    assert_eq!(
        w.status("SHOP-1").await,
        "open",
        "a body mention is not a claim"
    );
    assert_eq!(w.events("task.completed").await, 0);
    // the title, the branch and a person's link do
    w.send("pull_request", merged(2, "plain", "Fix (SHOP-2)", ""))
        .await;
    assert_eq!(w.status("SHOP-2").await, "done");
    w.send("pull_request", merged(3, "shop-3-x", "t", "")).await;
    assert_eq!(w.status("SHOP-3").await, "done");
    tool(
        &w.app,
        &w.alice,
        "link_pr",
        json!({"id": "SHOP-4", "url": "https://github.com/acme/shop/pull/4"}),
    )
    .await;
    w.send("pull_request", merged(4, "plain", "nothing", ""))
        .await;
    assert_eq!(w.status("SHOP-4").await, "done");
    assert_eq!(w.events("task.completed").await, 3);
}

#[tokio::test]
async fn a_merge_while_held_completes_the_task_on_release_or_lease_expiry() {
    let w = world(false).await;
    w.task("Released").await;
    w.task("Reaped").await;
    w.task("Reaped too").await;
    w.task("Body only").await;
    let cook = w.cook("cookie", "local").await;
    let mut epochs = vec![];
    for t in ["SHOP-1", "SHOP-2", "SHOP-3", "SHOP-4"] {
        let c = tool(&w.app, &cook, "claim_task", json!({"id": t})).await;
        epochs.push(c["lease"]["epoch"].clone());
    }
    for (n, t) in [(1, "SHOP-1"), (2, "SHOP-2"), (3, "SHOP-3")] {
        tool(
            &w.app,
            &w.alice,
            "link_pr",
            json!({"id": t, "url": format!("https://github.com/acme/shop/pull/{n}")}),
        )
        .await;
        w.send("pull_request", merged(n, "plain", "nothing", ""))
            .await;
        assert_eq!(w.status(t).await, "claimed", "a held task stays held");
    }
    // a body-only link does not complete on release (SHOP-4: merged PR, no qualifying link)
    w.send(
        "pull_request",
        merged(5, "plain", "nothing", "about SHOP-4"),
    )
    .await;
    assert_eq!(w.status("SHOP-4").await, "claimed");
    // release: done
    tool(
        &w.app,
        &cook,
        "release",
        json!({"id": "SHOP-1", "epoch": epochs[0], "reason": "PR open"}),
    )
    .await;
    assert_eq!(w.status("SHOP-1").await, "done");
    // lease expiry: done
    sqlx::query(
        "update tasks set lease_expires_at = now() - interval '1 second' where status = 'claimed'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(corntchap::jobs::reap(&w.app.state).await.unwrap(), 3);
    assert_eq!(w.status("SHOP-2").await, "done");
    assert_eq!(w.status("SHOP-3").await, "done");
    assert_eq!(
        w.status("SHOP-4").await,
        "open",
        "a body mention never completes"
    );
    let (n, actors): (i64, i64) = sqlx::query_as(
        "select count(*), count(*) filter (where actor_id = '00000000-0000-4000-8000-524553545552') from events where kind = 'task.completed'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!((n, actors), (3, 3));
}

#[tokio::test]
async fn a_merge_waits_for_the_tasks_dependencies() {
    let w = world(false).await;
    w.task("Base").await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "Needs base", "dependsOn": "SHOP-1"}),
    )
    .await;
    // the dependent is open but not ready: its own PR merging must not complete it
    w.send("pull_request", merged(2, "shop-2-x", "t", "")).await;
    assert_eq!(w.status("SHOP-2").await, "open", "a dependency is not done");
    w.decided("merge.kept_open", "dependencies", "SHOP-2").await;
    w.send("pull_request", merged(1, "shop-1-x", "t", "")).await;
    assert_eq!(w.status("SHOP-1").await, "done");
}

#[tokio::test]
async fn a_merge_does_not_complete_a_task_with_open_subtasks() {
    let w = world(false).await;
    w.task("Parent").await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "SHOP", "title": "Child", "parent": "SHOP-1"}),
    )
    .await;
    w.send(
        "pull_request",
        pr_event("opened", "shop", pr("shop", 1, "shop-1-p", "aaa", "t")),
    )
    .await;
    let mut p = pr("shop", 1, "shop-1-p", "aaa", "t");
    p["state"] = json!("closed");
    p["merged"] = json!(true);
    w.send("pull_request", pr_event("closed", "shop", p)).await;
    assert_eq!(w.status("SHOP-1").await, "open");
    w.decided("merge.kept_open", "subtasks", "SHOP-1").await;
}

// ---------- the manifest flow against a fake GitHub ----------

#[tokio::test]
async fn connecting_github_through_the_manifest_flow() {
    use axum::{Json, Router, extract::Path, routing::post};
    let calls: Arc<Mutex<Vec<String>>> = Default::default();
    let log = calls.clone();
    let fake = Router::new().route(
        "/app-manifests/{code}/conversions",
        post(move |Path(code): Path<String>| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(code.clone());
                if code == "bad" {
                    return (axum::http::StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"})));
                }
                (
                    axum::http::StatusCode::CREATED,
                    Json(json!({"id": 4242, "slug": "corntchap-test", "name": "corntchap test", "html_url": "https://github.com/apps/corntchap-test",
                            "client_id": "Iv1.client", "client_secret": "CLIENTSECRET", "pem": "-----BEGIN RSA PRIVATE KEY-----\nPEMSECRET\n-----END RSA PRIVATE KEY-----",
                            "webhook_secret": "whsec_from_fake", "owner": {"login": "acme"}})),
                )
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, fake).await.unwrap() });

    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    let enc = |s: &str| s.replace(':', "%3A").replace('/', "%2F");

    // a person who is not an admin is refused
    app.sign_in_as("sub-bob", "bob", false);
    let bob = app.browser_login("/").await;
    let r = app
        .http
        .get(format!("{}/forges/github/connect", app.base))
        .header("cookie", &bob)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);
    // signed out: sent to sign in
    let r = app
        .http
        .get(format!("{}/forges/github/connect", app.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 303);

    // an admin gets a page that posts the manifest to GitHub, with a state
    app.sign_in_as("sub-gha", "gha", true);
    let cookie = app.browser_login("/").await;
    let url = format!(
        "{}/forges/github/connect?api_url={}&org=acme",
        app.base,
        enc(&api)
    );
    let r = app
        .http
        .get(&url)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let html = r.text().await.unwrap();
    assert!(
        html.contains("https://github.com/organizations/acme/settings/apps/new?state="),
        "{html}"
    );
    let state = html
        .split("apps/new?state=")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let manifest = html
        .split("name=manifest value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .replace("&quot;", "\"")
        .replace("&amp;", "&");
    let m: Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(
        m["hook_attributes"]["url"],
        format!("{}/webhooks/github", app.base)
    );
    assert_eq!(
        m["redirect_url"],
        format!("{}/forges/github/callback", app.base)
    );
    for p in [
        "pull_requests",
        "checks",
        "statuses",
        "contents",
        "metadata",
        "issues",
    ] {
        assert_eq!(m["default_permissions"][p], "read", "{p}");
    }
    for e in [
        "pull_request",
        "pull_request_review",
        "pull_request_review_comment",
        "issue_comment",
        "check_suite",
        "check_run",
        "workflow_run",
        "push",
    ] {
        assert!(
            m["default_events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x == e),
            "{e}"
        );
    }
    // without an org the personal address is used
    let html2 = app
        .http
        .get(format!(
            "{}/forges/github/connect?api_url={}",
            app.base,
            enc(&api)
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html2.contains("https://github.com/settings/apps/new?state="));

    // the callback: a wrong state is refused and GitHub is not asked
    let r = app
        .http
        .get(format!(
            "{}/forges/github/callback?code=c0de&state=wrong",
            app.base
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(calls.lock().unwrap().is_empty());
    // the real one trades the code for the app
    let r = app
        .http
        .get(format!(
            "{}/forges/github/callback?code=c0de&state={state}",
            app.base
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 303, "{}", r.text().await.unwrap());
    assert_eq!(calls.lock().unwrap().as_slice(), ["c0de"]);
    let (secret, app_id, slug, api_url, key): (String, i64, String, String, String) =
        sqlx::query_as(
            "select webhook_secret, app_id, slug, api_url, private_key from forge_connections",
        )
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        (secret.as_str(), app_id, slug.as_str(), api_url.as_str()),
        ("whsec_from_fake", 4242, "corntchap-test", api.as_str())
    );
    assert!(key.contains("PEMSECRET"));
    // the state works once
    let r = app
        .http
        .get(format!(
            "{}/forges/github/callback?code=c0de&state={state}",
            app.base
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // GitHub refusing the code is shown, nothing stored
    let st2 = {
        let h = app
            .http
            .get(&url)
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        h.split("apps/new?state=")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let r = app
        .http
        .get(format!(
            "{}/forges/github/callback?code=bad&state={st2}",
            app.base
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 502);
    let n: i64 = sqlx::query_scalar("select count(*) from forge_connections")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // the new secret checks deliveries; an installation event fills the list
    let body = json!({"action": "created", "installation": {"id": 9, "account": {"login": "acme", "type": "Organization"}, "repository_selection": "selected"}});
    let raw = serde_json::to_vec(&body).unwrap();
    let r = app
        .http
        .post(format!("{}/webhooks/github", app.base))
        .header("x-github-event", "installation")
        .header("x-github-delivery", "g1")
        .header("x-hub-signature-256", signature("whsec_from_fake", &raw))
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 202);
    for _ in 0..50 {
        let n: i64 = sqlx::query_scalar("select count(*) from forge_installations")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
        if n == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // the admin API lists it without any secret, and only to admins
    let (code, list) = app.get(&root, "/forges").await;
    assert_eq!(code, 200);
    let text = list.to_string();
    for secret in ["whsec_from_fake", "CLIENTSECRET", "PEMSECRET"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    let c = &list["connections"][0];
    assert_eq!(
        c["install_url"],
        format!(
            "{}/apps/corntchap-test/installations/new",
            "https://github.com"
        )
    );
    assert_eq!(c["installations"][0]["account"], "acme");
    assert_eq!(c["last_delivery"]["event"], "installation");
    app.add_user("chef").await;
    let chef = admin::operator_token(&app.state.db, "chef", None)
        .await
        .unwrap();
    let (code, _) = app.get(&chef, "/forges").await;
    assert_eq!(code, 403);
    let id = c["id"].as_str().unwrap().to_string();
    let (code, _) = app
        .rest(
            &chef,
            reqwest::Method::DELETE,
            &format!("/forges/{id}"),
            None,
        )
        .await;
    assert_eq!(code, 403);
    let (code, _) = app
        .rest(
            &root,
            reqwest::Method::DELETE,
            &format!("/forges/{id}"),
            None,
        )
        .await;
    assert_eq!(code, 200);
    let n: i64 = sqlx::query_scalar("select count(*) from forge_connections")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_push_answers_changes_requested_and_parks_the_task_again() {
    let w = world(false).await;
    w.task("Cart").await;
    let p = pr("shop", 1, "shop-1-cart", "aaa", "Cart");
    w.send("pull_request", pr_event("opened", "shop", p.clone()))
        .await;
    let takeable = |w: &World| {
        let db = w.app.state.db.clone();
        async move {
            sqlx::query_scalar::<_, bool>(sqlx::AssertSqlSafe(format!(
                "select {} from tasks t where t.number = 1",
                corntchap::service::NO_OPEN_PR_SQL
            )))
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    // a healthy open PR: not free work
    assert!(!takeable(&w).await);
    // changes requested: the PR is parked, the task is work
    w.send(
        "pull_request_review",
        review("shop", p.clone(), "changes_requested", "bob", "rename it"),
    )
    .await;
    assert_eq!(w.pr_row("shop", 1).await.unwrap().2, "changes_requested");
    assert!(takeable(&w).await);
    // the author pushes: that review was about the old head, the task is parked again
    let push = json!({"ref": "refs/heads/shop-1-cart", "after": "ccc", "repository": repo("shop")});
    w.send("push", push).await;
    let row = w.pr_row("shop", 1).await.unwrap();
    assert_eq!((row.5.as_str(), row.2.as_str()), ("ccc", "none"));
    assert!(!takeable(&w).await);
}
