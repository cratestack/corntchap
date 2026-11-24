//! Memories get used: a claim lists the ones that fit the task, `learned` is required to complete
//! and becomes a memory, reading a memory counts as a use.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}

struct World {
    app: common::TestApp,
    root: String,
    c1: String,
}

async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Pay"}),
    )
    .await;
    let (_, v) = app
        .post(
            &root,
            "/agents",
            json!({"name": "c1", "kind": "claude-code", "runtime": "local", "max_leases": 3}),
        )
        .await;
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&root, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    World { app, root, c1 }
}

impl World {
    async fn remember(&self, title: &str, tags: &str, kind: &str) {
        ok(
            &self
                .app
                .mcp(
                    &self.root,
                    "remember",
                    json!({"project": "PAY", "title": title, "body": format!("{title}: the details."), "tags": tags, "kind": kind}),
                )
                .await,
            "remember",
        );
    }

    async fn task(&self, title: &str, labels: &[&str]) {
        let (s, _) = self
            .app
            .post(
                &self.root,
                "/projects/PAY/tasks",
                json!({"title": title, "labels": labels}),
            )
            .await;
        assert_eq!(s, 200);
    }

    async fn claim(&self) -> Value {
        ok(
            &self
                .app
                .mcp(&self.c1, "claim_next", json!({"project": "PAY"}))
                .await,
            "claim",
        )
    }
}

fn keys(c: &Value) -> Vec<String> {
    c["memories"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|m| m["key"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_tag_that_is_a_label_brings_the_memory_along() {
    let w = world().await;
    w.remember("Retry policy for the ledger", "backend", "note")
        .await;
    w.remember("Colour tokens", "frontend", "note").await;
    w.task("Rework settlement", &["backend"]).await;
    let c = w.claim().await;
    let k = keys(&c);
    assert_eq!(k, vec!["retry-policy-for-the-ledger"], "{c}");
    let m = &c["memories"][0];
    assert_eq!(m["kind"], "note");
    assert!(
        m["excerpt"].as_str().unwrap().starts_with("Retry policy"),
        "{m}"
    );
    assert!(m["excerpt"].as_str().unwrap().chars().count() <= 241);
}

#[tokio::test]
async fn conventions_and_title_words_count_and_five_is_the_most() {
    let w = world().await;
    w.remember("Never deploy on Fridays", "", "convention")
        .await;
    w.remember("Unrelated gotcha about fonts", "", "gotcha")
        .await;
    w.remember("Shuffle the zebra", "", "note").await;
    w.remember("Webhook signatures", "", "howto").await;
    w.task("Verify webhook signatures", &[]).await;
    let c = w.claim().await;
    let k = keys(&c);
    assert!(k.contains(&"never-deploy-on-fridays".to_string()), "{k:?}");
    assert!(
        k.contains(&"unrelated-gotcha-about-fonts".to_string()),
        "gotchas too: {k:?}"
    );
    assert!(
        k.contains(&"webhook-signatures".to_string()),
        "shares words with the title: {k:?}"
    );
    assert!(!k.contains(&"shuffle-the-zebra".to_string()), "{k:?}");

    // at most five
    let w = world().await;
    for i in 0..8 {
        w.remember(&format!("Backend fact {i}"), "backend", "fact")
            .await;
    }
    w.task("Anything", &["backend"]).await;
    let c = w.claim().await;
    assert_eq!(keys(&c).len(), 5, "{c}");
}

#[tokio::test]
async fn what_was_learned_on_the_parent_or_a_dependency_comes_first() {
    let w = world().await;
    w.remember("Lineage note", "", "note").await;
    w.remember("Other note about nothing", "", "note").await;
    w.task("Parent", &[]).await;
    w.app
        .post(
            &w.root,
            "/projects/PAY/tasks",
            json!({"title": "Child", "parent": "PAY-1"}),
        )
        .await;
    ok(
        &w.app
            .mcp(&w.root, "remember", json!({"project": "PAY", "key": "from-parent", "title": "Seen on the parent", "body": "x", "task": "PAY-1"}))
            .await,
        "remember",
    );
    let c = ok(
        &w.app.mcp(&w.c1, "claim_task", json!({"id": "PAY-2"})).await,
        "claim",
    );
    assert_eq!(keys(&c), vec!["from-parent"], "{c}");
}

#[tokio::test]
async fn complete_needs_learned_and_saves_it() {
    let w = world().await;
    w.task("Rework settlement", &["backend"]).await;
    let c = w.claim().await;
    let epoch = c["lease"]["epoch"].clone();
    // missing, empty and blank are refused, and the task stays held
    let r = w
        .app
        .mcp_exact(
            &w.c1,
            "complete",
            json!({"id": "PAY-1", "epoch": epoch, "result": "done"}),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    for blank in ["", "   "] {
        let r = w
            .app
            .mcp_exact(
                &w.c1,
                "complete",
                json!({"id": "PAY-1", "epoch": epoch, "result": "done", "learned": blank}),
            )
            .await;
        assert_eq!(r["isError"], true, "{r}");
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("learned is required"),
            "{r}"
        );
    }
    let (s, _) = w
        .app
        .post(
            &w.c1,
            "/tasks/PAY-1/complete",
            json!({"epoch": epoch, "result": "x", "learned": ""}),
        )
        .await;
    assert_eq!(s, 400);
    let status: String = sqlx::query_scalar("select status from tasks where number = 1")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(status, "claimed");

    ok(
        &w.app
            .mcp(
                &w.c1,
                "complete",
                json!({"id": "PAY-1", "epoch": epoch, "result": "done", "learned": "The ledger retries only on 503; wrap calls in with_retry()."}),
            )
            .await,
        "complete",
    );
    let m = ok(
        &w.app
            .mcp(
                &w.root,
                "get_memory",
                json!({"project": "PAY", "key": "learned-PAY-1"}),
            )
            .await,
        "get_memory",
    );
    assert_eq!(m["kind"], "note");
    assert_eq!(m["task"], "PAY-1");
    assert!(m["body"].as_str().unwrap().contains("with_retry"));
    assert!(
        m["tags"].as_array().unwrap().iter().any(|t| t == "backend"),
        "{m}"
    );
    assert!(
        m["title"]
            .as_str()
            .unwrap()
            .starts_with("PAY-1: Rework settlement")
    );
}

#[tokio::test]
async fn nothing_new_is_accepted_and_writes_no_memory() {
    let w = world().await;
    w.task("Tiny fix", &[]).await;
    let c = w.claim().await;
    ok(
        &w.app
            .mcp(
                &w.c1,
                "complete",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "result": "done", "learned": "Nothing new."}),
            )
            .await,
        "complete",
    );
    let n: i64 = sqlx::query_scalar("select count(*) from memories")
        .fetch_one(&w.app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn release_and_block_learned_append_to_the_same_memory() {
    let w = world().await;
    w.task("Hard one", &["backend"]).await;
    let c = w.claim().await;
    ok(
        &w.app
            .mcp(
                &w.c1,
                "release",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "reason": "stuck", "learned": "The sandbox has no network."}),
            )
            .await,
        "release",
    );
    let c = ok(
        &w.app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim again",
    );
    // the first note comes back with the claim
    assert_eq!(keys(&c), vec!["learned-pay-1"], "{c}");
    ok(
        &w.app
            .mcp(
                &w.c1,
                "complete",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "result": "done", "learned": "Mock the client instead."}),
            )
            .await,
        "complete",
    );
    let m = ok(
        &w.app
            .mcp(
                &w.root,
                "get_memory",
                json!({"project": "PAY", "key": "learned-PAY-1"}),
            )
            .await,
        "get_memory",
    );
    let body = m["body"].as_str().unwrap();
    assert!(
        body.contains("no network") && body.contains("Mock the client"),
        "{body}"
    );
    assert_eq!(m["revision"], 2);
}

