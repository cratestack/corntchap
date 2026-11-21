//! `corntchap-cli station`: register the agent sessions you work in (OpenCode, Claude Code, Codex)
//! as stations of a cook in a pot. The server wakes an idle station; the fire resumes that exact
//! session in its own folder, so no `[[target]]`/`directory` configuration is needed.

use std::{
    collections::BTreeMap,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::{
    api::Api,
    opencode,
    relay::{self, Target},
    ui::{self, ok_line, s, warn_line},
};

#[derive(Subcommand)]
pub enum StationCmd {
    /// Find the agent sessions of this repo and its worktrees and register them as stations (run it inside a git repo).
    Scan {
        /// The pot (project key); default: the pot whose repo_url matches this repo's `origin`.
        #[arg(long)]
        pot: Option<String>,
        /// The cook to register the sessions for (default: your cook of the runtime's kind).
        #[arg(long)]
        cook: Option<String>,
        /// Register every session found, not only the most recent per runtime and folder.
        #[arg(long)]
        all: bool,
        /// Show what would be registered without registering anything.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// List registered stations.
    Ls {
        #[arg(long)]
        pot: Option<String>,
        #[arg(long)]
        cook: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Unregister a station.
    Rm { id: String },
}

pub async fn run(cmd: StationCmd, api: &Api) -> Result<()> {
    match cmd {
        StationCmd::Scan {
            pot,
            cook,
            all,
            dry_run,
            json,
        } => scan(api, pot, cook, all, dry_run, json).await,
        StationCmd::Ls { pot, cook, json } => {
            let mut q = vec![];
            if let Some(p) = pot {
                q.push(("project", p.to_uppercase()));
            }
            if let Some(c) = cook {
                q.push(("cook", c));
            }
            let v = api
                .send(api.raw(reqwest::Method::GET, "/stations").query(&q))
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", ui::stations_table(&v));
            }
            Ok(())
        }
        StationCmd::Rm { id } => {
            api.delete(&format!("/stations/{id}")).await?;
            ok_line(format!("station {id} removed"));
            Ok(())
        }
    }
}

// ---- git ----

/// `host/path` in lower case, whatever the spelling: https, ssh://, scp-style `git@host:path`,
/// with or without `.git`, port, credentials or a trailing slash.
pub fn normalize_git_url(url: &str) -> String {
    let u = url.trim();
    let rest = match u.split_once("://") {
        Some((_, r)) => {
            let (auth, path) = r.split_once('/').unwrap_or((r, ""));
            let host = auth.rsplit('@').next().unwrap_or(auth);
            let host = host.split(':').next().unwrap_or(host);
            format!("{host}/{path}")
        }
        None => match u.split_once(':') {
            // scp-style: [user@]host:path
            Some((h, p)) if !h.contains('/') => {
                format!("{}/{}", h.rsplit('@').next().unwrap_or(h), p)
            }
            _ => u.to_string(),
        },
    };
    let mut r = rest.trim_end_matches('/').to_lowercase();
    if let Some(x) = r.strip_suffix(".git") {
        r = x.trim_end_matches('/').to_string();
    }
    r.trim_start_matches('/').to_string()
}

/// Keys of the pots whose `repo_url` is this origin.
pub fn match_pots(origin: &str, pots: &Value) -> Vec<String> {
    let want = normalize_git_url(origin);
    pots.as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["repo_url"]
                .as_str()
                .is_some_and(|u| !u.trim().is_empty() && normalize_git_url(u) == want)
        })
        .filter_map(|p| p["key"].as_str().map(String::from))
        .collect()
}

