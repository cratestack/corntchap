//! The restaurant: the server itself as a visible actor.
//!
//! Every decision the backend takes on its own (a lease that lapsed, a task blocked after five
//! give-backs, a cook that is not woken any more) is an event whose actor is the `restaurant`
//! principal (migration 0040), with a `reason` code and a `why` sentence the server builds. Decisions
//! that have no event of their own are `restaurant.decided` events, written through [`decide`], which
//! keeps a repeating decision quiet: within its window only a counter moves, and the next event
//! carries `repeated: n`.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{error::Result, service::emit, state::RESTAURANT};

pub const KIND: &str = "restaurant.decided";

/// How long a repeated decision stays quiet.
pub const QUIET_SECS: i64 = 30 * 60;
/// `role.kept` is told at most once a day.
pub const DAY_SECS: i64 = 24 * 3600;

/// Decisions that hold a cook back: `what_now` tells the cook about its latest one.
pub const HOLDING: &[&str] = &["wake.capped", "wake.withheld", "claim.skipping"];

/// One decision of the restaurant.
#[derive(Clone, Debug, Default)]
pub struct Decision {
    pub project: Option<Uuid>,
    /// `wake.held`, `wake.capped`, `wake.withheld`, `claim.skipping`, `catch_up`, `merge.kept_open`,
    /// `run.ended` or `role.kept`.
    pub decision: &'static str,
    /// The code: why (`backoff`, `retry_after`, `busy`, `stations_busy`, `sweep`, `retry`,
    /// `fire_and_forget`, `uncovered`, `bounce`, `away`, ...).
    pub reason: &'static str,
    /// The sentence people read, built here.
    pub text: String,
    pub agent: Option<String>,
    pub agent_id: Option<Uuid>,
    /// The task's id (event task) and its ref (PAY-3).
    pub task: Option<(Uuid, String)>,
    pub station: Option<String>,
    pub until: Option<DateTime<Utc>>,
    pub misses: Option<u32>,
    /// What tells this decision apart from others of the same kind for the quiet window (default:
    /// the task's ref).
    pub key: Option<String>,
    /// The quiet window in seconds (default [`QUIET_SECS`]).
    pub quiet: Option<i64>,
}

impl Decision {
    pub fn new(
        project: Option<Uuid>,
        decision: &'static str,
        reason: &'static str,
        text: String,
    ) -> Self {
        Decision {
            project,
            decision,
            reason,
            text,
            ..Default::default()
        }
    }
    pub fn cook(mut self, name: &str, id: Uuid) -> Self {
        self.agent = Some(name.to_string());
        self.agent_id = Some(id);
        self
    }
    pub fn on(mut self, task: Option<(Uuid, String)>) -> Self {
        self.task = task;
        self
    }
    pub fn at(mut self, station: Option<String>) -> Self {
        self.station = station;
        self
    }
    pub fn until(mut self, until: Option<DateTime<Utc>>) -> Self {
        self.until = until;
        self
    }
    pub fn misses(mut self, n: u32) -> Self {
        self.misses = Some(n);
        self
    }
    pub fn keyed(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
    pub fn quiet_for(mut self, secs: i64) -> Self {
        self.quiet = Some(secs);
        self
    }

    fn quiet_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.decision,
            self.reason,
            self.agent_id
                .map(|a| a.to_string())
                .or_else(|| self.agent.clone())
                .unwrap_or_default(),
            self.project.map(|p| p.to_string()).unwrap_or_default(),
            self.key
                .clone()
                .or_else(|| self.task.as_ref().map(|t| t.1.clone()))
                .unwrap_or_default()
        )
    }
}

/// Record a decision as a `restaurant.decided` event, unless the same one was recorded within its
/// quiet window: then only the counter moves and `None` comes back. Boxed: callers sit deep in the
/// forge, wake and claim paths, whose futures must not grow with this one.
pub fn decide(
    c: &mut PgConnection,
    d: Decision,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<i64>>> + Send + '_>> {
    Box::pin(decide_now(c, d))
}

