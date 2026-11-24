//! `corntchap-cli question`: a pot's question pool. Ask with a title, a description, a kind and
//! choices; answer with choices and/or text.

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::{Map, Value, json};

use crate::{
    api::Api,
    ui::{self, age, bold, dim, ok_line, s},
};

#[derive(Subcommand)]
pub enum QuestionCmd {
    /// List a pot's questions (open ones by default).
    #[command(name = "ls", alias = "list")]
    Ls {
        /// Pot key (case-insensitive).
        pot: String,
        /// open (default), answered, dismissed or all.
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One question with its choices, answer and thread.
    Show {
        /// Question id, e.g. PAY-Q3.
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Ask the pot's people (or one person or cook) a question.
    Ask {
        /// Pot key.
        pot: String,
        /// The question in one short line.
        #[arg(long)]
        title: String,
        /// Context for the answerer, markdown.
        #[arg(long)]
        description: Option<String>,
        /// single (radio list), multiple (checkboxes), free (text) or cards (quick radio cards).
        /// Default: single with choices, else free.
        #[arg(long)]
        kind: Option<String>,
        /// An answer proposal, "Label" or "Label: one line of description". Repeat for more.
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// Let the answerer type their own answer too (single, multiple, cards).
        #[arg(long)]
        allow_other: bool,
        /// multiple: the fewest choices.
        #[arg(long)]
        min: Option<i64>,
        /// multiple: the most choices.
        #[arg(long)]
        max: Option<i64>,
        /// The task it is about, e.g. PAY-42.
        #[arg(long)]
        task: Option<String>,
        /// Address it to one person (login) or cook.
        #[arg(long)]
        to: Option<String>,
        /// Park the task you hold until the answer.
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        json: bool,
    },
    /// Answer a question: pick choices (ids or labels), type text, or give the plain answer.
    Answer {
        /// Question id, e.g. PAY-Q3.
        id: String,
        /// A chosen choice (its id or label). Repeat for several.
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// The typed answer (free form, or your own answer next to the choices).
        #[arg(long)]
        text: Option<String>,
        /// The plain answer: a choice label, or text.
        answer: Option<String>,
    },
    /// Close a question without an answer.
    Dismiss {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

pub fn list_path(pot: &str, status: Option<&str>) -> String {
    let base = format!("/projects/{}/questions", pot.to_uppercase());
    match status {
        Some(st) => format!("{base}?status={st}"),
        None => base,
    }
}

/// `"Label"` or `"Label: description"` as a choice object.
pub fn choice_json(spec: &str) -> Value {
    match spec.split_once(": ") {
        Some((label, desc)) if !label.trim().is_empty() && !desc.trim().is_empty() => {
            json!({"label": label.trim(), "description": desc.trim()})
        }
        _ => json!({"label": spec.trim()}),
    }
}

/// Request body of `question ask`; unset options are left out.
#[allow(clippy::too_many_arguments)]
pub fn ask_body(
    title: &str,
    description: Option<&str>,
    kind: Option<&str>,
    choices: &[String],
    allow_other: bool,
    min: Option<i64>,
    max: Option<i64>,
    task: Option<&str>,
    to: Option<&str>,
    wait: bool,
) -> Value {
    let mut m = Map::new();
    m.insert("title".into(), json!(title));
    if let Some(d) = description {
        m.insert("description".into(), json!(d));
    }
    if let Some(k) = kind {
        m.insert("kind".into(), json!(k));
    }
    if !choices.is_empty() {
        m.insert(
            "choices".into(),
            Value::Array(choices.iter().map(|c| choice_json(c)).collect()),
        );
    }
    if allow_other {
        m.insert("allow_other".into(), json!(true));
    }
    if let Some(n) = min {
        m.insert("min".into(), json!(n));
    }
    if let Some(n) = max {
        m.insert("max".into(), json!(n));
    }
    if let Some(t) = task {
        m.insert("task".into(), json!(t));
    }
    if let Some(t) = to {
        m.insert("to".into(), json!(t));
    }
    if wait {
        m.insert("wait".into(), json!(true));
    }
    Value::Object(m)
}

/// Request body of `question answer`; unset options are left out.
pub fn answer_body(choices: &[String], text: Option<&str>, answer: Option<&str>) -> Value {
    let mut m = Map::new();
    if !choices.is_empty() {
        m.insert("choices".into(), json!(choices));
    }
    if let Some(t) = text {
        m.insert("text".into(), json!(t));
    }
    if let Some(a) = answer {
        m.insert("answer".into(), json!(a));
    }
    Value::Object(m)
}

/// A short hint of how a question is answered: "pick one", "pick 2-3 or more", "text".
pub fn kind_hint(q: &Value) -> String {
    let other = if q["allow_other"] == true {
        " + other"
    } else {
        ""
    };
    match q["kind"].as_str().unwrap_or("free") {
        "free" => "text".into(),
        "multiple" => {
            let range = match (q["min"].as_i64(), q["max"].as_i64()) {
                (Some(a), Some(b)) => format!(" {a}-{b}"),
                (Some(a), None) => format!(" {a}+"),
                (None, Some(b)) => format!(" up to {b}"),
                (None, None) => String::new(),
            };
            format!("pick{range}{other}")
        }
        "cards" => format!("quick pick{other}"),
        _ => format!("pick one{other}"),
    }
}

/// The question list: id, status, kind, age, asker and title.
pub fn questions_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| {
            vec![
                ui::plain(s(q, "id")),
                ui::status_cell(&s(q, "status")),
                ui::dim_cell(kind_hint(q)),
                ui::plain(age(&s(q, "created_at"))),
                ui::plain(s(q, "asked_by")),
                ui::plain(s(q, "title")),
            ]
        })
        .collect();
    ui::table(
        &["id", "status", "answer", "age", "asked by", "title"],
        rows,
        "no questions",
    )
}

/// One question in full: title, description, choices (the chosen ones marked), the answer.
pub fn question_detail(q: &Value) -> String {
    let mut out = format!("{}  {}\n", bold(&s(q, "id")), bold(&s(q, "title")));
    out.push_str(&dim(&format!(
        "{} · {} · asked by {} {} ago{}\n",
        s(q, "status"),
        kind_hint(q),
        s(q, "asked_by"),
        age(&s(q, "created_at")),
        q["task"]
            .as_str()
            .map(|t| format!(" · on {t}"))
            .unwrap_or_default()
    )));
    if let Some(d) = q["description"].as_str().filter(|d| !d.is_empty()) {
        out.push_str(&format!("\n{d}\n"));
    }
    let chosen: Vec<&str> = q["answer_choices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let choices = q["choices"].as_array().cloned().unwrap_or_default();
    if !choices.is_empty() {
        out.push('\n');
    }
    for c in &choices {
        let id = c["id"].as_str().unwrap_or("");
        let mark = if chosen.contains(&id) { "[x]" } else { "[ ]" };
        out.push_str(&format!(
            "  {mark} {}  {}",
            dim(id),
            c["label"].as_str().unwrap_or("")
        ));
        if let Some(d) = c["description"].as_str() {
            out.push_str(&dim(&format!(" - {d}")));
        }
        out.push('\n');
    }
    if q["status"] != "open" || q["answer"].is_string() {
        out.push_str(&format!(
            "\n{}: {}\n",
            if q["status"] == "dismissed" {
                "dismissed"
            } else {
                "answer"
            },
            q["answer"].as_str().unwrap_or("-")
        ));
        if let Some(by) = q["answered_by"].as_str() {
            out.push_str(&dim(&format!("by {by}\n")));
        }
    }
    for n in q["notes"].as_array().into_iter().flatten() {
        out.push_str(&format!("\n{}: {}", s(n, "author"), s(n, "body")));
    }
    out.trim_end().to_string()
}

pub async fn run(cmd: QuestionCmd, api: &Api) -> Result<()> {
    match cmd {
        QuestionCmd::Ls { pot, status, json } => {
            let v = api.get(&list_path(&pot, status.as_deref())).await?;
            crate::show(&v, json, questions_table);
        }
        QuestionCmd::Show { id, json } => {
            let v = api.get(&format!("/questions/{id}")).await?;
            crate::show(&v, json, question_detail);
        }
        QuestionCmd::Ask {
            pot,
            title,
            description,
            kind,
            choices,
            allow_other,
            min,
            max,
            task,
            to,
            wait,
            json,
        } => {
            let b = ask_body(
                &title,
                description.as_deref(),
                kind.as_deref(),
                &choices,
                allow_other,
                min,
                max,
                task.as_deref(),
                to.as_deref(),
                wait,
            );
            let v = api
                .post(&format!("/projects/{}/questions", pot.to_uppercase()), b)
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                ok_line(format!("asked {} ({})", s(&v, "id"), kind_hint(&v)));
            }
        }
        QuestionCmd::Answer {
            id,
            choices,
            text,
            answer,
        } => {
            if choices.is_empty() && text.is_none() && answer.is_none() {
                bail!("nothing to answer: pass --choice, --text or the answer");
            }
            let v = api
                .post(
                    &format!("/questions/{id}/answer"),
                    answer_body(&choices, text.as_deref(), answer.as_deref()),
                )
                .await?;
            ok_line(format!("{} answered: {}", s(&v, "id"), s(&v, "answer")));
        }
        QuestionCmd::Dismiss { id, reason } => {
            let b = json!({"reason": reason.unwrap_or_default()});
            let v = api.post(&format!("/questions/{id}/dismiss"), b).await?;
            ok_line(format!("{} dismissed", s(&v, "id")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_split_label_and_description() {
        assert_eq!(
            choice_json("Names only: no SKUs"),
            json!({"label": "Names only", "description": "no SKUs"})
        );
        assert_eq!(choice_json("Yes"), json!({"label": "Yes"}));
        assert_eq!(choice_json("At 10:30"), json!({"label": "At 10:30"}));
    }

    #[test]
    fn ask_body_leaves_out_what_was_not_asked() {
        let b = ask_body(
            "Round up?",
            None,
            Some("cards"),
            &["Yes".into(), "No".into()],
            false,
            None,
            None,
            None,
            None,
            false,
        );
        assert_eq!(
            b,
            json!({"title": "Round up?", "kind": "cards", "choices": [{"label": "Yes"}, {"label": "No"}]})
        );
        let b = ask_body(
            "Which?",
            Some("ctx"),
            Some("multiple"),
            &["A".into(), "B".into()],
            true,
            Some(1),
            Some(2),
            Some("PAY-1"),
            Some("sel"),
            true,
        );
        assert_eq!(b["allow_other"], true);
        assert_eq!((b["min"].as_i64(), b["max"].as_i64()), (Some(1), Some(2)));
        assert_eq!(b["wait"], true);
    }

    #[test]
    fn answer_body_carries_choices_and_text() {
        assert_eq!(
            answer_body(&["a".into()], Some("more"), None),
            json!({"choices": ["a"], "text": "more"})
        );
        assert_eq!(
            answer_body(&[], None, Some("yes")),
            json!({"answer": "yes"})
        );
    }

    #[test]
    fn the_detail_marks_the_chosen_and_shows_the_answer() {
        let q = json!({
            "id": "PAY-Q1", "title": "Which?", "description": "ctx", "kind": "multiple",
            "status": "answered", "asked_by": "c1", "created_at": "2026-01-01T00:00:00Z",
            "allow_other": true, "min": 1, "max": 2,
            "choices": [{"id": "a", "label": "A", "description": "first"}, {"id": "b", "label": "B"}],
            "answer_choices": ["b"], "answer": "B, plus: more", "answered_by": "alice"
        });
        let d = question_detail(&q);
        assert!(d.contains("[x]") && d.contains("[ ]"), "{d}");
        assert!(d.contains("answer: B, plus: more"), "{d}");
        assert_eq!(kind_hint(&q), "pick 1-2 + other");
    }
}