/// Directories of `git worktree list --porcelain` (bare and prunable entries left out).
pub fn parse_worktrees(porcelain: &str) -> Vec<String> {
    let mut out = vec![];
    for block in porcelain.split("\n\n") {
        let mut path = None;
        let mut skip = false;
        for l in block.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                path = Some(p.trim().to_string());
            } else if l == "bare" || l.starts_with("prunable") {
                skip = true;
            }
        }
        if let Some(p) = path
            && !skip
        {
            out.push(p);
        }
    }
    out
}

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .context("running git (is it installed?)")?;
    if !o.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ---- sessions of each runtime ----

/// A session found on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub runtime: &'static str,
    pub session: String,
    pub directory: String,
    /// Milliseconds since the epoch, for "most recent".
    pub updated: i64,
    pub title: Option<String>,
}

/// Claude Code's folder name for a working directory: every non-alphanumeric character becomes `-`.
pub fn claude_dir_name(dir: &str) -> String {
    dir.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn claude_root() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("projects")),
        _ => dirs::home_dir().map(|h| h.join(".claude/projects")),
    }
}

fn codex_root() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("sessions")),
        _ => dirs::home_dir().map(|h| h.join(".codex/sessions")),
    }
}

/// Claude Code sessions: `<root>/<encoded dir>/<session-id>.jsonl`.
pub fn claude_sessions(root: &Path, dirs: &[String]) -> Vec<Found> {
    let mut out = vec![];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(root.join(claude_dir_name(d))) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = p.file_stem().and_then(|x| x.to_str()) else {
                continue;
            };
            out.push(Found {
                runtime: "claude-code",
                session: id.to_string(),
                directory: d.clone(),
                updated: mtime_ms(&p),
                title: None,
            });
        }
    }
    out
}

/// `(id, cwd)` from the first line of a Codex rollout file: `{"type":"session_meta","payload":{"id","cwd"}}`.
pub fn parse_codex_meta(line: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v["type"].as_str() != Some("session_meta") {
        return None;
    }
    let p = if v["payload"].is_object() {
        &v["payload"]
    } else {
        &v
    };
    Some((
        p["id"].as_str()?.to_string(),
        p["cwd"].as_str()?.to_string(),
    ))
}

fn walk(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        {
            out.push(p);
        }
    }
}

fn first_line(p: &Path) -> Option<String> {
    use std::io::BufRead;
    let mut l = String::new();
    std::io::BufReader::new(std::fs::File::open(p).ok()?)
        .read_line(&mut l)
        .ok()?;
    Some(l)
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |x: &str| {
        std::fs::canonicalize(x)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| x.trim_end_matches('/').to_string())
    };
    a.trim_end_matches('/') == b.trim_end_matches('/') || norm(a) == norm(b)
}

/// Codex sessions modified in the last `days` days whose `cwd` is one of `dirs`.
pub fn codex_sessions(root: &Path, dirs: &[String], days: u64) -> Vec<Found> {
    let mut files = vec![];
    walk(root, 5, &mut files);
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days * 86400))
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut out = vec![];
    for f in files {
        let updated = mtime_ms(&f);
        if updated < cutoff {
            continue;
        }
        let Some((id, cwd)) = first_line(&f).and_then(|l| parse_codex_meta(&l)) else {
            continue;
        };
        if let Some(d) = dirs.iter().find(|d| same_path(d, &cwd)) {
            out.push(Found {
                runtime: "codex",
                session: id,
                directory: d.clone(),
                updated,
                title: None,
            });
        }
    }
    out
}

/// OpenCode sessions per directory, with the API version that answered; `Err` explains why none.
async fn opencode_sessions(dirs: &[String]) -> Result<(Vec<Found>, opencode::ApiVersion)> {
    let t = Target::blank("scan", "opencode");
    let ep = opencode::resolve(&t).await?;
    let mut out = vec![];
    for d in dirs {
        match opencode::list_sessions(&ep, Some(d)).await {
            Ok(list) => out.extend(list.into_iter().map(|x| Found {
                runtime: "opencode",
                session: x.id,
                directory: x.directory.unwrap_or_else(|| d.clone()),
                updated: x.updated,
                title: x.title,
            })),
            Err(e) => eprintln!("station: OpenCode sessions of {d}: {e:#}"),
        }
    }
    Ok((out, ep.api))
}

