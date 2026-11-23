//! `corntchap-cli hook`: agent hooks (Claude Code `PostToolUse`, an OpenCode plugin) that snapshot
//! the work tree into a hidden git ref after each edit or command, and tell corntchap what
//! happened so the console can show an activity timeline.
//!
//! Checkpoints never touch the user's index, HEAD or branch: the tree is built in a temporary
//! index (`GIT_INDEX_FILE`), committed with `git commit-tree` and stored in
//! `refs/corntchap/checkpoints/<session>`. Only paths and a short summary are sent, never contents.
//! A hook never fails the agent: it prints nothing, logs errors to `hook.log` in the config dir,
//! and always exits 0.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};

use crate::{
    api::{Api, config_dir},
    ui::{self, ok_line, warn_line},
};

const REF_PREFIX: &str = "refs/corntchap/checkpoints/";
const CHECKPOINT_AUTHOR: &str = "corntchap checkpoint";
const MAX_FILES: usize = 50;
const MAX_SUMMARY: usize = 200;
const CLAUDE_COMMAND: &str = "corntchap-cli hook claude-code";
const CLAUDE_MATCHER: &str = "Edit|Write|MultiEdit|NotebookEdit|Bash";
/// Tools after which a checkpoint is taken (lowercase; Claude Code and OpenCode spellings).
const CHANGING: &[&str] = &[
    "edit",
    "write",
    "multiedit",
    "notebookedit",
    "bash",
    "patch",
    "apply_patch",
    "applypatch",
    "shell",
    // GitHub Copilot (CLI and VS Code agent mode); names as documented or seen, best effort
    "create",
    "powershell",
    "str_replace_editor",
    "editfiles",
    "createfile",
    "create_file",
    "replace_string_in_file",
    "multi_replace_string_in_file",
    "insert_edit_into_file",
    "run_in_terminal",
    "runinterminal",
    "runcommands",
];
/// Tools whose input is a shell command line.
const SHELLS: &[&str] = &[
    "bash",
    "shell",
    "powershell",
    "run_in_terminal",
    "runinterminal",
];
const COPILOT_COMMAND: &str = "corntchap-cli hook copilot";
const COPILOT_FILE: &str = "corntchap.json";

#[derive(Clone, Copy, ValueEnum)]
pub enum Runtime {
    #[value(name = "claude-code")]
    ClaudeCode,
    #[value(name = "opencode")]
    Opencode,
    #[value(name = "copilot")]
    Copilot,
}

#[derive(Subcommand)]
pub enum HookCmd {
    /// Claude Code hook: reads the PostToolUse event JSON on stdin (installed by `hook install claude-code`).
    #[command(name = "claude-code")]
    ClaudeCode,
    /// OpenCode hook: reads the event JSON the plugin sends on stdin (installed by `hook install opencode`).
    #[command(name = "opencode")]
    Opencode,
    /// GitHub Copilot hook (CLI, desktop app, VS Code agent mode): reads the PostToolUse event JSON on stdin (installed by `hook install copilot`). Best effort.
    #[command(name = "copilot")]
    Copilot,
    /// Install the hook: merge it into .claude/settings.json (Claude Code), write the OpenCode plugin or .github/hooks/corntchap.json (Copilot).
    Install {
        runtime: Runtime,
        /// User-wide (~/.claude/settings.json, ~/.config/opencode/plugins) instead of this project.
        #[arg(long)]
        global: bool,
    },
    /// Remove the hook again (turns checkpoints and activity reports off for that runtime).
    Uninstall {
        runtime: Runtime,
        #[arg(long)]
        global: bool,
    },
}

pub async fn run(cmd: HookCmd) -> Result<()> {
    match cmd {
        HookCmd::ClaudeCode => event_main("claude-code").await,
        HookCmd::Opencode => event_main("opencode").await,
        HookCmd::Copilot => event_main("copilot").await,
        HookCmd::Install { runtime, global } => install(runtime, global),
        HookCmd::Uninstall { runtime, global } => uninstall(runtime, global),
    }
    Ok(())
}

// ---------------------------------------------------------------- event handling

/// What a runtime's hook reported.
#[derive(Debug, PartialEq)]
pub struct Event {
    pub runtime: String,
    pub session: String,
    pub cwd: PathBuf,
    pub tool: String,
    /// Paths named by the tool input, relative to `cwd` when below it.
    pub files: Vec<String>,
    pub summary: String,
}

fn str_of<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub fn clip(s: &str, max: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut t: String = one.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn rel(cwd: &Path, p: &str) -> String {
    Path::new(p)
        .strip_prefix(cwd)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.to_string())
}

