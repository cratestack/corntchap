//! `corntchap-cli relay`: one outbound SSE connection that turns `wake` events into local agent runs.

use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};

use crate::api::{Api, Credentials, config_dir};

pub const PROMPT: &str = "New work is ready in {project}. Call claim_next with project {project}, finish the task, then call it again until it returns none.";

/// The work prompt for a woken station: it must pass its id to claim_next so the server knows which
/// session holds the task.
pub const STATION_PROMPT: &str = "New work is ready in {project}. You are station {station}: pass station {station} to claim_next (and claim_task / ask). Call claim_next with project {project} and station {station}, finish the task, then call it again until it returns none.";

#[derive(Deserialize, Clone, Debug, PartialEq)]
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
    /// opencode: show wakes in the chat as your own messages (default: hidden, as synthetic input
    /// the agent acts on without a visible prompt).
    pub show_wakes: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct RelayConfig {
    /// Optional: connect here instead of the server stored by `corntchap-cli login`.
    pub server: Option<String>,
    #[serde(default)]
    pub target: Vec<Target>,
}

impl Target {
    /// A target with nothing but its adapter: the starting point for a station's runtime defaults.
    pub fn blank(agent: &str, adapter: &str) -> Target {
        Target {
            agent: agent.into(),
            adapter: adapter.into(),
            url: None,
            directory: None,
            username: None,
            password: None,
            password_env: None,
            command: None,
            cwd: None,
            timeout: None,
            show_wakes: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_config() -> Result<RelayConfig> {
    let p = config_path();
    let s = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        // no config.toml is fine: stations need no [[target]]
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RelayConfig::default()),
        Err(e) => return Err(anyhow!("reading {}: {e}", p.display())),
    };
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
#
# Normally you need NO [[target]] at all. Register the agent sessions you work in as stations,
# from the folder you run the agent in (a git checkout covers its worktrees too):
#   corntchap-cli station scan --pot KEY
# and the fire wakes exactly that session (OpenCode, Claude Code or Codex) in its own folder when
# work lands. A cook needs a relay wake target: corntchap-cli cook wake add <cook> --type relay
# Check everything with: corntchap-cli fire doctor
#
# A [[target]] is only a fallback (a wake that names no station) or a webhook, and may refine how a
# station's runtime is reached (an OpenCode password_env, a custom command containing {session}).
# `agent` is the cook's name on the server (see `corntchap-cli cook ls`).

# Optional: connect to this server instead of the one stored by `corntchap-cli login`.
# server = "https://corntchap.example.com"

# --- OpenCode: prompts the newest session in `directory` of a running OpenCode (no station) -----
# The server is found on its own (OpenCode v2's `opencode service status`, else the desktop app,
# TUI or `opencode serve`, whatever port it took); set `url` only to pin one. A password-protected
# 1.x server: password_env = "MY_OPENCODE_PASSWORD".
# [[target]]
# agent     = "opencode-laptop"
# adapter   = "opencode"
# directory = "~/code/payments"       # session directory to wake
# url       = "http://127.0.0.1:4096" # optional

# --- Codex: runs `codex exec resume --last` with the prompt on stdin (no station) ----------------
# [[target]]
# agent   = "codex-laptop"
# adapter = "exec"
# command = ["codex", "exec", "resume", "--last", "-"]
# cwd     = "~/code/payments"
# timeout = "45m"

# --- Claude Code: runs `claude -p --continue` with the prompt on stdin (no station) --------------
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
            "# Optional: stations (`corntchap-cli station scan`) wake the exact session; this is only the fallback.\n[[target]]\nagent     = \"{agent}\"\nadapter   = \"opencode\"\n# directory = \"~/code/your-project\"   # optional: only for wakes that name no station\n# url = \"http://127.0.0.1:4096\"   # optional: the running OpenCode is found on its own\n"
        ),
        "codex" => format!(
            "# Optional: stations (`corntchap-cli station scan`) wake the exact session; this is only the fallback.\n[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"codex\", \"exec\", \"resume\", \"--last\", \"-\"]\n# cwd   = \"~/code/your-project\"\ntimeout = \"45m\"\n"
        ),
        "claude-code" => format!(
            "# Optional: stations (`corntchap-cli station scan`) wake the exact session; this is only the fallback.\n[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"claude\", \"-p\", \"--continue\"]\n# cwd   = \"~/code/your-project\"\ntimeout = \"45m\"\n"
        ),
        _ => format!(
            "# Optional: stations (`corntchap-cli station scan`) wake the exact session; this is only the fallback.\n[[target]]\nagent   = \"{agent}\"\nadapter = \"exec\"\ncommand = [\"your-agent-cli\"]   # the prompt arrives on stdin\n# cwd   = \"~/code/your-project\"\ntimeout = \"45m\"\n"
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
        "Nothing else is needed: from the folder you run an agent in, `corntchap-cli station scan --pot KEY` registers your agent sessions as stations, then `corntchap-cli fire doctor`.\n[[target]] entries are optional (fallback / webhook); `corntchap-cli cook setup <name> --kind opencode|codex|claude-code` prints a matching block."
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
pub fn parse_duration(s: &str) -> Result<Duration, String> {
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

/// The session a wake names (a registered station).
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub id: String,
    pub runtime: String,
    pub session: String,
    pub directory: Option<String>,
    pub label: Option<String>,
    /// A session that has never run here (a station started from the server, `stationStart`): the
    /// fire starts it with that session id instead of resuming it.
    pub fresh: bool,
}

/// One `wake` event.
#[derive(Clone, Debug, PartialEq)]
pub struct Wake {
    pub agent: String,
    pub project: String,
    pub station: Option<Station>,
    /// The prompt the server worded (e.g. the answer to the cook's question); sent verbatim.
    pub text: Option<String>,
}

pub fn parse_wake(v: &Value) -> Option<Wake> {
    let str_of = |x: &Value, k: &str| {
        x[k].as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let station = v["station"].as_object().and_then(|_| {
        let st = &v["station"];
        Some(Station {
            id: str_of(st, "id")?,
            runtime: str_of(st, "runtime")?,
            session: str_of(st, "session")?,
            directory: str_of(st, "directory"),
            label: str_of(st, "label"),
            fresh: false,
        })
    });
    Some(Wake {
        agent: str_of(v, "agent")?,
        project: str_of(v, "project")?,
        station,
        text: v["text"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(String::from),
    })
}

/// Wakes run one at a time per station (else per cook), so two sessions of one cook are independent.
pub fn wake_key(w: &Wake) -> String {
    match &w.station {
        Some(s) => format!("station:{}", s.id),
        None => format!("agent:{}", w.agent),
    }
}

/// What to say: the server's own wording when present, else the work prompt (naming the station).
pub fn wake_prompt(w: &Wake) -> String {
    if let Some(t) = &w.text {
        return t.clone();
    }
    match &w.station {
        Some(s) => STATION_PROMPT
            .replace("{project}", &w.project)
            .replace("{station}", &s.id),
        None => PROMPT.replace("{project}", &w.project),
    }
}

/// How a wake is delivered.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Prompt this session of an OpenCode server; `target` only refines reaching the server.
    OpenCode {
        target: Target,
        session: String,
    },
    /// Run a command with the prompt on stdin.
    Exec {
        agent: String,
        command: Vec<String>,
        cwd: Option<String>,
        timeout: Option<String>,
    },
    /// No station: the cook's `[[target]]`, as before.
    Target(Target),
    Skip(String),
}

fn fill(arg: &str, s: &Station) -> String {
    arg.replace("{session}", &s.session)
        .replace("{directory}", s.directory.as_deref().unwrap_or("."))
}

/// Mark a Claude Code station whose transcript is not on this machine yet as fresh: it was started
/// from the server with a new session id, so the fire starts it (`--session-id`) instead of
/// resuming it.
pub fn mark_fresh(mut w: Wake, claude_root: Option<&std::path::Path>) -> Wake {
    if let (Some(s), Some(root)) = (w.station.as_mut(), claude_root)
        && s.runtime == "claude-code"
        && let Some(d) = &s.directory
    {
        let file = root
            .join(crate::station::claude_dir_name(&expand(d)))
            .join(format!("{}.jsonl", s.session));
        s.fresh = !file.exists();
    }
    w
}

/// Decide the adapter, command/cwd and session for a wake. Built-in defaults per runtime need no
/// `[[target]]`; one for the cook may refine them (OpenCode password/url, a custom command that
/// contains `{session}`, a timeout), but the session and directory always come from the station.
pub fn plan(w: &Wake, targets: &[Target]) -> Plan {
    let target = targets.iter().find(|t| t.agent == w.agent);
    let Some(s) = &w.station else {
        return match target {
            Some(t) => Plan::Target(t.clone()),
            None => Plan::Skip(format!(
                "{}: woken without a station and no [[target]] in config.toml: register a session with `corntchap-cli station scan`",
                w.agent
            )),
        };
    };
    let exec = |default: &[&str]| {
        let custom = target
            .filter(|t| t.adapter == "exec")
            .and_then(|t| t.command.as_ref())
            .filter(|c| !c.is_empty() && c.iter().any(|a| a.contains("{session}")));
        let command: Vec<String> = match custom {
            Some(c) => c.iter().map(|a| fill(a, s)).collect(),
            None => default.iter().map(|a| fill(a, s)).collect(),
        };
        Plan::Exec {
            agent: w.agent.clone(),
            command,
            cwd: s
                .directory
                .clone()
                .or_else(|| target.and_then(|t| t.cwd.clone())),
            timeout: target.and_then(|t| t.timeout.clone()),
        }
    };
    match s.runtime.as_str() {
        "opencode" => {
            let mut t = target
                .filter(|t| t.adapter == "opencode")
                .cloned()
                .unwrap_or_else(|| Target::blank(&w.agent, "opencode"));
            t.directory = None; // the session decides, not the folder
            Plan::OpenCode {
                target: t,
                session: s.session.clone(),
            }
        }
        "claude-code" if s.fresh => Plan::Exec {
            agent: w.agent.clone(),
            command: ["claude", "-p", "--session-id", "{session}"]
                .iter()
                .map(|a| fill(a, s))
                .collect(),
            cwd: s.directory.clone(),
            timeout: target.and_then(|t| t.timeout.clone()),
        },
        "claude-code" => exec(&["claude", "-p", "--resume", "{session}"]),
        "codex" => exec(&["codex", "exec", "resume", "{session}", "-"]),
        other => match target {
            Some(t) => Plan::Target(t.clone()),
            None => Plan::Skip(format!(
                "{}: station {} has runtime {other:?}, which the fire cannot resume by itself: add a [[target]] for it in config.toml",
                w.agent, s.id
            )),
        },
    }
}

#[derive(Default)]
struct RunState {
    running: bool,
    pending: Vec<Wake>, // wakes that came while a run was in flight
}

type States = Arc<Mutex<HashMap<String, RunState>>>;

pub async fn run(mut api: Api) -> Result<()> {
    let cfg = load_config()?;
    if let Some(sv) = &cfg.server {
        api.server = sv.trim_end_matches('/').to_string();
    }
    if cfg.target.is_empty() {
        eprintln!(
            "relay: no [[target]] in {}: waking registered stations only (`corntchap-cli station scan`)",
            config_path().display()
        );
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
            if ev.event == "shutdown" {
                eprintln!("relay: the server is shutting down; reconnecting");
                return Ok(());
            }
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
    let Some(w) = parse_wake(wake) else {
        return;
    };
    if let Plan::Skip(why) = plan(&w, &cfg.target) {
        eprintln!("relay: {why}");
        return;
    }
    let key = wake_key(&w);
    {
        let mut g = states.lock().await;
        let st = g.entry(key.clone()).or_default();
        if st.running {
            // Coalesce into one follow-up run when the current one ends.
            if !st.pending.contains(&w) {
                st.pending.push(w);
            }
            return;
        }
        st.running = true;
    }
    let (api, states, targets) = (api.clone(), states.clone(), cfg.target.clone());
    tokio::spawn(async move {
        let mut w = w;
        loop {
            report(&api, &w, "started", None).await;
            let code = run_wake(&w, &targets).await;
            report(&api, &w, "finished", Some(code)).await;
            let mut g = states.lock().await;
            let st = g.entry(key.clone()).or_default();
            if st.pending.is_empty() {
                st.running = false;
                return;
            }
            w = st.pending.remove(0);
        }
    });
}

async fn report(api: &Api, w: &Wake, event: &str, exit: Option<i32>) {
    let _ = api
        .post(
            "/relay/runs",
            json!({
                "agent": w.agent, "project": w.project, "event": event, "exit_code": exit,
                "station": w.station.as_ref().map(|s| s.id.clone()),
            }),
        )
        .await;
}

/// `fire poke`: run one target now, without the server.
pub async fn poke(cook: &str, pot: &str) -> Result<()> {
    let cfg = load_config()?;
    let t = cfg.target.iter().find(|t| t.agent == cook).ok_or_else(|| {
        anyhow!(
            "no [[target]] with agent = {cook:?} in {} (stations are woken by the server; poke only tries a [[target]])",
            config_path().display()
        )
    })?;
    match run_target(t, pot).await {
        0 => Ok(()),
        code => bail!("{cook}: the wake did not go through (exit {code})"),
    }
}

/// Deliver one wake; returns an exit code (0 ok, -1 skipped/failed to start).
pub async fn run_wake(w: &Wake, targets: &[Target]) -> i32 {
    let fresh = mark_fresh(w.clone(), crate::station::claude_root().as_deref());
    let w = &fresh;
    let prompt = wake_prompt(w);
    let who = match &w.station {
        Some(s) => format!("{} ({})", w.agent, s.id),
        None => w.agent.clone(),
    };
    let res = match plan(w, targets) {
        Plan::Skip(why) => Err(anyhow!("{why}")),
        Plan::OpenCode { target, session } => {
            crate::opencode::prompt_station(&target, &session, &prompt).await
        }
        Plan::Exec {
            agent,
            command,
            cwd,
            timeout,
        } => {
            exec_command(
                &agent,
                &command,
                cwd.as_deref(),
                timeout.as_deref(),
                &prompt,
            )
            .await
        }
        Plan::Target(t) => deliver_target(&t, &prompt, &w.project).await,
    };
    match res {
        Ok(code) => {
            eprintln!("relay: {who} finished ({code})");
            code
        }
        Err(e) => {
            eprintln!("relay: {who}: {e:#}");
            -1
        }
    }
}

/// Returns an exit code (0 ok, -1 skipped/failed to start).
pub async fn run_target(t: &Target, project: &str) -> i32 {
    let prompt = PROMPT.replace("{project}", project);
    match deliver_target(t, &prompt, project).await {
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

async fn deliver_target(t: &Target, prompt: &str, project: &str) -> Result<i32> {
    match t.adapter.as_str() {
        "opencode" => crate::opencode::prompt(t, prompt).await,
        "exec" => {
            let cmd = t
                .command
                .as_ref()
                .filter(|c| !c.is_empty())
                .ok_or_else(|| anyhow!("exec target needs command"))?;
            exec_command(
                &t.agent,
                cmd,
                t.cwd.as_deref(),
                t.timeout.as_deref(),
                prompt,
            )
            .await
        }
        "webhook" => webhook(t, prompt, project).await,
        other => Err(anyhow!("unknown adapter {other}")),
    }
}

async fn exec_command(
    agent: &str,
    cmd: &[String],
    cwd: Option<&str>,
    timeout: Option<&str>,
    prompt: &str,
) -> Result<i32> {
    let mut c = Command::new(program(&expand(&cmd[0])));
    c.args(cmd[1..].iter().map(|a| expand(a)))
        .env("PATH", search_path());
    if let Some(cwd) = cwd {
        let dir = expand(cwd);
        if !std::path::Path::new(&dir).is_dir() {
            bail!(
                "directory {dir} does not exist: re-scan with `corntchap-cli station scan` (or remove the station)"
            );
        }
        c.current_dir(dir);
    }
    c.stdin(Stdio::piped()).kill_on_drop(true);
    eprintln!(
        "relay: starting {agent}: {}{}",
        cmd.join(" "),
        cwd.map(|d| format!(" (in {d})")).unwrap_or_default()
    );
    let mut child = c.spawn().with_context(|| format!("starting {}", cmd[0]))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        drop(stdin); // prompt on stdin
    }
    let limit = timeout
        .map(parse_duration)
        .transpose()
        .map_err(|e| anyhow!("target {agent}: {e}"))?
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
                        crate::ui::person(&me),
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
        println!(
            "--   no [[target]] in {} (fine: stations are woken by the server)",
            config_path().display()
        );
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
                    "{}   {}: OpenCode at {} ({}, {}{}), {} session(s) in {}",
                    if t.directory.is_some() && ep.sessions.is_empty() {
                        "FAIL"
                    } else {
                        "ok  "
                    },
                    t.agent,
                    ep.base,
                    ep.api.label(),
                    ep.source,
                    if ep.auth.is_some() {
                        ", with password"
                    } else {
                        ""
                    },
                    ep.sessions.len(),
                    t.directory.as_deref().unwrap_or(
                        "no directory (wakes go to stations; this is only the fallback for wakes that name none)"
                    )
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
    crate::station::doctor(&server).await;
    Ok(())
}

pub fn which(cmd: &str) -> bool {
    resolve_in(cmd, Some(&search_path()), &pathext(), &|p| p.exists()).is_some()
}

static SEARCH_PATH: std::sync::OnceLock<std::ffi::OsString> = std::sync::OnceLock::new();

/// Where to look for agent programs: this process's PATH, then your login shell's PATH, then the
/// usual install folders. A service (systemd, launchd) starts with a bare PATH that misses
/// `~/.local/bin`, Homebrew, npm or bun globals: that is where `claude`, `codex` and `opencode` live.
/// Children get it too, so `claude` (a node script) finds `node`.
pub fn search_path() -> std::ffi::OsString {
    SEARCH_PATH
        .get_or_init(|| {
            let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect())
                .unwrap_or_default();
            if !cfg!(windows) {
                if let Some(p) = login_shell_path() {
                    dirs.extend(std::env::split_paths(&p));
                }
                if let Some(h) = dirs::home_dir() {
                    for d in [
                        ".local/bin",
                        ".claude/local",
                        ".npm-global/bin",
                        ".bun/bin",
                        ".cargo/bin",
                        ".volta/bin",
                        ".opencode/bin",
                        "bin",
                    ] {
                        dirs.push(h.join(d));
                    }
                }
                for d in [
                    "/opt/homebrew/bin",
                    "/usr/local/bin",
                    "/home/linuxbrew/.linuxbrew/bin",
                    "/usr/bin",
                    "/bin",
                ] {
                    dirs.push(PathBuf::from(d));
                }
            }
            let mut seen = std::collections::HashSet::new();
            dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
            std::env::join_paths(dirs)
                .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
        })
        .clone()
}

/// `$SHELL -l -c 'printf %s "$PATH"'`, given 3 seconds (a shell that prompts or hangs is ignored).
fn login_shell_path() -> Option<std::ffi::OsString> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(3)).ok()?.ok()?;
    let p = String::from_utf8(out.stdout).ok()?;
    let p = p.trim();
    (!p.is_empty()).then(|| p.into())
}

/// The extensions a bare command name may carry: `PATHEXT` on Windows (npm shims are `.cmd`), none elsewhere.
fn pathext() -> Vec<String> {
    if !cfg!(windows) {
        return vec![];
    }
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(String::from)
        .collect()
}

/// Find `cmd` the way a shell would: as given, then with each of `exts`, in each `PATH` directory
/// (or just as a path when it has a separator). `exists` is injected so this can be tested anywhere.
fn resolve_in(
    cmd: &str,
    path: Option<&std::ffi::OsStr>,
    exts: &[String],
    exists: &dyn Fn(&std::path::Path) -> bool,
) -> Option<PathBuf> {
    let with_exts = |base: PathBuf| -> Option<PathBuf> {
        if exists(&base) {
            return Some(base);
        }
        exts.iter().find_map(|e| {
            let mut s = base.clone().into_os_string();
            s.push(e);
            let p = PathBuf::from(s);
            exists(&p).then_some(p)
        })
    };
    if cmd.contains('/') || (cfg!(windows) && cmd.contains('\\')) {
        return with_exts(PathBuf::from(cmd));
    }
    std::env::split_paths(path?).find_map(|d| with_exts(d.join(cmd)))
}

/// The program to hand to `Command::new`: the full path found on [`search_path`] (on Windows
/// through `PATHEXT`, so `claude` finds `claude.cmd`), else the name unchanged.
pub fn program(cmd: &str) -> PathBuf {
    resolve_in(cmd, Some(&search_path()), &pathext(), &|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(cmd))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const NO_SERVICE_ON_WINDOWS: &str =
    "not supported on Windows; run `corntchap-cli fire` in a terminal or as a scheduled task";

/// systemd user unit (Linux) or launchd agent (macOS) so the relay starts at login.
pub fn install() -> Result<()> {
    if cfg!(windows) {
        bail!(NO_SERVICE_ON_WINDOWS);
    }
    let exe = std::env::current_exe()?;
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    if cfg!(target_os = "macos") {
        let path = unit_path(Platform::Mac, &home);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.corntchap.relay</string>\n<key>ProgramArguments</key><array><string>{}</string><string>relay</string></array>\n<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n<key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>\n<key>StandardErrorPath</key><string>{}/Library/Logs/corntchap-relay.log</string>\n</dict></plist>\n",
                exe.display(),
                xml_escape(&search_path().to_string_lossy()),
                home.display()
            ),
        )?;
        println!(
            "wrote {}\nstart it now with: launchctl load {}",
            path.display(),
            path.display()
        );
    } else {
        let path = unit_path(Platform::Linux, &home);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "[Unit]\nDescription=corntchap relay\nAfter=network-online.target\n\n[Service]\nExecStart={} relay\nEnvironment=\"PATH={}\"\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
                exe.display(),
                search_path()
                    .to_string_lossy()
                    .replace('"', "")
                    .replace('%', "%%")
            ),
        )?;
        println!(
            "wrote {}\nenable it with: systemctl --user daemon-reload && systemctl --user enable --now corntchap-relay",
            path.display()
        );
    }
    Ok(())
}

