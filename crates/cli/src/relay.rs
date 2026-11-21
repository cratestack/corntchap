//! `corntchap-cli relay`: one outbound SSE connection that turns `wake` events into local agent runs.

use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};

use crate::api::{Api, Credentials, config_dir};

pub const PROMPT: &str = "New work is ready in {project}. Call claim_next with project {project}, finish the task, then call it again until it returns none.";

#[derive(Deserialize, Clone, Debug)]
pub struct Target {
    pub agent: String,
    /// "opencode" | "exec" | "webhook"
    pub adapter: String,
    pub url: Option<String>,
    pub directory: Option<String>,
    pub command: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct RelayConfig {
    #[serde(default)]
    pub target: Vec<Target>,
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> Result<RelayConfig> {
    let p = config_path();
    let s = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
    toml::from_str(&s).with_context(|| format!("parsing {}", p.display()))
}

fn parse_duration(s: &str) -> Duration {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = n.parse().unwrap_or(30);
    Duration::from_secs(match unit {
        "h" => n * 3600,
        "m" => n * 60,
        _ => n,
    })
}

fn expand(p: &str) -> String {
    shellexpand::tilde(p).into_owned()
}

#[derive(Default)]
struct RunState {
    running: bool,
    pending: Vec<String>, // projects woken while a run was in flight
}

type States = Arc<Mutex<HashMap<String, RunState>>>;

pub async fn run(api: Api) -> Result<()> {
    let cfg = load_config()?;
    if cfg.target.is_empty() {
        bail!(
            "no [[target]] in {}; see `corntchap-cli relay doctor`",
            config_path().display()
        );
    }
    let states: States = Arc::new(Mutex::new(HashMap::new()));
    let mut last_id: Option<String> = None;
    let mut backoff = 1u64;
    eprintln!("relay: connecting to {}", api.server);
    loop {
        match stream_once(&api, &cfg, &states, &mut last_id).await {
            Ok(()) => backoff = 1,
            Err(e) => eprintln!("relay: stream lost: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(30);
    }
}

async fn stream_once(
    api: &Api,
    cfg: &RelayConfig,
    states: &States,
    last_id: &mut Option<String>,
) -> Result<()> {
    let mut req = api
        .raw(reqwest::Method::GET, "/relay/stream")
        .header("accept", "text/event-stream");
    if let Some(id) = last_id {
        req = req.header("last-event-id", id.clone());
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        bail!("server answered HTTP {}", resp.status());
    }
    eprintln!("relay: connected");
    let mut buf = String::new();
    let mut s = resp.bytes_stream();
    let (mut id, mut event, mut data) = (None::<String>, String::new(), String::new());
    while let Some(chunk) = s.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk?));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim_end_matches('\r').to_string();
            buf.drain(..=pos);
            if line.is_empty() {
                if event == "wake"
                    && !data.is_empty()
                    && let Ok(v) = serde_json::from_str::<Value>(&data)
                {
                    on_wake(api, cfg, states, &v).await;
                }
                if let Some(i) = id.take() {
                    *last_id = Some(i);
                }
                event.clear();
                data.clear();
            } else if let Some(v) = line.strip_prefix("id:") {
                id = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("event:") {
                event = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                data.push_str(v.trim_start());
            }
        }
    }
    Ok(())
}

async fn on_wake(api: &Api, cfg: &RelayConfig, states: &States, wake: &Value) {
    let (Some(agent), Some(project)) = (wake["agent"].as_str(), wake["project"].as_str()) else {
        return;
    };
    let Some(target) = cfg.target.iter().find(|t| t.agent == agent).cloned() else {
        return; // no target for that agent: ignore
    };
    {
        let mut g = states.lock().await;
        let st = g.entry(agent.to_string()).or_default();
        if st.running {
            // Coalesce into one follow-up run when the current one ends.
            if !st.pending.iter().any(|p| p == project) {
                st.pending.push(project.to_string());
            }
            return;
        }
        st.running = true;
    }
    let (api, states, project) = (api.clone(), states.clone(), project.to_string());
    tokio::spawn(async move {
        let mut project = project;
        loop {
            report(&api, &target.agent, &project, "started", None).await;
            let code = run_target(&target, &project).await;
            report(&api, &target.agent, &project, "finished", Some(code)).await;
            let mut g = states.lock().await;
            let st = g.entry(target.agent.clone()).or_default();
            if st.pending.is_empty() {
                st.running = false;
                return;
            }
            project = st.pending.remove(0);
        }
    });
}

async fn report(api: &Api, agent: &str, project: &str, event: &str, exit: Option<i32>) {
    let _ = api
        .post(
            "/relay/runs",
            json!({"agent": agent, "project": project, "event": event, "exit_code": exit}),
        )
        .await;
}

/// Returns an exit code (0 ok, -1 skipped/failed to start).
pub async fn run_target(t: &Target, project: &str) -> i32 {
    let prompt = PROMPT.replace("{project}", project);
    let res = match t.adapter.as_str() {
        "opencode" => opencode(t, &prompt).await,
        "exec" => exec(t, &prompt).await,
        "webhook" => webhook(t, &prompt, project).await,
        other => Err(anyhow!("unknown adapter {other}")),
    };
    match res {
        Ok(code) => {
            eprintln!("relay: {} finished ({code})", t.agent);
            code
        }
        Err(e) => {
            eprintln!("relay: {}: {e:#}", t.agent);
            -1
        }
    }
}

async fn exec(t: &Target, prompt: &str) -> Result<i32> {
    let cmd = t
        .command
        .as_ref()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| anyhow!("exec target needs command"))?;
    let mut c = Command::new(expand(&cmd[0]));
    c.args(cmd[1..].iter().map(|a| expand(a)));
    if let Some(cwd) = &t.cwd {
        c.current_dir(expand(cwd));
    }
    c.stdin(Stdio::piped()).kill_on_drop(true);
    eprintln!("relay: starting {}: {}", t.agent, cmd.join(" "));
    let mut child = c.spawn().with_context(|| format!("starting {}", cmd[0]))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        drop(stdin); // prompt on stdin
    }
    let limit = t
        .timeout
        .as_deref()
        .map(parse_duration)
        .unwrap_or(Duration::from_secs(45 * 60));
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => Ok(status?.code().unwrap_or(-1)),
        Err(_) => {
            let _ = child.kill().await;
            bail!("timed out after {limit:?}, killed")
        }
    }
}

