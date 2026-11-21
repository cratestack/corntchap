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
    /// opencode: HTTP basic user (default "opencode").
    pub username: Option<String>,
    /// opencode: server password (prefer `password_env`).
    pub password: Option<String>,
    /// opencode: name of an environment variable holding the server password.
    pub password_env: Option<String>,
    pub command: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct RelayConfig {
    /// Optional: connect here instead of the server stored by `corntchap-cli login`.
    pub server: Option<String>,
    #[serde(default)]
    pub target: Vec<Target>,
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> Result<RelayConfig> {
    let p = config_path();
    let s = std::fs::read_to_string(&p).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "{} does not exist yet. Run `corntchap-cli fire init` to write a commented template, then edit it.",
                p.display()
            )
        } else {
            anyhow!("reading {}: {e}", p.display())
        }
    })?;
    let cfg: RelayConfig =
        toml::from_str(&s).with_context(|| format!("parsing {}", p.display()))?;
    for t in &cfg.target {
        if let Some(d) = t.timeout.as_deref() {
            parse_duration(d).map_err(|e| anyhow!("{}: target {}: {e}", p.display(), t.agent))?;
        }
    }
    Ok(cfg)
}

/// The commented starter config written by `relay init`.
pub const TEMPLATE: &str = r##"# corntchap relay configuration.
# One [[target]] per local agent the relay may wake. `agent` is the agent's name on the server
# (see `corntchap-cli cook ls`); it must also have a relay wake target:
#   corntchap-cli cook wake add <agent> --type relay
# Check everything with: corntchap-cli fire doctor

# Optional: connect to this server instead of the one stored by `corntchap-cli login`.
# server = "https://corntchap.example.com"

# --- OpenCode: prompts the newest session in `directory` of a running OpenCode -------------------
# The server is found on its own (desktop app, TUI or `opencode serve`, whatever port it took);
# set `url` only to pin one. A password-protected server: password_env = "MY_OPENCODE_PASSWORD".
# [[target]]
# agent     = "opencode-laptop"
# adapter   = "opencode"
# directory = "~/code/payments"       # session directory to wake
# url       = "http://127.0.0.1:4096" # optional

# --- Codex: runs `codex exec resume --last` with the prompt on stdin -----------------------------
# [[target]]
# agent   = "codex-laptop"
# adapter = "exec"
# command = ["codex", "exec", "resume", "--last", "-"]
# cwd     = "~/code/payments"
# timeout = "45m"

# --- Claude Code: runs `claude -p --continue` with the prompt on stdin ---------------------------
# [[target]]
# agent   = "claude-laptop"
# adapter = "exec"
# command = ["claude", "-p", "--continue"]
# cwd     = "~/code/payments"
# timeout = "45m"

# --- Webhook: POST {agent, project, text} to a URL (e.g. a chat bridge) --------------------------
# [[target]]
# agent   = "bridge"
# adapter = "webhook"
# url     = "https://example.com/hooks/corntchap"
"##;

/// A ready-to-paste `[[target]]` block for an agent of the given kind.
pub fn target_block(agent: &str, kind: &str) -> String {
    match kind {
        "opencode" => format!(
            "[[target]]\nagent     = \"{agent}\"\nadapter   = \"opencode\"\ndirectory = \"~/code/your-project\"\n# url = \"http://127.0.0.1:4096\"   # optional: the running OpenCode is found on its own\n"
        ),
        "codex" => format!(
            "[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"codex\", \"exec\", \"resume\", \"--last\", \"-\"]\ncwd     = \"~/code/your-project\"\ntimeout = \"45m\"\n"
        ),
        "claude-code" => format!(
            "[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"claude\", \"-p\", \"--continue\"]\ncwd     = \"~/code/your-project\"\ntimeout = \"45m\"\n"
        ),
        _ => format!(
            "[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"your-agent-cli\"]   # the prompt arrives on stdin\ncwd     = \"~/code/your-project\"\ntimeout = \"45m\"\n"
        ),
    }
}

/// `relay init`: write the template unless the file exists (or `force`).
pub fn init(force: bool) -> Result<()> {
    let p = config_path();
    if p.exists() && !force {
        bail!(
            "{} already exists; edit it, or pass --force to overwrite it",
            p.display()
        );
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, TEMPLATE)?;
    crate::ui::ok_line(format!("wrote {}", p.display()));
    println!(
        "Uncomment and edit the [[target]] entries, then run `corntchap-cli fire doctor`.\nTip: `corntchap-cli cook setup <name> --kind opencode|codex|claude-code` prints the matching block."
    );
    Ok(())
}

// ---- last event id, persisted so a restart does not lose wakes ----

fn state_path() -> PathBuf {
    config_dir().join("relay-state.json")
}