/// Parse a hook payload. Claude Code sends `session_id`, `cwd`, `hook_event_name`, `tool_name`,
/// `tool_input`; the OpenCode plugin sends the same names, and `sessionID`/`tool`/`args` are
/// accepted too. GitHub Copilot sends the same snake_case names to a hook registered under a
/// PascalCase event (`PostToolUse`), and `sessionId`/`toolName`/`toolArgs` under a camelCase one
/// (`toolArgs` may be a JSON string). `None` when it is not a tool event we handle.
pub fn parse(runtime: &str, v: &Value) -> Option<Event> {
    if let Some(ev) = str_of(v, &["hook_event_name"])
        && !matches!(ev, "PostToolUse" | "postToolUse" | "tool.execute.after")
    {
        return None;
    }
    let session = str_of(v, &["session_id", "sessionID", "sessionId"])?.to_string();
    let cwd = PathBuf::from(str_of(v, &["cwd", "directory"])?);
    let tool = str_of(v, &["tool_name", "toolName", "tool"])?.to_string();
    let mut input = v
        .get("tool_input")
        .or_else(|| v.get("toolArgs"))
        .or_else(|| v.get("toolInput"))
        .or_else(|| v.get("args"))
        .cloned()
        .unwrap_or(Value::Null);
    if let Value::String(raw) = &input {
        input = serde_json::from_str(raw).unwrap_or(Value::Null);
    }
    let mut files: Vec<String> = vec![];
    for k in [
        "file_path",
        "filePath",
        "notebook_path",
        "notebookPath",
        "path",
    ] {
        if let Some(p) = input.get(k).and_then(Value::as_str).map(str::trim)
            && !p.is_empty()
        {
            let p = rel(&cwd, p);
            if !files.contains(&p) {
                files.push(p);
            }
        }
    }
    let summary = if SHELLS.iter().any(|t| tool.eq_ignore_ascii_case(t)) {
        clip(
            str_of(&input, &["command", "cmd"]).unwrap_or(""),
            MAX_SUMMARY,
        )
    } else {
        clip(&files.join(", "), MAX_SUMMARY)
    };
    Some(Event {
        runtime: runtime.to_string(),
        session,
        cwd,
        tool,
        files,
        summary,
    })
}

pub fn is_changing(tool: &str) -> bool {
    CHANGING.contains(&tool.to_lowercase().as_str())
}

fn log(msg: &str) {
    let path = config_dir().join("hook.log");
    let _ = fs::create_dir_all(config_dir());
    if fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = fs::remove_file(&path);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().to_rfc3339());
    }
}

/// Entry point of a runtime's hook. Prints nothing, never fails, always exits 0.
async fn event_main(runtime: &str) {
    if std::env::var("CORNTCHAP_HOOKS").is_ok_and(|v| matches!(v.as_str(), "off" | "0" | "false")) {
        std::process::exit(0);
    }
    let mut raw = String::new();
    let _ = std::io::stdin()
        .take(32 * 1024 * 1024)
        .read_to_string(&mut raw);
    let runtime = runtime.to_string();
    match tokio::time::timeout(Duration::from_secs(8), handle(&runtime, &raw)).await {
        Ok(Err(e)) => log(&format!("{runtime}: {e:#}")),
        Err(_) => log(&format!("{runtime}: timed out")),
        Ok(Ok(())) => {}
    }
    std::process::exit(0);
}

