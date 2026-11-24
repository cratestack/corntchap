//! `corntchap-cli what-now`: what should this cook and station do now (the `what_now` tool), and
//! `corntchap-cli pot guide`: read, set or clear a pot's guide.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    api::Api,
    station::{resolve_pot, work_folder},
    ui::{ok_line, s},
};

fn items(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn task_line(t: &Value) -> String {
    format!("  {} {}", s(t, "id"), s(t, "title"))
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

/// The briefing as text.
pub fn text(v: &Value) -> String {
    let mut o = format!("{}\n", s(v, "next"));
    if let Some(st) = v["station"].as_str() {
        o.push_str(&format!(
            "\nstation {st}{}{}\n",
            v["station_repo"]
                .as_str()
                .map(|r| format!(" on {r}"))
                .unwrap_or_default(),
            if v["confined"] == true {
                " (confined)"
            } else {
                ""
            }
        ));
    }
    section(
        &mut o,
        "holding",
        items(&v["holding"]).iter().map(task_line).collect(),
    );
    section(
        &mut o,
        "parked for you",
        items(&v["parked"]).iter().map(task_line).collect(),
    );
    let mut waiting: Vec<String> = items(&v["waiting_on_you"]["questions"])
        .iter()
        .map(|q| format!("  {} {}", s(q, "id"), s(q, "title")))
        .collect();
    waiting.extend(items(&v["waiting_on_you"]["comments"]).iter().map(|c| {
        format!(
            "  {} comment from {}: {}",
            s(c, "task"),
            s(c, "author"),
            s(c, "excerpt")
        )
    }));
    section(&mut o, "waiting on you", waiting);
    section(
        &mut o,
        &format!("ready here ({})", v["ready_here_total"]),
        items(&v["ready_here"]).iter().map(task_line).collect(),
    );
    let mut elsewhere = vec![];
    for g in items(&v["ready_elsewhere"]) {
        elsewhere.push(format!(
            "  {}: {} ready ({}{})",
            s(g, "repo"),
            g["count"],
            s(g, "url"),
            g["default_branch"]
                .as_str()
                .map(|b| format!(", branch {b}"))
                .unwrap_or_default()
        ));
        elsewhere.extend(
            items(&g["top"])
                .iter()
                .map(|t| format!("  {}", task_line(t))),
        );
    }
    section(&mut o, "ready elsewhere", elsewhere);
    let f = &v["folder_free"];
    let mut free: Vec<String> = vec![];
    for (k, label) in [
        ("open_questions", "question"),
        ("needs_split", "epic to split"),
        ("triage", "triage"),
        ("stale", "stale claim"),
        ("no_repo", "no repository"),
    ] {
        free.extend(
            items(&f[k])
                .iter()
                .map(|t| format!("  [{label}] {} {}", s(t, "id"), s(t, "title"))),
        );
    }
    if f["pantry_news"].as_u64().unwrap_or(0) > 0 {
        free.push(format!("  [pantry] {} change(s) to read", f["pantry_news"]));
    }
    section(&mut o, "needs no checkout", free);
    if let Some(g) = v["guide"].as_str() {
        o.push_str(&format!("\nguide\n{g}\n"));
    }
    o
}

/// `what-now`: the pot comes from `--pot`, else from this folder's git `origin`.
pub async fn what_now(
    api: &Api,
    pot: Option<String>,
    station: Option<String>,
    json_out: bool,
) -> Result<()> {
    let root = work_folder()?;
    let pot = resolve_pot(api, &root, pot).await?;
    let mut path = format!("/projects/{pot}/what-now");
    if let Some(st) = station.filter(|s| !s.trim().is_empty()) {
        path.push_str(&format!("?station={}", st.trim()));
    }
    let v = api.get(&path).await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", text(&v));
    }
    Ok(())
}

/// `pot guide KEY [--file F | --clear]`: print, set or clear the pot's guide.
pub async fn guide(api: &Api, key: &str, file: Option<String>, clear: bool) -> Result<()> {
    let key = key.to_uppercase();
    if clear || file.is_some() {
        let body = match (&file, clear) {
            (Some(_), true) => bail!("--file and --clear do not go together"),
            (Some(f), false) if f == "-" => {
                let mut b = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut b)?;
                b
            }
            (Some(f), false) => {
                std::fs::read_to_string(f).with_context(|| format!("reading {f}"))?
            }
            _ => String::new(),
        };
        let v = api
            .procedure("setPotGuide", json!({"project": key, "guide": body}))
            .await?;
        ok_line(if v["guide"].is_null() {
            format!("pot {key}: guide cleared")
        } else {
            format!("pot {key}: guide set ({} characters)", v["length"])
        });
        return Ok(());
    }
    let pots = api.get("/projects").await?;
    let pot = items(&pots).iter().find(|p| s(p, "key") == key);
    match pot.and_then(|p| p["guide"].as_str()) {
        Some(g) => println!("{g}"),
        None => println!("pot {key} has no guide yet (set one with --file guide.md)"),
    }
    Ok(())
}
