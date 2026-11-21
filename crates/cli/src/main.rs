mod api;
mod relay;

use anyhow::Result;
use api::{Api, Credentials, device_login};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

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
    },
    /// Who am I, and what do I hold?
    Whoami,
    /// Tasks: the board for humans, plus JSON commands for agents that only have a shell.
    #[command(subcommand)]
    Task(TaskCmd),
    #[command(subcommand)]
    User(UserCmd),
    #[command(subcommand)]
    Team(TeamCmd),
    #[command(subcommand)]
    Project(ProjectCmd),
    #[command(subcommand)]
    Agent(AgentCmd),
    /// Run the relay in the foreground (subcommands: install, doctor, enroll).
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
    /// Human shortcut: claim, then complete.
    Done {
        id: String,
        #[arg(long)]
        result: Option<String>,
    },
    Unblock {
        id: String,
    },
    Cancel {
        id: String,
    },
    /// Agent mode: claim the best ready task (JSON).
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
    Heartbeat {
        id: String,
        #[arg(long)]
        epoch: i64,
    },
    Complete {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        result: String,
    },
    Release {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    Block {
        id: String,
        #[arg(long)]
        epoch: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    Comment {
        id: String,
        body: String,
        #[arg(long)]
        epoch: Option<i64>,
    },
    /// Events after a cursor.
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
    Ls,
}

#[derive(Subcommand)]
enum TeamCmd {
    Add {
        name: String,
    },
    Ls,
    Members {
        team: String,
    },
    /// Add a user or agent to a team.
    Join {
        team: String,
        principal: String,
        #[arg(long, default_value = "member")]
        role: String,
    },
    Leave {
        team: String,
        principal: String,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    Add {
        team: String,
        key: String,
        name: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        lease_ttl_s: Option<i64>,
    },
    Ls,
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
    Ls,
    /// Revoke every token and print a fresh one.
    Rotate {
        name: String,
    },
    Disable {
        name: String,
    },
    Enable {
        name: String,
    },
    /// Wake targets: how and when an idle agent gets woken.
    #[command(subcommand)]
    Wake(WakeCmd),
}

#[derive(Subcommand)]
enum WakeCmd {
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
    Ls {
        agent: String,
    },
    Rm {
        id: String,
    },
}

#[derive(Subcommand)]
enum RelayCmd {
    /// Write a systemd user unit (Linux) or launchd agent (macOS).
    Install,
    /// Check the server, the credential and each target.
    Doctor,
    /// Register this machine as a relay (device flow) and store its credential.
    Enroll { name: String },
}

fn out(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(x) => x.clone(),
        Value::Null => "-".into(),
        o => o.to_string(),
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
        Cmd::Login { url, token } => login(&url, token).await,
        Cmd::Relay { cmd } => match cmd {
            None => relay::run(Api::from_creds(true)?).await,
            Some(RelayCmd::Install) => relay::install(),
            Some(RelayCmd::Doctor) => relay::doctor().await,
            Some(RelayCmd::Enroll { name }) => enroll(&name).await,
        },
        other => run(other, Api::from_creds(false)?).await,
    }
}

async fn login(url: &str, token: Option<String>) -> Result<()> {
    let url = url.trim_end_matches('/');
    let t = match token {
        Some(t) => t,
        None => device_login(url, "user", None, json!({})).await?.0,
    };
    let api = Api::new(url, Some(t.clone()));
    let me = api.get("/me").await?;
    Credentials {
        server: Some(url.to_string()),
        token: Some(t),
        relay_token: None,
    }
    .save()?;
    println!("signed in to {url} as {}", s(&me, "name"));
    Ok(())
}

async fn enroll(name: &str) -> Result<()> {
    let c = Credentials::load();
    let server = c
        .server
        .ok_or_else(|| anyhow::anyhow!("run `corntchap-cli login <url>` first"))?;
    let (t, p) = device_login(&server, "relay", Some(name), json!({})).await?;
    Credentials {
        relay_token: Some(t),
        ..Default::default()
    }
    .save()?;
    println!(
        "relay {} enrolled; credential stored. Next: edit ~/.config/corntchap/config.toml, then `corntchap-cli relay doctor`.",
        s(&p, "name")
    );
    Ok(())
}

fn print_tasks(v: &Value) {
    for t in v.as_array().into_iter().flatten() {
        let mark = match t["status"].as_str() {
            Some("done") => "x",
            _ => " ",
        };
        println!(
            "[{mark}] {:<8} {:<9} p{:<2} {}  ({}/{} items){}",
            s(t, "id"),
            s(t, "status"),
            t["priority"],
            s(t, "title"),
            t["total_items"].as_i64().unwrap_or(0) - t["open_items"].as_i64().unwrap_or(0),
            t["total_items"],
            t["claimed_by"]
                .as_str()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default()
        );
    }
}

async fn run(cmd: Cmd, api: Api) -> Result<()> {
    match cmd {
        Cmd::Whoami => out(&api.get("/me").await?),
        Cmd::Task(t) => task(t, &api).await?,
        Cmd::User(UserCmd::Add { login, admin }) => out(&api.post("/users", json!({"login": login, "is_admin": admin})).await?),
        Cmd::User(UserCmd::Ls) => {
            for u in api.get("/users").await?.as_array().into_iter().flatten() {
                println!("{:<24} {}{}", s(u, "name"), if u["is_admin"] == true { "admin " } else { "" }, if u["linked"] == true { "(signed in once)" } else { "(not signed in yet)" });
            }
        }
        Cmd::Team(TeamCmd::Add { name }) => out(&api.post("/teams", json!({"name": name})).await?),
        Cmd::Team(TeamCmd::Ls) => {
            for t in api.get("/teams").await?.as_array().into_iter().flatten() {
                println!("{:<20} role={} members={} projects={}", s(t, "name"), s(t, "role"), t["members"], t["projects"]);
            }
        }
        Cmd::Team(TeamCmd::Members { team }) => {
            for m in api.get(&format!("/teams/{team}/members")).await?.as_array().into_iter().flatten() {
                println!("{:<24} {:<6} {}", s(m, "name"), s(m, "kind"), s(m, "role"));
            }
        }
        Cmd::Team(TeamCmd::Join { team, principal, role }) => out(&api.post(&format!("/teams/{team}/members"), json!({"principal": principal, "role": role})).await?),
        Cmd::Team(TeamCmd::Leave { team, principal }) => out(&api.delete(&format!("/teams/{team}/members/{principal}")).await?),
        Cmd::Project(ProjectCmd::Add { team, key, name, repo, lease_ttl_s }) => {
            out(&api.post("/projects", json!({"team": team, "key": key, "name": name, "repo_url": repo, "lease_ttl_s": lease_ttl_s})).await?)
        }
        Cmd::Project(ProjectCmd::Ls) => {
            for p in api.get("/projects").await?.as_array().into_iter().flatten() {
                println!("{:<8} {:<24} team={} ready={} claimed={} done={}", s(p, "key"), s(p, "name"), s(p, "team"), p["ready"], p["claimed"], p["done"]);
            }
        }
        Cmd::Agent(a) => agent(a, &api).await?,
        Cmd::Login { .. } | Cmd::Relay { .. } => unreachable!(),
    }
    Ok(())
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
        AgentCmd::Ls => {
            for x in api.get("/agents").await?.as_array().into_iter().flatten() {
                println!(
                    "{:<22} {:<12} {:<6} owner={} busy={}{}",
                    s(x, "name"),
                    s(x, "agent_kind"),
                    s(x, "runtime"),
                    s(x, "owner"),
                    x["busy"],
                    if x["disabled"] == true {
                        " DISABLED"
                    } else {
                        ""
                    }
                );
            }
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
        AgentCmd::Disable { name } => out(&api
            .post(&format!("/agents/{name}/disable"), json!({}))
            .await?),
        AgentCmd::Enable { name } => out(&api
            .post(&format!("/agents/{name}/enable"), json!({}))
            .await?),
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
                out(&api.post(&format!("/agents/{agent}/wake-targets"), json!({"type": typ, "projects": projects, "labels": labels, "config": config})).await?)
            }
            WakeCmd::Ls { agent } => {
                out(&api.get(&format!("/agents/{agent}/wake-targets")).await?)
            }
            WakeCmd::Rm { id } => out(&api.delete(&format!("/wake-targets/{id}")).await?),
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
            if json { out(&v) } else { print_tasks(&v) }
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
            println!("created {}", s(&v, "id"));
        }
        TaskCmd::Show { id, json } => {
            let v = api.get(&format!("/tasks/{id}")).await?;
            if json {
                out(&v);
            } else {
                println!(
                    "{} {} [{}]{}\n",
                    s(&v, "id"),
                    s(&v, "title"),
                    s(&v, "status"),
                    v["claimed_by"]
                        .as_str()
                        .map(|b| format!(" held by {b}"))
                        .unwrap_or_default()
                );
                if !s(&v, "body").is_empty() && s(&v, "body") != "-" {
                    println!("{}\n", s(&v, "body"));
                }
                for i in v["items"].as_array().into_iter().flatten() {
                    println!(
                        "  {}. [{}] {}",
                        i["position"],
                        if i["done"] == true { "x" } else { " " },
                        s(i, "text")
                    );
                }
                for c in v["comments"].as_array().into_iter().flatten() {
                    println!("\n  {}: {}", s(c, "author"), s(c, "body"));
                }
            }
        }
        TaskCmd::Done { id, result } => {
            let c = api.post(&format!("/tasks/{id}/claim"), json!({})).await?;
            let epoch = c["lease"]["epoch"].as_i64().unwrap_or(0);
            api.post(
                &format!("/tasks/{id}/complete"),
                json!({"epoch": epoch, "result": result.unwrap_or_default()}),
            )
            .await?;
            println!("{id} done");
        }
        TaskCmd::Unblock { id } => {
            out(&api.post(&format!("/tasks/{id}/unblock"), json!({})).await?)
        }
        TaskCmd::Cancel { id } => out(&api.post(&format!("/tasks/{id}/cancel"), json!({})).await?),
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
