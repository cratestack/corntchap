//! Pots with several repositories: the migration of `repo_url`, who manages them, tasks naming
//! them, cook scopes and claim routing, wake candidates, split_by_repo, and links between
//! repositories (an upstream change reaches the dependents' cooks, one hop, never a loop).
mod common;
use std::time::Duration;

use common::TestApp;
use corntchap::admin;
use serde_json::{Value, json};

async fn tool(app: &TestApp, tok: &str, name: &str, args: Value) -> Value {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_ne!(r["isError"], true, "{name}({args}): {r}");
    TestApp::payload(&r)
}
async fn refused(app: &TestApp, tok: &str, name: &str, args: Value) -> String {
    let r = app.mcp(tok, name, args.clone()).await;
    assert_eq!(r["isError"], true, "{name}({args}) should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

struct World {
    app: TestApp,
    root: String,
    alice: String,
    bob: String,
}

/// octo (admin), alice (owner of team acme and its pot PAY), bob (a chef of acme).
async fn world(background: bool) -> World {
    let app = common::start(background).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
    app.add_user("bob").await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
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
        &root,
        "add_team_member",
        json!({"team": "acme", "principal": "bob", "role": "chef"}),
    )
    .await;
    tool(
        &app,
        &alice,
        "create_project",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    World {
        app,
        root,
        alice,
        bob,
    }
}

impl World {
    async fn cook(&self, name: &str) -> String {
        let (_, ag) = self
            .app
            .post(
                &self.root,
                "/agents",
                json!({"name": name, "kind": "other", "runtime": "local", "max_leases": 10}),
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

    async fn repos(&self, names: &[&str]) {
        for n in names {
            tool(
                &self.app,
                &self.alice,
                "add_repo",
                json!({"project": "PAY", "url": format!("https://github.com/acme/{n}.git")}),
            )
            .await;
        }
    }

    async fn task(&self, title: &str, repos: &str, priority: i64) -> Value {
        tool(
            &self.app,
            &self.alice,
            "create_task",
            json!({"project": "PAY", "title": title, "repos": repos, "priority": priority}),
        )
        .await
    }

    /// claim_next as a cook: the task id, or None.
    async fn next(&self, tok: &str) -> Option<String> {
        let r = tool(&self.app, tok, "claim_next", json!({"project": "PAY"})).await;
        r["task"]["id"].as_str().map(String::from)
    }

    /// Serve a task: claim it by id, then complete it.
    async fn serve(&self, tok: &str, id: &str, extra: Value) -> Value {
        let c = tool(&self.app, tok, "claim_task", json!({"id": id})).await;
        let epoch = c["lease"]["epoch"].as_i64().unwrap();
        let mut args = json!({"id": id, "epoch": epoch, "result": "added the endpoint"});
        for (k, v) in extra.as_object().into_iter().flatten() {
            args[k] = v.clone();
        }
        tool(&self.app, tok, "complete", args).await
    }

    async fn upstream_wakes(&self, agent: &str) -> i64 {
        sqlx::query_scalar(
            "select count(*) from events where kind = 'wake' and payload->>'reason' = 'upstream' and payload->>'agent' = $1",
        )
        .bind(agent)
        .fetch_one(&self.app.state.db)
        .await
        .unwrap()
    }

    async fn task_count(&self) -> i64 {
        sqlx::query_scalar("select count(*) from tasks")
            .fetch_one(&self.app.state.db)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn existing_repo_urls_become_the_first_repository() {
    let w = world(false).await;
    let db = &w.app.state.db;
    sqlx::query("delete from project_repos")
        .execute(db)
        .await
        .unwrap();
    sqlx::query(
        "update projects set repo_url = 'git@GitHub.com:Acme/Backend.git' where key = 'PAY'",
    )
    .execute(db)
    .await
    .unwrap();
    sqlx::query(
        "insert into projects (team_id, key, name, repo_url) select team_id, 'WEB', 'Web', 'https://user:tok@github.com:443/acme/web-app/' from projects where key = 'PAY'",
    )
    .execute(db)
    .await
    .unwrap();
    sqlx::query(
        "insert into projects (team_id, key, name) select team_id, 'NOURL', 'None' from projects where key = 'PAY'",
    )
    .execute(db)
    .await
    .unwrap();
    // run the migration's own backfill statement
    let sql = include_str!("../migrations/0024_pot_repos.sql");
    let from = sql.find("with n as (").unwrap();
    let to = from + sql[from..].find(";\n").unwrap();
    sqlx::query(&sql[from..to]).execute(db).await.unwrap();

    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "select pr.key, r.name, r.norm_url, r.url from project_repos r join projects pr on pr.id = r.project_id order by pr.key",
    )
    .fetch_all(db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "PAY".into(),
                "backend".into(),
                "github.com/acme/backend".into(),
                "git@GitHub.com:Acme/Backend.git".into()
            ),
            (
                "WEB".into(),
                "web-app".into(),
                "github.com/acme/web-app".into(),
                "https://user:tok@github.com:443/acme/web-app/".into()
            ),
        ]
    );
    // the Rust normalizer agrees with the SQL one
    for (_, _, norm, url) in &rows {
        assert_eq!(&corntchap::repos::normalize_git_url(url), norm);
    }
}

#[tokio::test]
async fn repositories_are_managed_by_owners_and_read_by_members() {
    let w = world(false).await;
    let app = &w.app;
    // a pot created with a repo_url starts with it as its first repository
    tool(
        app,
        &w.alice,
        "create_project",
        json!({"team": "acme", "key": "OPS", "name": "Ops", "repoUrl": "https://github.com/acme/infra.git"}),
    )
    .await;
    let pots = tool(app, &w.bob, "list_projects", json!({})).await;
    let ops = pots
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "OPS")
        .unwrap();
    assert_eq!(ops["repo_url"], "https://github.com/acme/infra.git");
    assert_eq!(ops["repos"][0]["name"], "infra");

    // the owner adds; the alias defaults to the last segment, and is made unique when defaulted
    let r = tool(
        app,
        &w.alice,
        "add_repo",
        json!({"project": "PAY", "url": "git@github.com:acme/backend.git", "defaultBranch": "main"}),
    )
    .await;
    assert_eq!(r["added"]["name"], "backend");
    assert_eq!(r["added"]["default_branch"], "main");
    let again = refused(
        app,
        &w.alice,
        "add_repo",
        json!({"project": "PAY", "url": "https://github.com/ACME/backend"}),
    )
    .await;
    assert!(again.contains("already"), "{again}");
    let r = tool(
        app,
        &w.alice,
        "add_repo",
        json!({"project": "PAY", "url": "https://gitlab.com/other/backend.git"}),
    )
    .await;
    assert_eq!(r["added"]["name"], "backend-2");
    let bad = refused(
        app,
        &w.alice,
        "add_repo",
        json!({"project": "PAY", "url": "https://github.com/acme/x.git", "name": "Bad Name!"}),
    )
    .await;
    assert!(bad.contains("repository name"), "{bad}");

    // a member reads them but cannot change anything
    let pots = tool(app, &w.bob, "list_projects", json!({})).await;
    let pay = pots
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "PAY")
        .unwrap();
    assert_eq!(pay["repos"].as_array().unwrap().len(), 2);
    // old clients read the first one
    assert_eq!(pay["repo_url"], "git@github.com:acme/backend.git");
    for (name, args) in [
        (
            "add_repo",
            json!({"project": "PAY", "url": "https://github.com/acme/web.git"}),
        ),
        ("remove_repo", json!({"project": "PAY", "name": "backend"})),
    ] {
        let e = refused(app, &w.bob, name, args).await;
        assert!(
            e.contains("may not do that") || e.contains("owner"),
            "{name}: {e}"
        );
    }

    // removing the first one moves repo_url to the next; admins may
    let r = tool(
        app,
        &w.root,
        "remove_repo",
        json!({"project": "PAY", "name": "backend"}),
    )
    .await;
    assert_eq!(r["repos"].as_array().unwrap().len(), 1);
    let pots = tool(app, &w.alice, "list_projects", json!({})).await;
    let pay = pots
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "PAY")
        .unwrap();
    assert_eq!(pay["repo_url"], "https://gitlab.com/other/backend.git");
}

