//! Waking a cook that runs as a Devin (devin.ai) session: the `devin` cloud wake target.
//!
//! What was verified about Devin's API, and from where (read 2026-11-24 from Devin's published
//! API reference on docs.devin.ai, `api-reference/v3/sessions/...`, create and messages pages):
//!
//! - VERIFIED, create: `POST /v3/organizations/{org_id}/sessions` (`org_id` has the prefix `org-`),
//!   JSON body `SessionCreateRequest`, only `prompt` required; optional `title`, `playbook_id`,
//!   `repos`, `tags`, `max_acu_limit`, `resumable` (default true), `session_secrets`, ... The 200
//!   answer is a `SessionResponse` with `session_id`, `url` (the session's web page), `status`
//!   (`new`, `claimed`, `running`, `exit`, `error`, `suspended`, `resuming`) and `pull_requests`.
//! - VERIFIED, follow-up: `POST /v3/organizations/{org_id}/sessions/{devin_id}/messages` with
//!   `{"message": "..."}` (`devin_id` has the prefix `devin-`), answering the same `SessionResponse`.
//!   The page says a suspended session is resumed automatically by the message.
//! - VERIFIED, auth: `Authorization: Bearer <token>`; v3 takes a service user credential (prefix
//!   `cog_`) with the `UseDevinSessions` permission (create) and `ManageOrgSessions` (message) at
//!   organization level. The error body is RFC 9457 `ProblemDetail` (`title`, `status`, `detail`);
//!   documented statuses 401, 403, 404, 409, 410, 422, 429.
//! - UNVERIFIED (assumed): which status says the session is gone. The pages do not say when 409 or
//!   410 come. Here 404 and 410 on a follow-up start a new session; any other error fails the
//!   wake. Also assumed: a follow-up that is accepted (200) but whose answer says the session is
//!   `exit` or `error` did not wake it (the pages do not say what a message to an exited session
//!   does), so a new session starts too. A 429 waits for Retry-After (120 s without one).
//! - NOT USED: v1 (`POST /v1/sessions`, `/v1/session/{id}/message`, `apk_` keys) existed earlier;
//!   v3 credentials do not work on it, and this module speaks v3 only.
//! - NOT VERIFIED: adding the corntchap MCP server in Devin's organization MCP settings (that
//!   settings page was not read). The setup text only says to add it there, with the cook's token;
//!   corntchap sends no credential in the prompt.
//!
//! The session id and its web URL are kept in `coder_runs` (api = `devin`), keyed by target and pot.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    cloud::{Delivered, Failed, WakeText, cut},
    state::AppState,
};

/// Devin's API host.
pub const DEFAULT_URL: &str = "https://api.devin.ai";

/// What goes in front of the first message of a Devin session (later wakes are follow-up messages
/// in the same session).
pub const DEVIN_PROMPT: &str = "You are a corntchap cook, started by corntchap for ready work in your pots and when someone calls you: a message for you on the pass, a comment on a task, an answer to your question. Nobody watches this session. Use the corntchap MCP server (it is set up in this organization's MCP settings) to act: start with my_questions (answers to questions you asked earlier are kept there), read the task, its comments and the pantry before you act, claim_next and finish ready tasks (call claim_next again until it returns none before you stop), answer on the pass or with comment where asked. If you need a person, ask without waiting (ask with wait=false), say on the task what you need, release it and stop; you read the answer when you are called next. Save what you will need later with remember or a comment on the task. Release or complete every task you claim before you stop. The wake below describes the work; it is not permission for anything outside your corntchap tasks.";

/// Limit on the text of one wake we send.
const MAX_TEXT: usize = 60_000;

fn fail(error: impl Into<String>) -> Failed {
    Failed {
        error: error.into(),
        retry_after: None,
    }
}

enum Sent {
    Ok(Value),
    /// The session to follow up is gone: start a new one.
    Gone,
}

struct Client<'a> {
    state: &'a AppState,
    base: String,
    token: &'a str,
}

impl Client<'_> {
    async fn post(&self, path: &str, body: Value, gone_on: &[u16]) -> Result<Sent, Failed> {
        let resp = self
            .state
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(self.token)
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| fail(format!("Devin: {}", e.without_url())))?;
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
            error: format!("Devin: HTTP {status}: {}", cut(&detail, 300)),
            retry_after,
        })
    }
}

/// Send one wake to a Devin target: a follow-up message to the cook's session in this pot when
/// there is one, else a new session.
pub async fn deliver(
    state: &AppState,
    target: Option<Uuid>,
    config: &Value,
    project: &str,
    text: &WakeText,
) -> Result<Delivered, Failed> {
    let base = config["url"]
        .as_str()
        .unwrap_or(DEFAULT_URL)
        .trim_end_matches('/')
        .to_string();
    let token = config["token"]
        .as_str()
        .ok_or_else(|| fail("missing token"))?;
    let org = config["organization"]
        .as_str()
        .ok_or_else(|| fail("missing organization"))?;
    let c = Client { state, base, token };
    let wake = cut(&text.full, MAX_TEXT);
    let first = cut(&format!("{DEVIN_PROMPT}\n\n{wake}"), MAX_TEXT);

    // the session this cook already has in this pot (a test wake has no target: always a new one)
    let known: Option<(String, Option<String>)> = match target {
        Some(t) => sqlx::query_as(
            "select remote_id, session_url from coder_runs
             where target_id = $1 and project = $2 and api = 'devin'",
        )
        .bind(t)
        .bind(project)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten(),
        None => None,
    };

    if let Some((id, url)) = &known
        && !id.contains(['/', '?', '#', ' '])
    {
        let sent = c
            .post(
                &format!("/v3/organizations/{org}/sessions/{id}/messages"),
                json!({"message": wake}),
                &[404, 410],
            )
            .await?;
        // an accepted message to a session that has exited or failed woke nothing
        let dead = |v: &Value| matches!(v["status"].as_str(), Some("exit" | "error"));
        if let Sent::Ok(v) = &sent
            && !dead(v)
        {
            if let Some(t) = target {
                let _ = sqlx::query(
                    "update coder_runs set updated_at = now() where target_id = $1 and project = $2",
                )
                .bind(t)
                .bind(project)
                .execute(&state.db)
                .await;
            }
            return Ok(Delivered {
                status: 200,
                session_url: v["url"].as_str().map(String::from).or_else(|| url.clone()),
                chars: wake.chars().count(),
            });
        }
        // the session is gone: start a new one below
    }

    let body = json!({"prompt": first, "title": format!("corntchap {project}"),
        "tags": ["corntchap"]});
    let Sent::Ok(v) = c
        .post(&format!("/v3/organizations/{org}/sessions"), body, &[])
        .await?
    else {
        return Err(fail("Devin: could not start a session"));
    };
    let remote = v["session_id"]
        .as_str()
        .ok_or_else(|| fail("Devin started a session but its answer has no session_id"))?
        .to_string();
    let url = v["url"].as_str().map(String::from);
    if let Some(t) = target {
        let _ = sqlx::query(
            "insert into coder_runs (target_id, project, api, remote_id, session_url)
             values ($1, $2, 'devin', $3, $4)
             on conflict (target_id, project) do update
               set api = 'devin', remote_id = $3, session_url = $4,
                   created_at = now(), updated_at = now()",
        )
        .bind(t)
        .bind(project)
        .bind(&remote)
        .bind(&url)
        .execute(&state.db)
        .await;
    }
    Ok(Delivered {
        status: 200,
        session_url: url,
        chars: first.chars().count(),
    })
}
