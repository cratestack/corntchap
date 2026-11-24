//! Agent activity: the hooks of the agent runtimes (`corntchap-cli hook`) report each edit or
//! command with the paths it changed and the git checkpoint taken after it. Stored as `activity`
//! events in the station's pot (no table of its own); a burst of calls is coalesced into one event
//! per station per two seconds. Activity never wakes anyone and stays out of the change feed.

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    admin,
    error::{Result, invalid},
    service::{emit, project_by_key, task_by_ref},
    state::Principal,
};

pub const KIND: &str = "activity";
const MAX_FILES: usize = 50;
const MAX_PATH: usize = 400;
const MAX_SUMMARY: usize = 300;
/// One event per station per this many seconds; calls in between are merged into it.
const COALESCE_SECS: i32 = 2;

pub struct Record {
    pub runtime: String,
    pub session: String,
    pub tool: String,
    pub files: Vec<String>,
    pub checkpoint: Option<String>,
    pub summary: Option<String>,
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Record what a session did. A session that is not a registered station of a cook the caller
/// owns (or is) is not an error (hooks must never fail): nothing is stored and the reply says why.
pub async fn record(db: &PgPool, p: &Principal, r: Record) -> Result<Value> {
    let runtime = r.runtime.trim().to_lowercase();
    let session = r.session.trim();
    let tool = clip(r.tool.trim(), 64);
    if session.is_empty() || tool.is_empty() {
        return invalid("session and tool are required");
    }
    let checkpoint = r
        .checkpoint
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if checkpoint
        .is_some_and(|c| c.len() > 64 || c.len() < 4 || !c.chars().all(|x| x.is_ascii_hexdigit()))
    {
        return invalid("checkpoint is a git commit id");
    }
    let mut files: Vec<String> = vec![];
    for f in r.files.iter().map(|f| clip(f.trim(), MAX_PATH)) {
        if !f.is_empty() && !files.contains(&f) && files.len() < MAX_FILES {
            files.push(f);
        }
    }
    let summary = r
        .summary
        .as_deref()
        .map(|s| clip(s.trim(), MAX_SUMMARY))
        .filter(|s| !s.is_empty());

    let who = admin::acting_or_self(db, p).await?;
    let mut tx = db.begin().await?;
    let st: Option<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "select st.id, st.ref, st.project_id, a.name from stations st join principals a on a.id = st.agent_id
         where st.runtime = $1 and st.session_ref = $2 and ($3 or a.id = $4 or a.owner_id = $4)
         order by st.created_at limit 1",
    )
    .bind(&runtime)
    .bind(session)
    .bind(who.is_super_admin())
    .bind(who.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((station_id, station, project, cook)) = st else {
        return Ok(json!({
            "recorded": false,
            "message": "unregistered session: run corntchap-cli station scan"
        }));
    };
    // an agent that is working keeps its leases: each tool use renews the leases this session holds
    // to the pot's lease length (never shortening a longer one it asked for)
    // (never past the stall limit or the maximum hold: activity is not progress)
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update tasks t set lease_expires_at = least(greatest(t.lease_expires_at, now() + pr.lease_ttl), {cap}), updated_at = now()
         from projects pr where pr.id = t.project_id and t.station_id = $1
           and t.status = 'claimed' and t.lease_expires_at > now()",
        cap = crate::service::lease_cap_sql("t"),
    )))
    .bind(station_id)
    .execute(&mut *tx)
    .await?;
    // serialize the burst of one station so two calls cannot both open a new event
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(&station)
        .execute(&mut *tx)
        .await?;
    let recent: Option<(i64, Value)> = sqlx::query_as(
        "select seq, payload from events where kind = 'activity' and project_id = $1
           and payload->>'station' = $2 and at > now() - make_interval(secs => $3::int)
         order by seq desc limit 1",
    )
    .bind(project)
    .bind(&station)
    .bind(COALESCE_SECS)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((seq, mut payload)) = recent {
        let mut all: Vec<String> = payload["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(String::from))
            .collect();
        for f in files {
            if !all.contains(&f) && all.len() < MAX_FILES {
                all.push(f);
            }
        }
        payload["files"] = json!(all);
        payload["tool"] = json!(tool);
        if checkpoint.is_some() {
            payload["checkpoint"] = json!(checkpoint);
        }
        if summary.is_some() {
            payload["summary"] = json!(summary);
        }
        payload["count"] = json!(payload["count"].as_i64().unwrap_or(1) + 1);
        sqlx::query("update events set payload = $2 where seq = $1")
            .bind(seq)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
    } else {
        let task: Option<Uuid> = sqlx::query_scalar(
            "select id from tasks where station_id = $1 and status = 'claimed' and lease_expires_at > now()
             order by updated_at desc limit 1",
        )
        .bind(station_id)
        .fetch_optional(&mut *tx)
        .await?;
        emit(
            &mut tx,
            Some(project),
            task,
            KIND,
            Some(who.id),
            json!({"station": station, "cook": cook, "tool": tool, "files": files,
                   "checkpoint": checkpoint, "summary": summary, "count": 1}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"recorded": true, "station": station}))
}

/// Recent activity of a pot, newest first.
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: &str,
    station: Option<&str>,
    task: Option<&str>,
    limit: Option<i64>,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let task_id = match task.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Some(task_by_ref(&mut c, p, t).await?.id),
        None => None,
    };
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('seq', e.seq, 'at', e.at, 'station', e.payload->>'station', 'cook', e.payload->>'cook',
            'tool', e.payload->>'tool', 'files', coalesce(e.payload->'files', '[]'::jsonb),
            'checkpoint', e.payload->>'checkpoint', 'summary', e.payload->>'summary',
            'task', (select pp.key || '-' || tt.number from tasks tt join projects pp on pp.id = tt.project_id where tt.id = e.task_id),
            'count', coalesce((e.payload->>'count')::bigint, 1))
         from events e
         where e.kind = 'activity' and e.project_id = $1
           and ($2::text is null or e.payload->>'station' = $2)
           and ($3::uuid is null or e.task_id = $3)
         order by e.seq desc limit $4",
    )
    .bind(pr.id)
    .bind(station.map(str::trim).filter(|s| !s.is_empty()))
    .bind(task_id)
    .bind(limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *c)
    .await?;
    Ok(Value::Array(rows))
}
