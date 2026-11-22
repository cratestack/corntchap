//! Finding and prompting a running OpenCode server (the `opencode` wake adapter).
//!
//! OpenCode v2 runs one shared background service: `opencode service status` prints its URL and
//! `opencode service get password` its password (HTTP basic, user `opencode`); its API lives under
//! `/api` (`GET /api/info`, `GET /api/session`, `POST /api/session/{id}/prompt`). That is tried
//! first. The 1.x discovery below stays as the fallback.
//!
//! OpenCode 1.x's TUI and desktop app start their server on a random port, so a fixed `url` breaks
//! on every restart. Without `url` (or with `url = "auto"`) the relay looks for the server:
//!   1. `OPENCODE_PORT` (the desktop app uses it for its server when set);
//!   2. running `opencode*` processes: their `--port`, or (Linux, macOS with lsof) the TCP ports
//!      they listen on;
//!   3. `http://127.0.0.1:4096` (`opencode serve`'s default).
//!
//! Each candidate is probed (`GET /api/info`, then `GET /session/status`). With several live
//! servers, the one that has a session in the target's `directory` wins. Servers protected by a
//! password (HTTP basic, user `opencode`) get it from the target (`password`, `password_env`), from
//! `OPENCODE_SERVER_PASSWORD` / `OPENCODE_PASSWORD`, or, on Linux, from the environment of the
//! OpenCode process itself (the desktop app hands its server a generated password that way).

use std::{collections::HashMap, time::Duration};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::relay::{Target, expand};

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub base: String,
    pub pid: Option<u32>,
    /// How it was found, for `fire doctor`.
    pub source: String,
}

/// Which OpenCode HTTP API a server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiVersion {
    V1,
    V2,
}

impl ApiVersion {
    pub fn label(self) -> &'static str {
        match self {
            ApiVersion::V1 => "OpenCode 1.x API",
            ApiVersion::V2 => "OpenCode v2 API",
        }
    }
}

/// A session as listed by either API.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub directory: Option<String>,
    /// Last update in milliseconds since the epoch (0 when unknown); only used for ordering.
    pub updated: i64,
    pub title: Option<String>,
}

/// A server and how to talk to it. No `Debug`: it holds the password.
#[derive(Clone)]
pub struct Endpoint {
    pub base: String,
    pub auth: Option<(String, String)>,
    pub source: String,
    pub api: ApiVersion,
    /// Sessions in the target's directory (none when the target names no directory).
    pub sessions: Vec<SessionInfo>,
}

enum Probe {
    Up(ApiVersion),
    NeedsAuth,
    Down,
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .unwrap_or_default()
}

/// `--port 1234`, `--port=1234`, `--hostname h` / `--hostname=h` from a command line.
pub fn port_from_args(args: &str) -> Option<(String, u16)> {
    let words: Vec<&str> = args.split_whitespace().collect();
    let mut host = "127.0.0.1".to_string();
    let mut port = None;
    for (i, w) in words.iter().enumerate() {
        let next = words.get(i + 1).copied();
        if let Some(v) = w.strip_prefix("--port=") {
            port = v.parse().ok();
        } else if *w == "--port" {
            port = next.and_then(|v| v.parse().ok());
        } else if let Some(v) = w.strip_prefix("--hostname=") {
            host = v.to_string();
        } else if *w == "--hostname"
            && let Some(v) = next
        {
            host = v.to_string();
        }
    }
    let host = match host.as_str() {
        "0.0.0.0" | "::" | "" => "127.0.0.1".to_string(),
        _ => host,
    };
    port.filter(|p| *p != 0).map(|p| (host, p))
}

/// `ps -eo pid=,args=` lines of OpenCode processes (the program's file name starts with
/// "opencode", or a script by that name is run by an interpreter).
pub fn opencode_processes(ps: &str) -> Vec<(u32, String)> {
    ps.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let (pid, args) = l.split_once(char::is_whitespace)?;
            let pid: u32 = pid.parse().ok()?;
            let args = args.trim().to_string();
            let name = |w: &str| {
                std::path::Path::new(w)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("")
                    .to_string()
            };
            let is_oc_name = |n: &str| n.starts_with("opencode") || n.starts_with(".opencode");
            let mut words = args.split_whitespace();
            let first = name(words.next().unwrap_or(""));
            let is_oc = is_oc_name(&first)
                || (matches!(
                    first.as_str(),
                    "node" | "bun" | "deno" | "python3" | "python"
                ) && is_oc_name(&name(words.next().unwrap_or(""))));
            is_oc.then_some((pid, args))
        })
        .collect()
}

