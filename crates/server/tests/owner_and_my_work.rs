//! Task owners (one accountable person per task, whoever does the work) and `my_work`: what the
//! signed-in principal owns, holds and is waited on for, across pots.
mod common;
use common::TestApp;
use corntchap::admin;
use reqwest::Method;
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
    carol: String,
    c1: String,
    c2: String,
}

/// octo (admin); team acme: alice (owner), bob (chef), cooks c1 and c2; carol is on no team. Two
/// pots, PAY and OPS, both of acme.
async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    for u in ["alice", "bob", "carol"] {
        app.add_user(u).await;
    }
    app.set_role("alice", "orchestrator").await; // owning a team needs the role
    let token = |u: &'static str| {
        let db = app.state.db.clone();
        async move { admin::operator_token(&db, u, None).await.unwrap() }
    };
    let (alice, bob, carol) = (
        token("alice").await,
        token("bob").await,
        token("carol").await,
    );
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
    for (key, name) in [("PAY", "Payments"), ("OPS", "Operations")] {
        tool(
            &app,
            &alice,
            "create_project",
            json!({"team": "acme", "key": key, "name": name}),
        )
        .await;
    }
    let mut cooks = vec![];
    for n in ["c1", "c2"] {
        let (_, ag) = app
            .post(
                &root,
                "/agents",
                json!({"name": n, "kind": "claude-code", "runtime": "local", "max_leases": 10}),
            )
            .await;
        app.post(&root, "/teams/acme/members", json!({"principal": n}))
            .await;
        cooks.push(ag["token"].as_str().unwrap().to_string());
    }
    World {
        app,
        root,
        alice,
        bob,
        carol,
        c1: cooks[0].clone(),
        c2: cooks[1].clone(),
    }
}