/// The most recent session per (runtime, directory), or everything with `all`; newest first.
pub fn select(mut found: Vec<Found>, all: bool) -> Vec<Found> {
    found.sort_by_key(|f| std::cmp::Reverse(f.updated));
    if all {
        return found;
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|f| seen.insert((f.runtime, f.directory.clone())));
    found
}

fn label_for(f: &Found) -> String {
    let base = Path::new(&f.directory)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&f.directory)
        .to_string();
    let l = match &f.title {
        Some(t) => format!("{base}: {t}"),
        None => base,
    };
    l.chars().take(80).collect()
}

// ---- cooks ----

/// Choose the cook for a runtime among the user's cooks of that kind.
fn pick_cook(runtime: &str, cooks: &[String], tty: bool) -> Result<Option<String>> {
    match cooks {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        many if tty => {
            eprintln!("Several cooks run {runtime}:");
            for (i, c) in many.iter().enumerate() {
                eprintln!("  {}) {c}", i + 1);
            }
            eprint!(
                "Register {runtime} sessions for which cook? [1-{}] ",
                many.len()
            );
            let _ = std::io::stderr().flush();
            let mut a = String::new();
            std::io::stdin().read_line(&mut a)?;
            let a = a.trim();
            let i = a
                .parse::<usize>()
                .ok()
                .filter(|i| (1..=many.len()).contains(i));
            match (i, many.iter().find(|c| c.as_str() == a)) {
                (Some(i), _) => Ok(Some(many[i - 1].clone())),
                (_, Some(c)) => Ok(Some(c.clone())),
                _ => bail!("no such choice: pass --cook NAME"),
            }
        }
        many => bail!(
            "several cooks run {runtime} ({}): pass --cook NAME",
            many.join(", ")
        ),
    }
}

// ---- scan ----

