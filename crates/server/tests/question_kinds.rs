//! Question kinds: a title, a description, single / multiple / free / cards, structured choices and
//! answers, `allow_other`, `min` / `max`, the older `question` + `options` ask with a plain-string
//! answer, the wake text, and the migration's backfill.
mod common;
use corntchap::admin;
use serde_json::{Value, json};

fn ok(r: &Value, what: &str) -> Value {
    assert_ne!(r["isError"], true, "{what}: {r}");
    common::TestApp::payload(r)
}
fn refused(r: &Value, what: &str) -> String {
    assert_eq!(r["isError"], true, "{what} should fail: {r}");
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

struct World {
    app: common::TestApp,
    alice: String,
    c1: String,
}

async fn world() -> World {
    let app = common::start(false).await;
    let (_, tok) = admin::init_admin(&app.state.db, Some("octo"))
        .await
        .unwrap();
    let root = tok.unwrap();
    app.add_user("alice").await;
    app.post(&root, "/teams", json!({"name": "acme"})).await;
    app.post(
        &root,
        "/teams/acme/members",
        json!({"principal": "alice", "role": "owner"}),
    )
    .await;
    let alice = admin::operator_token(&app.state.db, "alice", None)
        .await
        .unwrap();
    app.post(
        &alice,
        "/projects",
        json!({"team": "acme", "key": "PAY", "name": "Payments"}),
    )
    .await;
    let (st, v) = app
        .post(
            &alice,
            "/agents",
            json!({"name": "c1", "kind": "opencode", "runtime": "local", "max_leases": 3}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    let c1 = v["token"].as_str().unwrap().to_string();
    app.post(&alice, "/teams/acme/members", json!({"principal": "c1"}))
        .await;
    World { app, alice, c1 }
}

impl World {
    async fn ask(&self, args: Value) -> Value {
        let mut a = args;
        a["project"] = json!("PAY");
        ok(&self.app.mcp(&self.c1, "ask", a).await, "ask")
    }
    async fn try_ask(&self, args: Value) -> String {
        let mut a = args;
        a["project"] = json!("PAY");
        refused(&self.app.mcp(&self.c1, "ask", a).await, "ask")
    }
    async fn answer(&self, id: &str, args: Value) -> Value {
        let mut a = args;
        a["id"] = json!(id);
        ok(
            &self.app.mcp(&self.alice, "answer_question", a).await,
            "answer",
        )
    }
    async fn try_answer(&self, id: &str, args: Value) -> String {
        let mut a = args;
        a["id"] = json!(id);
        refused(
            &self.app.mcp(&self.alice, "answer_question", a).await,
            "answer",
        )
    }
}

#[tokio::test]
async fn kinds_choices_and_validation() {
    let w = world().await;

    // --- the new shape: title, description, structured choices, ids made from the labels ---
    let q = w
        .ask(json!({
            "title": "Which columns?", "description": "The **export** has room for two.",
            "kind": "multiple", "min": 1, "max": 2, "allowOther": true,
            "choices": json!([
                {"label": "Names and SKUs", "description": "the long form"},
                {"label": "Prices"},
                {"label": "Names and SKUs"}
            ]).to_string(),
        }))
        .await;
    assert_eq!(q["title"], "Which columns?");
    assert_eq!(q["description"], "The **export** has room for two.");
    assert_eq!(
        q["body"],
        "Which columns?\n\nThe **export** has room for two."
    );
    assert_eq!(q["kind"], "multiple");
    assert_eq!((q["min"].as_i64(), q["max"].as_i64()), (Some(1), Some(2)));
    assert_eq!(q["allow_other"], true);
    let ids: Vec<&str> = q["choices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["names-and-skus", "prices", "names-and-skus-2"]);
    assert_eq!(q["choices"][0]["description"], "the long form");
    assert_eq!(
        q["options"],
        json!(["Names and SKUs", "Prices", "Names and SKUs"])
    );
    let id = q["id"].as_str().unwrap().to_string();

    // multiple: min / max, labels or ids, text only with allow_other
    let e = w.try_answer(&id, json!({"choices": ""})).await;
    assert!(e.contains("at least one"), "{e}");
    let e = w
        .try_answer(
            &id,
            json!({"choices": "[\"prices\",\"names-and-skus\",\"names-and-skus-2\"]"}),
        )
        .await;
    assert!(e.contains("at most 2"), "{e}");
    let e = w.try_answer(&id, json!({"choices": "[\"nope\"]"})).await;
    assert!(e.contains("not one of the choices"), "{e}");
    let a = w
        .answer(&id, json!({"choices": "[\"prices\"]", "text": "also tags"}))
        .await;
    assert_eq!(a["answer"], "Prices — plus: also tags");
}

#[tokio::test]
async fn answers_are_structured_and_rendered() {
    let w = world().await;
    let q = w
        .ask(json!({
            "title": "Which columns?", "kind": "multiple", "min": 1, "max": 2, "allowOther": true,
            "choices": "Names and SKUs: the long form\nPrices\nStock",
        }))
        .await;
    let id = q["id"].as_str().unwrap();
    // three picks is over max; labels are matched case-insensitively
    let e = w
        .try_answer(id, json!({"choices": "Names and SKUs\nprices\nstock"}))
        .await;
    assert!(e.contains("at most 2"), "{e}");
    let a = w
        .answer(
            id,
            json!({"choices": "[\"Prices\", \"names-and-skus\"]", "text": "  "}),
        )
        .await;
    // rendered in the question's order, ids stored
    assert_eq!(a["answer"], "Names and SKUs, Prices");
    assert_eq!(a["answer_choices"], json!(["names-and-skus", "prices"]));
    assert_eq!(a["answer_text"], Value::Null);
    assert_eq!(a["status"], "answered");

    // an answer that is done cannot be given twice
    let e = w.try_answer(id, json!({"choices": "Stock"})).await;
    assert!(e.contains("already"), "{e}");

    // other next to a choice, rendered "plus:"
    let q = w
        .ask(
            json!({"title": "Columns again?", "kind": "multiple", "allowOther": true,
            "choices": "Names and SKUs\nPrices"}),
        )
        .await;
    let a = w
        .answer(
            q["id"].as_str().unwrap(),
            json!({"choices": "Names and SKUs", "text": "also tags"}),
        )
        .await;
    assert_eq!(a["answer"], "Names and SKUs — plus: also tags");
    assert_eq!(a["answer_text"], "also tags");
}

#[tokio::test]
async fn single_and_cards_take_one_choice_or_other() {
    let w = world().await;
    for kind in ["single", "cards"] {
        let q = w
            .ask(json!({"title": format!("{kind}?"), "kind": kind, "choices": "Yes\nNo"}))
            .await;
        let id = q["id"].as_str().unwrap();
        assert!(q["choices"][0]["description"].is_null());
        assert_eq!(q["allow_other"], false);
        // none, two, text without allow_other, a choice and text: all refused
        assert!(
            w.try_answer(id, json!({"choices": "[]"}))
                .await
                .contains("choose one")
        );
        let e = w
            .try_answer(id, json!({"choices": "[\"yes\",\"no\"]"}))
            .await;
        assert!(e.contains("choose one"), "{e}");
        let e = w.try_answer(id, json!({"text": "maybe"})).await;
        assert!(e.contains("not text"), "{e}");
        let a = w.answer(id, json!({"choices": "no"})).await;
        assert_eq!(a["answer"], "No");
        assert_eq!(a["answer_choices"], json!(["no"]));
    }
    // allow_other: text alone is the answer, text with a choice is refused
    let q = w
        .ask(json!({"title": "Pick?", "kind": "cards", "allowOther": true, "choices": "Red\nBlue"}))
        .await;
    let id = q["id"].as_str().unwrap();
    let e = w
        .try_answer(id, json!({"choices": "red", "text": "green"}))
        .await;
    assert!(e.contains("not both"), "{e}");
    let a = w.answer(id, json!({"text": "green"})).await;
    assert_eq!(a["answer"], "green");
    assert_eq!(a["answer_choices"], json!([]));
    assert_eq!(a["answer_text"], "green");
}

#[tokio::test]
async fn free_needs_text_and_asks_are_checked() {
    let w = world().await;
    let q = w.ask(json!({"title": "Why?"})).await;
    assert_eq!(q["kind"], "free");
    assert_eq!(q["choices"], json!([]));
    let id = q["id"].as_str().unwrap();
    assert!(
        w.try_answer(id, json!({"choices": "x"}))
            .await
            .contains("text")
    );
    let a = w.answer(id, json!({"text": "Because."})).await;
    assert_eq!(a["answer"], "Because.");
    assert_eq!(a["answer_text"], "Because.");

    // ask-side checks
    assert!(
        w.try_ask(json!({"kind": "single", "title": "x", "choices": "A", "min": 1}))
            .await
            .contains("multiple")
    );
    assert!(
        w.try_ask(json!({"title": "x", "kind": "free", "choices": "A\nB"}))
            .await
            .contains("no choices")
    );
    assert!(
        w.try_ask(json!({"title": "x", "kind": "cards", "choices": "Only"}))
            .await
            .contains("at least 2")
    );
    assert!(
        w.try_ask(json!({"title": "x", "kind": "huge"}))
            .await
            .contains("kind")
    );
    assert!(
        w.try_ask(json!({"title": "x", "kind": "multiple", "choices": "A\nB", "min": 3}))
            .await
            .contains("min")
    );
    assert!(
        w.try_ask(json!({"title": "x\ny"}))
            .await
            .contains("one line")
    );
    assert!(
        w.try_ask(json!({"description": "no title"}))
            .await
            .contains("title")
    );
}

#[tokio::test]
async fn the_older_question_and_options_still_work() {
    let w = world().await;
    let q = w
        .ask(json!({"question": "Round prices?\nHalf up or down? Tax is included.", "options": "Up\nDown"}))
        .await;
    assert_eq!(q["title"], "Round prices?");
    assert_eq!(q["description"], "Half up or down? Tax is included.");
    assert_eq!(q["kind"], "single");
    assert_eq!(q["choices"][0]["label"], "Up");
    assert_eq!(q["choices"][1]["id"], "down");
    assert_eq!(q["options"], json!(["Up", "Down"]));
    // a plain answer equal to a label picks that choice (the REST route the CLI and cooks use)
    let a = w
        .answer(q["id"].as_str().unwrap(), json!({"answer": "down"}))
        .await;
    assert_eq!(a["answer"], "Down");
    assert_eq!(a["answer_choices"], json!(["down"]));

    // a plain answer that is no label is the typed text: it counts as "other" even when the
    // question did not say so, as older cooks and chats answer in their own words
    let q = w
        .ask(json!({"question": "Colour?", "options": "Red\nBlue"}))
        .await;
    let id = q["id"].as_str().unwrap();
    let a = w.answer(id, json!({"answer": "green"})).await;
    assert_eq!(
        (a["answer"].as_str(), a["answer_text"].as_str()),
        (Some("green"), Some("green"))
    );

    // a question with no options is free; its plain answer is the text
    let q = w.ask(json!({"question": "Anything else?"})).await;
    assert_eq!(q["kind"], "free");
    let a = w
        .answer(q["id"].as_str().unwrap(), json!({"answer": "No."}))
        .await;
    assert_eq!(a["answer"], "No.");
    assert_eq!(a["answer_text"], "No.");

    // a very long first line is cut at a word boundary
    let long = format!("{} end", "word ".repeat(40));
    let q = w.ask(json!({"question": long})).await;
    let t = q["title"].as_str().unwrap();
    assert!(t.ends_with('…') && t.chars().count() <= 121, "{t}");
    assert!(q["description"].as_str().unwrap().ends_with("end"));

    // the REST ask takes the same fields as the console and the CLI send them
    let (st, v) = w
        .app
        .post(
            &w.alice,
            "/projects/PAY/questions",
            json!({"title": "Ship it?", "kind": "cards",
                   "choices": [{"label": "Yes"}, {"label": "No", "description": "wait"}], "allow_other": true}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        (v["kind"].as_str(), v["allow_other"].as_bool()),
        (Some("cards"), Some(true))
    );
    let (st, v) = w
        .app
        .post(
            &w.alice,
            &format!("/questions/{}/answer", v["id"].as_str().unwrap()),
            json!({"choices": ["yes"]}),
        )
        .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["answer"], "Yes");
}

#[tokio::test]
async fn the_wake_carries_the_structured_answer() {
    let w = world().await;
    let q = w
        .ask(
            json!({"title": "Which columns?", "kind": "multiple", "allowOther": true,
            "choices": "Names and SKUs\nPrices"}),
        )
        .await;
    w.answer(
        q["id"].as_str().unwrap(),
        json!({"choices": "[\"names-and-skus\",\"prices\"]", "text": "also tags"}),
    )
    .await;
    let wake: Value = sqlx::query_scalar(
        "select payload from events where kind = 'wake' order by seq desc limit 1",
    )
    .fetch_one(&w.app.state.db)
    .await
    .unwrap();
    let text = wake["text"].as_str().unwrap();
    assert!(
        text.contains("Names and SKUs, Prices — plus: also tags"),
        "{text}"
    );
    assert!(
        text.contains("Chosen (ids): names-and-skus, prices"),
        "{text}"
    );
    assert!(text.contains("Other (typed): also tags"), "{text}");
}

/// The migration's backfill, run on rows shaped like the old table's.
#[tokio::test]
async fn the_backfill_splits_titles_and_builds_choices() {
    let w = world().await;
    let db = &w.app.state.db;
    let project: uuid::Uuid = sqlx::query_scalar("select id from projects where key = 'PAY'")
        .fetch_one(db)
        .await
        .unwrap();
    let long = format!("{} tail", "alpha ".repeat(30));
    let legacy: Vec<(&str, String, Vec<&str>, Option<&str>)> = vec![
        ("one line", "Round up?".into(), vec![], None),
        (
            "two parts",
            "Which one?\n\nSome context\nmore".into(),
            vec!["Net", "Gross", "Net"],
            Some("Gross"),
        ),
        (
            "long",
            long.clone(),
            vec!["A b", "!!!"],
            Some("something else"),
        ),
    ];
    let mut tx = db.begin().await.unwrap();
    sqlx::query("alter table questions alter column title drop not null")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("alter table questions drop constraint questions_title_len")
        .execute(&mut *tx)
        .await
        .unwrap();
    for (i, (_, body, options, answer)) in legacy.iter().enumerate() {
        sqlx::query(
            "insert into questions (project_id, number, body, options, status, answer)
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(project)
        .bind(100 + i as i64)
        .bind(body)
        .bind(options)
        .bind(if answer.is_some() { "answered" } else { "open" })
        .bind(answer)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    let sql = include_str!("../migrations/0025_question_kinds.sql");
    let from = sql.find("-- Backfill").expect("backfill marker");
    sqlx::raw_sql(&sql[from..]).execute(&mut *tx).await.unwrap();
    type Row = (String, String, String, Value, Vec<String>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "select title, description, kind, choices, answer_choices, answer_text
         from questions where number >= 100 order by number",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(rows[0].0, "Round up?");
    assert_eq!((rows[0].1.as_str(), rows[0].2.as_str()), ("", "free"));
    assert_eq!(rows[0].3, json!([]));

    assert_eq!(rows[1].0, "Which one?");
    assert_eq!(rows[1].1, "Some context\nmore");
    assert_eq!(rows[1].2, "single");
    assert_eq!(
        rows[1].3,
        json!([{"id": "net-1", "label": "Net"}, {"id": "gross", "label": "Gross"}, {"id": "net-3", "label": "Net"}])
    );
    assert_eq!(rows[1].4, vec!["gross".to_string()]);
    assert_eq!(rows[1].5, None);

    // a long first line is cut at a word boundary; the cut-off part leads the description
    assert!(
        rows[2].0.ends_with('…') && rows[2].0.chars().count() <= 121,
        "{}",
        rows[2].0
    );
    assert!(rows[2].1.ends_with("tail"), "{}", rows[2].1);
    assert_eq!(rows[2].3[1]["id"], "choice-2");
    assert!(rows[2].4.is_empty());
    assert_eq!(rows[2].5.as_deref(), Some("something else"));
}

#[test]
fn choice_parsing_and_answer_checks_are_pure() {
    use corntchap::questions::{Choice, Reply, check_answer, parse_choices};
    let cs = parse_choices(Some(&json!("Yes: go\nNo")), &[]).unwrap();
    assert_eq!(
        cs,
        vec![
            Choice {
                id: "yes".into(),
                label: "Yes".into(),
                description: Some("go".into())
            },
            Choice {
                id: "no".into(),
                label: "No".into(),
                description: None
            },
        ]
    );
    let r = |c: Option<Value>, t: Option<&str>| Reply {
        choices: c,
        text: t.map(String::from),
        answer: None,
    };
    let g = check_answer(
        "cards",
        &cs,
        false,
        None,
        None,
        &r(Some(json!(["no"])), None),
    )
    .unwrap();
    assert_eq!(
        (g.choices, g.rendered.as_str()),
        (vec!["no".to_string()], "No")
    );
    assert!(check_answer("free", &[], false, None, None, &r(None, None)).is_err());
    assert!(
        check_answer(
            "multiple",
            &cs,
            false,
            Some(2),
            None,
            &r(Some(json!(["no"])), None)
        )
        .is_err()
    );
}