#[tokio::test]
async fn tasks_name_repositories_that_the_pot_has() {
    let w = world(false).await;
    // a pot with no repositories: a task without repos is as before, naming one is refused
    let t = w.task("plain", "", 0).await;
    assert_eq!(t["repos"], json!([]));
    let e = refused(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "PAY", "title": "x", "repos": "backend"}),
    )
    .await;
    assert!(e.contains("no repositories"), "{e}");

    w.repos(&["backend", "web"]).await;
    let t = w.task("both", "Backend, web,backend", 0).await;
    assert_eq!(t["repos"], json!(["backend", "web"]));
    let e = refused(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "PAY", "title": "x", "repos": "mobile"}),
    )
    .await;
    assert!(
        e.contains("no repository named mobile") && e.contains("backend, web"),
        "{e}"
    );

    // edit: replace, then clear with an empty string
    let id = t["id"].as_str().unwrap();
    let u = tool(
        &w.app,
        &w.alice,
        "update_task",
        json!({"id": id, "repos": "web"}),
    )
    .await;
    assert_eq!(u["repos"], json!(["web"]));
    refused(
        &w.app,
        &w.alice,
        "update_task",
        json!({"id": id, "repos": "nope"}),
    )
    .await;
    let u = tool(
        &w.app,
        &w.alice,
        "update_task",
        json!({"id": id, "repos": ""}),
    )
    .await;
    assert_eq!(u["repos"], json!([]));
    tool(
        &w.app,
        &w.alice,
        "update_task",
        json!({"id": id, "repos": "web"}),
    )
    .await;
    let l = tool(&w.app, &w.bob, "list_tasks", json!({"project": "PAY"})).await;
    assert!(
        l.as_array()
            .unwrap()
            .iter()
            .any(|t| t["repos"] == json!(["web"]))
    );

    // removing a repository takes it off open tasks
    tool(
        &w.app,
        &w.alice,
        "remove_repo",
        json!({"project": "PAY", "name": "web"}),
    )
    .await;
    let g = tool(&w.app, &w.alice, "get_task", json!({"id": id})).await;
    assert_eq!(g["repos"], json!([]));
}

