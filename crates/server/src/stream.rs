//! The relay's wake stream (SSE with `Last-Event-ID` replay) and the long-poll for work.

use std::{convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::Stream;
use serde_json::{Value, json};

use crate::{
    error::{Error, Result},
    service,
    state::{AppState, Principal},
};

pub const MAX_WAIT_S: i64 = 55;

/// Long-poll: return ready tasks as soon as there are any for this principal, or `[]` at the timeout.
pub async fn wait_for_work(
    state: &AppState,
    p: &Principal,
    project: Option<&str>,
    timeout_s: i64,
) -> Result<Value> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(timeout_s.clamp(0, MAX_WAIT_S) as u64);
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
        // Wake on any event; the periodic re-check also catches expiring leases. A shutdown ends
        // the wait with nothing, so the client simply asks again (of the next server).
        let wait = (deadline - now).min(Duration::from_secs(3));
        tokio::select! {
            _ = tokio::time::timeout(wait, rx.recv()) => {}
            _ = state.stopped() => return Ok(json!({"ready": []})),
        }
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
    let owner = relay
        .owner_id
        .ok_or_else(|| Error::Internal("relay without owner".into()))?;
    let mut last: i64 = match headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
    {
        Some(n) => n,
        None => {
            sqlx::query_scalar("select coalesce(max(seq), 0) from events")
                .fetch_one(&state.db)
                .await?
        }
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
                    "agent": payload["agent"], "at": at,
                    "summary": if payload["reason"].is_string() { "a message for you" } else { "new work is ready" },
                    // a registered session to resume (else the relay's [[target]] for the agent)
                    "station": payload["station"],
                    // the prompt to send, when the server words it (answers to questions); else the relay's own
                    "text": payload["text"], "reason": payload["reason"],
                });
                yield Ok(Event::default().id(seq.to_string()).event("wake").data(data.to_string()));
            }
            tokio::select! {
                _ = tokio::time::timeout(Duration::from_secs(5), rx.recv()) => {}
                _ = state.stopped() => {
                    // Tell the relay, then end the stream: it reconnects (from its Last-Event-ID).
                    yield Ok(Event::default().event("shutdown").data("{}"));
                    return;
                }
            }
        }
    };
    Ok(Sse::new(s).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// The live change feed for people, agents and tools that can hold a connection (the console,
/// `kitchen`): every event the caller may see, replayed from `Last-Event-ID` (or `?cursor=`,
/// default: from now) and then streamed as it happens, in one connection so nothing falls into
/// the gap between catching up and subscribing. Each SSE event has `id` = seq, `event` = kind,
/// `data` = the same JSON `changes_since` returns per event. Agents over MCP, which cannot hold
/// a stream, use `changes_since` plus the fire's wakes instead.
pub async fn events_stream(
    State(state): State<AppState>,
    auth: authkestra::axum::Auth<Principal>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>> {
    let p = auth.0.worker()?;
    let project = q.get("project").map(|k| k.to_uppercase());
    if let Some(k) = &project {
        // fail fast (404) instead of streaming nothing for a project the caller cannot see
        let mut c = state.db.acquire().await?;
        service::project_by_key(&mut c, &p, k).await?;
    }
    let from_header = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let from_query = q
        .get("cursor")
        .or_else(|| q.get("since"))
        .and_then(|v| v.parse::<i64>().ok());
    let mut last: i64 = match from_header.or(from_query) {
        Some(n) => n,
        None => {
            sqlx::query_scalar("select coalesce(max(seq), 0) from events")
                .fetch_one(&state.db)
                .await?
        }
    };
    let mut rx = state.events.subscribe();
    let s = async_stream::stream! {
        yield Ok(Event::default().event("ready").data(json!({"cursor": last}).to_string()));
        loop {
            // drain everything after `last` (pages of 200), then wait for the next NOTIFY
            loop {
                let page = service::changes_since(&state.db, &p, last, project.as_deref(), 200).await;
                let Ok(page) = page else {
                    yield Ok(Event::default().event("error").data(json!({"error": "access lost"}).to_string()));
                    return;
                };
                let events = page["events"].as_array().cloned().unwrap_or_default();
                for e in &events {
                    let seq = e["seq"].as_i64().unwrap_or(last);
                    last = last.max(seq);
                    yield Ok(Event::default()
                        .id(seq.to_string())
                        .event(e["kind"].as_str().unwrap_or("event"))
                        .data(e.to_string()));
                }
                if events.len() < 200 {
                    break;
                }
            }
            // Lagged or closed channels just mean "re-read"; a timeout bounds missed NOTIFYs.
            tokio::select! {
                _ = tokio::time::timeout(Duration::from_secs(10), rx.recv()) => {}
                _ = state.stopped() => {
                    yield Ok(Event::default().event("shutdown").data("{}"));
                    return;
                }
            }
        }
    };
    Ok(Sse::new(s).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
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
pub async fn report_run(
    State(state): State<AppState>,
    auth: authkestra::axum::Auth<Principal>,
    Json(r): Json<RunReport>,
) -> Result<Json<Value>> {
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
         where a.name = $1 and a.kind = 'agent' and a.owner_id = $2 and pr.key = $3
           and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = a.id)",
    )
    .bind(r.agent.to_lowercase())
    .bind(relay.owner_id)
    .bind(r.project.to_uppercase())
    .fetch_optional(&state.db)
    .await?;
    let (agent_id, project_id) =
        row.ok_or_else(|| Error::NotFound("unknown agent or project".into()))?;
    let mut c = state.db.acquire().await?;
    service::emit(
        &mut c,
        Some(project_id),
        None,
        kind,
        Some(agent_id),
        json!({"agent": r.agent, "exit_code": r.exit_code}),
    )
    .await?;
    Ok(Json(json!({"ok": true})))
}