#[tokio::test]
async fn reading_a_memory_counts_as_a_use_and_unused_ones_are_forget_candidates() {
    let w = world().await;
    w.remember("Old build trick", "", "howto").await;
    w.remember("Fresh one", "", "howto").await;
    for _ in 0..2 {
        ok(
            &w.app
                .mcp(
                    &w.c1,
                    "get_memory",
                    json!({"project": "PAY", "key": "old-build-trick"}),
                )
                .await,
            "get_memory",
        );
    }
    let (uses, used): (i32, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("select uses, last_used_at from memories where key = 'old-build-trick'")
            .fetch_one(&w.app.state.db)
            .await
            .unwrap();
    assert_eq!(uses, 2);
    assert!(used.is_some());
    // reading is not editing: nobody gets "pantry news" from it
    let edited: bool = sqlx::query_scalar(
        "select updated_at = created_at from memories where key = 'old-build-trick'",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    assert!(edited, "a read leaves updated_at alone");

    // 100 days later: the one nobody reads is a candidate, the read one is not
    sqlx::query(
        "update memories set created_at = now() - interval '100 days' where key = 'fresh-one'",
    )
    .execute(&w.app.state.db)
    .await
    .unwrap();
    let md = ok(
        &w.app
            .mcp(&w.c1, "read_pantry", json!({"project": "PAY"}))
            .await,
        "read_pantry",
    );
    let md = md["markdown"].as_str().unwrap();
    assert!(
        md.contains("Possibly stale") && md.contains("`fresh-one`"),
        "{md}"
    );
    assert!(
        !md.split("Possibly stale")
            .nth(1)
            .unwrap()
            .contains("old-build-trick"),
        "{md}"
    );
}

#[tokio::test]
async fn learned_never_fails_a_completion_and_keeps_a_promoted_kind() {
    let w = world().await;
    w.task("Hard one", &["backend"]).await;
    let c = w.claim().await;
    // a promoted, nearly full memory under the key learned-PAY-1
    ok(
        &w.app
            .mcp(
                &w.c1,
                "release",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "reason": "stuck", "learned": "First note."}),
            )
            .await,
        "release",
    );
    sqlx::query("update memories set kind = 'gotcha', body = repeat('x', 19990) where key = 'learned-pay-1'")
        .execute(&w.app.state.db)
        .await
        .unwrap();
    let c = ok(
        &w.app.mcp(&w.c1, "claim_task", json!({"id": "PAY-1"})).await,
        "claim again",
    );
    ok(
        &w.app
            .mcp(
                &w.c1,
                "complete",
                json!({"id": "PAY-1", "epoch": c["lease"]["epoch"], "result": "done", "learned": "Second note, which no longer fits."}),
            )
            .await,
        "complete",
    );
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select key, kind from memories where key like 'learned-pay-1%' order by key",
    )
    .fetch_all(&w.app.state.db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("learned-pay-1".to_string(), "gotcha".to_string()),
            ("learned-pay-1-2".to_string(), "note".to_string())
        ]
    );
}
