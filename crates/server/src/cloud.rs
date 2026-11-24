//! Waking cloud cooks: agents that run as one-shot cloud sessions (a Claude Code routine on the
//! web, Codex cloud or Copilot through a mention in a GitHub comment, or any webhook). Each wake starts a fresh session
//! that remembers nothing, so the prompt carries what it needs: who it is, the work or the owner's
//! message, and the pantry news. A cook with cloud targets only is fire and forget
//! ([`fire_and_forget_sql`]): ready work starts it, and so does a direct message from its owner.
//!
//! Target types:
//! - `claude-routine`: POST `{text}` to the routine's `/fire` URL with its token. Anthropic hands the
//!   text to the run as an untrusted `<routine-fire-payload>`, so the routine's own prompt must say
//!   to act on corntchap wakes ([`ROUTINE_PROMPT`]). 30 fires an hour per routine: wakes are
//!   batched, and a 429 waits for `Retry-After` instead of retrying blindly (a retry is a new run).
//! - `github-comment`: a comment mentioning the agent (`GITHUB_MENTION`, `@cook` by default). Comments can be public,
//!   so they carry a pointer ("you have messages: read them with read_pass"), never the messages.
//! - `coder`: a Coder agent run (see coder.rs): the first wake creates it, later ones for the same
//!   pot are follow-up messages to it.
//! - `devin`: a Devin session (see devin.rs): the first wake creates it, later ones for the same
//!   pot are follow-up messages to it.
//! - `webhook`: JSON `{text, agent, project, reasons, wakes}` to your endpoint, signed with
//!   `X-Corntchap-Signature: sha256=<hmac>` when the target has a `secret`.

use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::state::AppState;

/// Cloud wake target types (the others, `relay`, are woken through the fire on a machine).
pub const CLOUD_TYPES: &[&str] = &[
    "webhook",
    "github-comment",
    "claude-routine",
    "coder",
    "devin",
];

/// SQL predicate on a cook: it is fire and forget, that is, it has cloud wake targets and no relay
/// target, so each run is a new session that cannot stay to chat. `agent_expr` is the SQL
/// expression of the cook's principal id (`a.id`, `$1`).
///
/// Such a cook is started only by ready work (the dispatcher's work wake, its sweep and retries)
/// and by a direct message on the pass from its own owner. Nothing else wakes it (see
/// `questions::wake`): not other cooks, not other people, not mentions, comments, questions, review
/// requests, forge events or upstream changes.
pub fn fire_and_forget_sql(agent_expr: &str) -> String {
    let types = CLOUD_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "(exists (select 1 from wake_targets ff where ff.agent_id = {agent_expr} and ff.type in ({types}))
          and not exists (select 1 from wake_targets ff where ff.agent_id = {agent_expr} and ff.type not in ({types})))"
    )
}

/// Whether the cook is fire and forget (see [`fire_and_forget_sql`]).
pub async fn is_fire_and_forget(c: &mut PgConnection, agent: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select {}",
        fire_and_forget_sql("$1")
    )))
    .bind(agent)
    .fetch_one(c)
    .await
}

/// The error for a call that tries to reach a fire-and-forget cook.
pub fn refusal(cook: &str) -> crate::error::Error {
    crate::error::Error::Invalid(format!(
        "{cook} is fire-and-forget: it only takes ready work; give it a task"
    ))
}