async fn scan(
    api: &Api,
    pot: Option<String>,
    cook: Option<String>,
    all: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = git(&cwd, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        anyhow!("not inside a git repository: run `station scan` from your checkout")
    })?;
    let mut dirs: Vec<String> = vec![root.clone()];
    if let Ok(w) = git(Path::new(&root), &["worktree", "list", "--porcelain"]) {
        for d in parse_worktrees(&w) {
            if Path::new(&d).is_dir() && !dirs.iter().any(|x| same_path(x, &d)) {
                dirs.push(d);
            }
        }
    }
    // the pot
    let pots = api.get("/projects").await?;
    let pot = match pot {
        Some(p) => p.to_uppercase(),
        None => {
            let origin = git(Path::new(&root), &["remote", "get-url", "origin"]).map_err(|_| {
                anyhow!(
                    "this repo has no `origin` remote to match against the pots' repo_url: pass --pot KEY\n{}",
                    pot_list(&pots)
                )
            })?;
            let m = match_pots(&origin, &pots);
            match m.as_slice() {
                [one] => one.clone(),
                [] => bail!(
                    "no pot has repo_url {origin}: pass --pot KEY, or set the pot's repo_url\n{}",
                    pot_list(&pots)
                ),
                many => bail!(
                    "several pots have repo_url {origin} ({}): pass --pot KEY",
                    many.join(", ")
                ),
            }
        }
    };
    if !json {
        eprintln!(
            "scanning {} folder(s) of {} for pot {pot}",
            dirs.len(),
            root
        );
    }

    // sessions
    let mut found: Vec<Found> = vec![];
    match opencode_sessions(&dirs).await {
        Ok((f, api_v)) => {
            if !json {
                eprintln!("  opencode: {} session(s) ({})", f.len(), api_v.label());
            }
            found.extend(f);
        }
        Err(e) => {
            if !json {
                eprintln!("  opencode: skipped ({e:#})");
            }
        }
    }
    if let Some(r) = claude_root() {
        let f = claude_sessions(&r, &dirs);
        if !json {
            eprintln!("  claude-code: {} session(s) in {}", f.len(), r.display());
        }
        found.extend(f);
    }
    if let Some(r) = codex_root() {
        let f = codex_sessions(&r, &dirs, 30);
        if !json {
            eprintln!("  codex: {} session(s) in {}", f.len(), r.display());
        }
        found.extend(f);
    }
    let chosen = select(found, all);
    if chosen.is_empty() {
        if json {
            println!("[]");
        } else {
            println!(
                "no agent sessions found for these folders: start OpenCode, Claude Code or Codex in one, then scan again"
            );
        }
        return Ok(());
    }

    // cooks, per runtime
    let agents = api.get("/agents").await?;
    let tty = std::io::stdin().is_terminal();
    let mut cook_of: BTreeMap<&'static str, Option<String>> = BTreeMap::new();
    for f in &chosen {
        if cook_of.contains_key(f.runtime) {
            continue;
        }
        let c = match &cook {
            Some(c) => Some(c.clone()),
            None => {
                let names: Vec<String> = agents
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| {
                        a["agent_kind"].as_str() == Some(f.runtime) && a["disabled"] != true
                    })
                    .filter_map(|a| a["name"].as_str().map(String::from))
                    .collect();
                let c = pick_cook(f.runtime, &names, tty)?;
                if c.is_none() {
                    warn_line(format!(
                        "no cook of kind {rt}: create one with `corntchap-cli cook setup <name> --kind {rt}`; skipping its sessions",
                        rt = f.runtime
                    ));
                }
                c
            }
        };
        cook_of.insert(f.runtime, c);
    }

    let mut rows = vec![];
    for f in &chosen {
        let Some(Some(c)) = cook_of.get(f.runtime) else {
            continue;
        };
        let mut row = json!({
            "runtime": f.runtime, "cook": c, "project": pot, "session": f.session,
            "directory": f.directory, "label": label_for(f), "station": null,
            "dry_run": dry_run,
        });
        if !dry_run {
            let r = api
                .post(
                    "/stations",
                    json!({
                        "cook": c, "project": pot, "runtime": f.runtime, "session": f.session,
                        "directory": f.directory, "label": label_for(f),
                    }),
                )
                .await
                .with_context(|| format!("registering {} session {}", f.runtime, f.session))?;
            row["station"] = r["id"].clone();
        }
        rows.push(row);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("{}", scan_table(&rows));
        if dry_run {
            println!("dry run: nothing was registered");
        } else {
            ok_line(format!(
                "{} station(s) registered in {pot}; the fire resumes these sessions when work lands",
                rows.len()
            ));
        }
    }
    Ok(())
}

fn pot_list(pots: &Value) -> String {
    let l: Vec<String> = pots
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            format!(
                "  {}  {}",
                s(p, "key"),
                p["repo_url"].as_str().unwrap_or("(no repo_url)")
            )
        })
        .collect();
    if l.is_empty() {
        "no pots yet".into()
    } else {
        format!("Pots:\n{}", l.join("\n"))
    }
}

fn scan_table(rows: &[Value]) -> String {
    let cells = rows
        .iter()
        .map(|r| {
            vec![
                ui::plain(s(r, "runtime")),
                ui::plain(s(r, "cook")),
                ui::plain(s(r, "project")),
                ui::plain(s(r, "directory")),
                ui::plain(s(r, "session").chars().take(18).collect::<String>()),
                if r["dry_run"] == true {
                    ui::dim_cell("would register")
                } else {
                    ui::plain(s(r, "station"))
                },
            ]
        })
        .collect();
    ui::table(
        &["runtime", "cook", "pot", "directory", "session", "station"],
        cells,
        "nothing to register",
    )
}

// ---- fire doctor ----