pub fn load_last_id(server: &str) -> Option<String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(state_path()).ok()?).ok()?;
    (v["server"].as_str() == Some(server))
        .then(|| v["last_event_id"].as_str().map(String::from))
        .flatten()
}

pub fn save_last_id(server: &str, id: &str) {
    let body = json!({"server": server, "last_event_id": id});
    if let Err(e) = std::fs::create_dir_all(config_dir())
        .and_then(|_| std::fs::write(state_path(), body.to_string()))
    {
        eprintln!("relay: could not save {}: {e}", state_path().display());
    }
}

/// Incremental Server-Sent Events parser. Works on bytes so multibyte characters split across
/// network chunks survive; lines are decoded only once complete.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    id: Option<String>,
    event: String,
    data: String,
}

#[derive(Debug, PartialEq)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event: String,
    pub data: String,
}

impl SseParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = vec![];
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw[..raw.len() - 1]);
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                if !self.event.is_empty() || !self.data.is_empty() || self.id.is_some() {
                    out.push(SseEvent {
                        id: self.id.take(),
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data),
                    });
                }
            } else if let Some(v) = line.strip_prefix("id:") {
                self.id = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("event:") {
                self.event = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
        out
    }
}

/// Parses `45m`, `1h30m`, `90s` or a bare number of seconds (`90`). Units: h, m, s.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let bad =
        || format!("invalid duration {s:?}: use e.g. 90, 90s, 45m, 2h or 1h30m (units h, m, s)");
    let t = s.trim();
    if t.is_empty() {
        return Err(bad());
    }
    if let Ok(n) = t.parse::<u64>() {
        return Ok(Duration::from_secs(n));
    }
    let mut total = 0u64;
    let mut rest = t;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
        if end == 0 {
            return Err(bad());
        }
        let n: u64 = rest[..end].parse().map_err(|_| bad())?;
        let mult = match rest[end..].chars().next() {
            Some('h') => 3600,
            Some('m') => 60,
            Some('s') => 1,
            _ => return Err(bad()),
        };
        total = n
            .checked_mul(mult)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(bad)?;
        rest = &rest[end + 1..];
    }
    Ok(Duration::from_secs(total))
}

pub fn expand(p: &str) -> String {
    shellexpand::tilde(p).into_owned()
}

#[derive(Default)]
struct RunState {
    running: bool,
    pending: Vec<String>, // projects woken while a run was in flight
}

type States = Arc<Mutex<HashMap<String, RunState>>>;

