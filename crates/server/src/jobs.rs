//! Background work: the Postgres LISTEN fan-out (every replica), and — on the one replica
//! holding the advisory lock — the lease reaper, the dispatcher and event retention.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, postgres::PgListener};
use uuid::Uuid;

use crate::{service::emit, state::AppState};

const LOCK_ID: i64 = 0x636f726e74; // "cornt"

const LOCAL_COALESCE: Duration = Duration::from_secs(30);
const CLOUD_COALESCE: Duration = Duration::from_secs(300);

/// Every replica listens so SSE streams and long-polls anywhere hear about new events.
pub fn spawn_listener(state: AppState) {
    tokio::spawn(async move {
        loop {
            match PgListener::connect_with(&state.db).await {
                Ok(mut l) => {
                    if l.listen("events").await.is_err() {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    loop {
                        match l.recv().await {
                            Ok(n) => {
                                if let Ok(seq) = n.payload().parse::<i64>() {
                                    let _ = state.events.send(seq);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("event listener lost: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("event listener connect failed: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

/// Leader election by Postgres advisory lock; the lock dies with its connection.
pub fn spawn_leader(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Ok(mut conn) = PgConnection::connect(&state.config.database_url).await {
                let got: bool = sqlx::query_scalar("select pg_try_advisory_lock($1)")
                    .bind(LOCK_ID)
                    .fetch_one(&mut conn)
                    .await
                    .unwrap_or(false);
                if got {
                    tracing::info!("this replica runs the reaper and dispatcher");
                    let jobs = tokio::spawn(run_jobs(state.clone()));
                    loop {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        if sqlx::query("select 1").execute(&mut conn).await.is_err() {
                            break;
                        }
                    }
                    jobs.abort();
                    tracing::warn!("lost the background-jobs lock");
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

async fn run_jobs(state: AppState) {
    let s = state.clone();
    // Both loops live inside this future, so aborting it (on losing the lock) stops them too.
    let reaper = async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        let mut n = 0u64;
        loop {
            tick.tick().await;
            if let Err(e) = reap(&s).await {
                tracing::warn!("reaper: {e}");
            }
            if let Err(e) = nudge_unanswered(&s).await {
                tracing::warn!("comment nudges: {e}");
            }
            n += 1;
            if n % 240 == 1
                && let Err(e) = retain(&s).await
            {
                tracing::warn!("retention: {e}");
            }
        }
    };
    let w = state.clone();
    let files = async move {
        if let Some(st) = &w.storage
            && st.cfg.create_bucket
            && let Err(e) = st.ensure_bucket().await
        {
            tracing::warn!("storage: {e}");
        }
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            if let Err(e) = crate::files::watch(&w).await {
                tracing::warn!("file watcher: {e}");
            }
        }
    };
    tokio::select! {
        _ = reaper => {},
        _ = files => {},
        _ = Dispatcher::new(state).run() => {},
    }
}

/// Expired claims go back to `open`, with an event, in one statement.
pub async fn reap(state: &AppState) -> Result<u64, sqlx::Error> {
    let n = sqlx::query(
        "with old as (
           select id, claimed_by from tasks where status = 'claimed' and lease_expires_at < now() for update skip locked),
         expired as (
           update tasks t set status = 'open', claimed_by = null, lease_expires_at = null, updated_at = now()
           from old where t.id = old.id
           returning t.id, t.project_id, old.claimed_by as was, t.lease_epoch)
         insert into events (project_id, task_id, kind, actor_id, payload)
         select project_id, id, 'task.lease_expired', null,
                jsonb_build_object('epoch', lease_epoch, 'agent', (select name from principals where id = expired.was))
         from expired",
    )
    .execute(&state.db)
    .await?
    .rows_affected();
    Ok(n)
}

/// People's comments a cook has not answered in time (no comment, completion, release or block
/// by it on the task since): wake its main station once more with the comment. Returns how many.
pub async fn nudge_unanswered(state: &AppState) -> Result<u64, crate::error::Error> {
    let mut tx = state.db.begin().await?;
    // answered (or moot: the task was tossed) since the comment
    sqlx::query(
        "update comment_replies r set answered_at = now()
         where r.answered_at is null and r.nudged_at is null and (
           exists (select 1 from events e where e.task_id = r.task_id and e.actor_id = r.agent_id
                     and e.seq > r.since_seq
                     and e.kind in ('task.commented', 'task.completed', 'task.released', 'task.blocked'))
           or exists (select 1 from tasks t where t.id = r.task_id and t.status = 'cancelled'))",
    )
    .execute(&mut *tx)
    .await?;
    let due: Vec<(Uuid, Uuid, Uuid, Uuid, String, i64, String, String, Option<String>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "select r.comment_id, r.agent_id, r.task_id, r.project_id, pr.key, t.number, t.title, cm.body,
                    (select name from principals where id = r.author_id), r.created_at
             from comment_replies r
             join tasks t on t.id = r.task_id join projects pr on pr.id = r.project_id
             join comments cm on cm.id = r.comment_id
             where r.answered_at is null and r.nudged_at is null
               and r.created_at < now() - make_interval(mins => $1::int)
             for update of r skip locked",
        )
        .bind(state.config.comment_nudge_minutes as i32)
        .fetch_all(&mut *tx)
        .await?;
    let n = due.len() as u64;
    for (comment, agent, task, project, key, number, title, body, author, at) in due {
        let main: Option<Uuid> = sqlx::query_scalar(
            "select id from stations where agent_id = $1 and project_id = $2 and main",
        )
        .bind(agent)
        .bind(project)
        .fetch_optional(&mut *tx)
        .await?;
        let minutes = (chrono::Utc::now() - at).num_minutes().max(1);
        let tref = format!("{key}-{number}");
        let prompt = format!(
            "Still unanswered after {minutes} min: {} commented on {tref} \"{}\":\n\n{body}\n\n\
             The session that was woken did not reply. Answer with the `comment` tool on {tref} \
             (or hand it to the session that can).",
            author.as_deref().unwrap_or("someone"),
            crate::questions::short(&title),
        );
        crate::questions::wake(
            &mut tx,
            project,
            &key,
            agent,
            main,
            Some(task),
            &prompt,
            "comment-nudge",
        )
        .await?;
        sqlx::query(
            "update comment_replies set nudged_at = now() where comment_id = $1 and agent_id = $2",
        )
        .bind(comment)
        .bind(agent)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(n)
}

/// Keep 90 days of events (activity: 14), rolling older ones into per-day counts, and a week of the pass.
pub async fn retain(state: &AppState) -> Result<(), sqlx::Error> {
    // Agent activity is monitoring chatter: kept two weeks, never rolled into the daily counts.
    sqlx::query("delete from events where kind = 'activity' and at < now() - interval '14 days'")
        .execute(&state.db)
        .await?;
    sqlx::query(
        "with old as (delete from events where at < now() - interval '90 days' and project_id is not null returning project_id, kind, at::date as day),
              agg as (select day, project_id, kind, count(*) as n from old group by 1,2,3)
         insert into event_daily_counts (day, project_id, kind, count) select day, project_id, kind, n from agg
         on conflict (day, project_id, kind) do update set count = event_daily_counts.count + excluded.count",
    )
    .execute(&state.db)
    .await?;
    // The pass is working chatter: kept a week by default (PASS_RETENTION_DAYS).
    sqlx::query(
        "delete from pass_messages where created_at < now() - make_interval(days => $1::int)",
    )
    .bind(state.config.pass_retention_days as i32)
    .execute(&state.db)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow, Debug)]
struct Candidate {
    #[allow(dead_code)]
    target_id: Uuid,
    agent_id: Uuid,
    agent: String,
    runtime: Option<String>,
    typ: String,
    config: Value,
    busy: i64,
}

struct Dispatcher {
    state: AppState,
    last_wake: HashMap<Uuid, Instant>,
    /// Wakes held back by coalescing; flushed once the window passes (agent, project id).
    pending: HashSet<(Uuid, Uuid)>,
    /// Message wakes (pass, comments, answers) for cooks with cloud targets, batched per cook and
    /// pot into one cloud run at the next tick: (agent, project, wake payload).
    cloud: Vec<(Uuid, Uuid, Value)>,
    /// Cloud targets that asked to wait (HTTP 429): no delivery before then.
    cloud_wait: HashMap<Uuid, Instant>,
}

const TRIGGERS: &[&str] = &[
    "task.created",
    "task.released",
    "task.lease_expired",
    "task.completed",
    "task.unblocked",
    "task.cancelled",
];

impl Dispatcher {
    fn new(state: AppState) -> Self {
        Dispatcher {
            state,
            last_wake: HashMap::new(),
            pending: HashSet::new(),
            cloud: Vec::new(),
            cloud_wait: HashMap::new(),
        }
    }

    async fn run(mut self) {
        let mut rx = self.state.events.subscribe();
        // After a (re)start every project with ready work is considered once.
        if let Ok(projects) = sqlx::query_scalar::<_, Uuid>("select id from projects")
            .fetch_all(&self.state.db)
            .await
        {
            for p in projects {
                self.consider(p).await;
            }
        }
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                got = rx.recv() => match got {
                    Ok(seq) => self.on_event(seq).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if let Ok(ps) = sqlx::query_scalar::<_, Uuid>("select id from projects").fetch_all(&self.state.db).await {
                            for p in ps { self.consider(p).await; }
                        }
                    }
                    Err(_) => return,
                },
                _ = tick.tick() => {
                    self.flush_pending().await;
                    self.flush_cloud().await;
                }
            }
        }
    }

    async fn on_event(&mut self, seq: i64) {
        let row: Result<Option<(String, Option<Uuid>, Option<Uuid>, Value)>, _> =
            sqlx::query_as("select kind, project_id, actor_id, payload from events where seq = $1")
                .bind(seq)
                .fetch_optional(&self.state.db)
                .await;
        // A message wake (it has a reason) for a cook with cloud targets: batch it for a cloud run.
        if let Ok(Some((kind, Some(project), _, payload))) = &row
            && kind == "wake"
            && payload["reason"].is_string()
            && let Some(agent) = payload["agent_id"]
                .as_str()
                .and_then(|a| a.parse::<Uuid>().ok())
        {
            let has_cloud: bool = sqlx::query_scalar(
                "select exists (select 1 from wake_targets where agent_id = $1 and type = any($2))",
            )
            .bind(agent)
            .bind(crate::cloud::CLOUD_TYPES)
            .fetch_one(&self.state.db)
            .await
            .unwrap_or(false);
            if has_cloud {
                self.cloud.push((agent, *project, payload.clone()));
            }
            return;
        }
        if let Ok(Some((kind, Some(project), actor, _))) = row
            && TRIGGERS.contains(&kind.as_str())
        {
            // The cook whose own action this was is in the middle of a turn and goes back to
            // claim_next by itself: waking it too would queue a duplicate, stale prompt.
            self.consider_except(project, actor).await;
        }
    }

    async fn candidates(&self, project: Uuid, except: Option<Uuid>) -> Vec<Candidate> {
        sqlx::query_as(
            "select wt.id as target_id, a.id as agent_id, a.name as agent, a.runtime, wt.type as typ, wt.config,
                    (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) as busy
             from wake_targets wt
             join principals a on a.id = wt.agent_id and a.disabled_at is null
             join projects pr on pr.id = $1
             where exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = a.id)
               and ($2::uuid is null or a.id <> $2)
               and (wt.projects is null or cardinality(wt.projects) = 0 or pr.key = any(wt.projects))
               and exists (select 1 from tasks t where t.project_id = pr.id
                      and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
                      and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
                      and (wt.labels is null or cardinality(wt.labels) = 0 or t.labels && wt.labels))
               and (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) < a.max_leases",
        )
        .bind(project)
        .bind(except)
        .fetch_all(&self.state.db)
        .await
        .unwrap_or_default()
    }

    async fn consider(&mut self, project: Uuid) {
        self.consider_except(project, None).await;
    }

    async fn consider_except(&mut self, project: Uuid, except: Option<Uuid>) {
        let cands = self.candidates(project, except).await;
        let mut by_agent: HashMap<Uuid, Vec<Candidate>> = HashMap::new();
        for c in cands {
            by_agent.entry(c.agent_id).or_default().push(c);
        }
        for (agent, targets) in by_agent {
            let cloud = targets[0].runtime.as_deref() == Some("cloud");
            let window = if cloud {
                CLOUD_COALESCE
            } else {
                LOCAL_COALESCE
            };
            let recently = self
                .last_wake
                .get(&agent)
                .map(|t| t.elapsed() < window)
                .unwrap_or(false);
            // Cloud agents are never woken while they hold a lease: their session is already running.
            if recently || (cloud && targets[0].busy > 0) {
                self.pending.insert((agent, project));
                continue;
            }
            self.last_wake.insert(agent, Instant::now());
            self.pending.remove(&(agent, project));
            for t in targets {
                self.wake(project, t).await;
            }
        }
    }

    /// Deliver the batched message wakes: one cloud run per cook, pot and target.
    async fn flush_cloud(&mut self) {
        if self.cloud.is_empty() {
            return;
        }
        let mut batches: Vec<((Uuid, Uuid), Vec<Value>)> = vec![];
        for (agent, project, payload) in self.cloud.drain(..) {
            match batches.iter_mut().find(|b| b.0 == (agent, project)) {
                Some(b) => b.1.push(payload),
                None => batches.push(((agent, project), vec![payload])),
            }
        }
        let mut keep: Vec<(Uuid, Uuid, Value)> = vec![];
        for ((agent, project), wakes) in batches {
            let Ok(mut conn) = self.state.db.acquire().await else {
                keep.extend(wakes.into_iter().map(|w| (agent, project, w)));
                continue;
            };
            let row: Option<(String, String)> = sqlx::query_as(
                "select a.name, pr.key from principals a, projects pr where a.id = $1 and pr.id = $2",
            )
            .bind(agent)
            .bind(project)
            .fetch_optional(&mut *conn)
            .await
            .ok()
            .flatten();
            let Some((name, key)) = row else { continue };
            let targets: Vec<(Uuid, String, Value)> = sqlx::query_as(
                "select id, type, config from wake_targets where agent_id = $1 and type = any($2)
                   and (projects is null or cardinality(projects) = 0 or $3 = any(projects))",
            )
            .bind(agent)
            .bind(crate::cloud::CLOUD_TYPES)
            .bind(&key)
            .fetch_all(&mut *conn)
            .await
            .unwrap_or_default();
            let text =
                crate::cloud::messages_text(&mut conn, agent, &name, project, &key, &wakes).await;
            for (target, typ, config) in targets {
                if self
                    .cloud_wait
                    .get(&target)
                    .is_some_and(|t| *t > Instant::now())
                {
                    // the target asked us to wait: keep these for after
                    keep.extend(wakes.iter().map(|w| (agent, project, w.clone())));
                    continue;
                }
                let res =
                    crate::cloud::deliver(&self.state, &typ, &config, &name, &key, &text).await;
                if let Err(f) = &res
                    && let Some(wait) = f.retry_after
                {
                    self.cloud_wait.insert(target, Instant::now() + wait);
                    keep.extend(wakes.iter().map(|w| (agent, project, w.clone())));
                }
                record_delivery(&mut conn, project, &name, &key, &typ, wakes.len(), res).await;
            }
        }
        self.cloud.extend(keep);
    }

    async fn flush_pending(&mut self) {
        let pend: Vec<(Uuid, Uuid)> = self.pending.drain().collect(); // consider() re-adds what is still held back
        let mut projects: HashSet<Uuid> = HashSet::new();
        for (_, p) in pend {
            projects.insert(p);
        }
        for p in projects {
            self.consider(p).await;
        }
    }

    async fn wake(&self, project: Uuid, c: Candidate) {
        let key: String = match sqlx::query_scalar("select key from projects where id = $1")
            .bind(project)
            .fetch_one(&self.state.db)
            .await
        {
            Ok(k) => k,
            Err(_) => return,
        };
        let mut conn = match self.state.db.acquire().await {
            Ok(c) => c,
            Err(_) => return,
        };
        match c.typ.as_str() {
            "relay" => {
                // Wake an idle station (that very session) when the cook has any in this pot;
                // when all of them are busy, they pick the work up themselves.
                let station =
                    match crate::stations::pick_for_wake(&mut conn, c.agent_id, project).await {
                        Ok((true, None)) => return,
                        Ok((_, st)) => st,
                        Err(e) => {
                            tracing::warn!("station lookup failed: {e}");
                            None
                        }
                    };
                let _ = emit(
                    &mut conn,
                    Some(project),
                    None,
                    "wake",
                    None,
                    json!({"agent": c.agent, "agent_id": c.agent_id, "project": key, "station": station}),
                )
                .await;
            }
            "webhook" | "github-comment" | "claude-routine" => {
                let state = self.state.clone();
                let (typ, config, agent, agent_id) =
                    (c.typ.clone(), c.config.clone(), c.agent.clone(), c.agent_id);
                let key2 = key.clone();
                tokio::spawn(async move {
                    let Ok(mut conn) = state.db.acquire().await else {
                        return;
                    };
                    let text =
                        crate::cloud::work_text(&mut conn, agent_id, &agent, project, &key2).await;
                    let res =
                        crate::cloud::deliver(&state, &typ, &config, &agent, &key2, &text).await;
                    record_delivery(&mut conn, project, &agent, &key2, &typ, 0, res).await;
                });
            }
            _ => {}
        }
    }
}

/// Note a cloud delivery in the feed (with the session a Claude routine started).
async fn record_delivery(
    c: &mut PgConnection,
    project: Uuid,
    agent: &str,
    key: &str,
    typ: &str,
    messages: usize,
    res: Result<crate::cloud::Delivered, crate::cloud::Failed>,
) {
    let (kind, payload) = match res {
        Ok(d) => (
            "wake.delivered",
            json!({"agent": agent, "project": key, "type": typ, "status": d.status,
                   "session_url": d.session_url, "messages": messages}),
        ),
        Err(f) => {
            tracing::warn!("cloud wake for {agent} ({typ}) failed: {}", f.error);
            (
                "wake.failed",
                json!({"agent": agent, "project": key, "type": typ, "error": f.error,
                       "retry_in_s": f.retry_after.map(|d| d.as_secs()), "messages": messages}),
            )
        }
    };
    let _ = emit(c, Some(project), None, kind, None, payload).await;
}
