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

/// A person for humans: "Display Name · @handle", or just the handle without a display name.
pub fn person(v: &Value) -> String {
    match v["display_name"].as_str().filter(|d| !d.is_empty()) {
        Some(d) => format!("{d} · @{}", s(v, "name")),
        None => s(v, "name"),
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

/// Colour of a pantry memory kind.
pub fn kind_color(kind: &str) -> Color {
    match kind {
        "convention" => Color::Cyan,
        "gotcha" => Color::Red,
        "decision" => Color::Magenta,
        "howto" => Color::Green,
        "fact" => Color::Blue,
        _ => Color::Reset,
    }
}

pub fn kind_cell(kind: &str) -> Cell {
    colored(kind.to_string(), kind_color(kind))
}

/// "3m", "5h", "2d" between an RFC 3339 timestamp and `now`; "-" when it does not parse.
pub fn age_at(ts: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return "-".into();
    };
    let secs = (now - t.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        86400..=5_183_999 => format!("{}d", secs / 86400),
        _ => format!("{}mo", secs / 2_592_000),
    }
}

pub fn age(ts: &str) -> String {
    age_at(ts, chrono::Utc::now())
}

/// "5m ago", or "just now" within the first minute.
pub fn ago_phrase(ts: &str) -> String {
    match age(ts).as_str() {
        "now" => "just now".into(),
        "-" => "at an unknown time".into(),
        a => format!("{a} ago"),
    }
}

fn tags_text(v: &Value) -> String {
    match v["tags"].as_array() {
        Some(a) if !a.is_empty() => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(","),
        _ => "-".into(),
    }
}

/// Pantry list/search results. Server order is kept (best match first when searching); a RANK
/// column appears only when the server ranked the results.
pub fn memories_table(v: &Value) -> String {
    let items = v.as_array().cloned().unwrap_or_default();
    let ranked = items.iter().any(|m| m["rank"].is_number());
    let rows = items
        .iter()
        .map(|m| {
            let mut r = vec![
                dim_cell(s(m, "key")),
                kind_cell(&s(m, "kind")),
                plain(s(m, "title")),
                plain(tags_text(m)),
                plain(s(m, "updated_by")),
                plain(age(&s(m, "updated_at"))),
            ];
            if ranked {
                r.insert(
                    0,
                    dim_cell(m["rank"].as_f64().map_or("-".into(), |x| format!("{x:.2}"))),
                );
            }
            r
        })
        .collect();
    let headers: &[&str] = if ranked {
        &["rank", "key", "kind", "title", "tags", "by", "age"]
    } else {
        &["key", "kind", "title", "tags", "by", "age"]
    };
    table(headers, rows, "")
}

/// `corntchap-cli pantry show` for humans.
pub fn memory_detail(v: &Value) -> String {
    let kind = s(v, "kind");
    let kind_txt = if color_on() {
        match kind.as_str() {
            "convention" => kind.cyan().to_string(),
            "gotcha" => kind.red().to_string(),
            "decision" => kind.magenta().to_string(),
            "howto" => kind.green().to_string(),
            "fact" => kind.blue().to_string(),
            _ => kind.clone(),
        }
    } else {
        kind.clone()
    };
    let mut meta = vec![kind_txt, format!("tags {}", tags_text(v))];
    meta.push(format!("revision {}", v["revision"]));
    meta.push(format!(
        "updated by {} {}",
        s(v, "updated_by"),
        ago_phrase(&s(v, "updated_at"))
    ));
    if let Some(t) = v["task"].as_str() {
        meta.push(format!("task {t}"));
    }
    format!(
        "{} {}\n{}\n\n{}",
        dim(&s(v, "key")),
        bold(&s(v, "title")),
        meta.join("  ·  "),
        s(v, "body").trim_end()
    )
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

/// Small text progress bar: `████░░░░ 3/8`. Never shows a full bar until everything is done.
pub fn bar(done: i64, total: i64, width: usize) -> String {
    let (done, total) = (done.max(0), total.max(0));
    let filled = if total == 0 {
        0
    } else {
        ((done.min(total) as usize * width) / total as usize).min(width)
    };
    format!(
        "{}{} {done}/{total}",
        "█".repeat(filled),
        "░".repeat(width - filled)
    )
}

/// Progress bar text for a menu JSON (`done` of `total` tasks).
pub fn menu_bar(m: &Value, width: usize) -> String {
    bar(
        m["done"].as_i64().unwrap_or(0),
        m["total"].as_i64().unwrap_or(0),
        width,
    )
}

pub fn menus_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let served = m["status"] == "served";
            let count = |k: &str| {
                let n = m[k].as_i64().unwrap_or(0);
                if n == 0 {
                    dim_cell("-")
                } else {
                    plain(n.to_string())
                }
            };
            vec![
                plain(s(m, "key")),
                plain(s(m, "name")),
                colored(
                    s(m, "status"),
                    if served { Color::Green } else { Color::Cyan },
                ),
                plain(menu_bar(m, 8)),
                m["due_on"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
                count("open"),
                count("claimed"),
                match m["blocked"].as_i64().unwrap_or(0) {
                    0 => dim_cell("-"),
                    n => colored(n.to_string(), Color::Red),
                },
            ]
        })
        .collect();
    table(
        &[
            "key", "name", "status", "progress", "due", "open", "claimed", "blocked",
        ],
        rows,
        "no menus: add one with `menu add <POT> <NAME>`",
    )
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
                t["repos"]
                    .as_array()
                    .filter(|r| !r.is_empty())
                    .map(|r| {
                        plain(
                            r.iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(","),
                        )
                    })
                    .unwrap_or_else(|| dim_cell("-")),
                t["menu"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
                t["claimed_by"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
            ]
        })
        .collect();
    table(
        &[
            "id", "status", "pri", "items", "title", "repos", "menu", "held by",
        ],
        rows,
        "no tasks",
    )
}

