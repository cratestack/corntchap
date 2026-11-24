//! `corntchap-cli station`: register the agent sessions you work in (OpenCode, Claude Code, Codex, and best effort Copilot CLI and Antigravity)
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
    /// Find the agent sessions of this folder (in a git checkout: its root and worktrees) and register them as stations.
    Scan {
        /// The pot (project key); default: the pot with a repository matching this checkout's git `origin`.
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
        /// The session cannot leave its folder (a sandbox, a harness locked to one directory): `claim_next` from it hands out only tasks about its repository or about none, and the dispatcher keeps other repositories' work for other sessions.
        #[arg(long)]
        confined: bool,
        #[arg(long)]
        json: bool,
    },
    /// Register one session by its id, for harnesses `scan` cannot read: the Copilot CLI session id
    /// (`copilot --resume=<id>`) or the Antigravity conversation id (`agy --output-format json`).
    Add {
        /// opencode | claude-code | codex | copilot | antigravity
        #[arg(long)]
        runtime: String,
        /// The runtime's own session / conversation id.
        #[arg(long)]
        session: String,
        /// The folder the session works in (default: this one).
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        pot: Option<String>,
        /// The cook to register it for (default: your cook of the runtime's kind).
        #[arg(long)]
        cook: Option<String>,
        #[arg(long)]
        label: Option<String>,
        /// The session cannot leave its folder (a sandbox, a harness locked to one directory): `claim_next` from it hands out only tasks about its repository or about none, and the dispatcher keeps other repositories' work for other sessions.
        #[arg(long)]
        confined: bool,
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
    /// Make a station its cook's main one in its pot: the session that answers messages,
    /// mentions and question answers (without one, the session woken last answers).
    #[command(name = "main", alias = "pin")]
    Main {
        id: String,
        /// Stop it being the main station.
        #[arg(long)]
        off: bool,
    },
    /// Stop a station being the main one (same as `main <id> --off`).
    #[command(name = "unpin")]
    Unpin { id: String },
    /// Start a new Claude Code session in this folder (`claude -p`, in the background) and register
    /// it as a station, so the fire can wake it later. With --fire, the cook's relay starts it
    /// instead, in a folder where the cook already has a station (works from any machine).
    Start {
        /// What to tell the session, on top of the task.
        prompt: Vec<String>,
        /// A task it should take, e.g. PAY-12.
        #[arg(long)]
        task: Option<String>,
        /// The pot; default: the pot with a repository matching this repo's `origin`.
        #[arg(long)]
        pot: Option<String>,
        /// The cook it works as (default: your Claude Code cook).
        #[arg(long)]
        cook: Option<String>,
        #[arg(long)]
        label: Option<String>,
        /// Wait for the first run to finish, showing its output.
        #[arg(long)]
        wait: bool,
        /// Wait for the first run, then open the session in Claude Desktop.
        #[arg(long, conflicts_with = "fire")]
        desktop: bool,
        /// Let the cook's relay start it (needs --pot and --cook outside a repo; --dir picks one of its station folders).
        #[arg(long, conflicts_with = "wait")]
        fire: bool,
        /// With --fire: the folder, one of the cook's station folders in the pot.
        #[arg(long, requires = "fire")]
        dir: Option<String>,
        /// The new session cannot leave this folder (see `station scan --confined`). Not with --fire.
        #[arg(long, conflicts_with = "fire")]
        confined: bool,
        #[arg(long)]
        json: bool,
    },
    /// Open a station's session on this machine: in Claude Desktop (Claude Code), or with
    /// --terminal in a terminal (`claude --resume`, `codex resume`).
    Open {
        id: String,
        #[arg(long)]
        terminal: bool,
    },
}