// ---- fire uninstall ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    Linux,
    Mac,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

const SYSTEMD_UNIT: &str = "corntchap-relay";
const LAUNCHD_LABEL: &str = "dev.corntchap.relay";

/// Where `fire install` writes the service definition.
pub fn unit_path(platform: Platform, home: &std::path::Path) -> PathBuf {
    match platform {
        Platform::Mac => home.join(format!("Library/LaunchAgents/{LAUNCHD_LABEL}.plist")),
        Platform::Linux => home.join(format!(".config/systemd/user/{SYSTEMD_UNIT}.service")),
    }
}

/// One thing `fire uninstall` does, in order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Run a command; if it fails and `alt` is set, run that instead. Failures are tolerated.
    Run {
        cmd: Vec<String>,
        alt: Option<Vec<String>>,
    },
    /// Delete a file (a missing file means it was never there).
    Remove(PathBuf),
    /// Drop `relay_token` from credentials.toml, keeping the user's own login.
    DropRelayToken(PathBuf),
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// The ordered plan of `fire uninstall`; pure, so it can be tested without touching the system.
/// `uid` is the numeric user id launchd needs for `bootout gui/<uid>/<label>`.
pub fn uninstall_plan(
    platform: Platform,
    home: &std::path::Path,
    cfg_dir: &std::path::Path,
    uid: &str,
    purge: bool,
) -> Vec<Step> {
    let unit = unit_path(platform, home);
    let mut steps = match platform {
        Platform::Linux => vec![
            Step::Run {
                cmd: argv(&["systemctl", "--user", "stop", SYSTEMD_UNIT]),
                alt: None,
            },
            Step::Run {
                cmd: argv(&["systemctl", "--user", "disable", SYSTEMD_UNIT]),
                alt: None,
            },
            Step::Remove(unit),
            Step::Run {
                cmd: argv(&["systemctl", "--user", "daemon-reload"]),
                alt: None,
            },
        ],
        Platform::Mac => vec![
            Step::Run {
                cmd: argv(&[
                    "launchctl",
                    "bootout",
                    &format!("gui/{uid}/{LAUNCHD_LABEL}"),
                ]),
                alt: Some(argv(&["launchctl", "unload", &unit.display().to_string()])),
            },
            Step::Remove(unit),
        ],
    };
    if purge {
        steps.push(Step::Remove(cfg_dir.join("config.toml")));
        steps.push(Step::Remove(cfg_dir.join("relay-state.json")));
        steps.push(Step::DropRelayToken(cfg_dir.join("credentials.toml")));
    }
    steps
}

