//! Connecting a cloud cook in steps, from an agent's MCP tools (`connect_cloud`,
//! `connect_cloud_save`) or the console. A setup names the cook and the kind of wake; the secret
//! (a routine token, a GitHub token, a webhook signing secret) then comes in one of three ways:
//! the MCP App form the host shows next to the tool call (it calls `connect_cloud_save` itself,
//! the secret never reaches the model), the setup's console link (signed in), or the agent
//! passing it to `connect_cloud_save` (it then sits in the conversation). The setup records the
//! wake target it made and can send a test wake.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin,
    cloud::{self, WakeText},
    error::{Error, Result, invalid},
    state::{AppState, Principal},
    stations,
};

pub const KINDS: &[&str] = &["claude-routine", "github-comment", "webhook", "coder"];
/// The MCP App that renders a setup (served by the MCP front door).
pub const UI_URI: &str = "ui://corntchap/connect-cloud";
pub const UI_HTML: &str = include_str!("connect.html");
/// The only host a non-admin's routine trigger may point at.
const ROUTINE_PREFIX: &str = "https://api.anthropic.com/v1/claude_code/routines/";
const TTL_MINUTES: i64 = 60;

struct Setup {
    id: Uuid,
    code: String,
    agent_id: Uuid,
    cook: String,
    owner: Option<Uuid>,
    kind: String,
    projects: Vec<String>,
    created_by: Uuid,
    expired: bool,
    target: Option<Uuid>,
    target_type: Option<String>,
    test: Option<Value>,
    expires_at: String,
    done: bool,
}

fn new_code() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("cs_{}{}", &a[..16], &b[..16])
}

/// Open a setup for a cook: yours (a cook connecting itself) or one you manage.
pub async fn open(
    state: &AppState,
    p: &Principal,
    cook: Option<&str>,
    kind: &str,
    projects: Option<&str>,
) -> Result<Value> {
    let db = &state.db;
    let kind = kind.trim().to_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return invalid(format!("kind is one of {}", KINDS.join(", ")));
    }
    let cook = stations::cook_for(db, p, cook).await?;
    let projects: Vec<String> = projects
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let code = new_code();
    sqlx::query(
        "insert into cloud_setups (code, agent_id, type, projects, created_by, expires_at)
         values ($1, $2, $3, $4, $5, now() + make_interval(mins => $6::int))",
    )
    .bind(&code)
    .bind(cook.id)
    .bind(&kind)
    .bind(&projects)
    .bind(p.id)
    .bind(TTL_MINUTES as i32)
    .execute(db)
    .await?;
    status(state, p, &code).await
}

