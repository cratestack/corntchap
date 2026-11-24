//! `what_now`: a station's briefing (what it holds, what waits on it, ready for its folder, ready
//! in other repositories, work that needs no checkout, the guide, `next`), the pot guide and who
//! may set it, and confined stations (`claim_next` routing, wakes).
mod common;
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
    c1: String,
}

/// octo (admin), alice (owner of team acme and its pot PAY), bob (a chef of acme), cook c1, and
/// the pot's repositories backend and web.
async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob"] {
        app.add_user(u).await;
    }
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    let bob = admin::operator_token(&app.state.db, "bob", None)
        .await
        .unwrap();
    tool(&app, &root, "create_team", json!({"name": "acme"})).await;
    for (who, role) in [("alice", "owner"), ("bob", "chef")] {
        tool(
            &app,
            &root,
            "add_team_member",
            json!({"team": "acme", "principal": who, "role": role}),
        )
        .await;
    }
    tool(
        &app,
        &alice,
        "create_project",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    for n in ["backend", "web"] {
        tool(
            &app,
            &alice,
            "add_repo",
            json!({"project": "PAY", "url": format!("https://github.com/acme/{n}.git"), "defaultBranch": "main"}),
        )
        .await;
    }
    let (_, ag) = app
        .post(
            &root,
            "/agents",
            json!({"name": "c1", "kind": "claude-code", "runtime": "local", "max_leases": 10}),
        )
        .await;
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    World {
        app,
        root,
        alice,
        bob,
        c1: ag["token"].as_str().unwrap().to_string(),
    }
}

impl World {
    async fn task(&self, title: &str, repos: &str, priority: i64, labels: &str) -> String {
        let t = tool(
            &self.app,
            &self.alice,
            "create_task",
            json!({"project": "PAY", "title": title, "repos": repos, "priority": priority, "labels": labels}),
        )
        .await;
        t["id"].as_str().unwrap().to_string()
    }

