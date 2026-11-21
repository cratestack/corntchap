//! Finding and prompting a running OpenCode server (the `opencode` wake adapter).
//!
//! OpenCode's TUI and desktop app start their server on a random port, so a fixed `url` breaks
//! on every restart. Without `url` (or with `url = "auto"`) the relay looks for the server:
//!   1. `OPENCODE_PORT` (the desktop app uses it for its server when set);
//!   2. running `opencode*` processes: their `--port`, or (Linux, macOS with lsof) the TCP ports
//!      they listen on;
//!   3. `http://127.0.0.1:4096` (`opencode serve`'s default).
//!
//! Each candidate is probed (`GET /session/status`). With several live servers, the one that has a
//! session in the target's `directory` wins. Servers protected by a password (HTTP basic, user
//! `opencode`) get it from the target (`password`, `password_env`), from `OPENCODE_SERVER_PASSWORD`
//! / `OPENCODE_PASSWORD`, or, on Linux, from the environment of the OpenCode process itself (the
//! desktop app hands its server a generated password that way).

use std::{collections::HashMap, time::Duration};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::relay::{Target, expand};

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub base: String,
    pub pid: Option<u32>,
    /// How it was found, for `fire doctor`.
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base: String,
    pub auth: Option<(String, String)>,
    pub source: String,
    /// Sessions in the target's directory (all sessions without one).
    pub sessions: Vec<Value>,
}

enum Probe {
    Up,
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

async fn probe(http: &reqwest::Client, base: &str, auth: &Option<(String, String)>) -> Probe {
    match with_auth(http.get(format!("{base}/session/status")), auth)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => Probe::Up,
        Ok(r) if r.status().as_u16() == 401 => Probe::NeedsAuth,
        _ => Probe::Down,
    }
}

async fn sessions(
    http: &reqwest::Client,
    base: &str,
    auth: &Option<(String, String)>,
    dir: Option<&str>,
) -> Result<Vec<Value>> {
    let mut q = with_auth(http.get(format!("{base}/session")), auth);
    if let Some(d) = dir {
        q = q.query(&[("directory", d)]);
    }
    let all: Vec<Value> = q.send().await?.error_for_status()?.json().await?;
    Ok(all
        .into_iter()
        .filter(|s| dir.is_none_or(|d| s["directory"].as_str() == Some(d)))
        .collect())
}

/// The OpenCode server this target should prompt.
pub async fn resolve(t: &Target) -> Result<Endpoint> {
    let http = http();
    let dir = t.directory.as_deref().map(expand);
    let pinned = t
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty() && *u != "auto");
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
            Probe::Up => {
                let s = sessions(&http, &c.base, &auth, dir.as_deref())
                    .await
                    .unwrap_or_default();
                let ep = Endpoint {
                    base: c.base.clone(),
                    auth,
                    source: c.source.clone(),
                    sessions: s,
                };
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
            "no running OpenCode server found (tried {}). Start OpenCode, or pin its port: \
             OPENCODE_PORT=4096 for the desktop app, `opencode serve --port 4096` for a headless one",
            cands
                .iter()
                .map(|c| c.base.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Prompt the newest session in the target's directory; skip when it is busy.
pub async fn prompt(t: &Target, text: &str) -> Result<i32> {
    let ep = resolve(t).await?;
    let http = http();
    let session = ep
        .sessions
        .iter()
        .max_by_key(|s| s["time"]["updated"].as_i64().unwrap_or(0))
        .ok_or_else(|| {
            anyhow!(
                "OpenCode at {} has no session in {}: open the project in OpenCode once",
                ep.base,
                t.directory.as_deref().unwrap_or("any directory")
            )
        })?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow!("session without id"))?;
    if let Ok(r) = with_auth(http.get(format!("{}/session/status", ep.base)), &ep.auth)
        .send()
        .await
        && let Ok(v) = r.json::<Value>().await
        && v.get(id).and_then(|s| s["type"].as_str()) == Some("busy")
    {
        eprintln!("relay: opencode session {id} is busy, skipping");
        return Ok(0);
    }
    with_auth(
        http.post(format!("{}/session/{id}/prompt_async", ep.base)),
        &ep.auth,
    )
    .json(&json!({"parts": [{"type": "text", "text": text}]}))
    .send()
    .await?
    .error_for_status()?;
    Ok(0)
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
}