pub async fn run(cmd: StationCmd, api: &Api) -> Result<()> {
    match cmd {
        StationCmd::Scan {
            pot,
            cook,
            all,
            dry_run,
            confined,
            json,
        } => scan(api, pot, cook, all, dry_run, confined, json).await,
        StationCmd::Add {
            runtime,
            session,
            dir,
            pot,
            cook,
            label,
            confined,
            json,
        } => {
            add(
                api, &runtime, &session, dir, pot, cook, label, confined, json,
            )
            .await
        }
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
        StationCmd::Main { id, off } => set_main(api, &id, !off).await,
        StationCmd::Unpin { id } => set_main(api, &id, false).await,
        StationCmd::Start {
            prompt,
            task,
            pot,
            cook,
            label,
            wait,
            desktop,
            fire,
            dir,
            confined,
            json,
        } => {
            let prompt = Some(prompt.join(" ")).filter(|p| !p.trim().is_empty());
            let s = StartArgs {
                prompt,
                task,
                pot,
                cook,
                label,
                wait: wait || desktop,
                desktop,
                confined,
                json,
            };
            if fire {
                start_by_fire(api, s, dir).await
            } else {
                start_here(api, s).await
            }
        }
        StationCmd::Open { id, terminal } => open(api, &id, terminal).await,
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

/// A pot's repositories as `(alias, url)`: its `repos`, else (an older server) its single `repo_url`.
fn pot_repos(p: &Value) -> Vec<(String, String)> {
    match p["repos"].as_array() {
        Some(r) => r
            .iter()
            .filter_map(|x| {
                Some((
                    x["name"].as_str()?.to_string(),
                    x["url"].as_str()?.to_string(),
                ))
            })
            .collect(),
        None => p["repo_url"]
            .as_str()
            .map(|u| vec![(String::new(), u.to_string())])
            .unwrap_or_default(),
    }
}

/// Keys of the pots that have a repository (any of them) with this origin.
pub fn match_pots(origin: &str, pots: &Value) -> Vec<String> {
    let want = normalize_git_url(origin);
    pots.as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            pot_repos(p)
                .iter()
                .any(|(_, u)| !u.trim().is_empty() && normalize_git_url(u) == want)
        })
        .filter_map(|p| p["key"].as_str().map(String::from))
        .collect()
}

/// The alias of the repository of pot `key` that has this origin, if any.
pub fn match_repo(origin: &str, pots: &Value, key: &str) -> Option<String> {
    let want = normalize_git_url(origin);
    pots.as_array()?
        .iter()
        .filter(|p| {
            p["key"]
                .as_str()
                .is_some_and(|k| k.eq_ignore_ascii_case(key))
        })
        .flat_map(pot_repos)
        .find(|(a, u)| !a.is_empty() && !u.trim().is_empty() && normalize_git_url(u) == want)
        .map(|(a, _)| a)
}

/// The repository (alias) of `pot` that the folder `dir` is a checkout of, by its `origin`.
/// Best effort: no origin, no match or no network is simply `None`.
async fn repo_alias_of(api: &Api, dir: &str, pot: &str) -> Option<String> {
    let origin = git(Path::new(dir), &["remote", "get-url", "origin"]).ok()?;
    let pots = api.get("/projects").await.ok()?;
    match_repo(&origin, &pots, pot)
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

pub fn claude_root() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("projects")),
        _ => dirs::home_dir().map(|h| h.join(".claude/projects")),
    }
}

/// GitHub Copilot CLI's state folder (`COPILOT_HOME`, else `~/.copilot`) and its `session-state`.
pub fn copilot_root() -> Option<PathBuf> {
    match std::env::var_os("COPILOT_HOME") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d).join("session-state")),
        _ => dirs::home_dir().map(|h| h.join(".copilot/session-state")),
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

/// The working folder of a Copilot CLI session folder, best effort: the docs only promise
/// `session-state/<id>/events.jsonl`, so look for a `cwd:` line in `workspace.yaml` (seen in the wild,
/// not documented) and else a `cwd` field anywhere in the first lines of `events.jsonl`.
pub fn copilot_cwd(session_dir: &Path) -> Option<String> {
    if let Ok(y) = std::fs::read_to_string(session_dir.join("workspace.yaml")) {
        for l in y.lines() {
            if let Some(v) = l.strip_prefix("cwd:") {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    fn find(v: &Value) -> Option<String> {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(c)) = m.get("cwd") {
                    return Some(c.clone());
                }
                m.values().find_map(find)
            }
            Value::Array(a) => a.iter().find_map(find),
            _ => None,
        }
    }
    use std::io::BufRead;
    let f = std::fs::File::open(session_dir.join("events.jsonl")).ok()?;
    std::io::BufReader::new(f)
        .lines()
        .take(20)
        .map_while(|l| l.ok())
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .find_map(|v| find(&v))
}

