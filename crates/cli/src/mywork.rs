//! `corntchap-cli me work`: what you own, hold and are waited on for, across your pots (the
//! `my_work` tool).

use anyhow::Result;
use serde_json::Value;

use crate::{api::Api, ui::s};

fn items(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn task_line(t: &Value) -> String {
    let mut line = format!("  {} [{}] {}", s(t, "id"), s(t, "status"), s(t, "title"));
    if let Some(h) = t["holder"].as_str() {
        line.push_str(&format!(" (held by {h})"));
    }
    if let Some(p) = t["pr"].as_object() {
        line.push_str(&format!(
            " PR #{} {}",
            p.get("number").unwrap_or(&Value::Null),
            p.get("state").and_then(Value::as_str).unwrap_or("")
        ));
    }
    if let Some(r) = t["review"]["state"].as_str() {
        line.push_str(&format!(" review {r}"));
    }
    if let Some(b) = t["because"].as_str() {
        line.push_str(&format!(" <- {b}"));
    }
    line
}

fn section(o: &mut String, title: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    o.push_str(&format!("\n{title}\n"));
    for l in lines {
        o.push_str(&l);
        o.push('\n');
    }
}

/// The page as text.
pub fn text(v: &Value) -> String {
    let c = &v["counts"];
    let n = |k: &str| c[k].as_u64().unwrap_or(0);
    let mut o = format!(
        "@{}: {} owned, {} holding, {} waiting on you, {} blocked on you\n",
        s(v, "principal"),
        n("owned"),
        n("holding"),
        n("waiting"),
        n("blocked")
    );
    for pot in items(&v["owned"]) {
        section(
            &mut o,
            &format!("owned in {} ({})", s(pot, "project"), s(pot, "name")),
            items(&pot["tasks"]).iter().map(task_line).collect(),
        );
    }
    section(
        &mut o,
        "holding",
        items(&v["holding"]).iter().map(task_line).collect(),
    );
    let w = &v["waiting_on_me"];
    let mut waiting: Vec<String> = items(&w["questions"])
        .iter()
        .map(|q| {
            format!(
                "  [question] {} {} (from {})",
                s(q, "id"),
                s(q, "title"),
                q["from"].as_str().unwrap_or("?")
            )
        })
        .collect();
    waiting.extend(items(&w["reviews"]).iter().map(|r| {
        format!(
            "  [review] {} {} round {} (from {})",
            s(r, "id"),
            s(r, "task"),
            r["round"],
            r["from"].as_str().unwrap_or("?")
        )
    }));
    waiting.extend(items(&w["comments"]).iter().map(|m| {
        format!(
            "  [comment] {} {}: {}",
            s(m, "task"),
            m["author"].as_str().unwrap_or("?"),
            s(m, "excerpt")
        )
    }));
    section(&mut o, "waiting on you", waiting);
    let pool: Vec<String> = items(&w["open_pool"])
        .iter()
        .map(|q| format!("  {} {}", s(q, "id"), s(q, "title")))
        .collect();
    if !pool.is_empty() {
        section(
            &mut o,
            &format!(
                "open questions nobody picked ({} in all)",
                w["open_pool_total"]
            ),
            pool,
        );
    }
    section(
        &mut o,
        "blocked on you",
        items(&v["blocked_on_me"]).iter().map(task_line).collect(),
    );
    o
}

/// `me work [--pot KEY] [--json]`
pub async fn work(api: &Api, pot: Option<String>, json_out: bool) -> Result<()> {
    let path = match pot.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("/me/work?project={}", p.to_uppercase()),
        None => "/me/work".to_string(),
    };
    let v = api.get(&path).await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", text(&v));
    }
    Ok(())
}
