//! `corntchap-cli review`: ask for a review of a task, see the reviews of a pot, give a verdict.

use anyhow::Result;
use clap::Subcommand;
use serde_json::{Map, Value, json};

use crate::{
    api::Api,
    ui::{self, age, ok_line, s},
};

#[derive(Subcommand)]
pub enum ReviewCmd {
    /// List a pot's reviews (open ones first).
    #[command(name = "ls", alias = "list")]
    Ls {
        /// Pot key (case-insensitive).
        pot: String,
        /// Only the reviews waiting on you.
        #[arg(long)]
        mine: bool,
        /// requested, approved, changes_requested, commented, cancelled or all.
        #[arg(long)]
        state: Option<String>,
        /// Only this task's reviews, e.g. PAY-42.
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Ask a cook or a person to review a task.
    Request {
        /// The task, e.g. PAY-42.
        task: String,
        /// A cook or person of the pot's team (not you); leave out for anyone on the team.
        #[arg(long)]
        reviewer: Option<String>,
        /// What to look at: the pull request address, the branch.
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Approve a review (by its id, e.g. PAY-R3, or its task).
    Approve {
        id: String,
        #[arg(long)]
        body: Option<String>,
    },
    /// Ask for changes: say what to fix.
    Changes {
        id: String,
        #[arg(long)]
        body: String,
    },
    /// Comment without deciding.
    Comment {
        id: String,
        #[arg(long)]
        body: String,
    },
    /// Withdraw a review you asked for.
    Cancel { id: String },
}

pub fn list_path(pot: &str, mine: bool, state: Option<&str>, task: Option<&str>) -> String {
    let mut q: Vec<String> = vec![];
    if mine {
        q.push("mine=true".into());
    }
    if let Some(s) = state {
        q.push(format!("state={s}"));
    }
    if let Some(t) = task {
        q.push(format!("task={t}"));
    }
    let base = format!("/projects/{}/reviews", pot.to_uppercase());
    if q.is_empty() {
        base
    } else {
        format!("{base}?{}", q.join("&"))
    }
}

/// A review id (PAY-R3) names a review; a task id (PAY-42) names the review waiting on you.
pub fn verdict_path(id: &str) -> String {
    let is_review = id
        .rsplit_once('-')
        .is_some_and(|(_, n)| n.starts_with(['R', 'r']));
    if is_review {
        format!("/reviews/{id}/verdict")
    } else {
        format!("/tasks/{id}/review")
    }
}

pub fn verdict_body(verdict: &str, body: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("verdict".into(), json!(verdict));
    if let Some(b) = body {
        m.insert("body".into(), json!(b));
    }
    Value::Object(m)
}

pub fn reviews_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            vec![
                ui::plain(s(r, "id")),
                ui::plain(s(r, "task")),
                ui::status_cell(&s(r, "state")),
                ui::plain(format!("round {}", r["round"])),
                ui::plain(r["reviewer"].as_str().unwrap_or("anyone").to_string()),
                ui::plain(s(r, "requested_by")),
                ui::plain(age(&s(r, "created_at"))),
                ui::plain(s(r, "task_title")),
            ]
        })
        .collect();
    ui::table(
        &[
            "id", "task", "state", "round", "reviewer", "asked by", "age", "title",
        ],
        rows,
        "no reviews",
    )
}

pub async fn run(cmd: ReviewCmd, api: &Api) -> Result<()> {
    match cmd {
        ReviewCmd::Ls {
            pot,
            mine,
            state,
            task,
            json,
        } => {
            let v = api
                .get(&list_path(&pot, mine, state.as_deref(), task.as_deref()))
                .await?;
            crate::show(&v, json, reviews_table);
        }
        ReviewCmd::Request {
            task,
            reviewer,
            note,
            json,
        } => {
            let mut m = Map::new();
            if let Some(r) = reviewer {
                m.insert("reviewer".into(), json!(r));
            }
            if let Some(n) = note {
                m.insert("note".into(), json!(n));
            }
            let v = api
                .post(&format!("/tasks/{task}/review-request"), Value::Object(m))
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                ok_line(format!(
                    "asked for {} on {} (round {})",
                    s(&v, "id"),
                    s(&v, "task"),
                    v["round"]
                ));
            }
        }
        ReviewCmd::Approve { id, body } => verdict(api, &id, "approve", body.as_deref()).await?,
        ReviewCmd::Changes { id, body } => verdict(api, &id, "changes", Some(&body)).await?,
        ReviewCmd::Comment { id, body } => verdict(api, &id, "comment", Some(&body)).await?,
        ReviewCmd::Cancel { id } => {
            let v = api
                .post(&format!("/reviews/{id}/cancel"), json!({}))
                .await?;
            ok_line(format!("{} cancelled", s(&v, "id")));
        }
    }
    Ok(())
}

async fn verdict(api: &Api, id: &str, verdict: &str, body: Option<&str>) -> Result<()> {
    let v = api
        .post(&verdict_path(id), verdict_body(verdict, body))
        .await?;
    ok_line(format!("{} {}", s(&v, "id"), s(&v, "state")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_id_and_a_task_id_go_to_different_routes() {
        assert_eq!(verdict_path("PAY-R3"), "/reviews/PAY-R3/verdict");
        assert_eq!(verdict_path("PAY-42"), "/tasks/PAY-42/review");
    }

    #[test]
    fn list_paths_carry_the_filters() {
        assert_eq!(list_path("pay", false, None, None), "/projects/PAY/reviews");
        assert_eq!(
            list_path("pay", true, Some("requested"), Some("PAY-1")),
            "/projects/PAY/reviews?mine=true&state=requested&task=PAY-1"
        );
    }
}