async fn load(db: &PgPool, code: &str) -> Result<Setup> {
    let row: Option<(
        Uuid,
        String,
        Uuid,
        String,
        Option<Uuid>,
        String,
        Vec<String>,
        Uuid,
        bool,
        Option<Uuid>,
        Option<String>,
        Option<Value>,
        String,
        bool,
    )> = sqlx::query_as(
        "select s.id, s.code, s.agent_id, a.name, a.owner_id, s.type, s.projects, s.created_by,
                s.expires_at < now() and s.done_at is null, s.target_id, w.type, s.test,
                to_char(s.expires_at at time zone 'utc', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
                s.done_at is not null
         from cloud_setups s join principals a on a.id = s.agent_id
         left join wake_targets w on w.id = s.target_id
         where s.code = $1",
    )
    .bind(code.trim())
    .fetch_optional(db)
    .await?;
    let r = row.ok_or_else(|| Error::NotFound(format!("no cloud setup {}", code.trim())))?;
    Ok(Setup {
        id: r.0,
        code: r.1,
        agent_id: r.2,
        cook: r.3,
        owner: r.4,
        kind: r.5,
        projects: r.6,
        created_by: r.7,
        expired: r.8,
        target: r.9,
        target_type: r.10,
        test: r.11,
        expires_at: r.12,
        done: r.13,
    })
}

/// The cook itself, whoever opened the setup, the cook's owner (or a cook acting for them) and
/// admins may see and finish a setup. Returns whether the caller acts as an admin.
async fn allowed(db: &PgPool, p: &Principal, s: &Setup) -> Result<bool> {
    let who = admin::acting_or_self(db, p).await?;
    if p.id == s.agent_id || p.id == s.created_by || who.is_admin || s.owner == Some(who.id) {
        return Ok(who.is_admin);
    }
    Err(Error::NotFound(format!("no cloud setup {}", s.code)))
}

fn field(
    name: &str,
    label: &str,
    secret: bool,
    required: bool,
    placeholder: &str,
    help: &str,
) -> Value {
    json!({"name": name, "label": label, "secret": secret, "required": required,
           "placeholder": placeholder, "help": help})
}

fn guide(s: &Setup, default_mention: &str) -> (Vec<String>, Vec<Value>) {
    let cook = &s.cook;
    match s.kind.as_str() {
        "claude-routine" => (
            vec![
                "Open claude.ai/code/routines and create a routine (or open one) on the repository this cook works in.".into(),
                "Paste the routine prompt below into the routine's prompt, so each run acts on what corntchap sends.".into(),
                format!("Give the routine the corntchap MCP server as a connector, signed in with {cook}'s token."),
                "Add an API trigger to the routine and copy its URL and its token (the token is shown once).".into(),
                "Know that each wake starts a new session of the routine: it cannot resume or reach an existing session, so it works from what corntchap sends and keeps (task comments, the pass, the pantry). Do not change the routine's prompt to forward wakes elsewhere.".into(),
                "It is on call: a direct message, an @mention or a comment on its task starts a run; broadcasts and answers to its questions do not (it reads answers on its next run). Tick \"new work\" only if it should also start for every ready task.".into(),
                "Enter both here. corntchap keeps the token and never shows it again.".into(),
            ],
            vec![
                field("url", "Trigger URL", false, true, "https://api.anthropic.com/v1/claude_code/routines/trig_…/fire", "The routine's API trigger URL."),
                field("token", "Trigger token", true, true, "sk-ant-oat01-…", "The API trigger's token."),
            ],
        ),
        "github-comment" => (
            vec![
                "Connect the repository to the cloud agent you use (Codex cloud, Copilot, …) so that a comment mentioning it starts a task.".into(),
                "Pick an issue or pull request for corntchap to comment on: each wake is one comment there, mentioning the bot.".into(),
                "Create a GitHub token that may comment on that repository (fine-grained: Issues and Pull requests, read and write).".into(),
                "Enter the repository, the issue number and the token here.".into(),
            ],
            vec![
                field("repo", "Repository", false, true, "owner/name", "Where the comments go."),
                field("issue", "Issue or PR number", false, true, "42", "The thread corntchap comments on."),
                field("mention", "Mention", false, false, default_mention, &format!("Who each comment mentions, e.g. @codex or @copilot (default {default_mention}).")),
                field("token", "GitHub token", true, true, "github_pat_…", "Allowed to comment on that repository."),
            ],
        ),
        "coder" => (
            vec![
                "Coder (coder.com) runs the cook as an AI agent in an environment you control. Current Coder (v2.36 and later) uses Coder Agents: a chat per run. Coder Tasks, which older deployments use, was removed upstream: for those give a template instead of an organization.".into(),
                "In Coder, create a session token for the user the agent runs as (a service account is best). corntchap sends it as Coder-Session-Token and keeps it: it is never shown again.".into(),
                "Make the corntchap MCP server reachable from the agent, signed in with the cook's token: add it as an MCP server in Coder Agents (or, for Tasks, in the template's agent module). corntchap sends no credential in the prompt.".into(),
                "Enter the deployment URL and the organization id (coder organizations list). To run in a known environment, give a workspace id too: the agent then works in that workspace instead of picking a template itself.".into(),
                "Older Coder with Tasks: leave the organization empty and give the template (its name or version id), and optionally its preset id and the owner (default me).".into(),
                "The first wake starts a run; later wakes for the same pot are follow-up messages to it. It is on call: a direct message, an @mention or a comment on its task wakes it; broadcasts and answers do not. Tick 'new work' only if it should also start for every ready task. An admin connects Coder.".into(),
            ],
            vec![
                field("url", "Coder URL", false, true, "https://coder.example.com", "The deployment."),
                field("token", "Session token", true, true, "", "A Coder API token (Coder-Session-Token) of the user the agent runs as."),
                field("organization", "Organization id", false, false, "", "Coder Agents: the organization the chats belong to."),
                field("workspace", "Workspace id", false, false, "", "Coder Agents: run in this workspace (optional; else the agent picks a template)."),
                field("template", "Template", false, false, "", "Coder Tasks (older Coder): template name or version id."),
                field("preset", "Preset id", false, false, "", "Coder Tasks: the template version preset (optional)."),
                field("owner", "Owner", false, false, "me", "Coder Tasks: the user the task belongs to (default me)."),
            ],
        ),
        _ => (
            vec![
                format!("Your endpoint receives one POST per wake for {cook}, with the wake's text and context as JSON."),
                "Choose a signing secret: corntchap signs each body with HMAC-SHA256 in X-Corntchap-Signature, check it on your side.".into(),
                "Enter the URL and the secret here (an admin connects webhooks).".into(),
            ],
            vec![
                field("url", "Endpoint URL", false, true, "https://…", "Where corntchap POSTs each wake."),
                field("secret", "Signing secret", true, false, "", "HMAC-SHA256 key for X-Corntchap-Signature."),
                field("token", "Bearer token", true, false, "", "Sent as Authorization: Bearer, if your endpoint wants one."),
            ],
        ),
    }
}

fn view(state: &AppState, s: &Setup) -> Value {
    let link = format!("{}/connect/{}", state.config.public_url, s.code);
    let status = if s.target.is_some() {
        "connected"
    } else if s.done {
        "removed"
    } else if s.expired {
        "expired"
    } else {
        "waiting"
    };
    let (steps, fields) = guide(s, &state.config.github_mention);
    let next = match status {
        "connected" => format!(
            "{} is connected: {} wakes it from now on. Call connect_cloud with code {} and test=true to send a test wake.",
            s.cook, s.kind, s.code
        ),
        "expired" => "This setup expired: call connect_cloud again for a new one.".into(),
        "removed" => {
            "The wake target this setup made was deleted: call connect_cloud again to connect anew."
                .into()
        }
        _ => format!(
            "Walk the user through the steps. If your client shows the corntchap form, they enter the values there and nothing else is needed. Otherwise give them this link to enter them on corntchap (signed in): {link}. They may also paste the values to you and you call connect_cloud_save with code {}, but the secret then stays in this conversation: say so first. Afterwards call connect_cloud with the code to check.",
            s.code
        ),
    };
    let mut v = json!({
        "code": s.code, "cook": s.cook, "kind": s.kind, "projects": s.projects, "status": status,
        "expires_at": s.expires_at, "link": link, "steps": steps, "fields": fields, "next": next,
        "target": s.target.map(|t| json!({"id": t, "type": s.target_type})),
        "test": s.test,
    });
    if s.kind == "claude-routine" {
        v["routine_prompt"] = json!(cloud::ROUTINE_PROMPT);
    }
    v
}

/// A setup's state; with `test`, send a test wake through its (connected) target first.
pub async fn status(state: &AppState, p: &Principal, code: &str) -> Result<Value> {
    let s = load(&state.db, code).await?;
    allowed(&state.db, p, &s).await?;
    Ok(view(state, &s))
}

pub async fn test(state: &AppState, p: &Principal, code: &str) -> Result<Value> {
    let s = load(&state.db, code).await?;
    allowed(&state.db, p, &s).await?;
    let Some(target) = s.target else {
        return invalid("not connected yet: save the setup first (connect_cloud_save)");
    };
    let config: Value = sqlx::query_scalar("select config from wake_targets where id = $1")
        .bind(target)
        .fetch_one(&state.db)
        .await?;
    let typ = s.target_type.clone().unwrap_or_default();
    let project = s.projects.first().cloned().unwrap_or_else(|| "-".into());
    let text = format!(
        "corntchap connection test for cook {}: this {} now wakes it. Check that the corntchap MCP tools work by calling whoami, say so in one line, and stop. Do not claim tasks.",
        s.cook, s.kind
    );
    let wt = WakeText {
        full: text.clone(),
        pointer: text.clone(),
        meta: json!({"reasons": ["test"], "test": true}),
    };
    let result = match cloud::deliver(state, s.target, &typ, &config, &s.cook, &project, &wt).await
    {
        Ok(d) => json!({"ok": true, "status": d.status, "session_url": d.session_url}),
        Err(f) => json!({"ok": false, "error": f.error}),
    };
    let result = json!({"at": chrono::Utc::now().to_rfc3339(), "result": result});
    sqlx::query("update cloud_setups set test = $2 where id = $1")
        .bind(s.id)
        .bind(&result)
        .execute(&state.db)
        .await?;
    status(state, p, code).await
}

#[derive(Default)]
pub struct Save {
    pub url: Option<String>,
    pub token: Option<String>,
    pub secret: Option<String>,
    pub repo: Option<String>,
    pub issue: Option<i64>,
    pub mention: Option<String>,
    pub takes_work: Option<bool>,
    pub template: Option<String>,
    pub preset: Option<String>,
    pub owner: Option<String>,
    pub organization: Option<String>,
    pub workspace: Option<String>,
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// The one check for a cloud wake target's config, whoever adds it (a setup, `add_wake_target`,
/// the CLI, REST): returns the config to store. A cook's owner may connect a Claude routine
/// (Anthropic's trigger host) or GitHub comments; anything that can point elsewhere (a webhook,
/// another trigger host, a GitHub API URL) is an admin's call.
pub fn cloud_config(kind: &str, c: &Value, is_admin: bool) -> Result<Value> {
    let mut out = kind_config(kind, c, is_admin)?;
    // a cloud cook is on call: woken by name only, unless it should also pick up new work by itself
    if let Some(o) = out.as_object_mut() {
        o.remove("takes_work");
        if c["takes_work"].as_bool() == Some(true) || c["takes_work"].as_str() == Some("true") {
            o.insert("takes_work".into(), json!(true));
        }
    }
    Ok(out)
}

/// Pick up new work by itself: cloud targets that opted in (relays always do).
pub fn takes_work(config: &Value) -> bool {
    config["takes_work"].as_bool() == Some(true)
}

fn kind_config(kind: &str, c: &Value, is_admin: bool) -> Result<Value> {
    let get = |k: &str| c[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    match kind {
        "claude-routine" => {
            let url = get("url").ok_or_else(|| {
                Error::Invalid("url is required: the routine's API trigger URL".into())
            })?;
            let local = url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost");
            let ok = (url.starts_with(ROUTINE_PREFIX)
                || (is_admin && (local || url.starts_with("https://"))))
                && url.contains("/fire");
            if !ok {
                return invalid(format!(
                    "url must be a routine's API trigger URL: {ROUTINE_PREFIX}trig_…/fire"
                ));
            }
            let token = get("token").ok_or_else(|| {
                Error::Invalid("token is required: the trigger's token (sk-ant-oat01-…)".into())
            })?;
            Ok(json!({"url": url, "token": token}))
        }
        "github-comment" => {
            let repo = get("repo")
                .ok_or_else(|| Error::Invalid("repo is required (owner/name)".into()))?;
            let parts: Vec<&str> = repo.split('/').collect();
            if parts.len() != 2
                || parts
                    .iter()
                    .any(|x| x.is_empty() || x.contains(char::is_whitespace))
            {
                return invalid("repo is owner/name");
            }
            let issue = c["issue"]
                .as_i64()
                .or_else(|| get("issue").and_then(|i| i.trim_start_matches('#').parse().ok()))
                .filter(|i| *i > 0)
                .ok_or_else(|| Error::Invalid("issue is the issue or PR number".into()))?;
            let token = get("token").ok_or_else(|| {
                Error::Invalid("token is required: a GitHub token allowed to comment".into())
            })?;
            let mut out = json!({"repo": repo, "issue": issue, "token": token});
            // without one, the server's GITHUB_MENTION applies (and follows it if it changes)
            if let Some(m) = get("mention") {
                out["mention"] = json!(crate::state::mention(m));
            }
            if let Some(api) = get("api_url") {
                if !is_admin {
                    return Err(Error::Forbidden(
                        "only an admin can point GitHub comments at another API (api_url)".into(),
                    ));
                }
                out["api_url"] = json!(api);
            }
            Ok(out)
        }
        "coder" => {
            // the URL is any host the admin names (a self-hosted deployment): not a cook owner's call
            if !is_admin {
                return Err(Error::Forbidden(
                    "only an admin can connect Coder: its URL can point anywhere".into(),
                ));
            }
            let url = get("url")
                .ok_or_else(|| Error::Invalid("url is required: the Coder deployment".into()))?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return invalid("url must be http(s)");
            }
            let token = get("token")
                .ok_or_else(|| Error::Invalid("token is required: a Coder session token".into()))?;
            let mut out = json!({"url": url.trim_end_matches('/'), "token": token});
            match (get("organization"), get("template")) {
                (Some(_), Some(_)) => {
                    return invalid(
                        "give an organization (Coder Agents) or a template (Coder Tasks), not both",
                    );
                }
                (Some(org), None) => {
                    out["api"] = json!("chats");
                    out["organization"] = json!(org);
                    if let Some(w) = get("workspace") {
                        out["workspace"] = json!(w);
                    }
                }
                (None, Some(t)) => {
                    out["api"] = json!("tasks");
                    out["template"] = json!(t);
                    if let Some(p) = get("preset") {
                        out["preset"] = json!(p);
                    }
                    if let Some(o) = get("owner") {
                        if o.contains(['/', ' ', '?', '#']) {
                            return invalid("owner is a Coder username, or me");
                        }
                        out["owner"] = json!(o);
                    }
                }
                (None, None) => {
                    return invalid(
                        "organization is required (Coder Agents, current Coder), or template (Coder Tasks, older Coder)",
                    );
                }
            }
            Ok(out)
        }
        "webhook" => {
            if !is_admin {
                return Err(Error::Forbidden(
                    "only an admin can connect a webhook: Claude routines and GitHub comments are open to a cook's owner".into(),
                ));
            }
            let url = get("url").ok_or_else(|| Error::Invalid("url is required".into()))?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return invalid("url must be http(s)");
            }
            // an admin's webhook may also carry headers or a fixed body
            let mut out = c.clone();
            out["url"] = json!(url);
            if let Some(o) = out.as_object_mut() {
                o.retain(|_, v| !v.is_null() && v.as_str() != Some(""));
            }
            Ok(out)
        }
        _ => invalid("type must be relay, webhook, claude-routine, github-comment or coder"),
    }
}

/// Finish a setup: make its wake target. The answer never contains the secret.
pub async fn save(state: &AppState, p: &Principal, code: &str, a: Save) -> Result<Value> {
    let db = &state.db;
    let s = load(db, code).await?;
    let is_admin = allowed(db, p, &s).await?;
    if s.done {
        return invalid(
            "this setup is already connected: open a new one (connect_cloud) to change it",
        );
    }
    if s.expired {
        return invalid("this setup expired: open a new one (connect_cloud)");
    }
    let mut raw = json!({});
    for (k, v) in [
        ("url", &a.url),
        ("token", &a.token),
        ("secret", &a.secret),
        ("repo", &a.repo),
        ("mention", &a.mention),
        ("template", &a.template),
        ("preset", &a.preset),
        ("owner", &a.owner),
        ("organization", &a.organization),
        ("workspace", &a.workspace),
    ] {
        if let Some(x) = opt(v) {
            raw[k] = json!(x);
        }
    }
    if let Some(i) = a.issue {
        raw["issue"] = json!(i);
    }
    if a.takes_work == Some(true) {
        raw["takes_work"] = json!(true);
    }
    let config = cloud_config(&s.kind, &raw, is_admin)?;
    let mut tx = db.begin().await?;
    // the setup is finished once: lock it
    let done: Option<Option<Uuid>> =
        sqlx::query_scalar("select target_id from cloud_setups where id = $1 for update")
            .bind(s.id)
            .fetch_optional(&mut *tx)
            .await?;
    if !matches!(done, Some(None)) {
        return invalid("this setup is already connected");
    }
    let tid: Uuid = sqlx::query_scalar(
        "insert into wake_targets (agent_id, type, projects, labels, config) values ($1, $2, $3, '{}', $4) returning id",
    )
    .bind(s.agent_id)
    .bind(&s.kind)
    .bind(&s.projects)
    .bind(&config)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "update cloud_setups set target_id = $2, done_at = now(), done_by = $3 where id = $1",
    )
    .bind(s.id)
    .bind(tid)
    .bind(p.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    status(state, p, code).await
}

/// Whether a client renders MCP Apps, from its capabilities (`initialize`, or a 2026-07-28
/// request's `_meta`): it lists `io.modelcontextprotocol/ui` with our MIME type.
pub fn renders_ui(capabilities: &Value) -> bool {
    capabilities["extensions"]["io.modelcontextprotocol/ui"]["mimeTypes"]
        .as_array()
        .is_some_and(|m| {
            m.iter()
                .any(|t| t.as_str() == Some("text/html;profile=mcp-app"))
        })
}

/// Tell the agent exactly what to do with a waiting setup, now that we know whether its client
/// shows the form. Rewrites a `connect_cloud`/`connect_cloud_save` tool result in place.
pub fn guide_client(result: &mut Value, renders: bool) {
    let Some(v) = result.get_mut("structuredContent") else {
        return;
    };
    if v["status"] != "waiting" {
        return;
    }
    let code = v["code"].as_str().unwrap_or_default().to_string();
    let link = v["link"].as_str().unwrap_or_default().to_string();
    v["form_shown"] = json!(renders);
    v["next"] = json!(if renders {
        format!(
            "The corntchap form is shown to the user next to this tool call. Tell them to follow its steps and enter the values there; do not ask them for the secret and do not tell them to do it elsewhere. If they cannot see the form, give them this link instead: {link}. Then call connect_cloud with code {code} to check."
        )
    } else {
        format!(
            "This client cannot show the corntchap form. Walk the user through the steps (the parts on claude.ai or GitHub only they can do), then give them this link, where they enter the values on corntchap, signed in: {link}. Only if they prefer to paste the values to you, call connect_cloud_save with code {code}, after warning them that the secret then stays in this conversation. Then call connect_cloud with code {code} to check, and offer a test wake (test=true)."
        )
    });
    let text = v.to_string();
    result["content"] = json!([{"type": "text", "text": text}]);
}