/// For `fire doctor`: this user's stations and whether each runtime is reachable from here.
pub async fn doctor(server: &str) {
    let mut api = match Api::from_creds(false) {
        Ok(a) => a,
        Err(_) => {
            println!(
                "--   stations not checked: sign in first (`corntchap-cli login <server-url>`)"
            );
            return;
        }
    };
    api.server = server.trim_end_matches('/').to_string();
    let list = match api.get("/stations").await {
        Ok(v) => v,
        Err(e) => {
            println!("FAIL stations: {e:#}");
            return;
        }
    };
    let stations = list.as_array().cloned().unwrap_or_default();
    // OpenCode: one lookup for all stations
    let oc = if stations.iter().any(|x| x["runtime"] == "opencode") {
        let r = opencode::resolve(&Target::blank("doctor", "opencode")).await;
        match &r {
            Ok(ep) => println!(
                "ok   OpenCode at {} ({}, {})",
                ep.base,
                ep.api.label(),
                ep.source
            ),
            Err(e) => println!("FAIL OpenCode: {e:#}"),
        }
        Some(r)
    } else {
        None
    };
    if stations.is_empty() {
        println!("--   no stations yet: run `corntchap-cli station scan` inside a repo");
        return;
    }
    let targets = relay::load_config().map(|c| c.target).unwrap_or_default();
    for st in &stations {
        let (id, cook) = (s(st, "id"), s(st, "cook"));
        let runtime = st["runtime"].as_str().unwrap_or("?");
        let dir = st["directory"].as_str();
        let (ok, msg) = match runtime {
            "opencode" => match oc.as_ref() {
                Some(Ok(ep)) => {
                    match opencode::session_exists(ep, st["session"].as_str().unwrap_or("")).await {
                        Some(true) => (true, "session found".to_string()),
                        Some(false) => (
                            false,
                            "OpenCode has no such session: re-scan with `station scan`".into(),
                        ),
                        None => (
                            true,
                            "OpenCode reachable, could not check the session".into(),
                        ),
                    }
                }
                _ => (false, "OpenCode not reachable".into()),
            },
            "claude-code" | "codex" => {
                let bin = if runtime == "codex" {
                    "codex"
                } else {
                    "claude"
                };
                let dir_ok = dir.is_none_or(|d| Path::new(&relay::expand(d)).is_dir());
                match (relay::which(bin), dir_ok) {
                    (true, true) => (true, format!("`{bin}` on PATH, directory exists")),
                    (false, _) => (false, format!("`{bin}` not found on PATH")),
                    (true, false) => (false, format!("directory {} is gone", dir.unwrap_or("?"))),
                }
            }
            _ => {
                if targets.iter().any(|t| t.agent == cook) {
                    (true, "woken through its [[target]]".into())
                } else {
                    (
                        false,
                        "runtime `other` needs a [[target]] in config.toml".into(),
                    )
                }
            }
        };
        println!(
            "{}   {id} {cook} {runtime} {}: {msg}",
            if ok { "ok  " } else { "FAIL" },
            dir.unwrap_or("-")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cc-station-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn git_urls_normalize_to_one_form() {
        let want = "github.com/acme/pay";
        for u in [
            "https://github.com/acme/pay.git",
            "https://github.com/Acme/Pay",
            "https://user:tok@github.com/acme/pay.git/",
            "git@github.com:acme/pay.git",
            "ssh://git@github.com/acme/pay.git",
            "ssh://git@github.com:22/acme/pay",
            "GIT@GitHub.com:ACME/pay",
        ] {
            assert_eq!(normalize_git_url(u), want, "{u}");
        }
        assert_ne!(normalize_git_url("https://github.com/acme/pay2"), want);
    }

    #[test]
    fn pots_match_by_normalized_repo_url() {
        let pots = json!([
            {"key": "PAY", "repo_url": "git@github.com:acme/pay.git"},
            {"key": "WEB", "repo_url": "https://github.com/acme/web"},
            {"key": "NOURL", "repo_url": null},
        ]);
        assert_eq!(
            match_pots("https://github.com/Acme/pay.git", &pots),
            ["PAY"]
        );
        assert!(match_pots("https://github.com/acme/other", &pots).is_empty());
    }

    #[test]
    fn worktrees_are_parsed_from_porcelain() {
        let p = "worktree /home/a/pay\nHEAD abc\nbranch refs/heads/main\n\nworktree /home/a/pay-fix\nHEAD def\ndetached\n\nworktree /home/a/old\nHEAD 1\nprunable gitdir file points to non-existent location\n\nworktree /srv/pay.git\nbare\n";
        assert_eq!(parse_worktrees(p), ["/home/a/pay", "/home/a/pay-fix"]);
    }

    #[test]
    fn claude_dirs_replace_every_non_alphanumeric() {
        assert_eq!(claude_dir_name("/home/a/pay"), "-home-a-pay");
        assert_eq!(
            claude_dir_name("/Users/x/my.repo_2/feat"),
            "-Users-x-my-repo-2-feat"
        );
    }

    #[test]
    fn codex_meta_is_read_from_the_first_line() {
        let l = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"0199-abc","cwd":"/home/a/pay","originator":"codex_cli"}}"#;
        assert_eq!(
            parse_codex_meta(l),
            Some(("0199-abc".into(), "/home/a/pay".into()))
        );
        assert_eq!(
            parse_codex_meta(r#"{"type":"event_msg","payload":{}}"#),
            None
        );
        assert_eq!(parse_codex_meta("not json"), None);
    }

    #[test]
    fn claude_and_codex_sessions_are_found_on_disk() {
        let home = tmp("sessions");
        let proj = home.join("claude/-work-pay");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("sess-1.jsonl"), "{}\n").unwrap();
        std::fs::write(proj.join("notes.txt"), "x").unwrap();
        let dirs = vec!["/work/pay".to_string(), "/work/pay-fix".to_string()];
        let c = claude_sessions(&home.join("claude"), &dirs);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].session, "sess-1");
        assert_eq!(c[0].directory, "/work/pay");

        let day = home.join("codex/2026/01/02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-2026-01-02T00-00-00-x.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"cx-1\",\"cwd\":\"/work/pay-fix\"}}\n{}\n",
        )
        .unwrap();
        std::fs::write(
            day.join("rollout-2026-01-02T00-00-01-y.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"cx-2\",\"cwd\":\"/elsewhere\"}}\n",
        )
        .unwrap();
        let x = codex_sessions(&home.join("codex"), &dirs, 30);
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].session, "cx-1");
        assert_eq!(x[0].directory, "/work/pay-fix");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn select_keeps_the_most_recent_per_runtime_and_folder() {
        let f = |rt, s: &str, d: &str, u| Found {
            runtime: rt,
            session: s.into(),
            directory: d.into(),
            updated: u,
            title: None,
        };
        let all = vec![
            f("claude-code", "a", "/x", 1),
            f("claude-code", "b", "/x", 5),
            f("codex", "c", "/x", 2),
            f("claude-code", "d", "/y", 3),
        ];
        let one: Vec<String> = select(all.clone(), false)
            .into_iter()
            .map(|f| f.session)
            .collect();
        assert_eq!(one, ["b", "d", "c"]);
        assert_eq!(select(all, true).len(), 4);
    }

    #[test]
    fn cook_choice_needs_a_flag_without_a_terminal() {
        assert_eq!(pick_cook("codex", &[], false).unwrap(), None);
        assert_eq!(
            pick_cook("codex", &["a".into()], false).unwrap(),
            Some("a".into())
        );
        let e = pick_cook("codex", &["a".into(), "b".into()], false).unwrap_err();
        assert!(e.to_string().contains("--cook"));
    }
}
