//! Waking a cook that runs as a Coder (coder.com) agent: the `coder` cloud wake target.
//!
//! What was verified about Coder's API, and from where (read 2026-11-24 through Context7 from
//! `coder/coder` main: `docs/coder-agents/tasks-to-chats-migration.md`,
//! `docs/reference/api/templates.md`; github.com/coder/coder itself was not readable from here,
//! so no Go source such as `codersdk/aitasks.go`):
//!
//! - VERIFIED: Coder Tasks was deprecated in v2.34 and REMOVED, including the Tasks API
//!   (`/api/v2/tasks`); docs of releases before v2.36 still describe it. Its replacement is the
//!   Chats API (Coder Agents). So a current deployment needs `api = "chats"`, an older one
//!   `api = "tasks"`.
//! - VERIFIED, auth: header `Coder-Session-Token: <token>` on every call.
//! - VERIFIED, chats: `POST /api/v2/chats` with `{"organization_id", "content": [{"type": "text",
//!   "text"}], "workspace_id"?}` returns a Chat with `id` and `status` ("running"). The agent loop
//!   runs in Coder's control plane and picks a template itself unless `workspace_id` pins a
//!   workspace. Follow-up: `POST /api/v2/chats/{chat}/messages` with `{"content": [...],
//!   "busy_behavior"?: "steer" | "queue"}` returns `{"queued": bool}`.
//! - VERIFIED, tasks (older Coder): `POST /api/v2/tasks/{user}` with `{"template_version_id",
//!   "input"}` (`me` is the session's user; the migration guide also says
//!   `template_version_preset_id` was a field of it); follow-up
//!   `POST /api/v2/tasks/{user}/{task}/send` with `{"input"}` (the guide's example uses the task's
//!   name as `{task}`).
//! - VERIFIED, templates: `GET /api/v2/templates` is an array of templates with `id`, `name` and
//!   `active_version_id` (used to turn a template name into a version id).
//! - UNVERIFIED (assumed): the task create response carries `id` (and `name`), and `{task}`
//!   accepts the id; the status code Coder answers a follow-up to a finished or deleted run
//!   with. A 404 (tasks: also 409) on a follow-up starts a new run, any other error fails the
//!   wake. The chat's status is never read: a chat is multi-turn, so a follow-up to a finished
//!   chat is how it resumes (the migration guide: "resume" is a new message).
//! - The corntchap MCP server must be reachable from the agent: for chats it is a Coder-side MCP
//!   server configuration, for tasks the template's agent module. corntchap sends no credential.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    cloud::{Delivered, Failed, WakeText, cut},
    state::AppState,
    stations::RunStation,
};

/// What goes in front of the first message of a Coder run (later wakes are follow-ups in the same
/// conversation). The run is not a Claude routine, so there is no saved prompt to put it in.
pub const CODER_PROMPT: &str = concat!(
    crate::cloud::cloud_contract!(),
    " Later wakes for this pot arrive as follow-up messages in this run: each says what to do next."
);

/// Limit on the text of one wake we send.
const MAX_TEXT: usize = 60_000;

fn fail(error: impl Into<String>) -> Failed {
    Failed {
        error: error.into(),
        retry_after: None,
    }
}

/// `api` of a coder config: `chats` or `tasks`.
pub fn api_of(config: &Value) -> &str {
    match config["api"].as_str() {
        Some("tasks") => "tasks",
        _ => "chats",
    }
}

enum Sent {
    Ok(Value),
    /// The run to follow up is gone or cannot take input: start a new one.
    Gone,
}

struct Client<'a> {
    state: &'a AppState,
    base: String,
    token: &'a str,
}

impl Client<'_> {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        gone_on: &[u16],
    ) -> Result<Sent, Failed> {
        let mut r = self
            .state
            .http
            .request(method, format!("{}{path}", self.base))
            .header("coder-session-token", self.token)
            .header("accept", "application/json");
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.map_err(|e| fail(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(Sent::Ok(resp.json().await.unwrap_or(Value::Null)));
        }
        if gone_on.contains(&status.as_u16()) {
            return Ok(Sent::Gone);
        }
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .or((status.as_u16() == 429).then(|| std::time::Duration::from_secs(120)));
        let detail = resp.text().await.unwrap_or_default();
        Err(Failed {
            error: format!("Coder: HTTP {status}: {}", cut(&detail, 300)),
            retry_after,
        })
    }
}