impl World {
    async fn task(&self, pot: &str, title: &str, extra: Value) -> String {
        let mut args = json!({"project": pot, "title": title});
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            args[k] = v;
        }
        let t = tool(&self.app, &self.alice, "create_task", args).await;
        t["id"].as_str().unwrap().to_string()
    }

    async fn work(&self, tok: &str) -> Value {
        tool(&self.app, tok, "my_work", json!({})).await
    }

    async fn events(&self, kind: &str) -> Vec<Value> {
        sqlx::query_scalar("select payload from events where kind = $1 order by seq")
            .bind(kind)
            .fetch_all(&self.app.state.db)
            .await
            .unwrap()
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
async fn an_owner_is_set_cleared_and_checked_against_the_team() {
    let w = world().await;
    let app = &w.app;

    // set at creation
    let id = w
        .task("PAY", "Retry webhooks", json!({"owner": "bob"}))
        .await;
    let t = tool(app, &w.alice, "get_task", json!({"id": id})).await;
    assert_eq!(t["owner"], "bob", "{t}");
    assert_eq!(
        w.events("task.owner_changed").await,
        [json!({"old_owner": null, "new_owner": "bob"})]
    );

    // changed, then cleared (an empty string)
    let t = tool(
        app,
        &w.alice,
        "update_task",
        json!({"id": id, "owner": "c1"}),
    )
    .await;
    assert_eq!(t["owner"], "c1");
    let t = tool(app, &w.alice, "update_task", json!({"id": id, "owner": ""})).await;
    assert!(t["owner"].is_null(), "{t}");
    let ev = w.events("task.owner_changed").await;
    assert_eq!(
        ev[1..],
        [
            json!({"old_owner": "bob", "new_owner": "c1"}),
            json!({"old_owner": "c1", "new_owner": null}),
        ]
    );
    // an edit that leaves the owner alone, or sets the same one, is no owner change
    tool(
        app,
        &w.alice,
        "update_task",
        json!({"id": id, "title": "Retry hooks"}),
    )
    .await;
    tool(app, &w.alice, "update_task", json!({"id": id, "owner": ""})).await;
    assert_eq!(w.events("task.owner_changed").await.len(), 3);

    // the owner is on the pot's team: carol is not, and a stranger handle is not
    for who in ["carol", "ghost"] {
        let e = refused(
            app,
            &w.alice,
            "update_task",
            json!({"id": id, "owner": who}),
        )
        .await;
        assert!(e.contains("not in the team"), "{e}");
    }
    let e = refused(
        app,
        &w.alice,
        "create_task",
        json!({"project": "PAY", "title": "x", "owner": "carol"}),
    )
    .await;
    assert!(e.contains("not in the team"), "{e}");

    // a cook that holds nothing may not edit, so it may not set itself as owner
    let e = refused(app, &w.c1, "update_task", json!({"id": id, "owner": "c1"})).await;
    assert!(e.contains("may only edit tasks it currently holds"), "{e}");
    // holding the task, it may
    tool(app, &w.c1, "claim_task", json!({"id": id})).await;
    let t = tool(app, &w.c1, "update_task", json!({"id": id, "owner": "c1"})).await;
    assert_eq!(t["owner"], "c1");
    // and a person on the team (who can plan) sets it for anybody
    let t = tool(
        app,
        &w.bob,
        "update_task",
        json!({"id": id, "owner": "alice"}),
    )
    .await;
    assert_eq!(t["owner"], "alice");
    // a person outside the team does not even see the task
    let r = app
        .mcp(&w.carol, "update_task", json!({"id": id, "owner": "carol"}))
        .await;
    assert_eq!(r["isError"], true, "{r}");

    // REST: PATCH and POST take `owner` too
    let (st, v) = app
        .rest(
            &w.alice,
            Method::PATCH,
            &format!("/tasks/{id}"),
            Some(json!({"owner": "bob"})),
        )
        .await;
    assert_eq!((st, &v["owner"]), (200, &json!("bob")), "{v}");
    let (st, v) = app
        .post(
            &w.alice,
            "/projects/OPS/tasks",
            json!({"title": "Rotate keys", "owner": "c2"}),
        )
        .await;
    assert_eq!((st, &v["owner"]), (200, &json!("c2")), "{v}");
    let (st, v) = app
        .post(
            &w.alice,
            "/projects/OPS/tasks",
            json!({"title": "No such", "owner": "ghost"}),
        )
        .await;
    assert!((400..500).contains(&st), "{st} {v}");
}

#[tokio::test]
async fn list_tasks_filters_by_owner() {
    let w = world().await;
    let app = &w.app;
    let a = w.task("PAY", "Alice's", json!({"owner": "alice"})).await;
    let b = w.task("PAY", "Bob's", json!({"owner": "bob"})).await;
    let n = w.task("PAY", "Nobody's", json!({})).await;
    let list = |tok: &str, owner: Option<&str>| {
        let mut args = json!({"project": "PAY"});
        if let Some(o) = owner {
            args["owner"] = json!(o);
        }
        let tok = tok.to_string();
        async move { ids(&tool(app, &tok, "list_tasks", args).await) }
    };
    assert_eq!(list(&w.bob, None).await, [a.clone(), b.clone(), n.clone()]);
    assert_eq!(list(&w.bob, Some("me")).await, std::slice::from_ref(&b));
    assert_eq!(list(&w.alice, Some("me")).await, std::slice::from_ref(&a));
    assert_eq!(list(&w.alice, Some("bob")).await, std::slice::from_ref(&b));
    assert_eq!(list(&w.alice, Some("-")).await, std::slice::from_ref(&n));
    assert_eq!(
        list(&w.alice, Some("anyone")).await,
        [a.clone(), b.clone(), n]
    );
    assert!(list(&w.alice, Some("ghost")).await.is_empty());
    // every task shows its owner
    let all = tool(app, &w.alice, "list_tasks", json!({"project": "PAY"})).await;
    assert_eq!(all[1]["owner"], "bob");
    assert!(all[2]["owner"].is_null());
    // REST
    let (st, v) = app.get(&w.bob, "/projects/PAY/tasks?owner=me").await;
    assert_eq!(st, 200);
    assert_eq!(ids(&v), [b]);
}

#[tokio::test]
async fn the_label_migration_takes_owner_labels_for_team_members() {
    let w = world().await;
    let db = &w.app.state.db;
    let by_label = w
        .task("PAY", "Bob by label", json!({"labels": "owner:bob,infra"}))
        .await;
    let by_upper = w
        .task("PAY", "Alice, other case", json!({"labels": "Owner:Alice"}))
        .await;
    let cook = w
        .task("PAY", "A cook by label", json!({"labels": "owner:c2"}))
        .await;
    let outside = w
        .task(
            "PAY",
            "Carol is not on the team",
            json!({"labels": "owner:carol"}),
        )
        .await;
    let unknown = w
        .task(
            "PAY",
            "Nobody of that name",
            json!({"labels": "owner:ghost"}),
        )
        .await;
    let kept = w
        .task(
            "PAY",
            "Already owned",
            json!({"labels": "owner:bob", "owner": "c1"}),
        )
        .await;
    let plain = w.task("PAY", "No label", json!({})).await;
    let before: Vec<(String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("select pr.key || '-' || t.number, t.updated_at from tasks t join projects pr on pr.id = t.project_id order by t.number")
            .fetch_all(db)
            .await
            .unwrap();
    // the labels alone set no owner
    for id in [&by_label, &by_upper, &cook] {
        let t = tool(&w.app, &w.alice, "get_task", json!({"id": id})).await;
        assert!(t["owner"].is_null(), "{t}");
    }

    // the migration's backfill, run again on this data (the column part ran with the schema)
    let sql = include_str!("../migrations/0032_task_owner.sql");
    let backfill = &sql[sql.find("alter table tasks disable trigger").unwrap()..];
    let mut tx = db.begin().await.unwrap();
    sqlx::raw_sql(backfill).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let owner = |id: &str| {
        let id = id.to_string();
        let app = &w.app;
        let tok = w.alice.clone();
        async move { tool(app, &tok, "get_task", json!({"id": id})).await["owner"].clone() }
    };
    assert_eq!(owner(&by_label).await, "bob");
    assert_eq!(owner(&by_upper).await, "alice");
    assert_eq!(owner(&cook).await, "c2");
    assert!(owner(&outside).await.is_null());
    assert!(owner(&unknown).await.is_null());
    assert_eq!(owner(&kept).await, "c1");
    assert!(owner(&plain).await.is_null());
    // the labels stay, and the backfill is no edit
    let t = tool(&w.app, &w.alice, "get_task", json!({"id": by_label})).await;
    assert_eq!(t["labels"], json!(["owner:bob", "infra"]));
    let after: Vec<(String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("select pr.key || '-' || t.number, t.updated_at from tasks t join projects pr on pr.id = t.project_id order by t.number")
            .fetch_all(db)
            .await
            .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn my_work_for_a_person_across_two_pots() {
    let w = world().await;
    let app = &w.app;

    // bob owns PAY-1 (nobody holds it) and OPS-1 (c1 holds it); PAY-2 he claims himself
    let pay1 = w.task("PAY", "Owned, open", json!({"owner": "bob"})).await;
    let pay2 = w.task("PAY", "Claimed by bob", json!({})).await;
    let ops1 = w
        .task("OPS", "Owned, cooked by c1", json!({"owner": "bob"}))
        .await;
    let pay3 = w
        .task("PAY", "Depends on PAY-1", json!({"dependsOn": pay1}))
        .await;
    let finished = w
        .task("PAY", "Owned but finished", json!({"owner": "bob"}))
        .await;
    let alices = w.task("PAY", "Alice's", json!({"owner": "alice"})).await;
    tool(app, &w.bob, "claim_task", json!({"id": pay2})).await;
    let lease = tool(app, &w.c1, "claim_task", json!({"id": ops1})).await;
    assert!(lease["lease"]["epoch"].is_i64(), "{lease}");
    let done = tool(app, &w.c1, "claim_task", json!({"id": finished})).await;
    tool(
        app,
        &w.c1,
        "complete",
        json!({"id": finished, "epoch": done["lease"]["epoch"], "result": "done"}),
    )
    .await;

    // a question to bob, one for nobody in particular, one to alice; a review asked of bob
    tool(
        app,
        &w.alice,
        "ask",
        json!({"project": "PAY", "title": "Which PSP first?", "to": "bob"}),
    )
    .await;
    tool(
        app,
        &w.alice,
        "ask",
        json!({"project": "OPS", "title": "Refund window?"}),
    )
    .await;
    tool(
        app,
        &w.bob,
        "ask",
        json!({"project": "OPS", "title": "Bob's own open question"}),
    )
    .await;
    tool(
        app,
        &w.bob,
        "ask",
        json!({"project": "PAY", "title": "For alice only", "to": "alice"}),
    )
    .await;
    tool(
        app,
        &w.c1,
        "request_review",
        json!({"task": ops1, "reviewer": "bob", "note": "pull/9"}),
    )
    .await;
    // a review for the whole team, asked by bob: not waiting on bob; asked by c1: waiting on him
    tool(
        app,
        &w.c1,
        "request_review",
        json!({"task": ops1, "note": "anyone?"}),
    )
    .await;
    // c1 parks OPS-1 on a question bob must answer
    tool(
        app,
        &w.c1,
        "ask",
        json!({"task": ops1, "title": "Which region?", "to": "bob", "wait": true}),
    )
    .await;

    let v = w.work(&w.bob).await;
    assert_eq!(
        (v["principal"].as_str(), v["kind"].as_str()),
        (Some("bob"), Some("user"))
    );
    // owned, by pot (PAY before OPS is alphabetical: OPS first)
    let owned = v["owned"].as_array().unwrap();
    assert_eq!(
        owned
            .iter()
            .map(|g| g["project"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["OPS", "PAY"]
    );
    assert_eq!(owned[0]["name"], "Operations");
    assert_eq!(ids(&owned[0]["tasks"]), std::slice::from_ref(&ops1));
    assert_eq!(ids(&owned[1]["tasks"]), std::slice::from_ref(&pay1));
    let o = &owned[0]["tasks"][0];
    assert_eq!(
        (
            o["status"].as_str(),
            o["holder"].as_str(),
            o["owner"].as_str()
        ),
        (Some("blocked"), Some("c1"), Some("bob"))
    );
    assert!(
        o["updated_at"].is_string() && o["review"]["state"] == "requested",
        "{o}"
    );
    // holding: what bob claimed himself
    assert_eq!(ids(&v["holding"]), std::slice::from_ref(&pay2));
    assert_eq!(v["holding"][0]["lease_live"], true);
    // waiting: the question to bob, the two reviews, and the pool of unaddressed questions
    let w_ = &v["waiting_on_me"];
    let titles: Vec<&str> = w_["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Which PSP first?", "Which region?"], "{w_}");
    assert_eq!(w_["questions"][0]["from"], "alice");
    assert_eq!(w_["reviews"].as_array().unwrap().len(), 2, "{w_}");
    assert_eq!(w_["reviews"][0]["task"], json!(ops1));
    assert_eq!(w_["reviews"][0]["note"], "pull/9");
    assert_eq!(w_["reviews"][0]["from"], "c1");
    assert!(w_["comments"].as_array().unwrap().is_empty());
    let pool: Vec<&str> = w_["open_pool"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        pool,
        ["Refund window?"],
        "never his own, never one addressed to others"
    );
    assert_eq!(w_["open_pool_total"], 1);
    // blocked on me: OPS-1 parked on the question to bob, PAY-4 waits for PAY-1 which he owns
    let blocked = v["blocked_on_me"].as_array().unwrap();
    let pairs: Vec<(String, String, String)> = blocked
        .iter()
        .map(|t| {
            (
                t["id"].as_str().unwrap().into(),
                t["via"].as_str().unwrap().into(),
                t["because"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(
        pairs.contains(&(pay3.clone(), "dependency".into(), pay1.clone())),
        "{pairs:?}"
    );
    assert!(
        pairs
            .iter()
            .any(|(id, via, q)| *id == ops1 && via == "question" && q.starts_with("OPS-Q")),
        "{pairs:?}"
    );
    // counts: owned 2 (the finished one is not open), holding 1, waiting 4, blocked 2
    assert_eq!(
        v["counts"],
        json!({"owned": 2, "holding": 1, "waiting": 4, "blocked": 2, "open_pool": 1})
    );

    // one pot only
    let ops = tool(app, &w.bob, "my_work", json!({"project": "ops"})).await;
    assert_eq!(ops["project"], "OPS");
    assert_eq!(ids(&ops["owned"][0]["tasks"]), std::slice::from_ref(&ops1));
    assert!(ops["holding"].as_array().unwrap().is_empty());
    assert_eq!(ops["counts"]["waiting"], 3);
    // REST gives the same
    let (st, rest) = app.get(&w.bob, "/me/work").await;
    assert_eq!(st, 200);
    assert_eq!(rest["counts"], v["counts"]);
    let (st, rest) = app.get(&w.bob, "/me/work?project=PAY").await;
    assert_eq!((st, &rest["owned"][0]["project"]), (200, &json!("PAY")));
    // a pot nobody lets you see is an error, not an empty page
    let e = refused(app, &w.carol, "my_work", json!({"project": "PAY"})).await;
    assert!(e.contains("not found"), "{e}");

    // alice: her own task, the question bob asked her, and the review request she may answer
    let a = w.work(&w.alice).await;
    assert_eq!(ids(&a["owned"][0]["tasks"]), [alices]);
    assert_eq!(
        a["waiting_on_me"]["questions"][0]["title"],
        "For alice only"
    );
    assert_eq!(a["waiting_on_me"]["reviews"].as_array().unwrap().len(), 1);
    // the person outside every team has nothing, and no error
    let c = w.work(&w.carol).await;
    assert_eq!(
        c["counts"],
        json!({"owned": 0, "holding": 0, "waiting": 0, "blocked": 0, "open_pool": 0})
    );
    assert!(c["owned"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn my_work_for_a_cook() {
    let w = world().await;
    let app = &w.app;

    // c1 owns PAY-1 (and holds it), owns OPS-1 (nobody holds it) and holds PAY-3 (owned by bob)
    let pay1 = w
        .task("PAY", "Owned and held", json!({"owner": "c1"}))
        .await;
    let ops1 = w
        .task("OPS", "Owned, not held", json!({"owner": "c1"}))
        .await;
    let pay3 = w.task("PAY", "Held for bob", json!({"owner": "bob"})).await;
    let pay4 = w
        .task("PAY", "Waits for PAY-1", json!({"dependsOn": pay1}))
        .await;
    tool(app, &w.c1, "claim_task", json!({"id": pay1})).await;
    tool(app, &w.c1, "claim_task", json!({"id": pay3})).await;
    // a person comments on a task c1 holds: it waits for an answer
    tool(
        app,
        &w.alice,
        "comment",
        json!({"id": pay1, "body": "Please also cover refunds"}),
    )
    .await;
    // a question to c1, a review asked of c1 (by c2, which holds PAY-5 for the occasion)
    tool(
        app,
        &w.alice,
        "ask",
        json!({"project": "OPS", "title": "Use the sandbox?", "to": "c1"}),
    )
    .await;
    let pay5 = w.task("PAY", "Reviewed by c1", json!({})).await;
    tool(app, &w.c2, "claim_task", json!({"id": pay5})).await;
    tool(
        app,
        &w.c2,
        "request_review",
        json!({"task": pay5, "reviewer": "c1", "note": "branch x"}),
    )
    .await;
    // an open question for the pool: cooks do not get the people's pool here
    tool(
        app,
        &w.alice,
        "ask",
        json!({"project": "PAY", "title": "Anyone?"}),
    )
    .await;

    let v = w.work(&w.c1).await;
    assert_eq!(v["kind"], "agent");
    let owned: Vec<(String, Vec<String>)> = v["owned"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| (g["project"].as_str().unwrap().into(), ids(&g["tasks"])))
        .collect();
    assert_eq!(
        owned,
        [
            ("OPS".to_string(), vec![ops1.clone()]),
            ("PAY".to_string(), vec![pay1.clone()])
        ]
    );
    assert_eq!(ids(&v["holding"]), [pay1.clone(), pay3.clone()]);
    let w_ = &v["waiting_on_me"];
    assert_eq!(w_["questions"][0]["title"], "Use the sandbox?");
    assert_eq!(w_["reviews"][0]["task"], json!(pay5));
    assert_eq!(w_["reviews"][0]["from"], "c2");
    assert_eq!(w_["comments"][0]["task"], json!(pay1), "{w_}");
    assert_eq!(w_["comments"][0]["author"], "alice");
    assert!(
        w_["comments"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("refunds")
    );
    assert!(w_["open_pool"].as_array().unwrap().is_empty());
    assert_eq!(w_["open_pool_total"], 0);
    // PAY-4 depends on PAY-1, which c1 owns and has not finished
    assert_eq!(ids(&v["blocked_on_me"]), [pay4]);
    assert_eq!(
        v["counts"],
        json!({"owned": 2, "holding": 2, "waiting": 3, "blocked": 1, "open_pool": 0})
    );

    // answering takes the comment, and finishing the dependency takes the blocked task, off the page
    let held = tool(app, &w.c1, "get_task", json!({"id": pay1})).await;
    tool(
        app,
        &w.c1,
        "comment",
        json!({"id": pay1, "body": "Refunds are covered", "epoch": held["lease_epoch"]}),
    )
    .await;
    tool(
        app,
        &w.c1,
        "complete",
        json!({"id": pay1, "epoch": held["lease_epoch"], "result": "done"}),
    )
    .await;
    let v = w.work(&w.c1).await;
    assert!(
        v["waiting_on_me"]["comments"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{v}"
    );
    assert!(v["blocked_on_me"].as_array().unwrap().is_empty());
    assert_eq!(ids(&v["holding"]), [pay3]);
    assert_eq!(v["counts"]["owned"], 1);

    // the admin sees every pot but owns nothing
    let a = w.work(&w.root).await;
    assert_eq!(a["counts"]["owned"], 0);
}