async fn opencode(t: &Target, prompt: &str) -> Result<i32> {
    let base = t
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("opencode target needs url"))?
        .trim_end_matches('/')
        .to_string();
    let http = reqwest::Client::new();
    let dir = t.directory.as_deref().map(expand);
    let mut q = http.get(format!("{base}/session"));
    if let Some(d) = &dir {
        q = q.query(&[("directory", d)]);
    }
    let sessions: Vec<Value> = q.send().await?.error_for_status()?.json().await?;
    let session = sessions
        .iter()
        .filter(|s| {
            dir.as_deref()
                .map(|d| s["directory"].as_str() == Some(d))
                .unwrap_or(true)
        })
        .max_by_key(|s| s["time"]["updated"].as_i64().unwrap_or(0))
        .ok_or_else(|| anyhow!("no OpenCode session for that directory"))?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow!("session without id"))?;
    // Skip when the session is busy; the next wake retries.
    if let Ok(r) = http.get(format!("{base}/session/status")).send().await
        && let Ok(v) = r.json::<Value>().await
        && v.get(id).and_then(|s| s["type"].as_str()) == Some("busy")
    {
        eprintln!("relay: opencode session {id} is busy, skipping");
        return Ok(0);
    }
    http.post(format!("{base}/session/{id}/prompt_async"))
        .json(&json!({"parts": [{"type": "text", "text": prompt}]}))
        .send()
        .await?
        .error_for_status()?;
    Ok(0)
}