#[tokio::test]
async fn claim_next_routes_by_the_cooks_repository_scope() {
    let w = world(false).await;
    w.repos(&["backend", "web", "docs"]).await;
    let (back, web, any) = (
        w.cook("back").await,
        w.cook("webber").await,
        w.cook("anyone").await,
    );

    // a cook declares its own scope; its owner may too; an unknown name is refused
    let r = tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend"}),
    )
    .await;
    assert_eq!(r["repos"], json!(["backend"]));
    tool(
        &w.app,
        &w.root,
        "set_cook_repos",
        json!({"cook": "webber", "project": "PAY", "repos": "web,docs"}),
    )
    .await;
    refused(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "mobile"}),
    )
    .await;
    // another cook may not set it, nor a plain member
    refused(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"cook": "webber", "project": "PAY", "repos": "backend"}),
    )
    .await;
    refused(
        &w.app,
        &w.bob,
        "set_cook_repos",
        json!({"cook": "webber", "project": "PAY", "repos": "backend"}),
    )
    .await;
    let cooks = tool(&w.app, &w.root, "list_cooks", json!({})).await;
    let b = cooks
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "back")
        .unwrap();
    assert_eq!(
        b["repo_scopes"],
        json!([{"project": "PAY", "repos": ["backend"]}])
    );

    // highest priority first: web(9), backend(8), none(7), both(6), docs(5)
    let t_web = w.task("web part", "web", 9).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_back = w.task("backend part", "backend", 8).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_none = w.task("any repo", "", 7).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_both = w.task("both", "web,backend", 6).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let t_docs = w.task("docs part", "docs", 5).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // `back` skips web and docs: backend, then no-repo, then the one naming backend too
    let mine = [
        w.next(&back).await,
        w.next(&back).await,
        w.next(&back).await,
        w.next(&back).await,
    ];
    assert_eq!(
        mine,
        [
            Some(t_back.clone()),
            Some(t_none.clone()),
            Some(t_both.clone()),
            None
        ]
    );
    // `webber` (web, docs) takes web and docs; the rest is claimed already
    assert_eq!(w.next(&web).await, Some(t_web.clone()));
    assert_eq!(w.next(&web).await, Some(t_docs.clone()));
    assert_eq!(w.next(&web).await, None);

    // an unscoped cook takes anything, repo or not
    for id in [&t_back, &t_none] {
        let c = tool(&w.app, &back, "get_task", json!({"id": id})).await;
        let epoch = c["lease_epoch"].as_i64().unwrap();
        tool(
            &w.app,
            &back,
            "release",
            json!({"id": id, "epoch": epoch, "reason": "back"}),
        )
        .await;
    }
    let got = [w.next(&any).await, w.next(&any).await];
    assert_eq!(got, [Some(t_back.clone()), Some(t_none)]);

    // claim_task on a named task is never refused for scope: it warns
    tool(
        &w.app,
        &any,
        "release",
        json!({"id": t_back, "epoch": 2, "reason": "x"}),
    )
    .await;
    let c = tool(&w.app, &web, "claim_task", json!({"id": t_back})).await;
    assert!(
        c["warning"]
            .as_str()
            .unwrap()
            .contains("outside your repository scope"),
        "{c}"
    );
    // inside the scope, or a task with no repos: no warning
    tool(
        &w.app,
        &back,
        "release",
        json!({"id": t_both, "epoch": 1, "reason": "x"}),
    )
    .await;
    let c = tool(&w.app, &back, "claim_task", json!({"id": t_both})).await;
    assert!(c.get("warning").is_none() || c["warning"].is_null(), "{c}");

    // wait_for_work follows the same rule
    w.task("web only", "web", 1).await;
    let ready = tool(
        &w.app,
        &back,
        "wait_for_work",
        json!({"project": "PAY", "timeoutS": 0}),
    )
    .await;
    assert!(
        ready["ready"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["title"] != "web only"),
        "{ready}"
    );

    // an empty scope puts the cook back to all repositories
    let r = tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": ""}),
    )
    .await;
    assert_eq!(r["repos"], json!([]));
}