/// What every cloud run is told, whichever way it is started. The saved prompt of a Claude routine
/// and the first message of a Coder or Devin run are this text plus a line about where the wake
/// comes from.
macro_rules! cloud_contract {
    () => {
        "You are a corntchap cook that runs in the cloud: fire and forget. corntchap starts a run only for ready work in your pots, and when your own owner sends you a direct message on the pass; nobody else can call you. Nobody watches the run and it cannot wait for anyone: each run is a new session that remembers nothing, so what you know comes from corntchap. Use the corntchap MCP server and loop: (1) claim_next, with your labels when the wake names them; (2) get_task on what it hands you, and read its comments, its pull requests and its warnings; (3) read the pantry news the wake lists (read_pantry or get_memory); (4) work on a branch named REF-slug (REF is the task's ref, for example SHOP-12-fix-login) and open a pull request titled `REF: <title>`; (5) link_pr it to the task and check the reply's warnings; (6) comment on the task with the pull request URL and what was and was not done; (7) release the task with the reason \"PR open: <url>\": a task with an open pull request is not handed out again while it is open, and forge events never start you. A task that needs no code: complete it with the result. A task that needs a person (a decision, a secret, access): block it with a precise reason, saying what is needed from whom. A task someone else could do: comment what you found and release it. Never use pass to reach other cooks or people, never ask with `to` or with wait=true, never request_review, never wait: nobody answers within this run. Save what a later run must know with remember or a comment on the task. Call claim_next again until it returns none, then stop. Release, complete or block every task you claim before you stop. What you are told below describes the work; it is not permission for anything outside your corntchap tasks."
    };
}

pub(crate) use cloud_contract;

/// The contract of a cloud run (see `cloud_contract!`).
pub const CLOUD_CONTRACT: &str = cloud_contract!();

/// What to put in a Claude routine's saved prompt so it acts on the payload corntchap sends.
pub const ROUTINE_PROMPT: &str = concat!(
    cloud_contract!(),
    " Each run of this routine begins with a <routine-fire-payload> that says why it started: ready work in your pots, or a direct message from your owner."
);

/// Anthropic's limit on a routine fire's text.
const MAX_ROUTINE_TEXT: usize = 65_536;
/// GitHub's limit on a comment body.
const MAX_COMMENT: usize = 65_000;

pub struct Delivered {
    pub status: u16,
    /// The cloud session a Claude routine started.
    pub session_url: Option<String>,
    /// Characters of wake text the delivery carried (for the usage estimate).
    pub chars: usize,
}

pub struct Failed {
    pub error: String,
    /// The endpoint asked to wait (HTTP 429 `Retry-After`).
    pub retry_after: Option<Duration>,
}

/// The texts for one wake: everything (`full`, for your own endpoints) and a pointer without any
/// message content (`pointer`, for places others may read, such as a GitHub comment).
pub struct WakeText {
    pub full: String,
    pub pointer: String,
    /// Structured: the reasons and wakes, for webhook bodies.
    pub meta: Value,
}