/// TCP ports a process listens on: /proc on Linux, `lsof` elsewhere.
fn listening_ports(pid: u32) -> Vec<u16> {
    let mut inodes = std::collections::HashSet::new();
    if let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
        for fd in fds.flatten() {
            if let Ok(link) = std::fs::read_link(fd.path())
                && let Some(s) = link.to_str()
                && let Some(n) = s.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']'))
            {
                inodes.insert(n.to_string());
            }
        }
        let mut ports = vec![];
        for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
            let Ok(t) = std::fs::read_to_string(table) else {
                continue;
            };
            for line in t.lines().skip(1) {
                let f: Vec<&str> = line.split_whitespace().collect();
                // local_address rem_address st ... inode at index 9; 0A = LISTEN
                if f.len() > 9
                    && f[3] == "0A"
                    && inodes.contains(f[9])
                    && let Some(p) = f[1]
                        .rsplit(':')
                        .next()
                        .and_then(|h| u16::from_str_radix(h, 16).ok())
                {
                    ports.push(p);
                }
            }
        }
        return ports;
    }
    // macOS and friends
    let out = std::process::Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-iTCP",
            "-sTCP:LISTEN",
            "-p",
            &pid.to_string(),
            "-Fn",
        ])
        .output();
    out.ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.strip_prefix('n')?.rsplit(':').next()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The environment of one of the user's own processes (Linux only).