async fn wakes_for(app: &TestApp, agent: &str) -> i64 {
    sqlx::query_scalar("select count(*) from events where kind = 'wake' and payload->>'agent' = $1")
        .bind(agent)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn work_wakes_only_cooks_whose_scope_overlaps() {
    let w = world(true).await;
    w.repos(&["backend", "web"]).await;
    let (back, web, any) = (
        w.cook("back").await,
        w.cook("webber").await,
        w.cook("anyone").await,
    );
    for c in ["back", "webber", "anyone"] {
        let (st, _) = w
            .app
            .post(
                &w.root,
                &format!("/agents/{c}/wake-targets"),
                json!({"type": "relay"}),
            )
            .await;
        assert_eq!(st, 200);
    }
    tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend"}),
    )
    .await;
    tool(
        &w.app,
        &web,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "web"}),
    )
    .await;
    let _ = any;
    tokio::time::sleep(Duration::from_millis(800)).await;

    // a task about web: the web cook and the unscoped cook are woken, the backend cook is not
    w.task("web work", "web", 0).await;
    let mut n = (0, 0, 0);
    for _ in 0..40 {
        n = (
            wakes_for(&w.app, "back").await,
            wakes_for(&w.app, "webber").await,
            wakes_for(&w.app, "anyone").await,
        );
        if n.1 > 0 && n.2 > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        n.1 >= 1 && n.2 >= 1,
        "web and unscoped cooks are woken: {n:?}"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        wakes_for(&w.app, "back").await,
        0,
        "the backend cook is not woken for web work"
    );
}

