//! Human-facing output: aligned tables, short status lines, colour that respects NO_COLOR and pipes.

use std::{io::IsTerminal, sync::OnceLock};

use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets};
use owo_colors::OwoColorize;
use serde_json::Value;

/// Colour only on a TTY, never with NO_COLOR set, never on TERM=dumb. CLICOLOR_FORCE=1 forces it on.
pub fn color_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return false;
        }
        if std::env::var("CLICOLOR_FORCE").is_ok_and(|v| v != "0") {
            return true;
        }
        std::io::stdout().is_terminal()
            && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
    })
}

/// A JSON field as display text; null and missing become "-".
pub fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(x) => x.clone(),
        Value::Null => "-".into(),
        o => o.to_string(),
    }
}

pub fn status_color(status: &str) -> Color {
    match status {
        "open" => Color::Cyan,
        "claimed" => Color::Yellow,
        "blocked" => Color::Red,
        "done" => Color::Green,
        "cancelled" => Color::DarkGrey,
        _ => Color::Reset,
    }
}

fn colored(text: String, c: Color) -> Cell {
    if color_on() {
        Cell::new(text).fg(c)
    } else {
        Cell::new(text)
    }
}

/// An aligned, borderless table (kubectl style). Empty input yields `empty` instead.
pub fn table(headers: &[&str], rows: Vec<Vec<Cell>>, empty: &str) -> String {
    if rows.is_empty() {
        return empty.to_string();
    }
    let mut t = Table::new();
    t.load_style(presets::NOTHING);
    // Wrap long titles only when we know the real terminal width.
    match ratatui::crossterm::terminal::size() {
        Ok((w, _)) if w >= 40 && std::io::stdout().is_terminal() => {
            t.set_content_arrangement(ContentArrangement::Dynamic)
                .set_width(w);
        }
        _ => {
            t.set_content_arrangement(ContentArrangement::Disabled);
        }
    }
    if color_on() {
        t.enforce_styling();
    } else {
        t.force_no_tty();
    }
    t.set_header(headers.iter().map(|h| {
        let c = Cell::new(h.to_uppercase());
        if color_on() {
            c.add_attribute(Attribute::Bold)
        } else {
            c
        }
    }));
    for r in rows {
        t.add_row(r);
    }
    for c in t.column_iter_mut() {
        c.set_padding((0, 2));
    }
    t.to_string()
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn plain(v: impl Into<String>) -> Cell {
    Cell::new(v.into())
}

pub fn status_cell(status: &str) -> Cell {
    colored(status.to_string(), status_color(status))
}

pub fn dim_cell(v: impl Into<String>) -> Cell {
    colored(v.into(), Color::DarkGrey)
}

/// "3/5" progress text for a task JSON.
pub fn progress(t: &Value) -> String {
    let total = t["total_items"].as_i64().unwrap_or(0);
    let open = t["open_items"].as_i64().unwrap_or(0);
    if total == 0 {
        "-".into()
    } else {
        format!("{}/{}", total - open, total)
    }
}

pub fn tasks_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            vec![
                plain(s(t, "id")),
                status_cell(&s(t, "status")),
                plain(t["priority"].to_string()),
                plain(progress(t)),
                plain(s(t, "title")),
                t["claimed_by"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
            ]
        })
        .collect();
    table(
        &["id", "status", "pri", "items", "title", "held by"],
        rows,
        "no tasks",
    )
}

pub fn projects_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            vec![
                plain(s(p, "key")),
                plain(s(p, "name")),
                plain(s(p, "team")),
                plain(p["ready"].to_string()),
                plain(p["claimed"].to_string()),
                plain(p["blocked"].to_string()),
                plain(p["done"].to_string()),
            ]
        })
        .collect();
    table(
        &["key", "name", "team", "ready", "claimed", "blocked", "done"],
        rows,
        "no projects (create one with `project add <team> <KEY> <name>`)",
    )
}

pub fn teams_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            vec![
                plain(s(t, "name")),
                plain(s(t, "role")),
                plain(t["members"].to_string()),
                plain(match t["projects"].as_array() {
                    Some(a) if !a.is_empty() => a
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(","),
                    Some(_) => "-".into(),
                    None => t["projects"].to_string(),
                }),
            ]
        })
        .collect();
    table(&["team", "role", "members", "projects"], rows, "no teams")
}

pub fn members_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            vec![
                plain(s(m, "name")),
                plain(s(m, "kind")),
                plain(s(m, "role")),
            ]
        })
        .collect();
    table(&["name", "kind", "role"], rows, "no members")
}

pub fn users_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| {
            vec![
                plain(s(u, "name")),
                plain(if u["is_admin"] == true {
                    "admin"
                } else {
                    "member"
                }),
                if u["linked"] == true {
                    plain("signed in")
                } else {
                    dim_cell("not signed in yet")
                },
            ]
        })
        .collect();
    table(&["name", "role", "sso"], rows, "no users")
}

pub fn agents_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            vec![
                plain(s(x, "name")),
                plain(s(x, "agent_kind")),
                plain(s(x, "runtime")),
                plain(s(x, "owner")),
                plain(x["busy"].to_string()),
                if x["disabled"] == true {
                    colored("disabled".into(), Color::Red)
                } else {
                    plain("active")
                },
            ]
        })
        .collect();
    table(
        &["name", "kind", "runtime", "owner", "busy", "state"],
        rows,
        "no agents (create one with `agent setup <name> --kind … --runtime …`)",
    )
}

