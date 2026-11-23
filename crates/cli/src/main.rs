mod api;
mod file;
mod hook;
mod menu;
mod opencode;
mod pantry;
mod pass;
mod relay;
mod setup;
mod station;
mod tui;
mod ui;
mod view;

use anyhow::{Result, anyhow, bail};
use api::{Api, Credentials, device_login};
use clap::{CommandFactory, Parser, Subcommand};
use serde_json::{Value, json};
use ui::{ok_line, s, warn_line};

#[derive(Parser)]
#[command(
    name = "corntchap-cli",
    version,
    about = "The board from your terminal, plus the relay that wakes local agents"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sign in: device flow approved in the browser through your OIDC provider (Keycloak) (or --token to paste one).
    Login {
        url: String,
        #[arg(long)]
        token: Option<String>,
        /// With --token: store it without asking the server (used by `just init`, before the server runs).
        #[arg(long)]
        no_verify: bool,
    },
    /// Sign out: revoke this token on the server and delete it from credentials.toml (the relay token is kept unless --relay or --all).
    Logout {
        /// Sign out the relay credential instead of your user token.
        #[arg(long)]
        relay: bool,
        /// Sign out both the user token and the relay credential.
        #[arg(long)]
        all: bool,
        /// Only delete the stored tokens; do not call the server (used by `just reset`).
        #[arg(long)]
        local: bool,
    },
    /// Who am I, and what do I hold?
    Whoami {
        #[arg(long)]
        json: bool,
    },
    /// Your own account: change your handle (the @name others mention).
    #[command(subcommand, name = "me", alias = "self")]
    Me(MeCmd),
    /// Consult the marabout: checks the server, your credentials and the fire config.
    #[command(name = "marabou", alias = "doctor")]
    Doctor,
    /// The live kitchen board in your terminal (projects, tasks, checklists; tick, comment, add).
    #[command(name = "kitchen", alias = "tui")]
    Tui,
    /// Tasks: the board for humans, plus JSON commands for agents that only have a shell.
    #[command(subcommand)]
    Task(TaskCmd),
    /// Users (admin).
    #[command(subcommand)]
    User(UserCmd),
    /// Teams and their members.
    #[command(subcommand)]
    Team(TeamCmd),
    /// Projects.
    #[command(subcommand)]
    Project(ProjectCmd),
    /// The pantry: what a pot's cooks and people have learned (conventions, gotchas, decisions, how-tos), saved under stable keys.
    #[command(subcommand, name = "pantry", alias = "memory")]
    Pantry(pantry::PantryCmd),
    /// The pass: talk with a pot's chefs (people and cooks), on the pot's pass or directly.
    #[command(subcommand, name = "pass", alias = "chat")]
    Pass(pass::PassCmd),
    /// Saved views: a SQL condition and order over a pot's tasks, shown as a tab in the console.
    #[command(subcommand, name = "view", alias = "views")]
    View(view::ViewCmd),
    /// Files: share screenshots, logs and build artifacts (up to 1 GiB) in a pot, behind corntchap's login.
    #[command(subcommand, name = "file", alias = "files")]
    File(file::FileCmd),
    /// Menus (milestones): group a pot's tasks and watch them get served.
    #[command(subcommand, name = "menu", alias = "milestone")]
    Menu(menu::MenuCmd),
    /// The cooks (agents): create, set up, wake targets.
    #[command(subcommand, name = "cook", alias = "agent")]
    Agent(AgentCmd),
    /// Stations: the agent sessions (OpenCode, Claude Code, Codex) the fire wakes. `station scan` registers a repo's sessions.
    #[command(subcommand, name = "station", alias = "stations")]
    Station(station::StationCmd),
    /// Agent hooks: checkpoint the work tree after each edit or command and report agent activity. `hook install claude-code` sets it up.
    #[command(subcommand)]
    Hook(hook::HookCmd),
    /// List an agent session's checkpoints (refs/corntchap/checkpoints/<session>) and how to restore one.
    Checkpoints {
        /// The runtime's session id; without it, the sessions that have checkpoints in this repo.
        #[arg(long)]
        session: Option<String>,
        /// How many to list.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Restore the files of this checkpoint into the work tree (refuses when it has uncommitted changes).
        #[arg(long, value_name = "SHA")]
        restore: Option<String>,
        /// With --restore: overwrite uncommitted changes.
        #[arg(long, requires = "restore")]
        force: bool,
    },
    /// Keep the fire lit: the relay that wakes your local cooks when new work lands (subcommands: init, install, uninstall, doctor, enroll).
    #[command(name = "fire", alias = "relay")]
    Relay {
        #[command(subcommand)]
        cmd: Option<RelayCmd>,
    },
    /// Print shell completions: `corntchap-cli completions zsh > "${fpath[1]}/_corntchap-cli"` (Homebrew installs them for you).
    Completions {
        /// bash, zsh, fish, powershell or elvish
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
enum TaskCmd {
    /// List tasks in a project.
    Ls {
        project: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        ready: bool,
        #[arg(long)]
        label: Option<String>,
        /// Only tasks on this menu (`-` for tasks on no menu).
        #[arg(long, allow_hyphen_values = true)]
        menu: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Create a task.
    Add {
        project: String,
        title: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "item")]
        items: Vec<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long = "depends")]
        depends_on: Vec<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Put it on this menu (milestone key).
        #[arg(long)]
        menu: Option<String>,
    },
    /// Change a task's title, body, labels (replaced), priority or menu.
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Move it to this menu ("" takes it off its menu).
        #[arg(long, conflicts_with = "no_menu")]
        menu: Option<String>,
        /// Take it off its menu.
        #[arg(long)]
        no_menu: bool,
    },
    /// Show one task.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Serve it: human shortcut that claims, then completes.
    #[command(name = "serve", alias = "done")]
    Done {
        id: String,
        #[arg(long)]
        result: Option<String>,
    },
    /// Put a blocked task back to open.
    Unblock { id: String },
    /// Toss a task (people only): it leaves the pot, shown as Tossed; a cook holding it loses its lease.
    #[command(name = "toss", alias = "cancel")]
    Toss {
        id: String,
        /// Why (shown on the task).
        #[arg(long, short)]
        reason: Option<String>,
    },
    /// Put a tossed task back on the counter (open).
    Restore { id: String },
    /// Scoop the next ready portion: agent mode, claims the best ready task (JSON).
    #[command(name = "scoop", alias = "claim-next")]
    ClaimNext {
        #[arg(long)]
        project: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        /// Only take work from this menu (milestone key).
        #[arg(long)]
        menu: Option<String>,
        /// The station (st_…) this session is, as the wake prompt says: records which session holds the task.
        #[arg(long)]
        station: Option<String>,
        /// Lease length for long work, e.g. 45m or 2h (default: the pot's; at most a day).
        #[arg(long, value_parser = relay::parse_duration)]
        lease: Option<std::time::Duration>,
        #[arg(long)]
        json: bool,
    },
    /// Agent mode: claim one task (JSON).
    Claim {
        id: String,
        /// Lease length for long work, e.g. 45m or 2h (default: the pot's; at most a day).
        #[arg(long, value_parser = relay::parse_duration)]
        lease: Option<std::time::Duration>,
        #[arg(long)]
        json: bool,
    },
    /// Tick a checklist item (agents pass --epoch).
    Check {
        id: String,
        #[arg(long)]
        epoch: Option<i64>,
        #[arg(long)]
        item: i32,
        #[arg(long)]
        undo: bool,
    },
    /// Agent mode: extend a lease (JSON).
    Heartbeat {
        id: String,
        #[arg(long)]
        epoch: i64,
    },
    /// Agent mode: finish a claimed task (JSON).
    Complete {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        result: String,
    },
    /// Agent mode: give a task back (JSON).
    Release {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Agent mode: mark a claimed task blocked (JSON).
    Block {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Comment on a task (agents pass --epoch; JSON).
    Comment {
        id: String,
        body: String,
        #[arg(long)]
        epoch: Option<i64>,
        /// Your station (st_...), so people see which of your sessions wrote it.
        #[arg(long)]
        station: Option<String>,
    },
    /// Events after a cursor (JSON).
    Changes {
        #[arg(long, default_value_t = 0)]
        cursor: i64,
        #[arg(long)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
enum MeCmd {
    /// Rename yourself: letters, digits, '-', '_' and '.' (max 64); a taken or reserved handle is refused.
    Rename { handle: String },
}

#[derive(Subcommand)]
enum UserCmd {
    /// Make a user an admin, or not (admin). The last admin cannot be demoted.
    Admin {
        handle: String,
        #[arg(long, conflicts_with = "off", required_unless_present = "off")]
        on: bool,
        #[arg(long)]
        off: bool,
    },
    /// Disable a person (admin): their tokens and sessions stop working. The last admin who can sign in cannot be disabled.
    Disable { handle: String },
    /// Enable a disabled person again (admin). Their old tokens stay revoked: they sign in again.
    Enable { handle: String },
    /// List users.
    Ls {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TeamCmd {
    /// Create a team.
    Add { name: String },
    /// List your teams.
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// List a team's chefs (the people and cooks in it).
    #[command(name = "chefs", alias = "members")]
    Members {
        team: String,
        #[arg(long)]
        json: bool,
    },
    /// Add a user or agent to a team as one of its chefs.
    Join {
        team: String,
        principal: String,
        /// chef or owner.
        #[arg(long, default_value = "chef")]
        role: String,
    },
    /// Remove a user or agent from a team.
    Leave { team: String, principal: String },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Create a project in a team.
    Add {
        team: String,
        key: String,
        name: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        lease_ttl_s: Option<i64>,
    },
    /// List projects with their task counts.
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Set a pot's default lease length, e.g. `pot lease PAY 30m` (team owner or admin).
    Lease {
        key: String,
        #[arg(value_parser = relay::parse_duration)]
        length: std::time::Duration,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Create an agent; prints its token once (put it in the agent's MCP config).
    Add {
        name: String,
        #[arg(long, default_value = "other")]
        kind: String,
        #[arg(long, default_value = "local")]
        runtime: String,
        #[arg(long)]
        max_leases: Option<i64>,
        #[arg(long)]
        owner: Option<String>,
    },
    /// One step: create the agent, join a team, add a relay wake target (local agents) and print the MCP config plus the relay [[target]] block.
    Setup {
        name: String,
        /// opencode | codex | claude-code | other
        #[arg(long, default_value = "other")]
        kind: String,
        /// local | cloud
        #[arg(long, default_value = "local")]
        runtime: String,
        /// Team to add the agent to.
        #[arg(long)]
        team: Option<String>,
    },
    /// Register an agent through the device flow (approve in the browser); prints its token once.
    Enroll {
        name: String,
        /// opencode | codex | claude-code | other
        #[arg(long, default_value = "other")]
        kind: String,
        /// local | cloud
        #[arg(long, default_value = "local")]
        runtime: String,
    },
    /// List your agents.
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Revoke every token and print a fresh one.
    Rotate { name: String },
    /// What a cook may do beyond working its tasks: plan (edit, toss, restore, unblock any task in
    /// its teams), manage (act as you on teams, pots, cooks, fires), admin (your admin powers).
    /// Replaces the set; `--none` removes them all. A cook never exceeds your own rights.
    Grant {
        name: String,
        /// plan, manage and/or admin.
        #[arg(value_parser = ["plan", "manage", "admin"])]
        grants: Vec<String>,
        /// Take every grant away (work only).
        #[arg(long, conflicts_with = "grants")]
        none: bool,
    },
    /// How many tasks a cook may hold at once (its stations share them). Its owner or an admin.
    #[command(name = "limit", alias = "max-leases")]
    Limit {
        name: String,
        /// 1 to 50.
        #[arg(value_parser = clap::value_parser!(i64).range(1..=50))]
        leases: i64,
    },
    /// Stop an agent from claiming work.
    Disable { name: String },
    /// Let a disabled agent claim work again.
    Enable { name: String },
    /// Wake targets: how and when an idle agent gets woken.
    #[command(subcommand)]
    Wake(WakeCmd),
}

// parsed once from the command line: the size of the variants does not matter
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum WakeCmd {
    /// Add a wake target to an agent.
    Add {
        agent: String,
        /// relay | claude-routine | webhook | github-comment
        #[arg(long = "type", value_parser = ["relay", "claude-routine", "webhook", "github-comment"])]
        typ: String,
        #[arg(long = "project")]
        projects: Vec<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
        /// claude-routine: the routine's API trigger URL (…/routines/trig_…/fire); webhook: URL to POST
        #[arg(long)]
        url: Option<String>,
        /// claude-routine: the routine's API token (sk-ant-oat01-…); webhook: bearer token; github-comment: GitHub token
        #[arg(long)]
        token: Option<String>,
        /// webhook: sign each body with HMAC-SHA256 (X-Corntchap-Signature: sha256=…)
        #[arg(long)]
        secret: Option<String>,
        /// github-comment: owner/name and issue number to comment on
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        issue: Option<i64>,
        /// github-comment: who the comment mentions, e.g. @codex or @copilot (default: the server's GITHUB_MENTION, @cook)
        #[arg(long)]
        mention: Option<String>,
    },
    /// List wake targets (of one agent, or of all your agents).
    Ls {
        agent: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Remove a wake target by id.
    Rm { id: String },
}

#[derive(Subcommand)]
enum RelayCmd {
    /// Write a commented config.toml template (never overwrites unless --force).
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Write a systemd user unit (Linux) or launchd agent (macOS); not available on Windows.
    Install,
    /// Put the fire out: stop and disable the service and delete the unit/agent `fire install` wrote.
    Uninstall {
        /// Also remove config.toml, relay-state.json and the relay credential (your own login stays).
        #[arg(long)]
        purge: bool,
        /// Do not ask first.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Consult the marabout about the fire: the server, the credential and each target.
    #[command(alias = "marabou")]
    Doctor,
    /// Register this machine as a relay (device flow) and store its credential.
    Enroll { name: String },
    /// Wake one local cook now, as a wake event would (checks the wiring end to end).
    Poke {
        /// The cook's name (its [[target]] in config.toml).
        cook: String,
        /// Project key the prompt names.
        #[arg(long, default_value = "PAY")]
        pot: String,
    },
}

fn out(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

/// `--json` prints the raw JSON; otherwise the human rendering.
fn show(v: &Value, json: bool, human: impl FnOnce(&Value) -> String) {
    if json {
        out(v)
    } else {
        println!("{}", human(v))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Completions { shell } => {
            completions(shell, &mut std::io::stdout());
            Ok(())
        }
        Cmd::Hook(h) => hook::run(h).await,
        Cmd::Checkpoints {
            session,
            limit,
            restore,
            force,
        } => hook::checkpoints(session, limit, restore, force),
        Cmd::Login {
            url,
            token,
            no_verify,
        } => login(&url, token, no_verify).await,
        Cmd::Logout { relay, all, local } => logout(relay, all, local).await,
        Cmd::Doctor => relay::doctor(true).await,
        Cmd::Relay { cmd } => match cmd {
            None => relay::run(Api::from_creds(true)?).await,
            Some(RelayCmd::Init { force }) => relay::init(force),
            Some(RelayCmd::Install) => relay::install(),
            Some(RelayCmd::Uninstall { purge, yes }) => relay::uninstall(purge, yes),
            Some(RelayCmd::Doctor) => relay::doctor(false).await,
            Some(RelayCmd::Enroll { name }) => enroll(&name).await,
            Some(RelayCmd::Poke { cook, pot }) => relay::poke(&cook, &pot.to_uppercase()).await,
        },
        Cmd::Agent(AgentCmd::Enroll {
            name,
            kind,
            runtime,
        }) => agent_enroll(&name, &kind, &runtime).await,
        other => run(other, Api::from_creds(false)?).await,
    }
}

/// Completions for `shell`, generated from the same clap definitions as `--help` (needs no login or server).
fn completions(shell: clap_complete::Shell, out: &mut dyn std::io::Write) {
    clap_complete::generate(shell, &mut Cli::command(), "corntchap-cli", out);
}

async fn login(url: &str, token: Option<String>, no_verify: bool) -> Result<()> {
    let url = url.trim_end_matches('/');
    let t = match token {
        Some(t) => t,
        None => device_login(url, "user", None, json!({})).await?.0,
    };
    if no_verify {
        Credentials {
            server: Some(url.to_string()),
            token: Some(t),
            relay_token: None,
        }
        .save()?;
        ok_line(format!("stored the token for {url} (not verified)"));
        return Ok(());
    }
    let api = Api::new(url, Some(t.clone()));
    let me = api.get("/me").await.map_err(|e| {
        if e.downcast_ref::<api::Unauthorized>().is_some() {
            anyhow!("{url} rejected that token (wrong server, expired or revoked)")
        } else {
            e
        }
    })?;
    Credentials {
        server: Some(url.to_string()),
        token: Some(t),
        relay_token: None,
    }
    .save()?;
    ok_line(format!("signed in to {url} as {}", ui::person(&me)));
    Ok(())
}

async fn logout(relay: bool, all: bool, local: bool) -> Result<()> {
    let c = Credentials::load();
    let server = match (c.server.clone(), local) {
        (Some(s), _) => s,
        (None, true) => String::new(),
        (None, false) => return Err(anyhow!("not logged in: nothing to do")),
    };
    let mut targets: Vec<(&str, Option<String>)> = vec![];
    if all || !relay {
        targets.push(("user token", c.token.clone()));
    }
    if all || relay {
        targets.push(("relay credential", c.relay_token.clone()));
    }
    let mut any = false;
    for (what, tok) in targets {
        let Some(tok) = tok else { continue };
        any = true;
        if local {
            continue;
        }
        match Api::new(&server, Some(tok))
            .post("/logout", json!({}))
            .await
        {
            Ok(_) => {}
            Err(e) if e.to_string().contains("HTTP 404") => warn_line(format!(
                "{server} does not support POST /api/v1/logout yet; the {what} stays valid on the server until it expires or is rotated"
            )),
            Err(e) if e.downcast_ref::<api::Unauthorized>().is_some() => {} // already revoked
            Err(e) => warn_line(format!("could not revoke the {what} on the server: {e:#}")),
        }
    }
    Credentials::clear(all || !relay, all || relay)?;
    if any {
        ok_line(if local {
            "removed the stored tokens".to_string()
        } else {
            format!("signed out of {server}")
        });
    } else {
        println!("no stored credential to remove");
    }
    if std::env::var("CORNTCHAP_TOKEN").is_ok() {
        warn_line("CORNTCHAP_TOKEN is set in your environment; unset it to finish signing out");
    }
    Ok(())
}

async fn enroll(name: &str) -> Result<()> {
    let c = Credentials::load();
    let server = c
        .server
        .ok_or_else(|| anyhow!("run `corntchap-cli login <url>` first"))?;
    let (t, p) = device_login(&server, "relay", Some(name), json!({})).await?;
    Credentials {
        relay_token: Some(t),
        ..Default::default()
    }
    .save()?;
    ok_line(format!(
        "relay {} enrolled; credential stored",
        s(&p, "name")
    ));
    let cfg = relay::config_path();
    if cfg.exists() {
        println!(
            "Next: edit {}, then `corntchap-cli fire doctor`.",
            cfg.display()
        );
    } else {
        println!(
            "Next: `corntchap-cli fire init` writes a commented {}, then `corntchap-cli fire doctor`.",
            cfg.display()
        );
    }
    Ok(())
}

/// Device flow with kind=agent: a human approves in the browser, the token is printed once.
async fn agent_enroll(name: &str, kind: &str, runtime: &str) -> Result<()> {
    let c = Credentials::load();
    let server = c
        .server
        .ok_or_else(|| anyhow!("run `corntchap-cli login <url>` first"))?;
    let (token, p) = device_login(
        &server,
        "agent",
        Some(name),
        json!({"agent_kind": kind, "runtime": runtime}),
    )
    .await?;
    let n = s(&p, "name");
    ok_line(format!(
        "agent {n} enrolled. Token (shown once):\n\n  {token}\n"
    ));
    print!("{}", setup::mcp_snippet(kind, &server, &token));
    println!("\nNext: corntchap-cli team join <team> {n}");
    Ok(())
}

fn whoami_text(v: &Value, server: &str) -> String {
    let kind = s(v, "kind");
    let mut role = vec![kind.clone()];
    if kind == "agent" {
        role.push(format!("{} / {}", s(v, "agent_kind"), s(v, "runtime")));
    }
    if v["is_admin"] == true {
        role.push("admin".into());
    }
    let who = ui::person(v);
    let mut o = format!(
        "{}  ({})\n{}\n",
        ui::bold(&who),
        role.join(", "),
        ui::dim(server)
    );
    let teams: Vec<String> = v["teams"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| format!("{} ({})", s(t, "name"), s(t, "role")))
        .collect();
    o.push_str(&format!(
        "\nteams     {}",
        if teams.is_empty() {
            "-".into()
        } else {
            teams.join(", ")
        }
    ));
    let projects: Vec<&str> = v["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    o.push_str(&format!(
        "\nprojects  {}",
        if projects.is_empty() {
            "-".into()
        } else {
            projects.join(", ")
        }
    ));
    let leases = v["current_leases"].as_array().cloned().unwrap_or_default();
    if leases.is_empty() {
        o.push_str("\nleases    none");
    } else {
        o.push_str("\nleases");
        for l in &leases {
            o.push_str(&format!("\n  {} {}", s(l, "id"), s(l, "title")));
        }
    }
    o
}

async fn run(cmd: Cmd, api: Api) -> Result<()> {
    match cmd {
        Cmd::Whoami { json } => {
            let v = api.get("/me").await?;
            show(&v, json, |v| whoami_text(v, &api.server));
        }
        Cmd::Tui => tui::run(api).await?,
        Cmd::Task(t) => task(t, &api).await?,
        Cmd::User(UserCmd::Admin { handle, on, off: _ }) => {
            api.patch(&format!("/users/{handle}"), json!({"is_admin": on}))
                .await?;
            ok_line(format!(
                "{handle} is {} an admin (manual admin setting: the IdP role no longer changes it)",
                if on { "now" } else { "no longer" }
            ));
        }
        Cmd::User(UserCmd::Disable { handle }) => {
            api.patch(&format!("/users/{handle}"), json!({"disabled": true}))
                .await?;
            ok_line(format!(
                "{handle} is disabled: their tokens and sessions no longer work"
            ));
        }
        Cmd::User(UserCmd::Enable { handle }) => {
            api.patch(&format!("/users/{handle}"), json!({"disabled": false}))
                .await?;
            ok_line(format!("{handle} is enabled again (they sign in anew)"));
        }
        Cmd::Me(MeCmd::Rename { handle }) => {
            let v = api.patch("/me", json!({"name": handle})).await?;
            ok_line(format!(
                "you are now @{} (was @{})",
                s(&v, "name"),
                s(&v, "previous")
            ));
        }
        Cmd::User(UserCmd::Ls { json }) => {
            show(&api.get("/users").await?, json, ui::users_table);
        }
        Cmd::Team(TeamCmd::Add { name }) => {
            api.post("/teams", json!({"name": name})).await?;
            ok_line(format!("team {name} created"));
        }
        Cmd::Team(TeamCmd::Ls { json }) => {
            show(&api.get("/teams").await?, json, ui::teams_table);
        }
        Cmd::Team(TeamCmd::Members { team, json }) => {
            show(
                &api.get(&format!("/teams/{team}/members")).await?,
                json,
                ui::members_table,
            );
        }
        Cmd::Team(TeamCmd::Join {
            team,
            principal,
            role,
        }) => {
            api.post(
                &format!("/teams/{team}/members"),
                json!({"principal": principal, "role": role}),
            )
            .await?;
            ok_line(format!("{principal} joined team {team} as {role}"));
        }
        Cmd::Team(TeamCmd::Leave { team, principal }) => {
            api.delete(&format!("/teams/{team}/members/{principal}"))
                .await?;
            ok_line(format!("{principal} left team {team}"));
        }
        Cmd::Project(ProjectCmd::Add {
            team,
            key,
            name,
            repo,
            lease_ttl_s,
        }) => {
            let v = api
                .post(
                    "/projects",
                    json!({"team": team, "key": key, "name": name, "repo_url": repo, "lease_ttl_s": lease_ttl_s}),
                )
                .await?;
            let k = v["key"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| key.to_uppercase());
            ok_line(format!("project {k} ({name}) created in team {team}"));
        }
        Cmd::Project(ProjectCmd::Ls { json }) => {
            show(&api.get("/projects").await?, json, ui::projects_table);
        }
        Cmd::Project(ProjectCmd::Lease { key, length }) => {
            let v = api
                .procedure(
                    "setLeaseTtl",
                    json!({"project": key.to_uppercase(), "ttlS": length.as_secs()}),
                )
                .await?;
            ok_line(format!(
                "pot {} leases now last {}s by default",
                v["key"].as_str().unwrap_or("?"),
                v["lease_ttl_s"]
            ));
        }
        Cmd::Pantry(p) => pantry::run(p, &api).await?,
        Cmd::Menu(c) => menu::run(c, &api).await?,
        Cmd::File(c) => file::run(c, &api).await?,
        Cmd::Pass(c) => pass::run(c, &api).await?,
        Cmd::View(c) => view::run(c, &api).await?,
        Cmd::Agent(a) => agent(a, &api).await?,
        Cmd::Station(c) => station::run(c, &api).await?,
        Cmd::Login { .. }
        | Cmd::Logout { .. }
        | Cmd::Doctor
        | Cmd::Hook(_)
        | Cmd::Checkpoints { .. }
        | Cmd::Relay { .. }
        | Cmd::Completions { .. } => unreachable!(),
    }
    Ok(())
}

async fn agent_setup(
    api: &Api,
    name: &str,
    kind: &str,
    runtime: &str,
    team: Option<&str>,
) -> Result<()> {
    // Check the team first: a typo must not leave a half-made cook behind (or lose its token).
    if let Some(t) = team {
        let teams = api.get("/teams").await?;
        let names: Vec<&str> = teams
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str())
            .collect();
        if !names.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            bail!(
                "no team {t} that you can see; nothing was created. Teams: {}",
                if names.is_empty() {
                    "(none)".to_string()
                } else {
                    names.join(", ")
                }
            );
        }
    }
    let (n, token) = match api
        .post(
            "/agents",
            json!({"name": name, "kind": kind, "runtime": runtime}),
        )
        .await
    {
        Ok(r) => {
            ok_line(format!(
                "agent {} created ({kind}, {runtime})",
                s(&r, "name")
            ));
            (s(&r, "name"), Some(s(&r, "token")))
        }
        // Run again after a failure: finish setting up the cook you already have.
        Err(e) if format!("{e:#}").contains("409") => {
            let mine = api.get("/agents").await?;
            let exists = mine
                .as_array()
                .into_iter()
                .flatten()
                .any(|a| a["name"].as_str() == Some(&name.to_lowercase()));
            if !exists {
                bail!("the name {name} is taken: choose another one");
            }
            ok_line(format!(
                "agent {} already exists: finishing its setup",
                name.to_lowercase()
            ));
            (name.to_lowercase(), None)
        }
        Err(e) => return Err(e),
    };
    match team {
        Some(t) => match api
            .post(
                &format!("/teams/{t}/members"),
                json!({"principal": n, "role": "member"}),
            )
            .await
        {
            Ok(_) => ok_line(format!("{n} joined team {t}")),
            Err(e) if format!("{e:#}").contains("409") => {
                ok_line(format!("{n} is already in team {t}"))
            }
            Err(e) => warn_line(format!(
                "could not add {n} to team {t}: {e:#}\n  retry: corntchap-cli team join {t} {n}"
            )),
        },
        None => println!(
            "Next: corntchap-cli team join <team> {n}   (an agent only sees its team's projects)"
        ),
    }
    let had_relay = token.is_none()
        && api
            .get(&format!("/agents/{n}/wake-targets"))
            .await
            .ok()
            .and_then(|v| v.as_array().map(|a| a.iter().any(|t| t["type"] == "relay")))
            .unwrap_or(false);
    if runtime == "local" && !had_relay {
        match api
            .post(
                &format!("/agents/{n}/wake-targets"),
                json!({"type": "relay", "projects": [], "labels": [], "config": {}}),
            )
            .await
        {
            Ok(_) => ok_line(format!("relay wake target added for {n}")),
            Err(e) => warn_line(format!("could not add the relay wake target: {e:#}")),
        }
    }
    let Some(token) = token else {
        println!(
            "\nIts token was shown when it was created. Lost it? `corntchap-cli cook rotate {n}` revokes it and prints a new one."
        );
        return Ok(());
    };
    println!("\nToken (shown once):\n\n  {token}\n");
    print!("{}", setup::mcp_snippet(kind, &api.server, &token));
    print!("{}", setup::oauth_hint(&api.server));
    if runtime == "local" {
        println!(
            "\nWake it per session: from inside your repo run `corntchap-cli station scan` (registers its sessions as stations, no config needed), then `corntchap-cli fire doctor`.\nOptional fallback for wakes that name no station: paste this into {} (`corntchap-cli fire init` writes the file):\n",
            relay::config_path().display()
        );
        println!("{}", relay::target_block(&n, kind));
    } else {
        println!(
            "\nCloud agent: wake it with a webhook, e.g.\n  corntchap-cli cook wake add {n} --type webhook --url https://… --token …"
        );
    }
    Ok(())
}

async fn wake_targets(api: &Api, agent: Option<String>) -> Result<Vec<(String, Value)>> {
    let agents: Vec<String> = match agent {
        Some(a) => vec![a],
        None => api
            .get("/agents")
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["name"].as_str().map(String::from))
            .collect(),
    };
    let mut all = vec![];
    for a in agents {
        let v = api.get(&format!("/agents/{a}/wake-targets")).await?;
        for w in v.as_array().into_iter().flatten() {
            all.push((a.clone(), w.clone()));
        }
    }
    Ok(all)
}

async fn agent(a: AgentCmd, api: &Api) -> Result<()> {
    match a {
        AgentCmd::Add {
            name,
            kind,
            runtime,
            max_leases,
            owner,
        } => {
            let r = api.post("/agents", json!({"name": name, "kind": kind, "runtime": runtime, "max_leases": max_leases, "owner": owner})).await?;
            println!(
                "agent {} created. Token (shown once):\n\n  {}\n",
                s(&r, "name"),
                s(&r, "token")
            );
            println!(
                "MCP:  {}/mcp   header  Authorization: Bearer <token>",
                api.server
            );
            println!("Next: corntchap-cli team join <team> {}", s(&r, "name"));
        }
        AgentCmd::Setup {
            name,
            kind,
            runtime,
            team,
        } => agent_setup(api, &name, &kind, &runtime, team.as_deref()).await?,
        AgentCmd::Enroll { .. } => unreachable!(),
        AgentCmd::Ls { json } => show(&api.get("/agents").await?, json, ui::agents_table),
        AgentCmd::Grant { name, grants, none } => {
            if grants.is_empty() && !none {
                bail!(
                    "say what {name} may do (plan, manage, admin), or --none to take everything away"
                );
            }
            let r = api
                .post(&format!("/agents/{name}/grants"), json!({"grants": grants}))
                .await?;
            let g: Vec<&str> = r["grants"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str())
                .collect();
            if g.is_empty() {
                ok_line(format!("{name} works its tasks only"));
            } else {
                ok_line(format!("{name} may also {}", g.join(", ")));
            }
        }
        AgentCmd::Limit { name, leases } => {
            let list = api.get("/agents").await?;
            let id = list
                .as_array()
                .into_iter()
                .flatten()
                .find(|a| a["name"].as_str() == Some(name.to_lowercase().as_str()))
                .and_then(|a| a["id"].as_str().map(String::from))
                .ok_or_else(|| anyhow!("no cook named {name} among yours"))?;
            // the principals model: only maxLeases is writable, by the cook's owner or an admin
            let mut r = api
                .http
                .patch(format!("{}/api/v2/principals/{id}", api.server))
                .json(&json!({"maxLeases": leases}));
            if let Some(t) = &api.token {
                r = r.bearer_auth(t);
            }
            api.send(r).await?;
            ok_line(format!(
                "{name} may hold {leases} task{} at once",
                if leases == 1 { "" } else { "s" }
            ));
        }
        AgentCmd::Rotate { name } => {
            let r = api
                .post(&format!("/agents/{name}/rotate"), json!({}))
                .await?;
            println!(
                "new token for {} (old one stopped working):\n\n  {}",
                s(&r, "name"),
                s(&r, "token")
            );
        }
        AgentCmd::Disable { name } => {
            api.post(&format!("/agents/{name}/disable"), json!({}))
                .await?;
            ok_line(format!("{name} disabled"));
        }
        AgentCmd::Enable { name } => {
            api.post(&format!("/agents/{name}/enable"), json!({}))
                .await?;
            ok_line(format!("{name} enabled"));
        }
        AgentCmd::Wake(w) => match w {
            WakeCmd::Add {
                agent,
                typ,
                projects,
                labels,
                url,
                token,
                secret,
                repo,
                issue,
                mention,
            } => {
                let mut config = json!({});
                match typ.as_str() {
                    "webhook" => config = json!({"url": url, "token": token, "secret": secret}),
                    "claude-routine" => config = json!({"url": url, "token": token}),
                    "github-comment" => {
                        config = json!({"repo": repo, "issue": issue, "token": token});
                        if let Some(m) = mention.filter(|m| !m.trim().is_empty()) {
                            let m = m.trim();
                            config["mention"] = json!(if m.starts_with('@') {
                                m.to_string()
                            } else {
                                format!("@{m}")
                            });
                        }
                    }
                    _ => {}
                }
                let r = api.post(&format!("/agents/{agent}/wake-targets"), json!({"type": typ, "projects": projects, "labels": labels, "config": config})).await?;
                ok_line(format!(
                    "{typ} wake target added for {agent}{}",
                    r["id"]
                        .as_str()
                        .map(|i| format!(" ({i})"))
                        .unwrap_or_default()
                ));
                if let Some(prompt) = r["routine_prompt"].as_str() {
                    println!(
                        "\nPut this in the routine's prompt (claude.ai/code/routines), so each run acts on what corntchap sends:\n\n{prompt}\n\nThe routine also needs the corntchap MCP server, with {agent}'s token, in its connectors or its repo's .mcp.json."
                    );
                }
            }
            WakeCmd::Ls { agent, json } => {
                let all = wake_targets(api, agent).await?;
                if json {
                    out(&Value::Array(
                        all.iter()
                            .map(|(a, w)| {
                                let mut w = w.clone();
                                w["agent"] = json!(a);
                                w
                            })
                            .collect(),
                    ));
                } else {
                    println!("{}", ui::wake_table(&all));
                }
            }
            WakeCmd::Rm { id } => {
                api.delete(&format!("/wake-targets/{id}")).await?;
                ok_line(format!("wake target {id} removed"));
            }
        },
    }
    Ok(())
}

async fn task(t: TaskCmd, api: &Api) -> Result<()> {
    match t {
        TaskCmd::Ls {
            project,
            status,
            ready,
            label,
            menu,
            json,
        } => {
            let params =
                menu::task_ls_params(status.as_deref(), label.as_deref(), menu.as_deref(), ready);
            let r = api.raw(
                reqwest::Method::GET,
                &format!("/projects/{}/tasks", project.to_uppercase()),
            );
            let v = api.send(r.query(&params)).await?;
            show(&v, json, ui::tasks_table);
        }
        TaskCmd::Add {
            project,
            title,
            body,
            items,
            labels,
            depends_on,
            parent,
            priority,
            menu,
        } => {
            let v = api.post(&format!("/projects/{}/tasks", project.to_uppercase()), json!({"title": title, "body": body, "items": items, "labels": labels, "depends_on": depends_on, "parent": parent, "priority": priority, "menu": menu})).await?;
            ok_line(format!("created {}", s(&v, "id")));
        }
        TaskCmd::Edit {
            id,
            title,
            body,
            labels,
            priority,
            menu,
            no_menu,
        } => {
            let b = menu::task_edit_body(
                title.as_deref(),
                body.as_deref(),
                &labels,
                priority,
                menu.as_deref(),
                no_menu,
            );
            if b.as_object().is_some_and(|m| m.is_empty()) {
                return Err(anyhow!(
                    "nothing to change: pass --title, --body, --label, --priority, --menu or --no-menu"
                ));
            }
            let v = api.patch(&format!("/tasks/{id}"), b).await?;
            ok_line(format!(
                "{} updated{}",
                s(&v, "id"),
                v["menu"]
                    .as_str()
                    .map(|m| format!(" (menu {m})"))
                    .unwrap_or_default()
            ));
        }
        TaskCmd::Show { id, json } => {
            let v = api.get(&format!("/tasks/{id}")).await?;
            show(&v, json, ui::task_detail);
        }
        TaskCmd::Done { id, result } => {
            let c = api.post(&format!("/tasks/{id}/claim"), json!({})).await?;
            let epoch = c["lease"]["epoch"].as_i64().unwrap_or(0);
            api.post(
                &format!("/tasks/{id}/complete"),
                json!({"epoch": epoch, "result": result.unwrap_or_default()}),
            )
            .await?;
            ok_line(format!("{id} done"));
        }
        TaskCmd::Unblock { id } => {
            api.post(&format!("/tasks/{id}/unblock"), json!({})).await?;
            ok_line(format!("{id} unblocked"));
        }
        TaskCmd::Toss { id, reason } => {
            let body = match reason {
                Some(r) => json!({"reason": r}),
                None => json!({}),
            };
            api.post(&format!("/tasks/{id}/toss"), body).await?;
            ok_line(format!("{id} tossed"));
        }
        TaskCmd::Restore { id } => {
            api.post(&format!("/tasks/{id}/restore"), json!({})).await?;
            ok_line(format!("{id} is back on the counter"));
        }
        TaskCmd::ClaimNext {
            project,
            labels,
            menu,
            station,
            lease,
            ..
        } => {
            let mut body = menu::claim_next_body(&labels, menu.as_deref());
            if let Some(st) = station {
                body["station"] = json!(st);
            }
            if let Some(l) = lease {
                body["ttl_s"] = json!(l.as_secs());
            }
            out(&api
                .post(
                    &format!("/projects/{}/claim-next", project.to_uppercase()),
                    body,
                )
                .await?)
        }
        TaskCmd::Claim { id, lease, .. } => {
            let body = match lease {
                Some(l) => json!({"ttl_s": l.as_secs()}),
                None => json!({}),
            };
            out(&api.post(&format!("/tasks/{id}/claim"), body).await?)
        }
        TaskCmd::Check {
            id,
            epoch,
            item,
            undo,
        } => {
            api.post(
                &format!("/tasks/{id}/check"),
                json!({"epoch": epoch, "position": item, "done": !undo}),
            )
            .await?;
            println!(
                "{id} item {item} {}",
                if undo { "unchecked" } else { "checked" }
            );
        }
        TaskCmd::Heartbeat { id, epoch } => out(&api
            .post(&format!("/tasks/{id}/heartbeat"), json!({"epoch": epoch}))
            .await?),
        TaskCmd::Complete { id, epoch, result } => out(&api
            .post(
                &format!("/tasks/{id}/complete"),
                json!({"epoch": epoch, "result": result}),
            )
            .await?),
        TaskCmd::Release { id, epoch, reason } => out(&api
            .post(
                &format!("/tasks/{id}/release"),
                json!({"epoch": epoch, "reason": reason}),
            )
            .await?),
        TaskCmd::Block { id, epoch, reason } => out(&api
            .post(
                &format!("/tasks/{id}/block"),
                json!({"epoch": epoch, "reason": reason}),
            )
            .await?),
        TaskCmd::Comment {
            id,
            body,
            epoch,
            station,
        } => out(&api
            .post(
                &format!("/tasks/{id}/comment"),
                json!({"body": body, "epoch": epoch, "station": station}),
            )
            .await?),
        TaskCmd::Changes { cursor, project } => {
            let q = match project {
                Some(p) => format!("?cursor={cursor}&project={p}"),
                None => format!("?cursor={cursor}"),
            };
            out(&api.get(&format!("/changes{q}")).await?)
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(shell: clap_complete::Shell) -> String {
        let mut out = Vec::new();
        completions(shell, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn zsh_completions_cover_the_commands_and_their_aliases() {
        let z = script(clap_complete::Shell::Zsh);
        assert!(
            z.starts_with("#compdef corntchap-cli"),
            "{}",
            &z[..80.min(z.len())]
        );
        for word in [
            "kitchen",
            "task",
            "station",
            "fire",
            "marabou",
            "me",
            "completions",
            "rename",
        ] {
            assert!(
                z.contains(&format!("'{word}:")) || z.contains(&format!("({word})")),
                "zsh completion lacks `{word}`"
            );
        }
    }

    #[test]
    fn every_shell_homebrew_and_scoop_install_generates_a_script() {
        use clap_complete::Shell::*;
        for (shell, marker) in [
            (Bash, "complete -F"),
            (Fish, "complete -c corntchap-cli"),
            (PowerShell, "Register-ArgumentCompleter"),
        ] {
            assert!(
                script(shell).contains(marker),
                "{shell:?} completion lacks `{marker}`"
            );
        }
    }

    #[test]
    fn the_cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