pub async fn run(mut api: Api) -> Result<()> {
    let cfg = load_config()?;
    if cfg.target.is_empty() {
        bail!(
            "no [[target]] in {}; see `corntchap-cli fire doctor`",
            config_path().display()
        );
    }
    if let Some(sv) = &cfg.server {
        api.server = sv.trim_end_matches('/').to_string();
    }
    let states: States = Arc::new(Mutex::new(HashMap::new()));
    let mut last_id: Option<String> = load_last_id(&api.server);
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
    let mut parser = SseParser::default();
    let mut s = resp.bytes_stream();
    while let Some(chunk) = s.next().await {
        for ev in parser.feed(&chunk?) {
            if ev.event == "wake"
                && !ev.data.is_empty()
                && let Ok(v) = serde_json::from_str::<Value>(&ev.data)
            {
                on_wake(api, cfg, states, &v).await;
            }
            if let Some(i) = ev.id {
                save_last_id(&api.server, &i);
                *last_id = Some(i);
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

/// `fire poke`: run one target now, without the server.
pub async fn poke(cook: &str, pot: &str) -> Result<()> {
    let cfg = load_config()?;
    let t = cfg.target.iter().find(|t| t.agent == cook).ok_or_else(|| {
        anyhow!(
            "no [[target]] with agent = {cook:?} in {}",
            config_path().display()
        )
    })?;
    match run_target(t, pot).await {
        0 => Ok(()),
        code => bail!("{cook}: the wake did not go through (exit {code})"),
    }
}

/// Returns an exit code (0 ok, -1 skipped/failed to start).
pub async fn run_target(t: &Target, project: &str) -> i32 {
    let prompt = PROMPT.replace("{project}", project);
    let res = match t.adapter.as_str() {
        "opencode" => crate::opencode::prompt(t, &prompt).await,
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
        .transpose()
        .map_err(|e| anyhow!("target {}: {e}", t.agent))?
        .unwrap_or(Duration::from_secs(45 * 60));
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => Ok(status?.code().unwrap_or(-1)),
        Err(_) => {
            let _ = child.kill().await;
            bail!("timed out after {limit:?}, killed")
        }
    }
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

/// `relay doctor` (full = false) and the top-level `doctor` (full = true, also checks the user login).
pub async fn doctor(full: bool) -> Result<()> {
    let creds = Credentials::load();
    let cfg = load_config();
    let server = cfg
        .as_ref()
        .ok()
        .and_then(|c| c.server.clone())
        .or_else(|| creds.server.clone())
        .ok_or_else(|| anyhow!("no server configured (run `corntchap-cli login <url>`)"))?;
    let http = reqwest::Client::new();
    match http
        .get(format!("{}/healthz", server.trim_end_matches('/')))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let ver = r
                .headers()
                .get("server")
                .and_then(|v| v.to_str().ok())
                .map(|v| format!(", server {v}"))
                .unwrap_or_default();
            println!(
                "ok   server reachable: {server} (cli {}{ver})",
                env!("CARGO_PKG_VERSION")
            )
        }
        Ok(r) => println!("FAIL server answered HTTP {}", r.status()),
        Err(e) => println!("FAIL server unreachable: {e}"),
    }
    if full {
        match Api::from_creds(false) {
            Err(e) => println!("FAIL {e:#}"),
            Ok(mut api) => {
                api.server = server.trim_end_matches('/').to_string();
                match api.get("/me").await {
                    Ok(me) => println!(
                        "ok   signed in as {} ({}{})",
                        me["name"].as_str().unwrap_or("?"),
                        me["kind"].as_str().unwrap_or("?"),
                        if me["is_admin"] == true {
                            ", admin"
                        } else {
                            ""
                        }
                    ),
                    Err(e) => println!("FAIL {e:#}"),
                }
            }
        }
    }
    match &creds.relay_token {
        None if full => println!(
            "--   no relay credential (only needed to wake local agents: `corntchap-cli fire enroll <name>`)"
        ),
        None => println!(
            "FAIL no relay credential: run `corntchap-cli fire enroll <name>` (or set CORNTCHAP_RELAY_TOKEN)"
        ),
        Some(_) => {
            let mut api = Api::from_creds(true)?;
            api.server = server.trim_end_matches('/').to_string();
            match api.get("/me").await {
                Ok(me) => println!(
                    "ok   relay credential valid ({})",
                    me["name"].as_str().unwrap_or("?")
                ),
                Err(e) => println!("FAIL relay credential rejected: {e}"),
            }
        }
    }
    let cfg = match cfg {
        Ok(c) => c,
        Err(e) => {
            println!("{} {e:#}", if full { "warn" } else { "FAIL" });
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
            "opencode" => match crate::opencode::resolve(t).await {
                Ok(ep) => println!(
                    "{}   {}: OpenCode at {} ({}{}), {} session(s) in {}",
                    if ep.sessions.is_empty() {
                        "FAIL"
                    } else {
                        "ok  "
                    },
                    t.agent,
                    ep.base,
                    ep.source,
                    if ep.auth.is_some() {
                        ", with password"
                    } else {
                        ""
                    },
                    ep.sessions.len(),
                    t.directory.as_deref().unwrap_or("any directory")
                ),
                Err(e) => println!("FAIL {}: {e:#}", t.agent),
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_survives_multibyte_split_across_chunks() {
        let msg =
            "id: 7\nevent: wake\ndata: {\"agent\":\"caf\u{e9}\",\"project\":\"P\"}\n\n".as_bytes();
        let split = msg.iter().position(|&b| b == 0xc3).unwrap() + 1; // between the two bytes of 'e acute'
        let mut p = SseParser::default();
        assert!(p.feed(&msg[..split]).is_empty());
        let evs = p.feed(&msg[split..]);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].id.as_deref(), Some("7"));
        assert_eq!(evs[0].event, "wake");
        let v: Value = serde_json::from_str(&evs[0].data).unwrap();
        assert_eq!(v["agent"], "caf\u{e9}");
    }

    #[test]
    fn sse_parser_handles_crlf_and_keepalive_comments() {
        let mut p = SseParser::default();
        let evs = p.feed(b": ping\r\n\r\nid: 1\r\nevent: wake\r\ndata: {}\r\n\r\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].data, "{}");
    }

    #[test]
    fn parse_duration_combines_units_and_rejects_garbage() {
        let secs = |s: &str| parse_duration(s).unwrap().as_secs();
        assert_eq!(secs("1h30m"), 5400);
        assert_eq!(secs("45m"), 2700);
        assert_eq!(secs("2h"), 7200);
        assert_eq!(secs("1m30s"), 90);
        assert_eq!(secs("90"), 90);
        assert_eq!(secs(" 10s "), 10);
        for bad in ["", "abc", "h", "1x", "1h30", "-5m", "1.5h"] {
            assert!(parse_duration(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn template_parses_as_empty_config() {
        let c: RelayConfig = toml::from_str(TEMPLATE).unwrap();
        assert!(c.target.is_empty());
        let blk = target_block("a", "codex");
        let c: RelayConfig = toml::from_str(&blk).unwrap();
        assert_eq!(c.target[0].adapter, "exec");
    }
}
