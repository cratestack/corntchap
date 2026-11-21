//! The relay's wake stream (SSE with `Last-Event-ID` replay) and the long-poll for work.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::Stream;
use serde_json::{json, Value};

use crate::{
    error::{Error, Result},
    service,
    state::{AppState, Principal},
};

pub const MAX_WAIT_S: i64 = 55;

/// Long-poll: return ready tasks as soon as there are any for this principal, or `[]` at the timeout.
pub async fn wait_for_work(state: &AppState, p: &Principal, project: Option<&str>, timeout_s: i64) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_s.clamp(0, MAX_WAIT_S) as u64);
    let mut rx = state.events.subscribe();
    loop {
        let ready = service::ready_for(&state.db, p, project).await?;
        if !ready.is_empty() {
            return Ok(json!({"ready": ready}));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(json!({"ready": []}));
        }
        // Wake on any event; the periodic re-check also catches expiring leases.
        let wait = (deadline - now).min(Duration::from_secs(3));
        let _ = tokio::time::timeout(wait, rx.recv()).await;
    }
}

pub async fn relay_stream(
    State(state): State<AppState>,
    auth: authkestra::axum::Auth<Principal>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let relay = auth.0;
    if !relay.is_relay() {
        return Err(Error::Forbidden("the wake stream is for relays".into()));
    }
    let owner = relay.owner_id.ok_or_else(|| Error::Internal("relay without owner".into()))?;
    let mut last: i64 = match headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()) {
        Some(n) => n,
        None => sqlx::query_scalar("select coalesce(max(seq), 0) from events").fetch_one(&state.db).await?,
    };
    let mut rx = state.events.subscribe();
    let relay_id = relay.id;
    let s = async_stream::stream! {
        let mut last_seen = std::time::Instant::now() - Duration::from_secs(60);
        yield Ok(Event::default().event("ping").data("{}"));
        loop {
            if last_seen.elapsed() > Duration::from_secs(30) {
                let _ = sqlx::query("update principals set last_seen_at = now() where id = $1").bind(relay_id).execute(&state.db).await;
                last_seen = std::time::Instant::now();
            }
            let rows: Vec<(i64, Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
                "select e.seq, e.payload, e.at from events e
                 where e.seq > $1 and e.kind = 'wake'
                   and (e.payload->>'agent_id')::uuid in (select id from principals where owner_id = $2 and kind = 'agent')
                 order by e.seq limit 100",
            )
            .bind(last)
            .bind(owner)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
            for (seq, payload, at) in rows {
                last = seq;
                let data = json!({
                    "seq": seq, "kind": "wake", "project": payload["project"], "task": null,
                    "agent": payload["agent"], "at": at, "summary": "new work is ready",
                });
                yield Ok(Event::default().id(seq.to_string()).event("wake").data(data.to_string()));
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        }
    };
    Ok(Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("keep-alive")))
}

#[derive(serde::Deserialize)]
pub struct RunReport {
    pub agent: String,
    pub project: String,
    /// "started" or "finished"
    pub event: String,
    pub exit_code: Option<i32>,
}

/// The relay reports which of its owner's agents is awake, so the board can show it.
pub async fn report_run(State(state): State<AppState>, auth: authkestra::axum::Auth<Principal>, Json(r): Json<RunReport>) -> Result<Json<Value>> {
    let relay = auth.0;
    if !relay.is_relay() {
        return Err(Error::Forbidden("only relays report runs".into()));
    }
    let kind = match r.event.as_str() {
        "started" => "run.started",
        "finished" => "run.finished",
        _ => return Err(Error::Invalid("event must be started or finished".into())),
    };
    let row: Option<(uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "select a.id, pr.id from principals a, projects pr
         where a.name = $1 and a.kind = 'agent' and a.owner_id = $2 and pr.key = $3",
    )
    .bind(r.agent.to_lowercase())
    .bind(relay.owner_id)
    .bind(r.project.to_uppercase())
    .fetch_optional(&state.db)
    .await?;
    let (agent_id, project_id) = row.ok_or_else(|| Error::NotFound("unknown agent or project".into()))?;
    let mut c = state.db.acquire().await?;
    service::emit(&mut c, Some(project_id), None, kind, Some(agent_id), json!({"agent": r.agent, "exit_code": r.exit_code})).await?;
    Ok(Json(json!({"ok": true})))
}