/// Copilot CLI sessions modified in the last `days` days whose folder is one of `dirs`:
/// `<root>/<session-id>/`. Best effort (see `copilot_cwd`); `station add` registers the rest.
pub fn copilot_sessions(root: &Path, dirs: &[String], days: u64) -> Vec<Found> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(days * 86400))
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let Ok(rd) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        let Some(id) = p.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        if !p.is_dir() {
            continue;
        }
        let ev = p.join("events.jsonl");
        let updated = mtime_ms(if ev.is_file() { &ev } else { &p });
        if updated < cutoff {
            continue;
        }
        let Some(cwd) = copilot_cwd(&p) else {
            continue;
        };
        if let Some(d) = dirs.iter().find(|d| same_path(d, &cwd)) {
            out.push(Found {
                runtime: "copilot",
                session: id,
                directory: d.clone(),
                updated,
                title: None,
            });
        }
    }
    out
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
    confined: bool,
    json: bool,
) -> Result<()> {
    let root = work_folder()?;
    let mut dirs: Vec<String> = vec![root.clone()];
    if let Ok(w) = git(Path::new(&root), &["worktree", "list", "--porcelain"]) {
        for d in parse_worktrees(&w) {
            if Path::new(&d).is_dir() && !dirs.iter().any(|x| same_path(x, &d)) {
                dirs.push(d);
            }
        }
    }
    let pot = resolve_pot(api, &root, pot).await?;
    if !json {
        eprintln!(
            "scanning {} folder(s) of {} for pot {pot}",
            dirs.len(),
            root
        );
    }
    let repo = repo_alias_of(api, &root, &pot).await;
    scan_found(api, &dirs, pot, repo, cook, all, dry_run, confined, json).await
}

/// Where the sessions work: the git checkout's root when there is one (its worktrees are covered
/// too), else this very folder. Repositories are a convenience, never a requirement: a pot can
/// span many repos, or none.
pub(crate) fn work_folder() -> Result<String> {
    let cwd = std::env::current_dir()?;
    Ok(git(&cwd, &["rev-parse", "--show-toplevel"]).unwrap_or_else(|_| cwd.display().to_string()))
}