/// Send one wake to a Coder target: a follow-up to the cook's run in this pot when there is one,
/// else a new run.
pub async fn deliver(
    state: &AppState,
    target: Option<Uuid>,
    config: &Value,
    project: &str,
    text: &WakeText,
    station: Option<&RunStation>,
) -> Result<Delivered, Failed> {
    let base = config["url"]
        .as_str()
        .ok_or_else(|| fail("missing url"))?
        .trim_end_matches('/')
        .to_string();
    let token = config["token"]
        .as_str()
        .ok_or_else(|| fail("missing token"))?;
    let api = api_of(config);
    let c = Client { state, base, token };
    let mut full = text.full.clone();
    let wake = cut(&full, MAX_TEXT);

    // the run this cook already has in this pot (a test wake has no target: always a new run)
    let known: Option<String> = match target {
        Some(t) => sqlx::query_scalar(
            "select remote_id from coder_runs where target_id = $1 and project = $2 and api = $3",
        )
        .bind(t)
        .bind(project)
        .bind(api)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten(),
        None => None,
    };

    let owner = config["owner"].as_str().unwrap_or("me");
    if let Some(id) = &known {
        let sent = if api == "chats" {
            c.call(
                reqwest::Method::POST,
                &format!("/api/v2/chats/{id}/messages"),
                Some(
                    json!({"content": [{"type": "text", "text": wake}], "busy_behavior": "queue"}),
                ),
                &[404],
            )
            .await?
        } else {
            c.call(
                reqwest::Method::POST,
                &format!("/api/v2/tasks/{owner}/{id}/send"),
                Some(json!({"input": wake})),
                &[404, 409],
            )
            .await?
        };
        if let Sent::Ok(_) = sent {
            if let Some(t) = target {
                let _ = sqlx::query(
                    "update coder_runs set updated_at = now(), station_id = coalesce(station_id, $3)
                     where target_id = $1 and project = $2",
                )
                .bind(t)
                .bind(project)
                .bind(station.map(|s| s.id))
                .execute(&state.db)
                .await;
            }
            return Ok(Delivered {
                status: 200,
                session_url: None,
                session_id: Some(id.clone()),
                station: None,
                chars: wake.chars().count(),
            });
        }
        // the run is gone: start a new one below, with a station of its own
    }
    let mut run_station = station.map(|s| s.id);
    let mut fresh: Option<RunStation> = None;
    // only the station of a run that went on is replaced: one opened for this wake is the new run's
    if known.is_some()
        && let Some(old) = station
        && old.reused
        && let Ok(mut conn) = state.db.acquire().await
        && let Ok(new) =
            crate::stations::new_run(&mut conn, old.agent, old.project, old.target, &old.typ).await
    {
        full = full.replace(&old.reference, &new.reference);
        run_station = Some(new.id);
        fresh = Some(new);
    }
    let first = cut(
        &format!("{CODER_PROMPT}\n\n{}", cut(&full, MAX_TEXT)),
        MAX_TEXT,
    );

    let created = match create_run(&c, config, api, owner, &first).await {
        Ok(v) => v,
        Err(f) => {
            // told to wait: nothing was sent and its new station is not needed; any other failure
            // may have started the run, whose station then ends instead
            if let (Some(n), Ok(mut conn)) = (&fresh, state.db.acquire().await) {
                if f.retry_after.is_some() {
                    let _ = crate::stations::drop_run(&mut conn, n.id).await;
                } else {
                    let _ = crate::stations::end_if_idle(&mut conn, n.id).await;
                }
            }
            return Err(f);
        }
    };
    let Sent::Ok(v) = created else {
        // the service refused to start a run: nothing runs for the new station
        if let (Some(n), Ok(mut conn)) = (&fresh, state.db.acquire().await) {
            let _ = crate::stations::drop_run(&mut conn, n.id).await;
        }
        return Err(fail("Coder: could not start a run"));
    };
    let Some(remote) = v["id"]
        .as_str()
        .or_else(|| v["name"].as_str())
        .map(String::from)
    else {
        // the run started: its station stays, ended
        if let (Some(n), Ok(mut conn)) = (&fresh, state.db.acquire().await) {
            let _ = crate::stations::end_if_idle(&mut conn, n.id).await;
        }
        return Err(fail("Coder started a run but its answer has no id"));
    };
    if let Some(t) = target {
        let _ = sqlx::query(
            "insert into coder_runs (target_id, project, api, remote_id, station_id) values ($1, $2, $3, $4, $5)
             on conflict (target_id, project) do update
               set api = $3, remote_id = $4, station_id = $5, created_at = now(), updated_at = now()",
        )
        .bind(t)
        .bind(project)
        .bind(api)
        .bind(&remote)
        .bind(run_station)
        .execute(&state.db)
        .await;
    }
    Ok(Delivered {
        status: 200,
        session_url: None,
        session_id: Some(remote),
        station: fresh.map(|n| n.id),
        chars: first.chars().count(),
    })
}

/// Start a new Coder run with the first message.
async fn create_run(
    c: &Client<'_>,
    config: &Value,
    api: &str,
    owner: &str,
    first: &str,
) -> Result<Sent, Failed> {
    Ok(if api == "chats" {
        let org = config["organization"]
            .as_str()
            .ok_or_else(|| fail("missing organization"))?;
        let mut body = json!({"organization_id": org,
            "content": [{"type": "text", "text": first}]});
        if let Some(w) = config["workspace"].as_str() {
            body["workspace_id"] = json!(w);
        }
        c.call(reqwest::Method::POST, "/api/v2/chats", Some(body), &[])
            .await?
    } else {
        let template = config["template"]
            .as_str()
            .ok_or_else(|| fail("missing template"))?;
        let version = if Uuid::parse_str(template).is_ok() {
            template.to_string()
        } else {
            resolve_template(c, template).await?
        };
        let mut body = json!({"template_version_id": version, "input": first});
        if let Some(p) = config["preset"].as_str() {
            body["template_version_preset_id"] = json!(p);
        }
        c.call(
            reqwest::Method::POST,
            &format!("/api/v2/tasks/{owner}"),
            Some(body),
            &[],
        )
        .await?
    })
}

/// A template name to its active version id (`GET /api/v2/templates`).
async fn resolve_template(c: &Client<'_>, name: &str) -> Result<String, Failed> {
    let Sent::Ok(v) = c
        .call(reqwest::Method::GET, "/api/v2/templates", None, &[])
        .await?
    else {
        return Err(fail("Coder: could not list templates"));
    };
    v.as_array()
        .into_iter()
        .flatten()
        .find(|t| t["name"].as_str() == Some(name))
        .and_then(|t| t["active_version_id"].as_str())
        .map(String::from)
        .ok_or_else(|| fail(format!("Coder: no template named {name}")))
}