#[tokio::test]
async fn split_by_repo_makes_one_subtask_per_repository() {
    let w = world(false).await;
    w.repos(&["backend", "web"]).await;
    let (back, web) = (w.cook("back").await, w.cook("webber").await);
    tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend"}),
    )
    .await;
    tool(
        &w.app,
        &web,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "web"}),
    )
    .await;

    // no repos: nothing to split
    let plain = w.task("plain", "", 0).await;
    refused(
        &w.app,
        &w.alice,
        "split_by_repo",
        json!({"id": plain["id"]}),
    )
    .await;

    let t = w.task("Add the refund flow", "backend,web", 3).await;
    let id = t["id"].as_str().unwrap();
    let s = tool(&w.app, &w.alice, "split_by_repo", json!({"id": id})).await;
    let subs = s["subtasks"].as_array().unwrap();
    assert_eq!(subs.len(), 2);
    assert_eq!(subs[0]["title"], "[backend] Add the refund flow");
    assert_eq!(subs[0]["repos"], json!(["backend"]));
    assert_eq!(subs[1]["title"], "[web] Add the refund flow");
    assert_eq!(subs[1]["priority"], 3);
    assert_eq!(subs[0]["parent"], json!(id));
    // the parent waits for its subtasks
    let parent = tool(&w.app, &w.alice, "get_task", json!({"id": id})).await;
    assert_eq!(parent["ready"], false);
    assert_eq!(parent["subtasks"].as_array().unwrap().len(), 2);

    // each repository's cook takes its part, and only its part
    assert_eq!(w.next(&back).await.as_deref(), subs[0]["id"].as_str());
    assert_eq!(w.next(&web).await.as_deref(), subs[1]["id"].as_str());
    // the only thing left for them is the plain task, which names no repository: anyone takes it
    assert_eq!(w.next(&back).await.as_deref(), plain["id"].as_str());
    assert_eq!(w.next(&web).await, None);

    // splitting again adds nothing; a claimed task cannot be split
    let again = tool(&w.app, &w.alice, "split_by_repo", json!({"id": id})).await;
    assert!(again["subtasks"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn serving_a_task_tells_the_cooks_of_dependent_repositories() {
    let w = world(false).await;
    w.repos(&["backend", "app", "docs"]).await;
    let back = w.cook("back").await;
    let app_cook = w.cook("appy").await;
    let _unscoped = w.cook("anyone").await;
    let docs_cook = w.cook("docy").await;
    tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend"}),
    )
    .await;
    tool(
        &w.app,
        &app_cook,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "app"}),
    )
    .await;
    tool(
        &w.app,
        &docs_cook,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "docs"}),
    )
    .await;

    // links: managed like the repositories
    let e = refused(
        &w.app,
        &w.bob,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "backend"}),
    )
    .await;
    assert!(e.contains("may not do that") || e.contains("owner"), "{e}");
    refused(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "app"}),
    )
    .await;
    refused(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "nope"}),
    )
    .await;
    refused(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "backend", "mode": "x"}),
    )
    .await;
    let r = tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "backend"}),
    )
    .await;
    let app_repo = r["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "app")
        .unwrap();
    assert_eq!(
        app_repo["depends_on"],
        json!([{"name": "backend", "mode": "notify"}])
    );
    // docs depends on both backend and app, as a follow-up task
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "docs", "on": "backend", "mode": "task"}),
    )
    .await;

    // a task about backend only: appy (scoped to app) is told, anyone (unscoped) and docy are not woken;
    // docs gets a follow-up task instead
    let before = w.task_count().await;
    let t = w.task("Add the refund endpoint", "backend", 4).await;
    let id = t["id"].as_str().unwrap().to_string();
    tool(
        &w.app,
        &w.alice,
        "update_task",
        json!({"id": id, "menu": ""}),
    )
    .await;
    w.serve(&back, &id, json!({})).await;
    assert_eq!(w.upstream_wakes("appy").await, 1);
    assert_eq!(w.upstream_wakes("anyone").await, 0);
    assert_eq!(w.upstream_wakes("docy").await, 0);
    assert_eq!(w.upstream_wakes("back").await, 0);
    let text: String = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' and payload->>'reason' = 'upstream' and payload->>'agent' = 'appy'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        text.contains(&id)
            && text.contains("Add the refund endpoint")
            && text.contains("added the endpoint")
            && text.contains("`backend`"),
        "{text}"
    );
    // the served task says who was told, and the feed has the event
    let g = tool(&w.app, &w.alice, "get_task", json!({"id": id})).await;
    let last = g["comments"].as_array().unwrap().last().unwrap()["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        last.contains("app: told appy") && last.contains("docs: follow-up"),
        "{last}"
    );
    let feed = tool(
        &w.app,
        &w.alice,
        "changes_since",
        json!({"project": "PAY", "cursor": 0}),
    )
    .await;
    assert!(
        feed["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "task.propagated")
    );

    // task mode: a routed follow-up, `[docs] Follow up on PAY-n: title`, repos [docs], quoting the result
    assert_eq!(w.task_count().await, before + 2);
    let follow = tool(
        &w.app,
        &w.alice,
        "list_tasks",
        json!({"project": "PAY", "status": "open"}),
    )
    .await;
    let f = follow
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["title"]
                .as_str()
                .unwrap_or("")
                .starts_with("[docs] Follow up on")
        })
        .unwrap_or_else(|| panic!("{follow}"));
    assert_eq!(
        f["title"],
        format!("[docs] Follow up on {id}: Add the refund endpoint")
    );
    assert_eq!(f["repos"], json!(["docs"]));
    let fid = f["id"].as_str().unwrap();
    let full = tool(&w.app, &w.alice, "get_task", json!({"id": fid})).await;
    assert!(
        full["body"]
            .as_str()
            .unwrap()
            .contains("> added the endpoint")
    );
    // it routes to docs' cook by the normal claim rules, not to the others
    assert_eq!(w.next(&back).await, None);
    assert_eq!(w.next(&docs_cook).await.as_deref(), Some(fid));
}