/// The pot of a folder: the one given, else the pot with a repository matching its git `origin`.
pub(crate) async fn resolve_pot(api: &Api, root: &str, pot: Option<String>) -> Result<String> {
    let pots = api.get("/projects").await?;
    Ok(match pot {
        Some(p) => p.to_uppercase(),
        None => {
            let origin = git(Path::new(root), &["remote", "get-url", "origin"]).map_err(|_| {
                anyhow!(
                    "which pot? pass --pot KEY (this folder has no git `origin` to match against a pot's repositories)\n{}",
                    pot_list(&pots)
                )
            })?;
            let m = match_pots(&origin, &pots);
            match m.as_slice() {
                [one] => one.clone(),
                [] => bail!(
                    "no pot has a repository {origin}: pass --pot KEY, or add it to the pot (`corntchap-cli pot repo add KEY URL`)\n{}",
                    pot_list(&pots)
                ),
                many => bail!(
                    "several pots have a repository {origin} ({}): pass --pot KEY",
                    many.join(", ")
                ),
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn scan_found(
    api: &Api,
    dirs: &[String],
    pot: String,
    repo: Option<String>,
    cook: Option<String>,
    all: bool,
    dry_run: bool,
    confined: bool,
    json: bool,
) -> Result<()> {
    let dirs = dirs.to_vec();

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
    if let Some(r) = copilot_root() {
        let f = copilot_sessions(&r, &dirs, 30);
        if !json {
            eprintln!("  copilot: {} session(s) in {}", f.len(), r.display());
        }
        found.extend(f);
    }
    let chosen = select(found, all);
    if chosen.is_empty() {
        if json {
            println!("[]");
        } else {
            println!(
                "no agent sessions found for these folders: start OpenCode, Claude Code, Codex or the Copilot CLI in one, then scan again (Antigravity: `station add`)"
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
            "directory": f.directory, "label": label_for(f), "repo": repo, "station": null,
            "dry_run": dry_run,
        });
        if !dry_run {
            let r = api
                .post(
                    "/stations",
                    json!({
                        "cook": c, "project": pot, "runtime": f.runtime, "session": f.session,
                        "directory": f.directory, "label": label_for(f), "repo": repo,
                        "confined": confined.then_some(true),
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

/// `station add`: register one session by id.
#[allow(clippy::too_many_arguments)]
async fn add(
    api: &Api,
    runtime: &str,
    session: &str,
    dir: Option<String>,
    pot: Option<String>,
    cook: Option<String>,
    label: Option<String>,
    confined: bool,
    json: bool,
) -> Result<()> {
    let runtime = runtime.trim().to_lowercase();
    if !["opencode", "claude-code", "codex", "copilot", "antigravity"].contains(&runtime.as_str()) {
        bail!("--runtime: one of opencode, claude-code, codex, copilot, antigravity");
    }
    let dir = match dir {
        Some(d) => relay::expand(&d),
        None => work_folder()?,
    };
    let pot = resolve_pot(api, &dir, pot).await?;
    let cook = match cook {
        Some(c) => c,
        None => {
            let agents = api.get("/agents").await?;
            let names: Vec<String> = agents
                .as_array()
                .into_iter()
                .flatten()
                .filter(|a| {
                    a["agent_kind"].as_str() == Some(runtime.as_str()) && a["disabled"] != true
                })
                .filter_map(|a| a["name"].as_str().map(String::from))
                .collect();
            pick_cook(&runtime, &names, std::io::stdin().is_terminal())?.ok_or_else(|| {
                anyhow!("no {runtime} cook: create one with `corntchap-cli cook setup <name> --kind {runtime}`, or pass --cook")
            })?
        }
    };
    let label = label.unwrap_or_else(|| {
        Path::new(&dir)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("session")
            .to_string()
    });
    let repo = repo_alias_of(api, &dir, &pot).await;
    let r = api
        .post(
            "/stations",
            serde_json::json!({
                "cook": cook, "project": pot, "runtime": runtime, "session": session,
                "directory": dir, "label": label, "repo": repo,
                "confined": confined.then_some(true),
            }),
        )
        .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        ok_line(format!(
            "station {} registered for {cook} in {pot} ({runtime}, {dir})",
            r["id"].as_str().unwrap_or("?")
        ));
    }
    Ok(())
}

fn pot_list(pots: &Value) -> String {
    let l: Vec<String> = pots
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let urls: Vec<String> = pot_repos(p).into_iter().map(|(_, u)| u).collect();
            format!(
                "  {}  {}",
                s(p, "key"),
                if urls.is_empty() {
                    "(no repositories)".to_string()
                } else {
                    urls.join(", ")
                }
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
            "claude-code" | "codex" | "copilot" | "antigravity" => {
                let bin = match runtime {
                    "codex" => "codex",
                    "copilot" => "copilot",
                    "antigravity" => "agy",
                    _ => "claude",
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

// ---- start / open ----

pub struct StartArgs {
    prompt: Option<String>,
    task: Option<String>,
    pot: Option<String>,
    cook: Option<String>,
    label: Option<String>,
    wait: bool,
    desktop: bool,
    confined: bool,
    json: bool,
}

/// The first prompt of a session started here (the server words it the same way for --fire).
pub fn start_text(
    who: &str,
    pot: &str,
    station: &str,
    task: Option<&str>,
    prompt: Option<&str>,
) -> String {
    let mut t = format!(
        "{who} started this session for you in corntchap pot {pot}. Use the corntchap MCP tools."
    );
    match task {
        Some(r) => t.push_str(&format!(
            " Your task is {r}: read it (get_task), claim it (claim_task with your station), work it, tick its items and complete it. If it is taken, say so on the pass and take the next ready one (claim_next)."
        )),
        None if prompt.is_none() => t.push_str(
            " Take the next ready task (claim_next with your station), work it and complete it; when none is ready, stop.",
        ),
        None => {}
    }
    if let Some(p) = prompt {
        t.push_str(&format!("\n\n{p}"));
    }
    t.push_str(&format!(
        "\n\n(You are station {station}: pass `station: \"{station}\"` to pass, comment, ask and claim_next, so people see which session answers.)"
    ));
    t
}

/// Your Claude Code cook: the one given, else the only one (or a choice on a terminal).
async fn claude_cook(api: &Api, cook: Option<String>) -> Result<String> {
    if let Some(c) = cook {
        return Ok(c);
    }
    let agents = api.get("/agents").await?;
    let names: Vec<String> = agents
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["agent_kind"].as_str() == Some("claude-code") && a["disabled"] != true)
        .filter_map(|a| a["name"].as_str().map(String::from))
        .collect();
    pick_cook("claude-code", &names, std::io::stdin().is_terminal())?.ok_or_else(|| {
        anyhow!("no Claude Code cook: create one with `corntchap-cli cook setup <name> --kind claude-code`, or pass --cook")
    })
}

fn log_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("corntchap/sessions")
}

async fn start_here(api: &Api, a: StartArgs) -> Result<()> {
    if !relay::which("claude") {
        bail!(
            "`claude` is not on PATH: install Claude Code first (or use --fire to start it on the cook's machine)"
        );
    }
    let root = work_folder()?;
    let pot = resolve_pot(api, &root, a.pot).await?;
    let cook = claude_cook(api, a.cook).await?;
    let session = uuid::Uuid::new_v4().to_string();
    let base = Path::new(&root)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.clone());
    let task = a
        .task
        .map(|t| t.trim().to_uppercase())
        .filter(|t| !t.is_empty());
    let label = a.label.unwrap_or_else(|| match &task {
        Some(t) => format!("{base} ({t})"),
        None => format!("{base} (new)"),
    });
    let repo = repo_alias_of(api, &root, &pot).await;
    let st = api
        .post(
            "/stations",
            json!({"cook": cook, "project": pot, "runtime": "claude-code", "session": session,
                   "directory": root, "label": label, "repo": repo,
                   "confined": a.confined.then_some(true)}),
        )
        .await
        .context("registering the new session")?;
    let id = st["id"].as_str().unwrap_or("?").to_string();
    let who = api
        .get("/me")
        .await
        .ok()
        .and_then(|m| m["name"].as_str().map(String::from))
        .unwrap_or_else(|| "Your chef".into());
    let text = start_text(&who, &pot, &id, task.as_deref(), a.prompt.as_deref());
    let mut c = tokio::process::Command::new(relay::program("claude"));
    c.env("PATH", relay::search_path());
    c.args(["-p", "--session-id", &session])
        .current_dir(&root)
        .stdin(std::process::Stdio::piped());
    let log = log_dir().join(format!("{session}.log"));
    if !a.wait {
        #[cfg(unix)]
        c.process_group(0);
        std::fs::create_dir_all(log_dir())?;
        let f = std::fs::File::create(&log)?;
        c.stdout(f.try_clone()?).stderr(f);
    }
    let mut child = c.spawn().context("starting claude")?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(text.as_bytes()).await?;
    }
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"station": st, "session": session,
                "log": (!a.wait).then(|| log.display().to_string())}))?
        );
    } else {
        ok_line(format!(
            "{id}: Claude Code session {session} started for {cook} in {pot} ({})",
            root
        ));
    }
    if !a.wait {
        if !a.json {
            println!(
                "  running in the background; output: {}\n  open it: corntchap-cli station open {id}   (Claude Desktop, once this first run is done)",
                log.display()
            );
        }
        // the child keeps running after we exit (its own process group: closing the terminal does not stop it)
        drop(child);
        return Ok(());
    }
    let status = child.wait().await?;
    if !status.success() {
        bail!("claude exited with {status}");
    }
    if a.desktop {
        open_in(&root, &["claude", "--desktop", "--resume", &session])?;
    }
    Ok(())
}

async fn start_by_fire(api: &Api, a: StartArgs, dir: Option<String>) -> Result<()> {
    let pot = match a.pot {
        Some(p) => p.to_uppercase(),
        None => {
            let cwd = std::env::current_dir()?;
            let root = git(&cwd, &["rev-parse", "--show-toplevel"])
                .map_err(|_| anyhow!("pass --pot KEY (not inside a repo to match it from)"))?;
            resolve_pot(api, &root, None).await?
        }
    };
    let cook = claude_cook(api, a.cook).await?;
    let v = api
        .procedure(
            "stationStart",
            json!({"project": pot, "cook": cook, "task": a.task, "prompt": a.prompt,
                   "directory": dir, "label": a.label}),
        )
        .await?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        ok_line(format!(
            "{}: a new session for {cook} in {pot} ({}); its relay starts it when it is running (`corntchap-cli fire`)",
            v["id"].as_str().unwrap_or("?"),
            v["directory"].as_str().unwrap_or("?")
        ));
    }
    Ok(())
}

/// The command that opens a station's session on this machine.
pub fn open_command(runtime: &str, session: &str, terminal: bool) -> Result<Vec<String>> {
    let v: Vec<&str> = match (runtime, terminal) {
        ("claude-code", false) => vec!["claude", "--desktop", "--resume", session],
        ("claude-code", true) => vec!["claude", "--resume", session],
        ("codex", true) => vec!["codex", "resume", session],
        ("codex", false) => bail!("Codex sessions open in a terminal: add --terminal"),
        ("copilot", _) => vec!["copilot", "--resume", session],
        ("antigravity", _) => vec!["agy", "--conversation", session],
        ("opencode", _) => {
            bail!("open OpenCode in the station's folder and pick session {session}")
        }
        (other, _) => bail!("a {other} station cannot be opened from here"),
    };
    Ok(v.into_iter().map(String::from).collect())
}

fn open_in(dir: &str, cmd: &[&str]) -> Result<()> {
    if !relay::which(cmd[0]) {
        bail!("`{}` is not on PATH on this machine", cmd[0]);
    }
    let status = Command::new(relay::program(cmd[0]))
        .args(&cmd[1..])
        .current_dir(dir)
        .status()
        .with_context(|| format!("running {}", cmd.join(" ")))?;
    if !status.success() {
        bail!("{} exited with {status}", cmd.join(" "));
    }
    Ok(())
}

async fn open(api: &Api, id: &str, terminal: bool) -> Result<()> {
    let all = api.get("/stations").await?;
    let st = all
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["id"].as_str() == Some(id.trim()))
        .ok_or_else(|| anyhow!("no station {id} that you can see (`corntchap-cli station ls`)"))?;
    let runtime = st["runtime"].as_str().unwrap_or("other");
    let session = st["session"].as_str().unwrap_or_default();
    let cmd = open_command(runtime, session, terminal)?;
    let dir = st["directory"]
        .as_str()
        .map(relay::expand)
        .filter(|d| Path::new(d).is_dir())
        .ok_or_else(|| {
            anyhow!(
                "the station's folder ({}) is not on this machine: open it where the session runs",
                st["directory"].as_str().unwrap_or("none")
            )
        })?;
    let cmd: Vec<&str> = cmd.iter().map(String::as_str).collect();
    open_in(&dir, &cmd)
}

async fn set_main(api: &Api, id: &str, main: bool) -> Result<()> {
    let v = api
        .post(
            &format!("/stations/{id}/main"),
            serde_json::json!({"main": main}),
        )
        .await?;
    let who = format!(
        "{} in {}",
        v["cook"].as_str().unwrap_or("?"),
        v["project"].as_str().unwrap_or("?")
    );
    if main {
        ok_line(format!(
            "{id} is now the main station of {who}: it answers messages and mentions"
        ));
    } else {
        ok_line(format!("{id} is no longer the main station of {who}"));
    }
    Ok(())
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
            {"key": "PAY", "repo_url": "git@github.com:acme/pay.git",
             "repos": [{"name": "pay", "url": "git@github.com:acme/pay.git"}]},
            {"key": "WEB", "repo_url": "https://github.com/acme/web",
             "repos": [{"name": "web", "url": "https://github.com/acme/web"},
                       {"name": "api", "url": "https://github.com/acme/web-api.git", "default_branch": "main"}]},
            {"key": "OLD", "repo_url": "https://github.com/acme/old"},
            {"key": "NOURL", "repo_url": null, "repos": []},
        ]);
        assert_eq!(
            match_pots("https://github.com/Acme/pay.git", &pots),
            ["PAY"]
        );
        // any repository of a pot matches, not only the first
        assert_eq!(
            match_pots("git@github.com:acme/web-api.git", &pots),
            ["WEB"]
        );
        // a server without `repos` still matches on repo_url
        assert_eq!(
            match_pots("https://github.com/acme/old.git", &pots),
            ["OLD"]
        );
        assert!(match_pots("https://github.com/acme/other", &pots).is_empty());
    }

    #[test]
    fn a_checkout_finds_its_repository_alias_in_its_pot() {
        let pots = json!([
            {"key": "WEB", "repos": [{"name": "web", "url": "https://github.com/acme/web"},
                                      {"name": "api", "url": "https://github.com/acme/web-api.git"}]},
            {"key": "PAY", "repos": [{"name": "pay", "url": "https://github.com/acme/web-api"}]},
        ]);
        assert_eq!(
            match_repo("git@github.com:Acme/web-api.git", &pots, "web").as_deref(),
            Some("api")
        );
        assert_eq!(
            match_repo("git@github.com:Acme/web-api.git", &pots, "PAY").as_deref(),
            Some("pay")
        );
        assert_eq!(
            match_repo("https://github.com/acme/none", &pots, "WEB"),
            None
        );
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
    fn copilot_sessions_are_found_by_workspace_yaml_or_events() {
        let home = tmp("copilot");
        let root = home.join("session-state");
        // a workspace.yaml with a cwd
        let a = root.join("11111111-aaaa");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(
            a.join("workspace.yaml"),
            "id: 11111111-aaaa\ncwd: \"/work/pay\"\n",
        )
        .unwrap();
        // no yaml: a cwd nested in the first events
        let b = root.join("22222222-bbbb");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(
            b.join("events.jsonl"),
            "{\"type\":\"session.start\",\"data\":{\"context\":{\"cwd\":\"/work/pay\"}}}\n{\"type\":\"x\"}\n",
        )
        .unwrap();
        // another folder, and an unreadable one
        let c = root.join("33333333-cccc");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("workspace.yaml"), "cwd: /work/other\n").unwrap();
        std::fs::create_dir_all(root.join("44444444-dddd")).unwrap();
        std::fs::write(root.join("stray-file"), "x").unwrap();
        let dirs = vec!["/work/pay".to_string()];
        let mut f = copilot_sessions(&root, &dirs, 30);
        f.sort_by(|x, y| x.session.cmp(&y.session));
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].session, "11111111-aaaa");
        assert_eq!(f[1].session, "22222222-bbbb");
        assert!(
            f.iter()
                .all(|x| x.runtime == "copilot" && x.directory == "/work/pay")
        );
        assert!(copilot_sessions(&home.join("nope"), &dirs, 30).is_empty());
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn copilot_and_antigravity_open_with_their_own_resume() {
        assert_eq!(
            open_command("copilot", "s1", true).unwrap(),
            ["copilot", "--resume", "s1"]
        );
        assert_eq!(
            open_command("antigravity", "c1", false).unwrap(),
            ["agy", "--conversation", "c1"]
        );
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