fn env_of(pid: u32) -> HashMap<String, String> {
    std::fs::read(format!("/proc/{pid}/environ"))
        .map(|b| {
            b.split(|c| *c == 0)
                .filter_map(|kv| {
                    let s = String::from_utf8_lossy(kv);
                    let (k, v) = s.split_once('=')?;
                    Some((k.to_string(), v.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where an OpenCode server might be, best guess first, without duplicates.
pub fn candidates() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = vec![];
    let mut push = |c: Candidate| {
        if !out.iter().any(|o| o.base == c.base) {
            out.push(c);
        }
    };
    if let Ok(p) = std::env::var("OPENCODE_PORT")
        && let Ok(p) = p.trim().parse::<u16>()
    {
        push(Candidate {
            base: format!("http://127.0.0.1:{p}"),
            pid: None,
            source: "OPENCODE_PORT".into(),
        });
    }
    if let Ok(o) = std::process::Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
    {
        for (pid, args) in opencode_processes(&String::from_utf8_lossy(&o.stdout)) {
            let short: String = args.chars().take(60).collect();
            if let Some((host, port)) = port_from_args(&args) {
                push(Candidate {
                    base: format!("http://{host}:{port}"),
                    pid: Some(pid),
                    source: format!("process {pid} `{short}`"),
                });
            }
            for port in listening_ports(pid) {
                push(Candidate {
                    base: format!("http://127.0.0.1:{port}"),
                    pid: Some(pid),
                    source: format!("process {pid} `{short}` listening"),
                });
            }
        }
    }
    push(Candidate {
        base: "http://127.0.0.1:4096".into(),
        pid: None,
        source: "the `opencode serve` default port".into(),
    });
    out
}

fn configured_auth(t: &Target) -> Option<(String, String)> {
    let user = t
        .username
        .clone()
        .or_else(|| std::env::var("OPENCODE_SERVER_USERNAME").ok())
        .unwrap_or_else(|| "opencode".into());
    let pw = t
        .password
        .clone()
        .or_else(|| {
            t.password_env
                .as_deref()
                .and_then(|v| std::env::var(v).ok())
        })
        .or_else(|| std::env::var("OPENCODE_SERVER_PASSWORD").ok())
        .or_else(|| std::env::var("OPENCODE_PASSWORD").ok())?;
    Some((user, pw))
}

fn auth_from_process(pid: u32) -> Option<(String, String)> {
    let env = env_of(pid);
    let pw = env
        .get("OPENCODE_SERVER_PASSWORD")
        .or_else(|| env.get("OPENCODE_PASSWORD"))?
        .clone();
    let user = env
        .get("OPENCODE_SERVER_USERNAME")
        .cloned()
        .unwrap_or_else(|| "opencode".into());
    Some((user, pw))
}

fn with_auth(
    r: reqwest::RequestBuilder,
    auth: &Option<(String, String)>,
) -> reqwest::RequestBuilder {
    match auth {
        Some((u, p)) => r.basic_auth(u, Some(p)),
        None => r,
    }
}

// ---- v2: the shared service ----

/// What `opencode service status` / `service get password` (or the service files) told us.
#[derive(Clone)]
pub struct ServiceInfo {
    pub base: String,
    pub password: Option<String>,
    pub source: String,
}

/// The first `http(s)://…` word in text, without trailing punctuation.
pub fn parse_service_url(out: &str) -> Option<String> {
    let w = out
        .split_whitespace()
        .find(|w| w.contains("http://") || w.contains("https://"))?;
    let i = w.find("http")?;
    Some(
        w[i..]
            .trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '/')
            .trim_end_matches('/')
            .to_string(),
    )
}

/// The password a command printed: its last non-empty line.
pub fn parse_password(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(String::from)
}

async fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let fut = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(8), fut)
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// First string value stored under any of `keys`, searched through nested objects.
pub fn find_string(v: &Value, keys: &[&str]) -> Option<String> {
    match v {
        Value::Object(m) => {
            for k in keys {
                if let Some(Value::String(s)) = m.get(*k)
                    && !s.is_empty()
                {
                    return Some(s.clone());
                }
            }
            m.values().find_map(|x| find_string(x, keys))
        }
        Value::Array(a) => a.iter().find_map(|x| find_string(x, keys)),
        _ => None,
    }
}

fn find_u64(v: &Value, key: &str) -> Option<u64> {
    match v {
        Value::Object(m) => m
            .get(key)
            .and_then(|x| x.as_u64().or_else(|| x.as_str()?.parse().ok()))
            .or_else(|| m.values().find_map(|x| find_u64(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_u64(x, key)),
        _ => None,
    }
}

/// The URL in OpenCode's service registration file (`url`, or `hostname`/`host` plus `port`).
pub fn url_from_service_file(v: &Value) -> Option<String> {
    if let Some(u) = find_string(v, &["url", "baseUrl", "base_url", "origin"])
        && u.starts_with("http")
    {
        return Some(u.trim_end_matches('/').to_string());
    }
    let port = find_u64(v, "port")?;
    let host = find_string(v, &["hostname", "host"]).unwrap_or_else(|| "127.0.0.1".into());
    let host = match host.as_str() {
        "0.0.0.0" | "::" | "" => "127.0.0.1".into(),
        _ => host,
    };
    Some(format!("http://{host}:{port}"))
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Best-effort fallback: `~/.local/state/opencode/service.json` (registration) and
/// `~/.config/opencode/service.json` (private config with the password). The password is never logged.
fn service_from_files() -> Option<ServiceInfo> {
    let home = dirs::home_dir()?;
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("opencode/service.json");
    let conf = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("opencode/service.json");
    let base = read_json(&state).and_then(|v| url_from_service_file(&v))?;
    let password = read_json(&conf)
        .and_then(|v| find_string(&v, &["password", "serverPassword", "server_password"]));
    Some(ServiceInfo {
        base,
        password,
        source: "OpenCode service files".into(),
    })
}

/// OpenCode v2's shared service, found with `program service status` (+ `service get password`),
/// else from its files.
pub async fn service_endpoint(program: &str) -> Option<ServiceInfo> {
    if let Some(out) = run_capture(program, &["service", "status"]).await
        && let Some(base) = parse_service_url(&out)
    {
        let password = run_capture(program, &["service", "get", "password"])
            .await
            .and_then(|o| parse_password(&o))
            .or_else(|| service_from_files().and_then(|s| s.password));
        return Some(ServiceInfo {
            base,
            password,
            source: format!("`{program} service status`"),
        });
    }
    service_from_files()
}

// ---- v2: response shapes (parsed tolerantly; they could not be checked against a real server) ----

/// Milliseconds from a number (seconds or milliseconds) or an RFC 3339 string; 0 when unknown.
fn ts_ms(v: &Value) -> i64 {
    match v {
        Value::Number(n) => {
            let x = n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .unwrap_or(0);
            if x > 0 && x < 100_000_000_000 {
                x * 1000
            } else {
                x
            }
        }
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp_millis())
            .unwrap_or(0),
        _ => 0,
    }
}

/// One session object: `id`; `directory` or `location.directory`; times under `time.updated` / `updated`.
pub fn parse_session(v: &Value) -> Option<SessionInfo> {
    let id = v["id"]
        .as_str()
        .or_else(|| v["sessionID"].as_str())?
        .to_string();
    let directory = v["directory"]
        .as_str()
        .or_else(|| v["location"]["directory"].as_str())
        .map(String::from);
    let updated = [
        &v["time"]["updated"],
        &v["updated"],
        &v["time"]["created"],
        &v["created"],
    ]
    .into_iter()
    .map(ts_ms)
    .find(|t| *t > 0)
    .unwrap_or(0);
    let title = v["title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(String::from);
    Some(SessionInfo {
        id,
        directory,
        updated,
        title,
    })
}

/// A page of sessions: a bare array, or an object with the array under `data`, `sessions` or
/// `items`; the next-page cursor under `cursor.next` (or `next_cursor` / `nextCursor`).
pub fn parse_session_page(v: &Value) -> (Vec<SessionInfo>, Option<String>) {
    let arr = match v {
        Value::Array(a) => Some(a),
        Value::Object(_) => ["data", "sessions", "items"]
            .iter()
            .find_map(|k| v[*k].as_array()),
        _ => None,
    };
    let items = arr
        .map(|a| a.iter().filter_map(parse_session).collect())
        .unwrap_or_default();
    let cursor = v["cursor"]["next"]
        .as_str()
        .or_else(|| v["cursor"].as_str())
        .or_else(|| v["next_cursor"].as_str())
        .or_else(|| v["nextCursor"].as_str())
        .filter(|c| !c.is_empty())
        .map(String::from);
    (items, cursor)
}

/// The prompt bodies to try, in order: durable queued admission, plain text, parts.
pub fn prompt_bodies(text: &str) -> [Value; 3] {
    [
        json!({"text": text, "delivery": "queue"}),
        json!({"text": text}),
        json!({"parts": [{"type": "text", "text": text}]}),
    ]
}

async fn probe(http: &reqwest::Client, base: &str, auth: &Option<(String, String)>) -> Probe {
    // v2: /api/info answers JSON (a 1.x server may answer HTML for unknown paths, so check the body)
    match with_auth(http.get(format!("{base}/api/info")), auth)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            if r.json::<Value>().await.is_ok_and(|v| v.is_object()) {
                return Probe::Up(ApiVersion::V2);
            }
        }
        Ok(r) if r.status().as_u16() == 401 => return Probe::NeedsAuth,
        _ => {}
    }
    match with_auth(http.get(format!("{base}/session/status")), auth)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => Probe::Up(ApiVersion::V1),
        Ok(r) if r.status().as_u16() == 401 => Probe::NeedsAuth,
        _ => Probe::Down,
    }
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// Sessions of a server, optionally only those in `dir`, in the server's order (newest first).
pub async fn list_sessions(ep: &Endpoint, dir: Option<&str>) -> Result<Vec<SessionInfo>> {
    let http = http();
    let mut out: Vec<SessionInfo> = vec![];
    match ep.api {
        ApiVersion::V1 => {
            let mut q = with_auth(http.get(format!("{}/session", ep.base)), &ep.auth);
            if let Some(d) = dir {
                q = q.query(&[("directory", d)]);
            }
            let v: Value = q.send().await?.error_for_status()?.json().await?;
            out = parse_session_page(&v).0;
        }
        ApiVersion::V2 => {
            let mut cursor: Option<String> = None;
            for _ in 0..5 {
                let mut q = with_auth(http.get(format!("{}/api/session", ep.base)), &ep.auth)
                    .query(&[("limit", "100"), ("order", "desc")]);
                if let Some(d) = dir {
                    q = q.query(&[("directory", d)]);
                }
                if let Some(c) = &cursor {
                    q = q.query(&[("cursor", c.as_str())]);
                }
                let v: Value = q.send().await?.error_for_status()?.json().await?;
                let (items, next) = parse_session_page(&v);
                out.extend(items);
                match next {
                    Some(n) if Some(&n) != cursor.as_ref() => cursor = Some(n),
                    _ => break,
                }
            }
        }
    }
    out.retain(|s| match (dir, &s.directory) {
        (Some(d), Some(sd)) => same_dir(d, sd),
        _ => true,
    });
    Ok(out)
}

/// `Some(true)` when the session exists, `Some(false)` on 404, `None` when the server could not say.
pub async fn session_exists(ep: &Endpoint, id: &str) -> Option<bool> {
    let url = match ep.api {
        ApiVersion::V1 => format!("{}/session/{id}", ep.base),
        ApiVersion::V2 => format!("{}/api/session/{id}", ep.base),
    };
    match with_auth(http().get(url), &ep.auth).send().await {
        Ok(r) if r.status().is_success() => Some(true),
        Ok(r) if r.status().as_u16() == 404 => Some(false),
        _ => None,
    }
}

async fn endpoint_with_sessions(
    base: String,
    auth: Option<(String, String)>,
    source: String,
    api: ApiVersion,
    dir: Option<&str>,
) -> Endpoint {
    let ep = Endpoint {
        base,
        auth,
        source,
        api,
        sessions: vec![],
    };
    let sessions = match dir {
        Some(d) => list_sessions(&ep, Some(d)).await.unwrap_or_default(),
        None => vec![],
    };
    Endpoint { sessions, ..ep }
}

/// The OpenCode server this target should prompt: a pinned `url`, else OpenCode v2's service
/// (`opencode service status`), else the 1.x discovery.
pub async fn resolve(t: &Target) -> Result<Endpoint> {
    resolve_with(t, "opencode").await
}

pub async fn resolve_with(t: &Target, program: &str) -> Result<Endpoint> {
    let http = http();
    let dir = t.directory.as_deref().map(expand);
    let pinned = t
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty() && *u != "auto");
    if pinned.is_none()
        && let Some(svc) = service_endpoint(program).await
    {
        let auth = configured_auth(t)
            .or_else(|| svc.password.clone().map(|p| ("opencode".to_string(), p)));
        if let Probe::Up(api) = probe(&http, &svc.base, &auth).await {
            return Ok(
                endpoint_with_sessions(svc.base, auth, svc.source, api, dir.as_deref()).await,
            );
        }
    }
    let cands = match pinned {
        Some(u) => vec![Candidate {
            base: u.trim_end_matches('/').to_string(),
            pid: None,
            source: "url in config.toml".into(),
        }],
        None => candidates(),
    };
    let mut live: Vec<Endpoint> = vec![];
    let mut locked: Vec<String> = vec![];
    for c in &cands {
        let mut auth = configured_auth(t);
        let mut state = probe(&http, &c.base, &auth).await;
        if matches!(state, Probe::NeedsAuth)
            && let Some(pid) = c.pid
            && let Some(a) = auth_from_process(pid)
        {
            auth = Some(a);
            state = probe(&http, &c.base, &auth).await;
        }
        match state {
            Probe::Up(api) => {
                let ep = endpoint_with_sessions(
                    c.base.clone(),
                    auth,
                    c.source.clone(),
                    api,
                    dir.as_deref(),
                )
                .await;
                if dir.is_none() || !ep.sessions.is_empty() {
                    return Ok(ep);
                }
                live.push(ep);
            }
            Probe::NeedsAuth => locked.push(c.base.clone()),
            Probe::Down => {}
        }
    }
    if let Some(ep) = live.into_iter().next() {
        return Ok(ep);
    }
    if !locked.is_empty() {
        bail!(
            "OpenCode at {} wants a password: set `password_env` (or `password`) on the target, or export OPENCODE_SERVER_PASSWORD for the relay",
            locked.join(", ")
        );
    }
    match pinned {
        Some(u) => bail!("no OpenCode server answers at {u}"),
        None => bail!(
            "no running OpenCode server found (tried `{program} service status`, then {}). Start OpenCode, or pin its port: \
             OPENCODE_PORT=4096 for the desktop app, `opencode serve --port 4096` for a headless one",
            cands
                .iter()
                .map(|c| c.base.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Send `text` to one session. v2: queued (the server waits for the current turn), falling back to
/// simpler bodies on 400. v1: `prompt_async`, skipped when the session is busy.
pub async fn prompt_session(ep: &Endpoint, id: &str, text: &str) -> Result<i32> {
    let http = http();
    let gone = || {
        anyhow!(
            "OpenCode no longer has session {id} (HTTP 404): re-scan its folder with `corntchap-cli station scan`"
        )
    };
    match ep.api {
        ApiVersion::V2 => {
            let url = format!("{}/api/session/{id}/prompt", ep.base);
            let mut last = String::new();
            for body in prompt_bodies(text) {
                let r = with_auth(http.post(&url), &ep.auth)
                    .json(&body)
                    .send()
                    .await?;
                let st = r.status();
                if st.is_success() {
                    return Ok(0);
                }
                if st.as_u16() == 404 {
                    return Err(gone());
                }
                let msg = r.text().await.unwrap_or_default();
                if st.as_u16() != 400 {
                    bail!("OpenCode answered HTTP {st} to the prompt: {msg}");
                }
                last = msg;
            }
            bail!("OpenCode rejected every prompt format for session {id} (HTTP 400): {last}")
        }
        ApiVersion::V1 => {
            if let Ok(r) = with_auth(http.get(format!("{}/session/status", ep.base)), &ep.auth)
                .send()
                .await
                && let Ok(v) = r.json::<Value>().await
                && v.get(id).and_then(|s| s["type"].as_str()) == Some("busy")
            {
                eprintln!("relay: opencode session {id} is busy, skipping");
                return Ok(0);
            }
            let r = with_auth(
                http.post(format!("{}/session/{id}/prompt_async", ep.base)),
                &ep.auth,
            )
            .json(&json!({"parts": [{"type": "text", "text": text}]}))
            .send()
            .await?;
            if r.status().as_u16() == 404 {
                return Err(gone());
            }
            r.error_for_status()?;
            Ok(0)
        }
    }
}

/// Prompt the newest session in the target's directory (the pre-station behaviour).
pub async fn prompt(t: &Target, text: &str) -> Result<i32> {
    let ep = resolve(t).await?;
    let id = ep
        .sessions
        .iter()
        .max_by_key(|s| s.updated)
        .map(|s| s.id.clone())
        .ok_or_else(|| match t.directory.as_deref() {
            Some(d) => anyhow!(
                "OpenCode at {} has no session in {d}: open the project in OpenCode once, or register sessions with `corntchap-cli station scan`",
                ep.base
            ),
            None => anyhow!(
                "this wake names no station and the target has no directory, so OpenCode at {} has no session to fall back to: register the session with `corntchap-cli station scan` (wakes then name it), or set `directory` on the target",
                ep.base
            ),
        })?;
    prompt_session(&ep, &id, text).await
}

/// Prompt one registered session (a station); the target only refines how to reach the server.
pub async fn prompt_station(t: &Target, session: &str, text: &str) -> Result<i32> {
    let ep = resolve(t).await?;
    eprintln!("relay: opencode at {} ({})", ep.base, ep.api.label());
    prompt_session(&ep, session, text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_from_command_lines() {
        assert_eq!(
            port_from_args("/opt/OpenCode/opencode-cli serve --hostname 127.0.0.1 --port 41234"),
            Some(("127.0.0.1".into(), 41234))
        );
        assert_eq!(
            port_from_args("opencode serve --port=4096 --hostname=0.0.0.0"),
            Some(("127.0.0.1".into(), 4096))
        );
        assert_eq!(port_from_args("opencode serve --port 0"), None);
        assert_eq!(port_from_args("opencode"), None);
    }

    #[test]
    fn finds_opencode_processes() {
        let ps = "  1 /sbin/init\n 4242 /opt/OpenCode/opencode-cli serve --port 41234\n 77 node /home/u/.npm/bin/opencode\n 88 vim opencode.json\n 99 /usr/bin/.opencode tui\n";
        let found: Vec<u32> = opencode_processes(ps).into_iter().map(|(p, _)| p).collect();
        assert_eq!(found, [4242, 77, 99]);
    }

    // ---- v2 ----

    #[test]
    fn service_status_url_and_password_are_parsed() {
        assert_eq!(
            parse_service_url("OpenCode service is running at http://127.0.0.1:49374\n"),
            Some("http://127.0.0.1:49374".into())
        );
        assert_eq!(
            parse_service_url("url: https://oc.local:8443/.\n"),
            Some("https://oc.local:8443".into())
        );
        assert_eq!(parse_service_url("not running"), None);
        assert_eq!(parse_password("\n  s3cret \n\n"), Some("s3cret".into()));
        assert_eq!(parse_password("  \n"), None);
    }

    #[test]
    fn service_file_url_is_found_defensively() {
        let a = json!({"url": "http://127.0.0.1:5000/"});
        assert_eq!(
            url_from_service_file(&a).as_deref(),
            Some("http://127.0.0.1:5000")
        );
        let b = json!({"server": {"hostname": "0.0.0.0", "port": 4321}});
        assert_eq!(
            url_from_service_file(&b).as_deref(),
            Some("http://127.0.0.1:4321")
        );
        assert_eq!(url_from_service_file(&json!({"pid": 1})), None);
        let c = json!({"auth": {"password": "pw"}});
        assert_eq!(find_string(&c, &["password"]).as_deref(), Some("pw"));
    }

    #[test]
    fn session_lists_parse_in_every_accepted_shape() {
        let s1 = json!({"id": "a", "directory": "/w/x", "time": {"updated": 1700000005000i64}, "title": "T"});
        let s2 = json!({"id": "b", "location": {"directory": "/w/y"}, "updated": "2026-01-01T00:00:00Z"});
        for page in [
            json!([s1, s2]),
            json!({"data": [s1, s2]}),
            json!({"sessions": [s1, s2]}),
            json!({"items": [s1, s2], "cursor": {"next": null}}),
        ] {
            let (items, cursor) = parse_session_page(&page);
            assert_eq!(items.len(), 2, "{page}");
            assert_eq!(cursor, None);
            assert_eq!(items[0].id, "a");
            assert_eq!(items[0].directory.as_deref(), Some("/w/x"));
            assert_eq!(items[0].updated, 1_700_000_005_000);
            assert_eq!(items[0].title.as_deref(), Some("T"));
            assert_eq!(items[1].directory.as_deref(), Some("/w/y"));
            assert_eq!(items[1].updated, 1_767_225_600_000);
        }
        let (_, c) = parse_session_page(&json!({"data": [], "cursor": {"next": "abc"}}));
        assert_eq!(c.as_deref(), Some("abc"));
        assert!(parse_session_page(&json!({"nothing": 1})).0.is_empty());
    }

    #[test]
    fn prompt_bodies_fall_back_in_order() {
        let b = prompt_bodies("hi");
        assert_eq!(b[0], json!({"text": "hi", "delivery": "queue"}));
        assert_eq!(b[1], json!({"text": "hi"}));
        assert_eq!(b[2], json!({"parts": [{"type": "text", "text": "hi"}]}));
    }

    // A tiny HTTP server standing in for OpenCode v2 (basic auth opencode:s3cret required).
    // Records (method, path+query, authorization, body) of every request.
    type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, String, String, String)>>>;

    const BASIC: &str = "Basic b3BlbmNvZGU6czNjcmV0"; // opencode:s3cret

    async fn fake_v2(reject_delivery: bool) -> (String, Seen) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen: Seen = Default::default();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = l.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = vec![];
                    let mut tmp = [0u8; 4096];
                    let (head, body) = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..p]).to_string();
                            let want = head
                                .lines()
                                .find_map(|l| {
                                    l.to_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            if buf.len() >= p + 4 + want {
                                let body =
                                    String::from_utf8_lossy(&buf[p + 4..p + 4 + want]).to_string();
                                break (head, body);
                            }
                        }
                    };
                    let mut first = head.lines().next().unwrap_or("").split_whitespace();
                    let (method, path) = (
                        first.next().unwrap_or("").to_string(),
                        first.next().unwrap_or("").to_string(),
                    );
                    let auth = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("authorization")
                                .then(|| v.trim().to_string())
                        })
                        .unwrap_or_default();
                    log.lock().unwrap().push((
                        method.clone(),
                        path.clone(),
                        auth.clone(),
                        body.clone(),
                    ));
                    let b: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                    let (code, out) = if auth != BASIC {
                        (401, json!({"error": "unauthorized"}))
                    } else if path.starts_with("/api/info") {
                        (200, json!({"version": "2.0.0", "ready": true}))
                    } else if path.starts_with("/api/session?") && method == "GET" {
                        (
                            200,
                            json!({"data": [
                                {"id": "ses_old", "location": {"directory": "/work/pay"}, "time": {"updated": 1000}},
                                {"id": "ses_new", "directory": "/work/pay", "time": {"updated": 9000}},
                                {"id": "ses_other", "directory": "/work/else", "time": {"updated": 99999}}
                            ], "cursor": {"next": null}}),
                        )
                    } else if path == "/api/session/ses_new" && method == "GET" {
                        (200, json!({"id": "ses_new"}))
                    } else if path == "/api/session/ses_new/prompt" && method == "POST" {
                        if reject_delivery && b.get("delivery").is_some() {
                            (400, json!({"error": "unknown field delivery"}))
                        } else {
                            (200, json!({"ok": true}))
                        }
                    } else {
                        (404, json!({"error": "not found"}))
                    };
                    let body = out.to_string();
                    let resp = format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (base, seen)
    }

    /// A fake `opencode` executable answering `service status` / `service get password`.
    fn fake_opencode_bin(dir: &std::path::Path, url: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("opencode");
        let script = format!(
            "#!/bin/sh\nif [ \"$1 $2\" = \"service status\" ]; then echo \"OpenCode service running at {url}\"; \
             elif [ \"$1 $2 $3\" = \"service get password\" ]; then echo s3cret; else exit 1; fi\n"
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cc-oc-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn v2_service_discovery_auth_listing_and_queued_prompt() {
        let (base, seen) = fake_v2(false).await;
        let dir = scratch("ok");
        let bin = fake_opencode_bin(&dir, &base);
        let t = Target::blank("c", "opencode");
        let ep = resolve_with(&t, &bin).await.unwrap();
        assert_eq!(ep.api, ApiVersion::V2);
        assert_eq!(ep.base, base);
        assert!(ep.source.contains("service status"), "{}", ep.source);

        let list = list_sessions(&ep, Some("/work/pay")).await.unwrap();
        let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ses_old", "ses_new"]); // ses_other is another directory
        assert_eq!(session_exists(&ep, "ses_new").await, Some(true));
        assert_eq!(session_exists(&ep, "nope").await, Some(false));

        assert_eq!(
            prompt_session(&ep, "ses_new", "hello fire").await.unwrap(),
            0
        );
        let err = prompt_session(&ep, "gone", "x").await.unwrap_err();
        assert!(err.to_string().contains("station scan"), "{err}");

        let seen = seen.lock().unwrap().clone();
        // every request carried Basic opencode:s3cret
        assert!(seen.iter().all(|r| r.2 == BASIC), "{seen:?}");
        assert!(seen.iter().any(|r| r.1.starts_with("/api/info")));
        let l = seen
            .iter()
            .find(|r| r.1.starts_with("/api/session?"))
            .unwrap();
        assert!(l.1.contains("directory=%2Fwork%2Fpay"), "{}", l.1);
        assert!(l.1.contains("order=desc"), "{}", l.1);
        let p = seen
            .iter()
            .find(|r| r.0 == "POST" && r.1 == "/api/session/ses_new/prompt")
            .unwrap();
        let body: Value = serde_json::from_str(&p.3).unwrap();
        assert_eq!(body, json!({"text": "hello fire", "delivery": "queue"}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn v2_prompt_retries_with_plainer_bodies_on_400() {
        let (base, seen) = fake_v2(true).await;
        let dir = scratch("retry");
        let bin = fake_opencode_bin(&dir, &base);
        let ep = resolve_with(&Target::blank("c", "opencode"), &bin)
            .await
            .unwrap();
        prompt_session(&ep, "ses_new", "again").await.unwrap();
        let bodies: Vec<Value> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.0 == "POST")
            .map(|r| serde_json::from_str(&r.3).unwrap())
            .collect();
        assert_eq!(
            bodies,
            [
                json!({"text": "again", "delivery": "queue"}),
                json!({"text": "again"})
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
