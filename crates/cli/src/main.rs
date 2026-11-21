mod api;
mod opencode;
mod pantry;
mod relay;
mod setup;
mod tui;
mod ui;

use anyhow::{Result, anyhow};
use api::{Api, Credentials, device_login};
use clap::{Parser, Subcommand};
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
    /// The cooks (agents): create, set up, wake targets.
    #[command(subcommand, name = "cook", alias = "agent")]
    Agent(AgentCmd),
    /// Keep the fire lit: the relay that wakes your local cooks when new work lands (subcommands: init, install, doctor, enroll).
    #[command(name = "fire", alias = "relay")]
    Relay {
        #[command(subcommand)]
        cmd: Option<RelayCmd>,
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
    /// Cancel a task.
    Cancel { id: String },
    /// Scoop the next ready portion: agent mode, claims the best ready task (JSON).
    #[command(name = "scoop", alias = "claim-next")]
    ClaimNext {
        #[arg(long)]
        project: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Agent mode: claim one task (JSON).
    Claim {
        id: String,
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
enum UserCmd {
    /// Add a user by their OIDC username (admin).
    Add {
        login: String,
        #[arg(long)]
        admin: bool,
    },
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
    /// List a team's members.
    Members {
        team: String,
        #[arg(long)]
        json: bool,
    },
    /// Add a user or agent to a team.
    Join {
        team: String,
        principal: String,
        #[arg(long, default_value = "member")]
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
    /// Stop an agent from claiming work.
    Disable { name: String },
    /// Let a disabled agent claim work again.
    Enable { name: String },
    /// Wake targets: how and when an idle agent gets woken.
    #[command(subcommand)]
    Wake(WakeCmd),
}

#[derive(Subcommand)]
enum WakeCmd {
    /// Add a wake target to an agent.
    Add {
        agent: String,
        /// relay | webhook | github-comment
        #[arg(long = "type")]
        typ: String,
        #[arg(long = "project")]
        projects: Vec<String>,
        #[arg(long = "label")]
        labels: Vec<String>,
        /// webhook: URL to POST (e.g. a Claude routine /fire URL)
        #[arg(long)]
        url: Option<String>,
        /// webhook: bearer token for the URL; github-comment: GitHub token
        #[arg(long)]
        token: Option<String>,
        /// github-comment: owner/name and issue number to comment on
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        issue: Option<i64>,
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
    /// Write a systemd user unit (Linux) or launchd agent (macOS).
    Install,
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
    ok_line(format!("signed in to {url} as {}", s(&me, "name")));
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
    let mut o = format!(
        "{}  ({})\n{}\n",
        ui::bold(&s(v, "name")),
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
        Cmd::User(UserCmd::Add { login, admin }) => {
            api.post("/users", json!({"login": login, "is_admin": admin}))
                .await?;
            ok_line(format!(
                "added user {login}{} (they sign in at {}/board or with `corntchap-cli login`)",
                if admin { " as admin" } else { "" },
                api.server
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
        Cmd::Pantry(p) => pantry::run(p, &api).await?,
        Cmd::Agent(a) => agent(a, &api).await?,
        Cmd::Login { .. } | Cmd::Logout { .. } | Cmd::Doctor | Cmd::Relay { .. } => unreachable!(),
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
    let r = api
        .post(
            "/agents",
            json!({"name": name, "kind": kind, "runtime": runtime}),
        )
        .await?;
    let n = s(&r, "name");
    let token = s(&r, "token");
    ok_line(format!("agent {n} created ({kind}, {runtime})"));
    match team {
        Some(t) => {
            api.post(
                &format!("/teams/{t}/members"),
                json!({"principal": n, "role": "member"}),
            )
            .await?;
            ok_line(format!("{n} joined team {t}"));
        }
        None => println!(
            "Next: corntchap-cli team join <team> {n}   (an agent only sees its team's projects)"
        ),
    }
    if runtime == "local" {
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
    println!("\nToken (shown once):\n\n  {token}\n");
    print!("{}", setup::mcp_snippet(kind, &api.server, &token));
    if runtime == "local" {
        println!(
            "\nRelay: paste this into {} (run `corntchap-cli fire init` first if it does not exist),\nthen `corntchap-cli fire doctor`:\n",
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
                repo,
                issue,
            } => {
                let mut config = json!({});
                match typ.as_str() {
                    "webhook" => config = json!({"url": url, "token": token}),
                    "github-comment" => {
                        config = json!({"repo": repo, "issue": issue, "token": token})
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
            json,
        } => {
            let mut q = vec![];
            if let Some(s) = status {
                q.push(format!("status={s}"));
            }
            if let Some(l) = label {
                q.push(format!("label={l}"));
            }
            if ready {
                q.push("ready_only=true".into());
            }
            let v = api
                .get(&format!(
                    "/projects/{}/tasks?{}",
                    project.to_uppercase(),
                    q.join("&")
                ))
                .await?;
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
        } => {
            let v = api.post(&format!("/projects/{}/tasks", project.to_uppercase()), json!({"title": title, "body": body, "items": items, "labels": labels, "depends_on": depends_on, "parent": parent, "priority": priority})).await?;
            ok_line(format!("created {}", s(&v, "id")));
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
        TaskCmd::Cancel { id } => {
            api.post(&format!("/tasks/{id}/cancel"), json!({})).await?;
            ok_line(format!("{id} cancelled"));
        }
        TaskCmd::ClaimNext {
            project, labels, ..
        } => out(&api
            .post(
                &format!("/projects/{}/claim-next", project.to_uppercase()),
                json!({"labels": labels}),
            )
            .await?),
        TaskCmd::Claim { id, .. } => {
            out(&api.post(&format!("/tasks/{id}/claim"), json!({})).await?)
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
        TaskCmd::Comment { id, body, epoch } => out(&api
            .post(
                &format!("/tasks/{id}/comment"),
                json!({"body": body, "epoch": epoch}),
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