    async fn station(&self, session: &str, repo: &str, confined: Option<bool>) -> String {
        let mut body = json!({
            "project": "PAY", "runtime": "claude-code", "session": session,
            "directory": format!("/work/{repo}-{session}"), "repo": repo,
        });
        if let Some(c) = confined {
            body["confined"] = json!(c);
        }
        let (st, v) = self.app.post(&self.c1, "/stations", body).await;
        assert_eq!(st, 200, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    async fn now(&self, station: &str) -> Value {
        tool(
            &self.app,
            &self.c1,
            "what_now",
            json!({"project": "PAY", "station": station}),
        )
        .await
    }
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn what_now_briefs_a_station_on_one_repository() {
    let w = world().await;
    let t_backend = w.task("Backend endpoint", "backend", 5, "").await;
    let t_web = w.task("Web page", "web", 9, "").await;
    let t_web2 = w.task("Web footer", "web", 2, "").await;
    let t_free = w.task("Write the runbook", "", 1, "").await;
    let t_epic = w.task("Payments roadmap", "", 0, "epic").await;
    let st = w.station("ses_a", "backend", None).await;

    // a question addressed to the cook, from alice
    let q = tool(
        &w.app,
        &w.alice,
        "ask",
        json!({"project": "PAY", "title": "Which PSP first?", "to": "c1"}),
    )
    .await;
    // and an open question for the pot's people, nobody picked
    tool(
        &w.app,
        &w.alice,
        "ask",
        json!({"project": "PAY", "title": "Refund window?"}),
    )
    .await;

    let v = w.now(&st).await;
    assert_eq!(v["project"], "PAY");
    assert_eq!(v["cook"], "c1");
    assert_eq!(v["station"], json!(st));
    assert_eq!(v["station_repo"], "backend");
    assert_eq!(v["confined"], false);
    // ready_here: backend's and the repo-less ones, by priority; never web's
    assert_eq!(
        ids(&v["ready_here"]),
        [t_backend.clone(), t_free.clone(), t_epic.clone()]
    );
    assert_eq!(v["ready_here_total"], 3);
    // ready_elsewhere: web, with url, branch, count and the top ones
    let el = v["ready_elsewhere"].as_array().unwrap();
    assert_eq!(el.len(), 1, "{v}");
    assert_eq!(el[0]["repo"], "web");
    assert_eq!(el[0]["url"], "https://github.com/acme/web.git");
    assert_eq!(el[0]["default_branch"], "main");
    assert_eq!(el[0]["count"], 2);
    assert_eq!(ids(&el[0]["top"]), [t_web.clone(), t_web2.clone()]);
    // the question to the cook waits on it, the other one is folder-free
    let qs = v["waiting_on_you"]["questions"].as_array().unwrap();
    assert_eq!(qs.len(), 1, "{v}");
    assert_eq!(qs[0]["id"], q["id"]);
    assert_eq!(qs[0]["title"], "Which PSP first?");
    assert_eq!(v["waiting_on_you"]["comments"], json!([]));
    let ff = &v["folder_free"];
    assert_eq!(ff["open_questions_total"], 1, "{v}");
    assert_eq!(ff["open_questions"][0]["title"], "Refund window?");
    assert_eq!(ids(&ff["needs_split"]), std::slice::from_ref(&t_epic));
    assert_eq!(ff["stale"], json!([]));
    assert_eq!(ff["pantry_news"], 0);
    assert!(v["guide"].is_null());

    // next: nothing held, so the question to the cook comes first
    let next = v["next"].as_str().unwrap();
    assert!(
        next.starts_with(&format!("Answer {}", q["id"].as_str().unwrap())),
        "{next}"
    );

    // answered: claim what is ready for the folder
    tool(
        &w.app,
        &w.alice,
        "answer_question",
        json!({"id": q["id"], "text": "Stripe"}),
    )
    .await;
    let v = w.now(&st).await;
    assert_eq!(v["waiting_on_you"]["questions"], json!([]));
    let next = v["next"].as_str().unwrap();
    assert!(
        next.starts_with(&format!("Claim {t_backend} "))
            && next.contains("claim_task")
            && next.contains(&st),
        "{next}"
    );

    // holding: finish what you hold first
    let c = tool(
        &w.app,
        &w.c1,
        "claim_task",
        json!({"id": t_backend, "station": st}),
    )
    .await;
    assert_eq!(c["task"]["id"], json!(t_backend));
    let v = w.now(&st).await;
    assert_eq!(ids(&v["holding"]), std::slice::from_ref(&t_backend));
    assert_eq!(v["holding"][0]["station"], json!(st));
    assert!(v["holding"][0]["lease_expires_at"].is_string());
    assert!(
        v["next"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Finish {t_backend} ")),
        "{v}"
    );

    // a pot-scoped MCP URL needs no project; the REST route answers the same
    let (s, r) = w
        .app
        .get(&w.c1, &format!("/projects/PAY/what-now?station={st}"))
        .await;
    assert_eq!(s, 200, "{r}");
    assert_eq!(r["station_repo"], "backend");
}

#[tokio::test]
async fn next_follows_the_fixed_order() {
    let w = world().await;
    let st = w.station("ses_a", "backend", None).await;
    // an empty pot: wait
    let v = w.now(&st).await;
    assert!(v["next"].as_str().unwrap().contains("wait_for_work"), "{v}");
    // only web work is ready: open a worktree of web, with its url, and say how to confine
    let t_web = w.task("Web page", "web", 5, "").await;
    let v = w.now(&st).await;
    assert_eq!(v["ready_here"], json!([]));
    let next = v["next"].as_str().unwrap();
    assert!(
        next.contains("worktree")
            && next.contains("https://github.com/acme/web.git")
            && next.contains(&t_web)
            && next.contains("confined=true"),
        "{next}"
    );
    // a triage task needs no checkout and comes before work elsewhere
    let t_triage = w.task("Sort the backlog", "web", 1, "triage").await;
    let v = w.now(&st).await;
    assert_eq!(
        ids(&v["folder_free"]["triage"]),
        std::slice::from_ref(&t_triage)
    );
    assert!(
        v["next"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Triage {t_triage}")),
        "{v}"
    );
    // an epic with subtasks is not "needs split"
    let epic = w.task("Epic with parts", "", 0, "epic").await;
    tool(
        &w.app,
        &w.alice,
        "create_task",
        json!({"project": "PAY", "title": "part", "parent": epic, "repos": "web"}),
    )
    .await;
    let v = w.now(&st).await;
    assert_eq!(v["folder_free"]["needs_split"], json!([]), "{v}");
}

#[tokio::test]
async fn stale_claims_need_no_checkout() {
    let w = world().await;
    let t = w.task("Lapsed work", "web", 1, "").await;
    let st = w.station("ses_a", "backend", None).await;
    tool(&w.app, &w.c1, "claim_task", json!({"id": t})).await;
    // the lease lapses and nobody opened a pull request
    sqlx::query(
        "update tasks set lease_expires_at = now() - interval '1 hour' where status = 'claimed'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    let v = w.now(&st).await;
    assert_eq!(
        ids(&v["folder_free"]["stale"]),
        std::slice::from_ref(&t),
        "{v}"
    );
    assert_eq!(v["folder_free"]["stale"][0]["holder"], "c1");
    assert!(
        v["next"]
            .as_str()
            .unwrap()
            .starts_with(&format!("Check {t}"))
    );
    // reaped: still stale (open, last lease event is the expiry)
    corntchap::jobs::reap(&w.app.state).await.unwrap();
    let v = w.now(&st).await;
    assert_eq!(
        ids(&v["folder_free"]["stale"]),
        std::slice::from_ref(&t),
        "{v}"
    );
}

#[tokio::test]
async fn the_guide_round_trips_and_is_set_by_owners_and_admins() {
    let w = world().await;
    let st = w.station("ses_a", "backend", None).await;

    // nobody but a team owner (or an admin) writes it
    for who in [&w.bob, &w.c1] {
        let t = refused(
            &w.app,
            who,
            "set_pot_guide",
            json!({"project": "PAY", "guide": "x"}),
        )
        .await;
        assert!(t.contains("owner") || t.contains("not found"), "{t}");
    }
    let guide = "# How we work\n\nSmall pull requests. Ask on the pass first.";
    let set = tool(
        &w.app,
        &w.alice,
        "set_pot_guide",
        json!({"project": "PAY", "guide": guide}),
    )
    .await;
    assert_eq!(set["guide"], guide);
    assert_eq!(set["length"], guide.chars().count());
    // what_now returns it whole, list_projects too (the console's About page)
    assert_eq!(w.now(&st).await["guide"], guide);
    let pots = tool(&w.app, &w.bob, "list_projects", json!({})).await;
    assert_eq!(pots[0]["guide"], guide);
    // whoami with a project carries it; without, not
    let me = tool(&w.app, &w.c1, "whoami", json!({"project": "PAY"})).await;
    assert_eq!(me["guide"], guide);
    let me = tool(&w.app, &w.c1, "whoami", json!({})).await;
    assert!(me["guide"].is_null());
    // register_station carries it
    let (s, reg) = w
        .app
        .post(
            &w.c1,
            "/stations",
            json!({"project": "PAY", "runtime": "claude-code", "session": "ses_b", "repo": "web"}),
        )
        .await;
    assert_eq!(s, 200, "{reg}");
    assert_eq!(reg["guide"], guide);
    // the wake prompt says there is a guide
    tool(
        &w.app,
        &w.alice,
        "pass",
        json!({"project": "PAY", "message": "hello", "to": "c1"}),
    )
    .await;
    let text: String = sqlx::query_scalar(
        "select payload->>'text' from events where kind = 'wake' and payload->>'agent' = 'c1' order by seq desc limit 1",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(
        text.contains("This pot has a guide: call what_now"),
        "{text}"
    );

    // a long guide: the first 1500 characters and a pointer
    let long = format!("{}{}", "a".repeat(1500), "TAIL".repeat(50));
    let (s, v) = w
        .app
        .rest(
            &w.root,
            reqwest::Method::PUT,
            "/projects/PAY/guide",
            Some(json!({"guide": long})),
        )
        .await;
    assert_eq!(s, 200, "an admin may: {v}");
    let me = tool(&w.app, &w.c1, "whoami", json!({"project": "PAY"})).await;
    let g = me["guide"].as_str().unwrap();
    assert!(
        g.starts_with(&"a".repeat(1500)) && !g.contains("TAIL"),
        "{g}"
    );
    assert!(g.contains("read the rest with what_now"));
    assert_eq!(w.now(&st).await["guide"], long, "what_now has it whole");
    // too long is refused; empty clears
    let t = refused(
        &w.app,
        &w.alice,
        "set_pot_guide",
        json!({"project": "PAY", "guide": "x".repeat(20_001)}),
    )
    .await;
    assert!(t.contains("20000"), "{t}");
    let cleared = tool(
        &w.app,
        &w.alice,
        "set_pot_guide",
        json!({"project": "PAY", "guide": "  "}),
    )
    .await;
    assert!(cleared["guide"].is_null());
    assert!(w.now(&st).await["guide"].is_null());
}

#[tokio::test]
async fn a_confined_station_gets_only_its_repository_and_repoless_tasks() {
    let w = world().await;
    let t_backend = w.task("Backend, high priority", "backend", 9, "").await;
    let t_web = w.task("Web", "web", 1, "").await;
    let confined = w.station("ses_web", "web", Some(true)).await;
    let free = w.station("ses_free", "web", None).await;

    // the station list says so
    let (_, l) = w.app.get(&w.c1, "/stations?project=PAY").await;
    let by_id = |id: &str| {
        l.as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(&confined)["confined"], true);
    assert_eq!(by_id(&free)["confined"], false);

    // confined: web's task although backend's has the higher priority, then nothing
    let c = tool(
        &w.app,
        &w.c1,
        "claim_next",
        json!({"project": "PAY", "station": confined}),
    )
    .await;
    assert_eq!(c["task"]["id"], json!(t_web), "{c}");
    let c = tool(
        &w.app,
        &w.c1,
        "claim_next",
        json!({"project": "PAY", "station": confined}),
    )
    .await;
    assert_eq!(c["none"], true, "{c}");
    // not confined: routing is unchanged, the highest priority wins
    let c = tool(
        &w.app,
        &w.c1,
        "claim_next",
        json!({"project": "PAY", "station": free}),
    )
    .await;
    assert_eq!(c["task"]["id"], json!(t_backend), "{c}");

    // what_now for a confined station does not send it elsewhere
    tool(
        &w.app,
        &w.c1,
        "complete",
        json!({"id": t_web, "epoch": 1, "result": "done", "noPropagate": true}),
    )
    .await;
    let b = w.task("Another backend", "backend", 3, "").await;
    let web2 = w.task("More web", "web", 1, "").await;
    let v = w.now(&confined).await;
    assert_eq!(v["confined"], true);
    assert_eq!(ids(&v["ready_here"]), std::slice::from_ref(&web2));
    assert_eq!(
        ids(&v["ready_elsewhere"][0]["top"]),
        std::slice::from_ref(&b)
    );
    let next = v["next"].as_str().unwrap();
    assert!(next.starts_with(&format!("Claim {web2} ")), "{next}");
    tool(
        &w.app,
        &w.c1,
        "claim_task",
        json!({"id": web2, "station": confined}),
    )
    .await;
    let v = w.now(&confined).await;
    assert!(
        v["next"].as_str().unwrap().starts_with("Finish"),
        "{}",
        v["next"]
    );
}

#[tokio::test]
async fn re_registering_keeps_or_changes_confined() {
    let w = world().await;
    let st = w.station("ses_a", "web", Some(true)).await;
    // a later scan that says nothing keeps it
    let again = w.station("ses_a", "web", None).await;
    assert_eq!(again, st);
    let (_, l) = w.app.get(&w.c1, "/stations?project=PAY").await;
    assert_eq!(l[0]["confined"], true);
    // and false lifts it
    w.station("ses_a", "web", Some(false)).await;
    let (_, l) = w.app.get(&w.c1, "/stations?project=PAY").await;
    assert_eq!(l[0]["confined"], false);
}

#[tokio::test]
async fn the_dispatcher_does_not_wake_a_confined_station_for_other_repositories() {
    let w = world().await;
    let confined = w.station("ses_web", "web", Some(true)).await;
    let free = w.station("ses_free", "backend", None).await;
    w.task("Backend only", "backend", 1, "").await;
    let mut conn = w.app.state.db.acquire().await.unwrap();
    let agent: uuid::Uuid = sqlx::query_scalar("select id from principals where name = 'c1'")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let project: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    // the confined station (the main one would be first) is skipped: only backend work is ready
    sqlx::query("update stations set main = true where ref = $1")
        .bind(&confined)
        .execute(&mut *conn)
        .await
        .unwrap();
    let (has, picked) = corntchap::stations::pick_for_wake(&mut conn, agent, project)
        .await
        .unwrap();
    assert!(has);
    assert_eq!(picked.unwrap()["id"], json!(free));
}