/// A pot's repositories: name, url, default branch and what each depends on.
pub fn repos_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let deps: Vec<String> = r["depends_on"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| format!("{} ({})", s(d, "name"), s(d, "mode")))
                .collect();
            vec![
                plain(s(r, "name")),
                plain(s(r, "url")),
                r["default_branch"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
                if deps.is_empty() {
                    dim_cell("-")
                } else {
                    plain(deps.join(", "))
                },
            ]
        })
        .collect();
    table(
        &["name", "url", "branch", "depends on"],
        rows,
        "no repositories (add one with `project repo add <POT> <URL>`)",
    )
}

/// What `project repo available` prints: `*` marks a repository already in the pot.
pub fn forge_repos_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let mut flags = vec![];
            if r["private"] == true {
                flags.push("private");
            }
            if r["archived"] == true {
                flags.push("archived");
            }
            vec![
                plain(if r["in_pot"] == true { "*" } else { "" }),
                plain(s(r, "full_name")),
                r["default_branch"]
                    .as_str()
                    .map(plain)
                    .unwrap_or_else(|| dim_cell("-")),
                plain(flags.join(", ")),
                plain(s(r, "pushed_at").chars().take(10).collect::<String>()),
            ]
        })
        .collect();
    table(
        &["", "repository", "branch", "", "pushed"],
        rows,
        "no repositories visible to the connected GitHub Apps",
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
                plain(role_label(&s(t, "role"))),
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
    table(&["team", "role", "chefs", "projects"], rows, "no teams")
}

/// Team roles as shown: the stored `member` reads as chef.
fn role_label(r: &str) -> String {
    if r == "member" {
        "chef".into()
    } else {
        r.to_string()
    }
}

pub fn members_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            vec![
                plain(s(m, "name")),
                plain(s(m, "display_name")),
                plain(s(m, "kind")),
                plain(role_label(&s(m, "role"))),
            ]
        })
        .collect();
    table(&["handle", "name", "kind", "role"], rows, "no chefs")
}