/// A step as one line of text, for the confirmation prompt.
pub fn describe_step(s: &Step) -> String {
    match s {
        Step::Run { cmd, alt } => match alt {
            Some(a) => format!("run {} (else {})", cmd.join(" "), a.join(" ")),
            None => format!("run {}", cmd.join(" ")),
        },
        Step::Remove(p) => format!("remove {}", p.display()),
        Step::DropRelayToken(p) => format!("remove the relay credential from {}", p.display()),
    }
}

fn run_cmd(cmd: &[String]) -> std::result::Result<(), String> {
    let out = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{} is not available here", cmd[0])
            } else {
                e.to_string()
            }
        })?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let first = err.lines().next().unwrap_or("").trim();
    Err(if first.is_empty() {
        format!("exited with {}", out.status)
    } else {
        first.to_string()
    })
}

fn do_step(s: &Step) -> Result<()> {
    use crate::ui::{dim, ok_line, warn_line};
    match s {
        Step::Run { cmd, alt } => {
            let line = cmd.join(" ");
            match run_cmd(cmd) {
                Ok(()) => ok_line(line),
                Err(first) => match alt.as_ref().map(|a| (a.join(" "), run_cmd(a))) {
                    Some((l, Ok(()))) => ok_line(l),
                    Some((l, Err(e))) => println!("{} ({e})", dim(&format!("skipped {l}"))),
                    None => println!("{} ({first})", dim(&format!("skipped {line}"))),
                },
            }
        }
        Step::Remove(p) => match std::fs::remove_file(p) {
            Ok(()) => ok_line(format!("removed {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("{}", dim(&format!("{} was not there", p.display())))
            }
            Err(e) => warn_line(format!("could not remove {}: {e}", p.display())),
        },
        Step::DropRelayToken(_) => match Credentials::drop_relay_token() {
            Ok(true) => ok_line("removed the relay credential (your own login is kept)"),
            Ok(false) => println!("{}", dim("no relay credential was stored")),
            Err(e) => warn_line(format!("{e:#}")),
        },
    }
    Ok(())
}

/// `fire uninstall`: stop and disable the service, delete what `fire install` wrote; --purge also
/// removes the relay's own config, state file and credential.
pub fn uninstall(purge: bool, yes: bool) -> Result<()> {
    use std::io::{IsTerminal, Write};
    if cfg!(windows) {
        bail!(NO_SERVICE_ON_WINDOWS);
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "501".into());
    let plan = uninstall_plan(Platform::current(), &home, &config_dir(), &uid, purge);
    if !yes && std::io::stdin().is_terminal() {
        println!("This will:");
        for s in &plan {
            println!("  {}", describe_step(s));
        }
        print!("Put out the fire? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut a = String::new();
        if !(std::io::stdin().read_line(&mut a).is_ok() && matches!(a.trim(), "y" | "Y" | "yes")) {
            println!("kept the fire lit");
            return Ok(());
        }
    }
    for s in &plan {
        do_step(s)?;
    }
    crate::ui::ok_line(if purge {
        "the fire is out and its files are gone"
    } else {
        "the fire is out (config and credentials kept; add --purge to remove them)"
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // The launchd/systemd plans only run on macOS/Linux (`fire install`/`uninstall` refuse on Windows),
    // and their expected paths use `/`.
    #[cfg(unix)]
    #[test]
    fn linux_plan_stops_disables_removes_and_reloads() {
        let plan = uninstall_plan(
            Platform::Linux,
            Path::new("/h"),
            Path::new("/h/.config/corntchap"),
            "1000",
            false,
        );
        let lines: Vec<String> = plan.iter().map(describe_step).collect();
        assert_eq!(
            lines,
            vec![
                "run systemctl --user stop corntchap-relay",
                "run systemctl --user disable corntchap-relay",
                "remove /h/.config/systemd/user/corntchap-relay.service",
                "run systemctl --user daemon-reload",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn mac_plan_boots_out_with_unload_fallback() {
        let plan = uninstall_plan(
            Platform::Mac,
            Path::new("/Users/a"),
            Path::new("/c"),
            "501",
            false,
        );
        assert_eq!(
            plan[0],
            Step::Run {
                cmd: argv(&["launchctl", "bootout", "gui/501/dev.corntchap.relay"]),
                alt: Some(argv(&[
                    "launchctl",
                    "unload",
                    "/Users/a/Library/LaunchAgents/dev.corntchap.relay.plist"
                ])),
            }
        );
        assert_eq!(
            plan[1],
            Step::Remove(PathBuf::from(
                "/Users/a/Library/LaunchAgents/dev.corntchap.relay.plist"
            ))
        );
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn purge_adds_the_relay_files_and_credential_last() {
        let plain = uninstall_plan(
            Platform::Linux,
            Path::new("/h"),
            Path::new("/c"),
            "1",
            false,
        );
        let purge = uninstall_plan(Platform::Linux, Path::new("/h"), Path::new("/c"), "1", true);
        assert_eq!(&purge[..plain.len()], &plain[..]);
        assert_eq!(
            &purge[plain.len()..],
            &[
                Step::Remove(PathBuf::from("/c/config.toml")),
                Step::Remove(PathBuf::from("/c/relay-state.json")),
                Step::DropRelayToken(PathBuf::from("/c/credentials.toml")),
            ]
        );
    }

    #[test]
    fn unit_path_matches_what_install_writes() {
        assert_eq!(
            unit_path(Platform::Linux, Path::new("/h")),
            PathBuf::from("/h/.config/systemd/user/corntchap-relay.service")
        );
    }

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

    fn wake(v: Value) -> Wake {
        parse_wake(&v).unwrap()
    }

    fn st(runtime: &str, dir: Option<&str>) -> Value {
        json!({"id": "st_abc", "runtime": runtime, "session": "sess-1", "directory": dir, "label": null})
    }

    #[test]
    fn wake_without_a_station_uses_the_cooks_target_or_is_skipped() {
        let w = wake(json!({"agent": "c", "project": "PAY", "station": null}));
        assert_eq!(wake_prompt(&w), PROMPT.replace("{project}", "PAY"));
        assert_eq!(wake_key(&w), "agent:c");
        assert!(matches!(plan(&w, &[]), Plan::Skip(_)));
        let t = Target::blank("c", "exec");
        assert_eq!(plan(&w, std::slice::from_ref(&t)), Plan::Target(t));
    }

    #[test]
    fn a_started_claude_session_without_a_transcript_is_started_not_resumed() {
        let root = std::env::temp_dir().join(format!("ct-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = wake(
            json!({"agent": "c", "project": "PAY", "station": st("claude-code", Some("/w/pay"))}),
        );
        let w = mark_fresh(w, Some(&root));
        assert!(w.station.as_ref().unwrap().fresh);
        // a custom resume command does not apply to a session that does not exist yet
        let mut t = Target::blank("c", "exec");
        t.command = Some(vec!["my-claude".into(), "--resume={session}".into()]);
        match plan(&w, std::slice::from_ref(&t)) {
            Plan::Exec { command, cwd, .. } => {
                assert_eq!(command, ["claude", "-p", "--session-id", "sess-1"]);
                assert_eq!(cwd.as_deref(), Some("/w/pay"));
            }
            other => panic!("{other:?}"),
        }
        // once its transcript is on disk it is resumed like any other
        let dir = root.join("-w-pay");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-1.jsonl"), "{}\n").unwrap();
        let w = mark_fresh(
            wake(
                json!({"agent": "c", "project": "PAY", "station": st("claude-code", Some("/w/pay"))}),
            ),
            Some(&root),
        );
        assert!(!w.station.as_ref().unwrap().fresh);
        match plan(&w, &[]) {
            Plan::Exec { command, .. } => {
                assert_eq!(command, ["claude", "-p", "--resume", "sess-1"])
            }
            other => panic!("{other:?}"),
        }
        // other runtimes are never fresh
        let w = mark_fresh(
            wake(json!({"agent": "c", "project": "PAY", "station": st("codex", Some("/w/pay"))})),
            Some(&root),
        );
        assert!(!w.station.as_ref().unwrap().fresh);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn station_wakes_need_no_target() {
        let w = wake(
            json!({"agent": "c", "project": "PAY", "station": st("claude-code", Some("/w/pay"))}),
        );
        assert_eq!(wake_key(&w), "station:st_abc");
        assert_eq!(
            plan(&w, &[]),
            Plan::Exec {
                agent: "c".into(),
                command: ["claude", "-p", "--resume", "sess-1"]
                    .map(String::from)
                    .to_vec(),
                cwd: Some("/w/pay".into()),
                timeout: None,
            }
        );
        let w =
            wake(json!({"agent": "c", "project": "PAY", "station": st("codex", Some("/w/pay"))}));
        assert_eq!(
            plan(&w, &[]),
            Plan::Exec {
                agent: "c".into(),
                command: ["codex", "exec", "resume", "sess-1", "-"]
                    .map(String::from)
                    .to_vec(),
                cwd: Some("/w/pay".into()),
                timeout: None,
            }
        );
        let w = wake(
            json!({"agent": "c", "project": "PAY", "station": st("opencode", Some("/w/pay"))}),
        );
        match plan(&w, &[]) {
            Plan::OpenCode { target, session } => {
                assert_eq!(session, "sess-1");
                assert_eq!(target.directory, None);
            }
            p => panic!("{p:?}"),
        }
        let w = wake(json!({"agent": "c", "project": "PAY", "station": st("other", None)}));
        assert!(matches!(plan(&w, &[]), Plan::Skip(m) if m.contains("[[target]]")));
    }

    #[test]
    fn a_target_refines_but_never_replaces_session_and_directory() {
        let mut oc = Target::blank("c", "opencode");
        oc.password_env = Some("PW".into());
        oc.directory = Some("/somewhere/else".into());
        let w = wake(
            json!({"agent": "c", "project": "PAY", "station": st("opencode", Some("/w/pay"))}),
        );
        match plan(&w, &[oc]) {
            Plan::OpenCode { target, session } => {
                assert_eq!(target.password_env.as_deref(), Some("PW"));
                assert_eq!(target.directory, None);
                assert_eq!(session, "sess-1");
            }
            p => panic!("{p:?}"),
        }
        // a custom command is used only when it names {session}; the old `--continue` one is not
        let mut ex = Target::blank("c", "exec");
        ex.command = Some(vec!["claude".into(), "-p".into(), "--continue".into()]);
        ex.cwd = Some("/old".into());
        ex.timeout = Some("10m".into());
        let w = wake(
            json!({"agent": "c", "project": "PAY", "station": st("claude-code", Some("/w/pay"))}),
        );
        let Plan::Exec {
            command,
            cwd,
            timeout,
            ..
        } = plan(&w, std::slice::from_ref(&ex))
        else {
            panic!()
        };
        assert_eq!(command, ["claude", "-p", "--resume", "sess-1"]);
        assert_eq!(cwd.as_deref(), Some("/w/pay"));
        assert_eq!(timeout.as_deref(), Some("10m"));
        ex.command = Some(vec!["my-claude".into(), "--resume={session}".into()]);
        let Plan::Exec { command, .. } = plan(&w, &[ex]) else {
            panic!()
        };
        assert_eq!(command, ["my-claude", "--resume=sess-1"]);
    }

    #[test]
    fn station_prompt_names_the_station_and_server_text_wins() {
        let w = wake(json!({"agent": "c", "project": "PAY", "station": st("codex", None)}));
        let p = wake_prompt(&w);
        assert!(p.contains("You are station st_abc"), "{p}");
        assert!(p.contains("pass station st_abc to claim_next"), "{p}");
        assert!(p.contains("project PAY and station st_abc"), "{p}");
        let w = wake(json!({
            "agent": "c", "project": "PAY", "station": st("codex", None),
            "text": "The answer to your question is: yes.", "reason": "answer"
        }));
        assert_eq!(wake_prompt(&w), "The answer to your question is: yes.");
    }

    #[test]
    fn two_stations_of_one_cook_have_separate_keys() {
        let a = wake(json!({"agent": "c", "project": "P", "station": st("codex", None)}));
        let mut s2 = st("codex", None);
        s2["id"] = json!("st_two");
        let b = wake(json!({"agent": "c", "project": "P", "station": s2}));
        assert_ne!(wake_key(&a), wake_key(&b));
    }

    #[test]
    fn resolve_honours_the_extension_list() {
        let path = std::env::join_paths(["/a", "/b"]).unwrap();
        // `Path::join` uses `\\` on Windows: compare with `/` so the same cases run on every OS.
        let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
        let has = move |files: &'static [&'static str]| {
            move |p: &Path| files.contains(&slash(p).as_str())
        };
        let found = |r: Option<PathBuf>| r.map(|p| slash(&p));
        let exts = vec![".EXE".to_string(), ".CMD".to_string()];
        // an npm shim is found through the extension list, the first directory and extension win
        assert_eq!(
            found(resolve_in(
                "claude",
                Some(&path),
                &exts,
                &has(&["/b/claude.CMD", "/b/claude.EXE"])
            )),
            Some("/b/claude.EXE".to_string())
        );
        assert_eq!(
            found(resolve_in(
                "claude",
                Some(&path),
                &exts,
                &has(&["/b/claude.CMD"])
            )),
            Some("/b/claude.CMD".to_string())
        );
        // the bare name still wins where it exists, and with no extensions (Unix) nothing is appended
        assert_eq!(
            found(resolve_in(
                "claude",
                Some(&path),
                &exts,
                &has(&["/a/claude", "/a/claude.EXE"])
            )),
            Some("/a/claude".to_string())
        );
        assert_eq!(
            found(resolve_in(
                "claude",
                Some(&path),
                &[],
                &has(&["/a/claude.CMD"])
            )),
            None
        );
        assert_eq!(
            found(resolve_in("claude", None, &exts, &has(&["/a/claude"]))),
            None
        );
        // a path with a separator is not searched for
        assert_eq!(
            found(resolve_in(
                "/x/tool",
                Some(&path),
                &exts,
                &has(&["/x/tool.CMD", "/a/tool"])
            )),
            Some("/x/tool.CMD".to_string())
        );
    }
}