async fn decide_now(c: &mut PgConnection, d: Decision) -> Result<Option<i64>> {
    let key = d.quiet_key();
    sqlx::query(
        "insert into restaurant_quiet (key, last_at, suppressed) values ($1, 'epoch', 0) on conflict (key) do nothing",
    )
    .bind(&key)
    .execute(&mut *c)
    .await?;
    // `skip locked`: when another transaction is deciding the same key right now, it records the
    // decision; waiting for it could deadlock two requests that decide about two cooks in opposite
    // order.
    let row: Option<(bool, i32)> = sqlx::query_as(
        "select last_at > now() - make_interval(secs => $2::float8), suppressed
         from restaurant_quiet where key = $1 for update skip locked",
    )
    .bind(&key)
    .bind(d.quiet.unwrap_or(QUIET_SECS) as f64)
    .fetch_optional(&mut *c)
    .await?;
    let Some((quiet, suppressed)) = row else {
        return Ok(None);
    };
    if quiet {
        sqlx::query("update restaurant_quiet set suppressed = suppressed + 1 where key = $1")
            .bind(&key)
            .execute(&mut *c)
            .await?;
        return Ok(None);
    }
    let mut payload = json!({"decision": d.decision, "reason": d.reason, "text": d.text});
    let o = payload.as_object_mut().expect("object");
    if let Some(a) = &d.agent {
        o.insert("agent".into(), json!(a));
    }
    if let Some(a) = d.agent_id {
        o.insert("agent_id".into(), json!(a));
    }
    if let Some((_, r)) = &d.task {
        o.insert("task".into(), json!(r));
    }
    if let Some(s) = &d.station {
        o.insert("station".into(), json!(s));
    }
    if let Some(u) = d.until {
        o.insert("until".into(), json!(u));
    }
    if let Some(m) = d.misses {
        o.insert("misses".into(), json!(m));
    }
    if suppressed > 0 {
        o.insert("repeated".into(), json!(suppressed));
    }
    let seq = emit(
        c,
        d.project,
        d.task.as_ref().map(|t| t.0),
        KIND,
        Some(RESTAURANT),
        payload,
    )
    .await?;
    sqlx::query(
        "update restaurant_quiet set last_at = now(), last_seq = $2, suppressed = 0 where key = $1",
    )
    .bind(&key)
    .bind(seq)
    .execute(&mut *c)
    .await?;
    Ok(Some(seq))
}

/// A decision of the restaurant that holds this cook back, for `what_now`: the line to show, when
/// its latest decision about it is one of [`HOLDING`] and it has not claimed anything since.
pub async fn holding_line(
    c: &mut PgConnection,
    agent: Uuid,
    project: Uuid,
) -> Result<Option<String>> {
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "select seq, payload->>'decision', payload->>'text' from events
         where actor_id = $1 and kind = $2 and payload->>'agent_id' = $3::text and project_id = $4
         order by seq desc limit 1",
    )
    .bind(RESTAURANT)
    .bind(KIND)
    .bind(agent)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?;
    let Some((seq, decision, text)) = row else {
        return Ok(None);
    };
    if !HOLDING.contains(&decision.as_str()) {
        return Ok(None);
    }
    let claimed: bool = sqlx::query_scalar(
        "select exists (select 1 from events where actor_id = $1 and kind = 'task.claimed' and seq > $2)",
    )
    .bind(agent)
    .bind(seq)
    .fetch_one(&mut *c)
    .await?;
    Ok((!claimed).then_some(text))
}

/// The restaurant's own decision, as a short sentence about a moment in time.
pub fn clock(at: DateTime<Utc>) -> String {
    at.format("%H:%M UTC").to_string()
}

/// A decision's payload for `kind`s that are not `restaurant.decided`: the reason code and the
/// sentence.
pub fn because(reason: &str, why: &str) -> Value {
    json!({"reason": reason, "why": why})
}

/// Drop the quiet rows that have outlived their window (retention).
pub async fn prune(db: &sqlx::PgPool) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("delete from restaurant_quiet where last_at < now() - interval '2 days'")
        .execute(db)
        .await?;
    Ok(())
}

/// What the restaurant did about one cook, newest first, for the cook's page in the console
/// (`cookDecisions`): its decisions, its wakes (sent, delivered, failed) and its lapsed leases. Only
/// events of pots the caller can see.
pub async fn cook_log(
    db: &sqlx::PgPool,
    p: &crate::state::Principal,
    agent: &str,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let paging = crate::paging::paging(page, "-seq", 1, 20, crate::paging::MAX_LIMIT)?;
    let mut c = db.acquire().await?;
    let id: Uuid =
        sqlx::query_scalar("select id from principals where name = $1 and kind = 'agent'")
            .bind(agent.to_lowercase())
            .fetch_optional(&mut *c)
            .await?
            .ok_or_else(|| crate::error::Error::NotFound(format!("no cook named {agent}")))?;
    let sql = format!(
        "select jsonb_build_object('seq', e.seq, 'kind', e.kind, 'project', pr.key,
            'task', (select pp.key || '-' || tt.number from tasks tt join projects pp on pp.id = tt.project_id where tt.id = e.task_id),
            'actor', 'restaurant', 'actor_kind', 'restaurant', 'at', e.at, 'payload', e.payload),
            array[e.seq::text]
         from events e join projects pr on pr.id = e.project_id
         where e.actor_id = $1
           and e.kind in ('restaurant.decided', 'wake', 'wake.delivered', 'wake.failed', 'task.lease_expired')
           and (e.payload->>'agent_id' = $2::text or (e.payload->>'agent_id' is null and e.payload->>'agent' = $3))
           and (e.audience is null or $5 = any(e.audience)) and {}
           and ($6::text is null or e.seq < $6::text::bigint)
         order by e.seq desc limit $7",
        crate::service::access_sql("$4", "$5")
    );
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(RESTAURANT)
        .bind(id)
        .bind(agent.to_lowercase())
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(paging.key(0))
        .bind(paging.fetch())
        .fetch_all(&mut *c)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}