/// Wake targets, each tagged with the agent it belongs to.
pub fn wake_table(targets: &[(String, Value)]) -> String {
    let rows = targets
        .iter()
        .map(|(agent, w)| {
            let list = |k: &str| {
                let v: Vec<&str> = w[k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                if v.is_empty() {
                    "*".to_string()
                } else {
                    v.join(",")
                }
            };
            vec![
                plain(agent.clone()),
                plain(s(w, "type")),
                plain(list("projects")),
                plain(list("labels")),
                dim_cell(s(w, "id")),
            ]
        })
        .collect();
    table(
        &["agent", "type", "projects", "labels", "id"],
        rows,
        "no wake targets",
    )
}

/// Print a success line with a green check mark when colour is on.
pub fn ok_line(msg: impl AsRef<str>) {
    if color_on() {
        println!("{} {}", "ok".green().bold(), msg.as_ref());
    } else {
        println!("ok {}", msg.as_ref());
    }
}

pub fn warn_line(msg: impl AsRef<str>) {
    if color_on() {
        eprintln!("{} {}", "warning".yellow().bold(), msg.as_ref());
    } else {
        eprintln!("warning: {}", msg.as_ref());
    }
}

pub fn bold(v: &str) -> String {
    if color_on() {
        v.bold().to_string()
    } else {
        v.to_string()
    }
}

pub fn dim(v: &str) -> String {
    if color_on() {
        v.dimmed().to_string()
    } else {
        v.to_string()
    }
}

/// `corntchap-cli task show` for humans.
pub fn task_detail(v: &Value) -> String {
    let mut o = String::new();
    let status = s(v, "status");
    let status_txt = if color_on() {
        match status.as_str() {
            "open" => status.cyan().to_string(),
            "claimed" => status.yellow().to_string(),
            "blocked" => status.red().to_string(),
            "done" => status.green().to_string(),
            _ => status.dimmed().to_string(),
        }
    } else {
        status.clone()
    };
    o.push_str(&format!("{} {}\n", bold(&s(v, "id")), bold(&s(v, "title"))));
    let mut meta = vec![
        format!("status {status_txt}"),
        format!("priority {}", v["priority"]),
    ];
    if let Some(b) = v["claimed_by"].as_str() {
        meta.push(format!("held by {b}"));
    }
    if let Some(l) = v["labels"].as_array().filter(|l| !l.is_empty()) {
        meta.push(format!(
            "labels {}",
            l.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    o.push_str(&meta.join("  ·  "));
    o.push('\n');
    let body = s(v, "body");
    if body != "-" && !body.is_empty() {
        o.push_str(&format!("\n{body}\n"));
    }
    if let Some(items) = v["items"].as_array().filter(|i| !i.is_empty()) {
        o.push_str(&format!("\n{}\n", bold("Checklist")));
        for i in items {
            let done = i["done"] == true;
            let text = s(i, "text");
            o.push_str(&format!(
                "  {:>2}. [{}] {}\n",
                i["position"],
                if done { "x" } else { " " },
                if done { dim(&text) } else { text }
            ));
        }
    }
    if let Some(d) = v["dependencies"].as_array().filter(|d| !d.is_empty()) {
        o.push_str(&format!("\n{}\n", bold("Depends on")));
        for x in d {
            o.push_str(&format!(
                "  {} [{}] {}\n",
                s(x, "id"),
                s(x, "status"),
                s(x, "title")
            ));
        }
    }
    if let Some(c) = v["comments"].as_array().filter(|c| !c.is_empty()) {
        o.push_str(&format!("\n{}\n", bold("Comments")));
        for x in c {
            o.push_str(&format!("  {}: {}\n", bold(&s(x, "author")), s(x, "body")));
        }
    }
    if let Some(r) = v["result"].as_str().filter(|r| !r.is_empty()) {
        o.push_str(&format!("\n{}\n  {r}\n", bold("Result")));
    }
    o.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tasks_table_is_aligned_and_plain_without_tty() {
        let v = json!([
            {"id":"PAY-1","status":"open","priority":2,"title":"First","total_items":2,"open_items":1,"claimed_by":null},
            {"id":"PAY-10","status":"claimed","priority":0,"title":"Second","total_items":0,"open_items":0,"claimed_by":"bot"}
        ]);
        let t = tasks_table(&v);
        assert!(
            !t.contains('\u{1b}'),
            "no escapes when stdout is not a tty: {t}"
        );
        assert!(t.contains("ID") && t.contains("HELD BY"));
        assert!(t.contains("1/2") && t.contains("bot"));
        let cols: Vec<usize> = t
            .lines()
            .filter_map(|l| l.find("First").or(l.find("Second")).or(l.find("TITLE")))
            .collect();
        assert!(
            cols.windows(2).all(|w| w[0] == w[1]),
            "title column aligned: {t}"
        );
    }

    #[test]
    fn empty_table_prints_hint() {
        assert_eq!(tasks_table(&json!([])), "no tasks");
    }

    #[test]
    fn detail_shows_checklist_comments_and_deps() {
        let v = json!({"id":"P-1","title":"T","status":"open","priority":1,"body":"hello",
            "items":[{"position":1,"text":"a","done":true},{"position":2,"text":"b","done":false}],
            "comments":[{"author":"ann","body":"hi"}],
            "dependencies":[{"id":"P-0","status":"done","title":"base"}]});
        let d = task_detail(&v);
        assert!(d.contains("[x] a") && d.contains("[ ] b"));
        assert!(d.contains("ann: hi") && d.contains("P-0 [done] base"));
    }
}