pub fn users_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|u| {
            vec![
                plain(s(u, "name")),
                plain(s(u, "display_name")),
                plain(match u["role"].as_str() {
                    Some("super_admin") => "super-admin",
                    Some("orchestrator") => "orchestrator",
                    _ => "member",
                }),
                match u["linked"].as_bool() {
                    Some(true) => plain("signed in"),
                    Some(false) => dim_cell("not signed in yet"),
                    None => plain(""),
                },
            ]
        })
        .collect();
    table(&["handle", "name", "role", "sso"], rows, "no users")
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
                plain(
                    x["grants"]
                        .as_array()
                        .filter(|g| !g.is_empty())
                        .map(|g| {
                            g.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_else(|| "-".into()),
                ),
                plain(format!("{}/{}", x["busy"], x["max_leases"])),
                if x["disabled"] == true {
                    colored("disabled".into(), Color::Red)
                } else {
                    plain("active")
                },
            ]
        })
        .collect();
    table(
        &[
            "name", "kind", "runtime", "owner", "grants", "leases", "state",
        ],
        rows,
        "no agents (create one with `cook setup <name> --kind … --runtime …`)",
    )
}

/// Stations: one agent session each, with the tasks it holds or waits on.
pub fn stations_table(v: &Value) -> String {
    let list = |x: &Value, k: &str| {
        x[k].as_array().filter(|a| !a.is_empty()).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
    };
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let tasks = match (list(x, "holding"), list(x, "waiting")) {
                (None, None) => "-".to_string(),
                (h, w) => format!(
                    "{}{}",
                    h.map(|h| format!("holding {h}")).unwrap_or_default(),
                    w.map(|w| format!(" waiting {w}")).unwrap_or_default()
                )
                .trim()
                .to_string(),
            };
            vec![
                plain(if x["main"] == true {
                    format!("{} ★", s(x, "id"))
                } else {
                    s(x, "id")
                }),
                plain(s(x, "cook")),
                plain(s(x, "project")),
                plain(s(x, "runtime")),
                plain({
                    let w = x["label"]
                        .as_str()
                        .or_else(|| x["directory"].as_str())
                        .unwrap_or("-");
                    match x["repo"].as_str() {
                        Some(r) => format!("{w} [{r}]"),
                        None => w.to_string(),
                    }
                }),
                plain(tasks),
                dim_cell(
                    x["last_woken_at"]
                        .as_str()
                        .map(age)
                        .unwrap_or_else(|| "never".into()),
                ),
            ]
        })
        .collect();
    table(
        &[
            "id",
            "cook",
            "pot",
            "runtime",
            "label",
            "tasks",
            "last woken",
        ],
        rows,
        "no stations (register your sessions with `station scan`, run inside a repo); ★ marks the main station",
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
/// One pull request of a task: `acme/shop#12 open  checks failure (build, lint)  review changes_requested  conflict`.
pub fn pr_line(p: &Value) -> String {
    let mut parts = vec![
        format!("{}#{}", s(p, "repo"), p["number"]),
        s(p, "state"),
        format!("checks {}", s(p, "checks")),
    ];
    if let Some(f) = p["failing"].as_array().filter(|f| !f.is_empty()) {
        let names: Vec<&str> = f.iter().filter_map(Value::as_str).collect();
        if let Some(last) = parts.last_mut() {
            last.push_str(&format!(" ({})", names.join(", ")));
        }
    }
    if p["review"] != "none" {
        parts.push(format!("review {}", s(p, "review")));
    }
    if p["mergeable"] == "false" {
        parts.push("conflict".into());
    }
    parts.push(s(p, "url"));
    parts.join("  ")
}

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
    if let Some(m) = v["menu"].as_str() {
        meta.push(format!("menu {m}"));
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
    if let Some(r) = v["repos"].as_array().filter(|l| !l.is_empty()) {
        meta.push(format!(
            "repos {}",
            r.iter()
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
    if let Some(prs) = v["prs"].as_array().filter(|p| !p.is_empty()) {
        o.push_str(&format!("\n{}\n", bold("Pull requests")));
        for p in prs {
            o.push_str(&format!("  {}\n", pr_line(p)));
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
    if let Some(qs) = v["questions"].as_array().filter(|q| !q.is_empty()) {
        o.push_str(&format!("\n{}\n", bold("Questions")));
        for x in qs {
            o.push_str(&format!(
                "  {} [{}]{} {}\n",
                s(x, "id"),
                s(x, "status"),
                if x["blocks"] == true { " blocks" } else { "" },
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
    fn bar_renders_fractions_and_never_fills_early() {
        assert_eq!(bar(3, 8, 8), "███░░░░░ 3/8");
        assert_eq!(bar(0, 0, 8), "░░░░░░░░ 0/0");
        assert_eq!(bar(7, 8, 8), "███████░ 7/8");
        assert_eq!(bar(8, 8, 8), "████████ 8/8");
        assert_eq!(bar(1, 8, 8), "█░░░░░░░ 1/8");
    }

    #[test]
    fn menus_table_shows_bar_due_and_counts() {
        let v = json!([
            {"key":"mvp","name":"MVP","status":"open","due_on":"2026-12-01","total":8,"done":3,"open":4,"claimed":1,"blocked":0},
            {"key":"beta","name":"Beta","status":"served","total":0,"done":0,"open":0,"claimed":0,"blocked":2}
        ]);
        let t = menus_table(&v);
        assert!(
            t.contains("███░░░░░ 3/8") && t.contains("2026-12-01"),
            "{t}"
        );
        assert!(t.contains("PROGRESS") && t.contains("BLOCKED") && t.contains("served"));
        assert!(menus_table(&json!([])).starts_with("no menus"));
    }

    #[test]
    fn task_table_and_detail_show_the_menu() {
        let v = json!([{"id":"P-1","status":"open","priority":0,"title":"T","menu":"mvp"}]);
        assert!(tasks_table(&v).contains("mvp"));
        let d =
            task_detail(&json!({"id":"P-1","title":"T","status":"open","priority":0,"menu":"mvp"}));
        assert!(d.contains("menu mvp"));
        let d = task_detail(&json!({"id":"P-1","title":"T","status":"open","priority":0}));
        assert!(!d.contains("menu"));
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

    #[test]
    fn age_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-21T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(age_at("2026-11-21T11:59:30.5+00:00", now), "now");
        assert_eq!(age_at("2026-11-21T11:45:00+00:00", now), "15m");
        assert_eq!(age_at("2026-11-21T07:00:00Z", now), "5h");
        assert_eq!(age_at("2026-11-19T12:00:00Z", now), "2d");
        assert_eq!(age_at("2026-05-21T12:00:00Z", now), "6mo");
        assert_eq!(age_at("garbage", now), "-");
    }

    #[test]
    fn memories_table_ranks_only_when_searching() {
        let m = |rank: Value| {
            json!([{"key":"retries","kind":"gotcha","title":"Retry on 429","tags":["http","api"],
            "updated_by":"ann","updated_at":"2026-11-21T10:00:00Z","rank":rank}])
        };
        let plain_t = memories_table(&m(Value::Null));
        assert!(!plain_t.contains("RANK") && plain_t.contains("http,api"));
        let ranked = memories_table(&m(json!(0.5)));
        assert!(ranked.contains("RANK") && ranked.contains("0.50"));
    }

    #[test]
    fn memory_detail_has_meta_and_body() {
        let d = memory_detail(
            &json!({"key":"k","title":"T","kind":"howto","tags":[],"revision":3,
            "updated_by":"bob","updated_at":"2026-11-21T10:00:00Z","task":"PAY-1","body":"hello\n"}),
        );
        assert!(d.contains("howto") && d.contains("revision 3") && d.contains("task PAY-1"));
        assert!(d.ends_with("hello"));
    }
}