async fn handle(runtime: &str, raw: &str) -> Result<()> {
    let v: Value = serde_json::from_str(raw).context("stdin is not JSON")?;
    let Some(ev) = parse(runtime, &v) else {
        return Ok(());
    };
    let (mut checkpoint, mut files) = (None, ev.files.clone());
    if is_changing(&ev.tool) {
        let (cwd, session, tool, summary) = (
            ev.cwd.clone(),
            ev.session.clone(),
            ev.tool.clone(),
            ev.summary.clone(),
        );
        let snap =
            tokio::task::spawn_blocking(move || checkpoint_for(&cwd, &session, &tool, &summary))
                .await
                .context("checkpoint task")?;
        match snap {
            Ok(Some(s)) => {
                checkpoint = s.sha;
                files = s.files;
            }
            // not a git work tree: report the tool's own paths
            Ok(None) => {}
            Err(e) => log(&format!("checkpoint in {}: {e:#}", ev.cwd.display())),
        }
    }
    files.truncate(MAX_FILES);
    let api = Api::from_creds(false)?;
    let args = json!({
        "runtime": ev.runtime, "session": ev.session, "tool": ev.tool,
        "files": files, "checkpoint": checkpoint, "summary": ev.summary,
    });
    let res = tokio::time::timeout(
        Duration::from_secs(3),
        api.procedure("activityRecord", args),
    )
    .await
    .map_err(|_| anyhow!("server did not answer within 3 s"))??;
    if res["recorded"] == false {
        log(&format!(
            "{}: {}",
            ev.session,
            res["message"].as_str().unwrap_or("not recorded")
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- checkpoints

pub struct Snapshot {
    /// The new checkpoint commit; `None` when nothing changed since the previous one.
    pub sha: Option<String>,
    /// Paths that differ from the previous checkpoint (or HEAD for the first one).
    pub files: Vec<String>,
}

fn git(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Result<std::process::Output> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "gc.auto=0"])
        .args(["-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
        .args(args)
        .envs(envs.iter().copied())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .context("running git")?;
    Ok(out)
}

fn git_ok(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Result<String> {
    let out = git(dir, envs, args)?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A name safe as one ref component.
pub fn ref_name(session: &str) -> String {
    let s: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    format!("{REF_PREFIX}{s}")
}

/// Snapshot the work tree of `cwd` for `session`. `Ok(None)` outside a git work tree.
/// Copy the user's index for the snapshot, keeping its mtime. Git trusts an entry's cached stat only
/// when the file is older than the index itself ("racy git"): a fresh copy would look newer than a
/// file edited in the same second as the real index, and a same-size edit would then be missed.
fn copy_index(from: &Path, to: &Path) -> bool {
    let Ok(modified) = fs::metadata(from).and_then(|m| m.modified()) else {
        return false;
    };
    fs::copy(from, to).is_ok()
        && fs::File::options()
            .write(true)
            .open(to)
            .and_then(|f| f.set_modified(modified))
            .is_ok()
}

pub fn checkpoint_for(
    cwd: &Path,
    session: &str,
    tool: &str,
    summary: &str,
) -> Result<Option<Snapshot>> {
    if !cwd.is_dir() {
        return Ok(None);
    }
    let inside = git(cwd, &[], &["rev-parse", "--is-inside-work-tree"])?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Ok(None);
    }
    let top = PathBuf::from(git_ok(cwd, &[], &["rev-parse", "--show-toplevel"])?);
    let refname = ref_name(session);
    let head = git_ok(&top, &[], &["rev-parse", "--verify", "-q", "HEAD^{commit}"]).ok();
    let prev = git_ok(
        &top,
        &[],
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{refname}^{{commit}}"),
        ],
    )
    .ok();

    // the snapshot index lives next to the real one but is never the real one
    let real_index = PathBuf::from(git_ok(
        &top,
        &[],
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    let tmp = real_index.with_file_name(format!("corntchap-index-{}", std::process::id()));
    let tmp_s = tmp.to_string_lossy().into_owned();
    let result = (|| -> Result<Snapshot> {
        let envs = [("GIT_INDEX_FILE", tmp_s.as_str())];
        // Start from the user's index when there is one: its stat cache keeps `add -A` fast.
        if real_index.is_file() && copy_index(&real_index, &tmp) {
        } else if let Some(h) = &head {
            git_ok(&top, &envs, &["read-tree", h])?;
        }
        git_ok(&top, &envs, &["add", "-A"])?;
        let tree = git_ok(&top, &envs, &["write-tree"])?;
        let base = prev.as_ref().or(head.as_ref());
        let base_tree = match base {
            Some(b) => git_ok(&top, &[], &["rev-parse", &format!("{b}^{{tree}}")])?,
            None => git_ok(&top, &[], &["hash-object", "-t", "tree", "/dev/null"])?,
        };
        if tree == base_tree {
            return Ok(Snapshot {
                sha: None,
                files: vec![],
            });
        }
        let out = git(
            &top,
            &[],
            &[
                "diff-tree",
                "-r",
                "--no-renames",
                "--name-only",
                "-z",
                &base_tree,
                &tree,
            ],
        )?;
        let files: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter(|f| !f.is_empty())
            .take(MAX_FILES)
            .map(String::from)
            .collect();
        let msg = format!("{tool}: {}", clip(summary, MAX_SUMMARY));
        let ident = [
            ("GIT_AUTHOR_NAME", CHECKPOINT_AUTHOR),
            ("GIT_AUTHOR_EMAIL", "checkpoint@corntchap.invalid"),
            ("GIT_COMMITTER_NAME", CHECKPOINT_AUTHOR),
            ("GIT_COMMITTER_EMAIL", "checkpoint@corntchap.invalid"),
        ];
        let mut args = vec!["commit-tree", tree.as_str()];
        if let Some(b) = base {
            args.extend(["-p", b.as_str()]);
        }
        args.extend(["-m", msg.as_str()]);
        let sha = git_ok(&top, &ident, &args)?;
        // compare-and-swap on the previous value: a parallel hook of the same session loses cleanly
        let zero = "0".repeat(sha.len());
        git_ok(
            &top,
            &[],
            &[
                "update-ref",
                &refname,
                &sha,
                prev.as_deref().unwrap_or(&zero),
            ],
        )?;
        Ok(Snapshot {
            sha: Some(sha),
            files,
        })
    })();
    let _ = fs::remove_file(&tmp);
    result.map(Some)
}

// ---------------------------------------------------------------- install

fn claude_settings_path(global: bool) -> Result<PathBuf> {
    Ok(if global {
        dirs::home_dir()
            .ok_or_else(|| anyhow!("no home directory"))?
            .join(".claude/settings.json")
    } else {
        PathBuf::from(".claude/settings.json")
    })
}

/// Copilot reads every `*.json` of `.github/hooks` (CLI and VS Code) and of `~/.copilot/hooks`
/// (`$COPILOT_HOME/hooks`, CLI only); the file is ours alone.
fn copilot_hook_path(global: bool) -> Result<PathBuf> {
    Ok(if global {
        match std::env::var_os("COPILOT_HOME") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => dirs::home_dir()
                .ok_or_else(|| anyhow!("no home directory"))?
                .join(".copilot"),
        }
        .join("hooks")
        .join(COPILOT_FILE)
    } else {
        PathBuf::from(".github/hooks").join(COPILOT_FILE)
    })
}

/// The hook file: a `PostToolUse` command hook. The PascalCase event name is the VS Code spelling
/// the Copilot CLI accepts as an alias, so one file serves both and the payload is snake_case.
pub fn copilot_hook_doc() -> Value {
    json!({
        "version": 1,
        "hooks": {
            "PostToolUse": [{
                "type": "command",
                "command": COPILOT_COMMAND,
                "timeoutSec": 15
            }]
        }
    })
}

fn opencode_plugin_path(global: bool) -> Result<PathBuf> {
    Ok(if global {
        dirs::home_dir()
            .ok_or_else(|| anyhow!("no home directory"))?
            .join(".config/opencode/plugins/corntchap.js")
    } else {
        PathBuf::from(".opencode/plugins/corntchap.js")
    })
}

fn is_ours(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hs| {
        hs.iter().any(|h| {
            h["command"]
                .as_str()
                .is_some_and(|c| c.contains("corntchap-cli hook claude-code"))
        })
    })
}

/// Merge our PostToolUse hook into a Claude Code settings document, keeping everything else.
/// Returns whether it changed anything.
pub fn merge_claude_settings(doc: &mut Value) -> Result<bool> {
    if !doc.is_object() {
        bail!("settings.json is not a JSON object");
    }
    let hooks = doc
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        bail!("\"hooks\" in settings.json is not an object");
    };
    let post = hooks.entry("PostToolUse").or_insert_with(|| json!([]));
    let Some(post) = post.as_array_mut() else {
        bail!("\"hooks.PostToolUse\" in settings.json is not an array");
    };
    if post.iter().any(is_ours) {
        return Ok(false);
    }
    post.push(json!({
        "matcher": CLAUDE_MATCHER,
        "hooks": [{"type": "command", "command": CLAUDE_COMMAND, "timeout": 10}]
    }));
    Ok(true)
}

/// Remove our hook from a settings document; returns whether it was there.
pub fn remove_claude_settings(doc: &mut Value) -> bool {
    let Some(post) = doc["hooks"]["PostToolUse"].as_array_mut() else {
        return false;
    };
    let before = post.len();
    post.retain(|g| !is_ours(g));
    let changed = post.len() != before;
    if post.is_empty()
        && changed
        && let Some(h) = doc["hooks"].as_object_mut()
    {
        h.remove("PostToolUse");
        if h.is_empty() {
            doc.as_object_mut().map(|d| d.remove("hooks"));
        }
    }
    changed
}

fn read_settings(path: &Path) -> Result<Value> {
    match fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => serde_json::from_str(&t).map_err(|e| {
            anyhow!(
                "{} is not valid JSON ({e}); left untouched, add the hook by hand",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn write_settings(path: &Path, doc: &Value) -> Result<()> {
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(doc)? + "\n")?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub const OPENCODE_PLUGIN: &str = r#"// corntchap: after each edit or command, `corntchap-cli hook opencode` snapshots the work tree into a
// hidden git ref (refs/corntchap/checkpoints/<session>) and reports the tool, paths and checkpoint.
// Only paths and a short summary are sent, never file contents. Turn off: CORNTCHAP_HOOKS=off, or
// `corntchap-cli hook uninstall opencode`.
import { spawn } from "node:child_process"

const CHANGING = new Set(["edit", "write", "multiedit", "patch", "apply_patch", "bash", "shell"])
const args = new Map()

export const CorntchapPlugin = async ({ directory }) => ({
  "tool.execute.before": async (input, output) => {
    try {
      if (!CHANGING.has(String(input.tool).toLowerCase())) return
      args.set(input.callID, output && output.args)
      if (args.size > 200) args.delete(args.keys().next().value)
    } catch {}
  },
  "tool.execute.after": async (input) => {
    try {
      if (process.env.CORNTCHAP_HOOKS === "off") return
      const tool = String(input.tool)
      if (!CHANGING.has(tool.toLowerCase())) return
      const event = {
        hook_event_name: "tool.execute.after",
        session_id: input.sessionID,
        cwd: directory,
        tool_name: tool,
        tool_input: input.args ?? args.get(input.callID) ?? {},
      }
      args.delete(input.callID)
      const child = spawn("corntchap-cli", ["hook", "opencode"], { stdio: ["pipe", "ignore", "ignore"] })
      child.on("error", () => {})
      child.stdin.on("error", () => {})
      child.stdin.end(JSON.stringify(event))
      child.unref()
    } catch {}
  },
})

export default { id: "corntchap", server: CorntchapPlugin }
"#;

fn install(runtime: Runtime, global: bool) {
    let r = match runtime {
        Runtime::ClaudeCode => (|| {
            let path = claude_settings_path(global)?;
            let mut doc = read_settings(&path)?;
            if merge_claude_settings(&mut doc)? {
                write_settings(&path, &doc)?;
                ok_line(format!(
                    "added the corntchap PostToolUse hook to {}",
                    path.display()
                ));
            } else {
                println!("already installed in {}", path.display());
            }
            Ok::<_, anyhow::Error>(())
        })(),
        Runtime::Copilot => (|| {
            let path = copilot_hook_path(global)?;
            if let Some(d) = path.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(
                &path,
                serde_json::to_string_pretty(&copilot_hook_doc())? + "\n",
            )?;
            ok_line(format!("wrote the Copilot hook {}", path.display()));
            Ok(())
        })(),
        Runtime::Opencode => (|| {
            let path = opencode_plugin_path(global)?;
            if let Some(d) = path.parent() {
                fs::create_dir_all(d)?;
            }
            fs::write(&path, OPENCODE_PLUGIN)?;
            ok_line(format!("wrote the OpenCode plugin {}", path.display()));
            Ok(())
        })(),
    };
    if let Err(e) = r {
        warn_line(format!("{e:#}"));
        std::process::exit(1);
    }
}

fn uninstall(runtime: Runtime, global: bool) {
    let r = match runtime {
        Runtime::ClaudeCode => (|| {
            let path = claude_settings_path(global)?;
            let mut doc = read_settings(&path)?;
            if remove_claude_settings(&mut doc) {
                write_settings(&path, &doc)?;
                ok_line(format!(
                    "removed the corntchap hook from {}",
                    path.display()
                ));
            } else {
                println!("no corntchap hook in {}", path.display());
            }
            Ok::<_, anyhow::Error>(())
        })(),
        Runtime::Copilot => (|| {
            let path = copilot_hook_path(global)?;
            if path.exists() {
                fs::remove_file(&path)?;
                ok_line(format!("removed {}", path.display()));
            } else {
                println!("no hook file at {}", path.display());
            }
            Ok(())
        })(),
        Runtime::Opencode => (|| {
            let path = opencode_plugin_path(global)?;
            if path.exists() {
                fs::remove_file(&path)?;
                ok_line(format!("removed {}", path.display()));
            } else {
                println!("no plugin at {}", path.display());
            }
            Ok(())
        })(),
    };
    if let Err(e) = r {
        warn_line(format!("{e:#}"));
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------- checkpoints command

/// `corntchap-cli checkpoints`: list a session's checkpoints and how to restore one.
pub fn checkpoints(
    session: Option<String>,
    limit: usize,
    restore: Option<String>,
    force: bool,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let top = PathBuf::from(
        git_ok(&cwd, &[], &["rev-parse", "--show-toplevel"])
            .map_err(|_| anyhow!("not inside a git work tree"))?,
    );
    if let Some(sha) = restore {
        return restore_checkpoint(&top, &sha, force);
    }
    let Some(session) = session else {
        let out = git_ok(
            &top,
            &[],
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:strip=3)\t%(objectname:short)\t%(committerdate:relative)\t%(subject)",
                REF_PREFIX,
            ],
        )?;
        if out.is_empty() {
            println!(
                "no checkpoints in this repo yet (install the hook: corntchap-cli hook install claude-code)"
            );
            return Ok(());
        }
        println!("{}", ui::bold("sessions with checkpoints (newest first)"));
        for l in out.lines() {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            if let [s, sha, when, subject] = f[..] {
                println!("  {s}  {sha}  {when}  {subject}");
            }
        }
        println!("\nlist one: corntchap-cli checkpoints --session <session>");
        return Ok(());
    };
    let refname = ref_name(&session);
    let author = format!("--author={CHECKPOINT_AUTHOR}");
    let n = format!("-n{}", limit.max(1));
    let out = git_ok(
        &top,
        &[],
        &["log", &n, &author, "--format=%h\t%cr\t%s", &refname, "--"],
    )
    .map_err(|_| anyhow!("no checkpoints for session {session}"))?;
    println!(
        "{}",
        ui::bold(&format!("checkpoints of {session} (newest first)"))
    );
    for l in out.lines() {
        let f: Vec<&str> = l.splitn(3, '\t').collect();
        if let [sha, when, subject] = f[..] {
            println!("  {sha}  {when:<14}  {subject}");
        }
    }
    println!(
        "\nrestore one file:   git restore --source <sha> -- <path>\n\
         restore everything: git checkout <sha> -- .      (files created later stay; nothing is deleted)\n\
         or let me do it:    corntchap-cli checkpoints --restore <sha>   (refuses when the tree has uncommitted changes, unless --force)\n\
         what changed:       git diff <sha>^ <sha>"
    );
    Ok(())
}

fn restore_checkpoint(top: &Path, sha: &str, force: bool) -> Result<()> {
    let full = git_ok(
        top,
        &[],
        &["rev-parse", "--verify", &format!("{sha}^{{commit}}")],
    )
    .map_err(|_| anyhow!("{sha} is not a commit here"))?;
    let author = git_ok(top, &[], &["log", "-1", "--format=%an", &full])?;
    if author != CHECKPOINT_AUTHOR {
        bail!(
            "{sha} is not a corntchap checkpoint (list them with `corntchap-cli checkpoints --session <session>`)"
        );
    }
    let dirty = git_ok(top, &[], &["status", "--porcelain"])?;
    if !dirty.is_empty() && !force {
        bail!(
            "the work tree has uncommitted changes; restoring would overwrite them.\n\
             Commit or stash them, or take a checkpoint first, or pass --force:\n{}",
            dirty.lines().take(10).collect::<Vec<_>>().join("\n")
        );
    }
    git_ok(
        top,
        &[],
        &["restore", "--source", &full, "--worktree", "--", ":/"],
    )?;
    ok_line(format!(
        "restored the files of {} into the work tree (index, HEAD and branch untouched; files created later were not deleted)",
        &full[..full.len().min(10)]
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Repo {
            let d = std::env::temp_dir().join(format!("ct-hook-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&d).unwrap();
            let r = Repo(d);
            r.git(&["init", "-q", "-b", "main"]);
            r.git(&["config", "user.email", "t@example.test"]);
            r.git(&["config", "user.name", "t"]);
            // byte-exact files on every OS (Windows runners default to core.autocrlf=true)
            r.git(&["config", "core.autocrlf", "false"]);
            r
        }
        fn git(&self, args: &[&str]) -> String {
            git_ok(&self.0, &[], args).unwrap()
        }
        fn write(&self, name: &str, body: &str) {
            let p = self.0.join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn checkpoints_leave_the_user_state_alone() {
        let r = Repo::new();
        r.write("a.txt", "one\n");
        r.git(&["add", "a.txt"]);
        r.git(&["commit", "-q", "-m", "init"]);
        r.write("staged.txt", "s\n");
        r.git(&["add", "staged.txt"]);
        r.write("a.txt", "two\n");
        r.write("new/untracked.txt", "u\n");
        r.write(".gitignore", "ignored.log\n");
        r.write("ignored.log", "x\n");
        let (head, branch, index, status) = (
            r.git(&["rev-parse", "HEAD"]),
            r.git(&["symbolic-ref", "HEAD"]),
            r.git(&["ls-files", "-s"]),
            r.git(&["status", "--porcelain"]),
        );

        let s1 = checkpoint_for(&r.0, "sess/1", "Edit", "a.txt")
            .unwrap()
            .unwrap();
        let c1 = s1.sha.clone().expect("a checkpoint");
        assert!(s1.files.contains(&"a.txt".to_string()), "{:?}", s1.files);
        assert!(s1.files.contains(&"new/untracked.txt".to_string()));
        assert!(!s1.files.contains(&"ignored.log".to_string()));
        // the untracked file is in the snapshot, the ignored one is not
        let ls = r.git(&["ls-tree", "-r", "--name-only", &c1]);
        assert!(
            ls.contains("new/untracked.txt") && !ls.contains("ignored.log"),
            "{ls}"
        );
        assert_eq!(r.git(&["show", &format!("{c1}:a.txt")]), "two");
        assert_eq!(r.git(&["log", "-1", "--format=%s", &c1]), "Edit: a.txt");

        // the user's HEAD, branch, index and work tree are exactly as they were
        assert_eq!(r.git(&["rev-parse", "HEAD"]), head);
        assert_eq!(r.git(&["symbolic-ref", "HEAD"]), branch);
        assert_eq!(r.git(&["ls-files", "-s"]), index);
        assert_eq!(r.git(&["status", "--porcelain"]), status);
        assert_eq!(fs::read_to_string(r.0.join("a.txt")).unwrap(), "two\n");
        assert_eq!(
            r.git(&["rev-parse", &ref_name("sess/1")]),
            c1,
            "the ref points at the checkpoint"
        );
        assert!(!fs::read_dir(r.0.join(".git")).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("corntchap-index")
        }));

        // nothing changed: no new commit
        let s2 = checkpoint_for(&r.0, "sess/1", "Bash", "ls")
            .unwrap()
            .unwrap();
        assert!(s2.sha.is_none() && s2.files.is_empty());
        assert_eq!(r.git(&["rev-parse", &ref_name("sess/1")]), c1);

        // a change grows the chain, and only the new change is listed
        r.write("b.txt", "b\n");
        let s3 = checkpoint_for(&r.0, "sess/1", "Write", "b.txt")
            .unwrap()
            .unwrap();
        let c3 = s3.sha.unwrap();
        assert_eq!(s3.files, vec!["b.txt".to_string()]);
        assert_eq!(r.git(&["rev-parse", &format!("{c3}^")]), c1);
        assert_eq!(r.git(&["rev-parse", &format!("{c1}^")]), head);
        assert_eq!(r.git(&["rev-parse", &ref_name("sess/1")]), c3);
        // another session starts its own chain from HEAD
        let o = checkpoint_for(&r.0, "other", "Edit", "x").unwrap().unwrap();
        assert_eq!(r.git(&["rev-parse", &format!("{}^", o.sha.unwrap())]), head);
    }

    #[test]
    fn works_without_a_commit_and_outside_git() {
        let r = Repo::new();
        r.write("a.txt", "one\n");
        let s = checkpoint_for(&r.0, "s", "Write", "a.txt")
            .unwrap()
            .unwrap();
        assert_eq!(s.files, vec!["a.txt".to_string()]);
        let sha = s.sha.unwrap();
        assert_eq!(r.git(&["rev-list", "--count", &sha]), "1");
        assert!(r.git(&["status", "--porcelain"]).contains("?? a.txt"));
        let plain = std::env::temp_dir().join(format!("ct-plain-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&plain).unwrap();
        assert!(checkpoint_for(&plain, "s", "Edit", "x").unwrap().is_none());
        let _ = fs::remove_dir_all(plain);
    }

    #[test]
    fn restore_is_guarded() {
        let r = Repo::new();
        r.write("a.txt", "one\n");
        r.git(&["add", "."]);
        r.git(&["commit", "-q", "-m", "init"]);
        r.write("a.txt", "two\n");
        let c = checkpoint_for(&r.0, "s", "Edit", "a")
            .unwrap()
            .unwrap()
            .sha
            .unwrap();
        r.write("a.txt", "three\n");
        let e = restore_checkpoint(&r.0, &c, false).unwrap_err().to_string();
        assert!(e.contains("uncommitted"), "{e}");
        assert_eq!(fs::read_to_string(r.0.join("a.txt")).unwrap(), "three\n");
        restore_checkpoint(&r.0, &c, true).unwrap();
        assert_eq!(fs::read_to_string(r.0.join("a.txt")).unwrap(), "two\n");
        let head = r.git(&["rev-parse", "HEAD"]);
        assert!(
            restore_checkpoint(&r.0, &head, true)
                .unwrap_err()
                .to_string()
                .contains("not a corntchap checkpoint")
        );
    }

    #[test]
    fn parses_claude_code_payloads() {
        let v = json!({"session_id": "abc", "cwd": "/w/repo", "hook_event_name": "PostToolUse",
            "tool_name": "Edit", "tool_input": {"file_path": "/w/repo/src/a.rs", "old_string": "x", "new_string": "y"},
            "tool_response": {}});
        let e = parse("claude-code", &v).unwrap();
        assert_eq!(e.session, "abc");
        assert_eq!(e.tool, "Edit");
        assert_eq!(e.files, vec!["src/a.rs"]);
        assert_eq!(e.summary, "src/a.rs");
        assert!(is_changing(&e.tool));
        let long = "echo ".to_string() + &"x".repeat(500);
        let b = json!({"session_id": "abc", "cwd": "/w", "hook_event_name": "PostToolUse",
            "tool_name": "Bash", "tool_input": {"command": long}});
        let e = parse("claude-code", &b).unwrap();
        assert_eq!(e.summary.chars().count(), 200);
        assert!(e.files.is_empty());
        let nb = json!({"session_id": "s", "cwd": "/w", "tool_name": "NotebookEdit",
            "tool_input": {"notebook_path": "/w/n.ipynb"}});
        assert_eq!(parse("claude-code", &nb).unwrap().files, vec!["n.ipynb"]);
        // read-only tools and other events are not checkpointed / ignored
        assert!(!is_changing("Read") && !is_changing("Grep"));
        let stop = json!({"session_id": "s", "cwd": "/w", "hook_event_name": "Stop"});
        assert!(parse("claude-code", &stop).is_none());
        assert!(parse("claude-code", &json!({"cwd": "/w"})).is_none());
    }

    #[test]
    fn parses_copilot_payloads() {
        // VS Code format (what a PascalCase registration delivers)
        let v = json!({
            "hook_event_name": "PostToolUse", "session_id": "cp-1", "cwd": "/w/pay",
            "tool_name": "edit", "tool_input": {"path": "/w/pay/src/a.rs"}
        });
        let e = parse("copilot", &v).unwrap();
        assert_eq!(e.runtime, "copilot");
        assert_eq!(e.session, "cp-1");
        assert_eq!(e.files, vec!["src/a.rs"]);
        assert!(is_changing(&e.tool));
        // camelCase format, toolArgs as a JSON string, a shell command
        let c = json!({
            "sessionId": "cp-2", "cwd": "/w/pay", "toolName": "bash",
            "toolArgs": "{\"command\": \"cargo test\"}",
            "toolResult": {"resultType": "success", "textResultForLlm": "ok"}
        });
        let e = parse("copilot", &c).unwrap();
        assert_eq!(e.session, "cp-2");
        assert_eq!(e.summary, "cargo test");
        // SessionStart carries no tool
        let start = json!({"hook_event_name": "SessionStart", "session_id": "cp-1", "cwd": "/w"});
        assert!(parse("copilot", &start).is_none());
        assert!(
            parse(
                "copilot",
                &json!({"sessionId": "x", "cwd": "/w", "source": "new"})
            )
            .is_none()
        );
    }

    #[test]
    fn copilot_hook_file_names_our_command_under_post_tool_use() {
        let d = copilot_hook_doc();
        assert_eq!(d["version"], 1);
        assert_eq!(
            d["hooks"]["PostToolUse"][0]["command"],
            "corntchap-cli hook copilot"
        );
        assert_eq!(d["hooks"]["PostToolUse"][0]["type"], "command");
        assert_eq!(
            copilot_hook_path(false).unwrap(),
            PathBuf::from(".github/hooks/corntchap.json")
        );
    }

    #[test]
    fn parses_opencode_payloads() {
        let v = json!({"hook_event_name": "tool.execute.after", "session_id": "ses_1", "cwd": "/w",
            "tool_name": "edit", "tool_input": {"filePath": "/w/x.ts", "oldString": "a", "newString": "b"}});
        let e = parse("opencode", &v).unwrap();
        assert_eq!((e.session.as_str(), e.tool.as_str()), ("ses_1", "edit"));
        assert_eq!(e.files, vec!["x.ts"]);
        let alt = json!({"sessionID": "ses_2", "directory": "/w", "tool": "bash", "args": {"command": "cargo test"}});
        let e = parse("opencode", &alt).unwrap();
        assert_eq!(
            (e.session.as_str(), e.summary.as_str()),
            ("ses_2", "cargo test")
        );
        assert!(is_changing("bash") && is_changing("patch") && !is_changing("read"));
    }

    #[test]
    fn claude_settings_merge_is_idempotent_and_keeps_hooks() {
        let mut doc = json!({"model": "x", "hooks": {
            "PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "fmt.sh"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "bye.sh"}]}]}});
        assert!(merge_claude_settings(&mut doc).unwrap());
        assert!(!merge_claude_settings(&mut doc).unwrap());
        let post = doc["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 2);
        assert_eq!(post[0]["hooks"][0]["command"], "fmt.sh");
        assert_eq!(post[1]["hooks"][0]["command"], CLAUDE_COMMAND);
        assert_eq!(doc["hooks"]["Stop"][0]["hooks"][0]["command"], "bye.sh");
        assert_eq!(doc["model"], "x");
        assert!(remove_claude_settings(&mut doc));
        assert_eq!(doc["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
        let mut empty = json!({});
        assert!(merge_claude_settings(&mut empty).unwrap());
        assert!(remove_claude_settings(&mut empty));
        assert_eq!(empty, json!({}));
        assert!(merge_claude_settings(&mut json!({"hooks": []})).is_err());
    }
}