#[tokio::test]
async fn propagation_has_its_skip_cases_and_never_loops() {
    let w = world(false).await;
    w.repos(&["backend", "app"]).await;
    let back = w.cook("back").await;
    let app_cook = w.cook("appy").await;
    let both = w.cook("both").await;
    tool(
        &w.app,
        &back,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend"}),
    )
    .await;
    tool(
        &w.app,
        &app_cook,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "app"}),
    )
    .await;
    tool(
        &w.app,
        &both,
        "set_cook_repos",
        json!({"project": "PAY", "repos": "backend,app"}),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "backend"}),
    )
    .await;

    // skip: the task already names the dependent repository
    let t = w.task("both at once", "backend,app", 0).await;
    w.serve(&back, t["id"].as_str().unwrap(), json!({})).await;
    assert_eq!(w.upstream_wakes("appy").await, 0);

    // skip: noPropagate
    let t = w.task("internal only", "backend", 0).await;
    w.serve(
        &back,
        t["id"].as_str().unwrap(),
        json!({"noPropagate": true}),
    )
    .await;
    assert_eq!(w.upstream_wakes("appy").await, 0);

    // skip: a task with no repository; and a repository nobody depends on (app is not upstream of anything)
    let t = w.task("no repo", "", 0).await;
    w.serve(&back, t["id"].as_str().unwrap(), json!({})).await;
    let t = w.task("app only", "app", 0).await;
    w.serve(&app_cook, t["id"].as_str().unwrap(), json!({}))
        .await;
    assert_eq!(w.upstream_wakes("appy").await, 0);
    assert_eq!(w.upstream_wakes("back").await, 0);

    // the actor itself is never woken, though it is scoped to the dependent
    let t = w.task("by both", "backend", 0).await;
    w.serve(&both, t["id"].as_str().unwrap(), json!({})).await;
    assert_eq!(w.upstream_wakes("both").await, 0);
    assert_eq!(w.upstream_wakes("appy").await, 1);

    // a cycle: backend depends on app too, both as follow-up tasks. Serving a backend task makes
    // one follow-up for app; serving that follow-up makes none for backend: one hop, no loop.
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "app", "on": "backend", "mode": "task"}),
    )
    .await;
    tool(
        &w.app,
        &w.alice,
        "link_repo",
        json!({"project": "PAY", "repo": "backend", "on": "app", "mode": "task"}),
    )
    .await;
    let n0 = w.task_count().await;
    let t = w.task("cycle start", "backend", 0).await;
    w.serve(&back, t["id"].as_str().unwrap(), json!({})).await;
    assert_eq!(w.task_count().await, n0 + 2, "the task and one follow-up");
    let fid: String = sqlx::query_scalar(
        "select pr.key || '-' || t.number from tasks t join projects pr on pr.id = t.project_id where t.from_upstream order by t.number desc limit 1",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    w.serve(&app_cook, &fid, json!({})).await;
    assert_eq!(
        w.task_count().await,
        n0 + 2,
        "serving the follow-up does not bounce back"
    );
    let wakes = w.upstream_wakes("appy").await;
    assert_eq!(wakes, 1, "no further upstream wakes either");

    // unlink removes the dependency
    tool(
        &w.app,
        &w.alice,
        "unlink_repo",
        json!({"project": "PAY", "repo": "backend", "on": "app"}),
    )
    .await;
    refused(
        &w.app,
        &w.alice,
        "unlink_repo",
        json!({"project": "PAY", "repo": "backend", "on": "app"}),
    )
    .await;
}