async fn webhook(t: &Target, prompt: &str, project: &str) -> Result<i32> {
    let url = t
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("webhook target needs url"))?;
    reqwest::Client::new()
        .post(url)
        .json(&json!({"agent": t.agent, "project": project, "text": prompt}))
        .send()
        .await?
        .error_for_status()?;
    Ok(0)
}

pub async fn doctor() -> Result<()> {
    let creds = Credentials::load();
    let server = creds
        .server
        .clone()
        .ok_or_else(|| anyhow!("no server configured (run `corntchap-cli login <url>`)"))?;
    let http = reqwest::Client::new();
    match http
        .get(format!("{}/healthz", server.trim_end_matches('/')))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => println!("ok   server reachable: {server}"),
        Ok(r) => println!("FAIL server answered HTTP {}", r.status()),
        Err(e) => println!("FAIL server unreachable: {e}"),
    }
    match &creds.relay_token {
        None => println!(
            "FAIL no relay credential: run `corntchap-cli relay enroll <name>` (or set CORNTCHAP_RELAY_TOKEN)"
        ),
        Some(_) => {
            let api = Api::from_creds(true)?;
            match api.get("/me").await {
                Ok(me) => println!(
                    "ok   relay credential valid ({})",
                    me["name"].as_str().unwrap_or("?")
                ),
                Err(e) => println!("FAIL relay credential rejected: {e}"),
            }
        }
    }
    let cfg = match load_config() {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL {e:#}");
            return Ok(());
        }
    };
    if cfg.target.is_empty() {
        println!("FAIL no [[target]] entries in {}", config_path().display());
    }
    for t in &cfg.target {
        match t.adapter.as_str() {
            "exec" => {
                let cmd = t.command.clone().unwrap_or_default();
                let found = cmd.first().map(|c| which(&expand(c))).unwrap_or(false);
                println!(
                    "{}   {}: would run `{}` in {} (prompt on stdin)",
                    if found { "ok  " } else { "FAIL" },
                    t.agent,
                    cmd.join(" "),
                    t.cwd.as_deref().unwrap_or(".")
                );
            }
            "opencode" => {
                let url = t.url.clone().unwrap_or_default();
                let ok = http
                    .get(format!("{}/session/status", url.trim_end_matches('/')))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                println!(
                    "{}   {}: would POST prompt_async to {url} for {}",
                    if ok { "ok  " } else { "FAIL" },
                    t.agent,
                    t.directory.as_deref().unwrap_or("(any directory)")
                );
            }
            "webhook" => println!(
                "ok     {}: would POST to {}",
                t.agent,
                t.url.clone().unwrap_or_default()
            ),
            other => println!("FAIL {}: unknown adapter {other}", t.agent),
        }
    }
    Ok(())
}

fn which(cmd: &str) -> bool {
    if cmd.contains('/') {
        return std::path::Path::new(cmd).exists();
    }
    std::env::var("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).exists()))
        .unwrap_or(false)
}

/// systemd user unit (Linux) or launchd agent (macOS) so the relay starts at login.
pub fn install() -> Result<()> {
    let exe = std::env::current_exe()?;
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    if cfg!(target_os = "macos") {
        let path = home.join("Library/LaunchAgents/dev.corntchap.relay.plist");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.corntchap.relay</string>\n<key>ProgramArguments</key><array><string>{}</string><string>relay</string></array>\n<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n<key>StandardErrorPath</key><string>{}/Library/Logs/corntchap-relay.log</string>\n</dict></plist>\n",
                exe.display(),
                home.display()
            ),
        )?;
        println!(
            "wrote {}\nstart it now with: launchctl load {}",
            path.display(),
            path.display()
        );
    } else {
        let path = home.join(".config/systemd/user/corntchap-relay.service");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "[Unit]\nDescription=corntchap relay\nAfter=network-online.target\n\n[Service]\nExecStart={} relay\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
                exe.display()
            ),
        )?;
        println!(
            "wrote {}\nenable it with: systemctl --user daemon-reload && systemctl --user enable --now corntchap-relay",
            path.display()
        );
    }
    Ok(())
}