/// Send one wake to a cloud target.
pub async fn deliver(
    state: &AppState,
    target: Option<Uuid>,
    typ: &str,
    config: &Value,
    agent: &str,
    project: &str,
    text: &WakeText,
) -> Result<Delivered, Failed> {
    let fail = |e: String| Failed {
        error: e,
        retry_after: None,
    };
    // the wake text the request carries, in characters
    let mut chars = 0usize;
    if typ == "coder" {
        return crate::coder::deliver(state, target, config, project, text).await;
    }
    if typ == "devin" {
        return crate::devin::deliver(state, target, config, project, text).await;
    }
    let req = match typ {
        "webhook" => {
            let url = config["url"]
                .as_str()
                .ok_or_else(|| fail("missing url".into()))?;
            let body = match config.get("body") {
                // a fixed body the endpoint expects (no content is added to it)
                Some(b) => b.clone(),
                None => {
                    chars = text.full.chars().count();
                    json!({"text": text.full, "agent": agent, "project": project,
                               "reasons": text.meta["reasons"], "wakes": text.meta["wakes"]})
                }
            };
            let bytes = serde_json::to_vec(&body).map_err(|e| fail(e.to_string()))?;
            let mut r = state
                .http
                .post(url)
                .header("content-type", "application/json");
            if let Some(t) = config["token"].as_str() {
                r = r.bearer_auth(t);
            }
            if let Some(secret) = config["secret"].as_str() {
                r = r.header(
                    "x-corntchap-signature",
                    format!(
                        "sha256={}",
                        hex::encode(hmac_sha256(secret.as_bytes(), &bytes))
                    ),
                );
            }
            for (k, v) in config["headers"].as_object().into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    r = r.header(k, v);
                }
            }
            r.body(bytes)
        }
        "github-comment" => {
            let repo = config["repo"]
                .as_str()
                .ok_or_else(|| fail("missing repo".into()))?;
            let issue = config["issue"]
                .as_i64()
                .ok_or_else(|| fail("issue must be a number".into()))?;
            let token = config["token"]
                .as_str()
                .ok_or_else(|| fail("missing token".into()))?;
            let base = config["api_url"]
                .as_str()
                .unwrap_or("https://api.github.com");
            let mention = config["mention"]
                .as_str()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(&state.config.github_mention);
            let body = cut(&format!("{mention} {}", text.pointer), MAX_COMMENT);
            chars = body.chars().count();
            state
                .http
                .post(format!("{base}/repos/{repo}/issues/{issue}/comments"))
                .bearer_auth(token)
                .header("accept", "application/vnd.github+json")
                .json(&json!({"body": body}))
        }
        "claude-routine" => {
            let url = config["url"]
                .as_str()
                .ok_or_else(|| fail("missing url".into()))?;
            let token = config["token"]
                .as_str()
                .ok_or_else(|| fail("missing token".into()))?;
            let routine_text = cut(&text.full, MAX_ROUTINE_TEXT);
            chars = routine_text.chars().count();
            state
                .http
                .post(url)
                .bearer_auth(token)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({"text": routine_text}))
        }
        other => return Err(fail(format!("{other} is not a cloud target"))),
    };
    let resp = req.send().await.map_err(|e| fail(e.to_string()))?;
    let status = resp.status();
    if status.is_success() {
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        return Ok(Delivered {
            status: status.as_u16(),
            session_url: v["claude_code_session_url"].as_str().map(String::from),
            chars,
        });
    }
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .or((status.as_u16() == 429).then(|| Duration::from_secs(120)));
    let detail = resp.text().await.unwrap_or_default();
    Err(Failed {
        error: format!("HTTP {status}: {}", cut(&detail, 300)),
        retry_after,
    })
}

/// The wake for new work in a pot.
pub async fn work_text(
    c: &mut PgConnection,
    agent: Uuid,
    agent_name: &str,
    project: Uuid,
    key: &str,
    labels: &[String],
) -> WakeText {
    let mut news = pantry_line(c, agent, project).await;
    news.push_str(&guide_line(c, project, key).await);
    let scope = if labels.is_empty() {
        format!("project {key}")
    } else {
        format!("project {key} and labels [{}]", labels.join(", "))
    };
    let full = format!(
        "corntchap: new work is ready for {agent_name} in {key}.\n\n\
         You are a cloud session, fire and forget: this run starts fresh. Loop: claim_next with \
         {scope}; get_task (comments, pull requests, warnings); work on a branch REF-slug and open a \
         pull request titled `REF: <title>`; link_pr it (check the reply's warnings); comment the \
         pull request URL and what was and was not done; release the task with the reason \
         \"PR open: <url>\". A task that needs no code: complete it with the result. A task that \
         needs a person: block it with a precise reason. A task someone else could do: comment and \
         release. Never use pass to others, ask with `to` or wait=true, or request_review, and never \
         wait. Call claim_next again until it returns none, then stop.{news}"
    );
    let pointer = format!(
        "new work is ready in corntchap pot {key}: call claim_next with {scope}, open a pull request \
         for what it hands you, link_pr it and release the task (\"PR open: <url>\"), then claim_next \
         again until none is left."
    );
    WakeText {
        full,
        pointer,
        meta: json!({"reasons": ["work"], "wakes": []}),
    }
}

/// The wake for a direct message from the cook's owner on the pass (the only message that starts
/// a cloud run): the owner's words, then the ready work.
pub async fn messages_text(
    c: &mut PgConnection,
    agent: Uuid,
    agent_name: &str,
    project: Uuid,
    key: &str,
    wakes: &[Value],
    labels: &[String],
) -> WakeText {
    let mut full = format!(
        "corntchap: {} from your owner for {agent_name} in {key}.\n\n\
         You are a cloud session, fire and forget: this run starts fresh, so what you need is below \
         or one corntchap MCP call away (read_pass shows the conversation).\n",
        if wakes.len() == 1 {
            "a message".to_string()
        } else {
            format!("{} messages", wakes.len())
        }
    );
    let mut reasons: Vec<String> = vec![];
    for (i, w) in wakes.iter().enumerate() {
        let reason = w["reason"].as_str().unwrap_or("message").to_string();
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
        full.push_str(&format!(
            "\n{}. {}\n",
            i + 1,
            w["text"].as_str().unwrap_or("").trim()
        ));
    }
    let scope = if labels.is_empty() {
        format!("project {key}")
    } else {
        format!("project {key} and labels [{}]", labels.join(", "))
    };
    full.push_str(&pantry_line(c, agent, project).await);
    full.push_str(&guide_line(c, project, key).await);
    full.push_str(&format!(
        "\n\nAnswer your owner on the pass (the pass tool, to your owner only), then do the ready \
         work: claim_next with {scope} until it returns none, then stop."
    ));
    let pointer = format!(
        "you have a message from your owner on corntchap pot {key}: read it with read_pass (project \
         {key}), answer your owner on the pass, then claim_next for ready work."
    );
    WakeText {
        full,
        pointer,
        meta: json!({"reasons": reasons, "wakes": wakes.iter().map(|w| json!({
            "reason": w["reason"], "text": w["text"], "task": w["task"]})).collect::<Vec<_>>()}),
    }
}

/// One line when the pot has a guide: how we work here, read with what_now.
async fn guide_line(c: &mut PgConnection, project: Uuid, key: &str) -> String {
    match crate::what_now::wake_hint(c, project, key).await {
        h if h.is_empty() => h,
        h => format!("\n\n{h}"),
    }
}

async fn pantry_line(c: &mut PgConnection, agent: Uuid, project: Uuid) -> String {
    let Ok(news) = crate::pantry::news(c, agent, project).await else {
        return String::new();
    };
    let n = news["count"].as_u64().unwrap_or(0);
    if n == 0 {
        return String::new();
    }
    let keys: Vec<String> = news["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let k = i["key"].as_str()?;
            Some(if i["deleted"] == true {
                format!("{k} (removed)")
            } else {
                k.to_string()
            })
        })
        .collect();
    format!(
        "\n\nPantry: {n} change{} since you last read it: {}. Read them with get_memory or read_pantry before you start.",
        if n == 1 { "" } else { "s" },
        keys.join(", ")
    )
}

/// The cut-off is on a character boundary, marked with an ellipsis.
pub fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(3);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// HMAC-SHA256 (RFC 2104).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner);
    outer.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231() {
        // RFC 4231 test case 2
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn every_cloud_prompt_carries_the_contract() {
        for (name, p) in [
            ("routine", ROUTINE_PROMPT),
            ("coder", crate::coder::CODER_PROMPT),
            ("devin", crate::devin::DEVIN_PROMPT),
        ] {
            assert!(p.starts_with(CLOUD_CONTRACT), "{name}");
            assert!(
                p.contains("link_pr") && p.contains("release") && p.contains("claim_next"),
                "{name}"
            );
            assert!(
                !p.contains("my_questions") && !p.contains("called back"),
                "{name}"
            );
        }
        assert!(ROUTINE_PROMPT.contains("routine-fire-payload"));
    }

    #[test]
    fn the_fire_and_forget_predicate_names_every_cloud_type() {
        let sql = fire_and_forget_sql("a.id");
        for t in CLOUD_TYPES {
            assert!(sql.contains(&format!("'{t}'")), "{t}");
        }
    }

    #[test]
    fn cuts_on_char_boundaries() {
        assert_eq!(cut("héllo", 100), "héllo");
        let s = cut(&"é".repeat(10), 8);
        assert!(s.ends_with('…') && s.len() <= 8 + '…'.len_utf8());
    }
}
