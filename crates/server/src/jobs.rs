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

/// Sweep-originated wakes stop for a cook after this many work wakes in a row that led to no claim.
const SWEEP_MAX_MISSES: u32 = 3;

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
            // cloud runs that went quiet without releasing anything
            if n % 80 == 1
                && let Err(e) = crate::stations::end_stale_runs(&s.db).await
            {
                tracing::warn!("ending stale cloud runs: {e}");
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
        _ = async move {
            let (dispatcher, failures) = Dispatcher::new(state);
            dispatcher.run(failures).await
        } => {},
    }
}

/// Expired claims go back to `open`, with an event, in one statement. The event says why: the
/// lease `lapsed` (no heartbeat), the holder `stalled` (no progress for STALL_MINUTES) or held the
/// task for `max_hold` (MAX_HOLD_HOURS). For the last two the task gets a note from the server and
/// the holder is told. A task given back too often is blocked for a person.
pub async fn reap(state: &AppState) -> Result<u64, sqlx::Error> {
    let mut tx = state.db.begin().await?;
    let sql = format!(
        "with old as (
           select id, claimed_by,
                  case when coalesce(held_since, updated_at) + make_interval(secs => {max}) <= now() then 'max_hold'
                       when coalesce(progress_at, updated_at) + make_interval(secs => {stall}) <= now() then 'stalled'
                       else 'lapsed' end as why
           from tasks where status = 'claimed' and lease_expires_at < now() for update skip locked),
         expired as (
           update tasks t set status = 'open', claimed_by = null, lease_expires_at = null, updated_at = now()
           from old where t.id = old.id
           returning t.id, t.project_id, old.claimed_by as was, t.lease_epoch, t.station_id, old.why),
         logged as (
           insert into events (project_id, task_id, kind, actor_id, payload)
           select project_id, id, 'task.lease_expired', '{restaurant}'::uuid,
                  jsonb_build_object('epoch', lease_epoch,
                                     'why', case why when 'max_hold' then 'the lease ended: the task was held for more than {max_text}'
                                                     when 'stalled' then 'the lease ended: no progress for {stall_text}'
                                                     else 'the lease ended: it ran out without a heartbeat' end, 'agent', (select name from principals where id = expired.was),
                                     'agent_id', expired.was, 'reason', why,
                                     'on_pr', (select not ({np}) from tasks t where t.id = expired.id),
                                     'station', (select s.ref from stations s where s.id = expired.station_id))
           from expired returning task_id)
         select id, project_id, was, why, station_id from expired",
        stall = crate::service::stall_secs(),
        max = crate::service::max_hold_secs(),
        np = crate::service::NO_OPEN_PR_SQL,
        restaurant = crate::state::RESTAURANT,
        max_text = crate::service::hours_text(crate::service::max_hold_secs()),
        stall_text = crate::service::hours_text(crate::service::stall_secs()),
    );
    let reaped: Vec<(Uuid, Uuid, Option<Uuid>, String, Option<Uuid>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .fetch_all(&mut *tx)
            .await?;
    for (id, project, was, why, station) in &reaped {
        // a savepoint: one task that cannot be settled must not undo the reaping of the rest
        let mut sp = tx.begin().await?;
        let settled = async {
            settle_reaped(&mut sp, *id, *project, *was, why).await?;
            // a cloud run whose last lease lapsed is over
            if let Some(st) = station {
                crate::stations::end_if_idle(&mut sp, *st).await?;
            }
            Ok::<(), crate::error::Error>(())
        }
        .await;
        match settled {
            Ok(_) => sp.commit().await?,
            Err(e) => {
                tracing::warn!("reaper: settling {id}: {e}");
                sp.rollback().await?;
            }
        }
    }
    tx.commit().await?;
    Ok(reaped.len() as u64)
}

/// After a lease was reaped: say so on the task and tell the holder when the server ended it for
/// stalling or holding too long; complete the task when its PR merged meanwhile; block it when it
/// has been given back too often.
async fn settle_reaped(
    c: &mut PgConnection,
    task: Uuid,
    project: Uuid,
    was: Option<Uuid>,
    why: &str,
) -> Result<(), crate::error::Error> {
    if why != "lapsed" {
        let holder: Option<String> = match was {
            Some(w) => {
                sqlx::query_scalar("select name from principals where id = $1")
                    .bind(w)
                    .fetch_optional(&mut *c)
                    .await?
            }
            None => None,
        };
        let (tref, key): (String, String) = sqlx::query_as(
            "select pr.key || '-' || t.number, pr.key from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
        )
        .bind(task)
        .fetch_one(&mut *c)
        .await?;
        let limit = if why == "max_hold" {
            format!(
                "it was held for more than {}",
                crate::service::hours_text(crate::service::max_hold_secs())
            )
        } else {
            format!(
                "no progress for {}",
                crate::service::hours_text(crate::service::stall_secs())
            )
        };
        let holder_name = holder.as_deref().unwrap_or("a cook that is gone");
        crate::service::system_comment(
            c,
            task,
            &format!("Lease ended: {limit}; holder {holder_name}. The task is open again."),
        )
        .await?;
        if let Some(agent) = was {
            let text = format!(
                "Your lease on {tref} was ended by the server: {limit}. Stop work on it and report \
                 what is done in a comment; it is open for someone else."
            );
            crate::questions::wake(
                c,
                project,
                &key,
                agent,
                None,
                Some(task),
                &text,
                "lease-lost",
            )
            .await?;
        }
    }
    crate::forge::events::complete_if_merged(c, task, None).await?;
    crate::service::block_if_bounced(c, task, project).await?;
    Ok(())
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
    #[allow(clippy::type_complexity)]
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

/// Keep 90 days of events (activity: 14, restaurant decisions: 30), rolling older ones into per-day counts, and a week of the pass.
pub async fn retain(state: &AppState) -> Result<(), sqlx::Error> {
    // Agent activity is monitoring chatter: kept two weeks, never rolled into the daily counts.
    sqlx::query("delete from events where kind = 'activity' and at < now() - interval '14 days'")
        .execute(&state.db)
        .await?;
    // The restaurant's decisions are monitoring too: a month, and never rolled into the daily counts.
    sqlx::query(
        "delete from events where kind = 'restaurant.decided' and at < now() - interval '30 days'",
    )
    .execute(&state.db)
    .await?;
    crate::restaurant::prune(&state.db).await?;
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
    crate::usage::prune(state).await?;
    // Cloud runs (stations) that ended are kept as long as the events that mention them.
    sqlx::query(
        "delete from stations where kind = 'cloud-run' and ended_at < now() - interval '90 days'",
    )
    .execute(&state.db)
    .await?;
    // Forge deliveries (src/forge): kept a week, enough to dedupe a redelivery and show the last one.
    sqlx::query("delete from forge_deliveries where at < now() - interval '7 days'")
        .execute(&state.db)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow, Debug)]
struct Candidate {
    target_id: Uuid,
    agent_id: Uuid,
    agent: String,
    typ: String,
    config: Value,
    labels: Vec<String>,
    busy: i64,
}

/// A cloud work wake that failed: (target, agent, project, the wait the endpoint asked for).
type WakeFailure = (Uuid, Uuid, Uuid, Option<Duration>);

struct Dispatcher {
    state: AppState,
    last_wake: HashMap<(Uuid, bool), Instant>,
    /// Wakes held back by coalescing; flushed once the window passes (agent, project id).
    pending: HashSet<(Uuid, Uuid)>,
    /// Message wakes (pass, comments, answers) for cooks with cloud targets, batched per cook and
    /// pot into one cloud run at the next tick: (agent, project, wake payload).
    cloud: Vec<(Uuid, Uuid, Value)>,
    /// Failed cloud work wakes to look at again, with the earliest time (agent, project). They go
    /// through the same capped path as a sweep wake (see [`SWEEP_MAX_MISSES`]).
    retry: HashMap<(Uuid, Uuid), Instant>,
    /// Cloud targets that asked to wait (HTTP 429): no delivery before then.
    cloud_wait: HashMap<Uuid, Instant>,
    /// Work wakes in a row that led to no claim, per cook and kind (cloud or not): each one doubles
    /// the wait before the next (see [`backoff`]); a claim resets it. A cook that cannot claim (a
    /// refused tool, a broken session) is not woken every 30 seconds forever.
    misses: HashMap<(Uuid, bool), u32>,
    /// Where the spawned cloud deliveries report a failed work wake.
    failed: tokio::sync::mpsc::UnboundedSender<WakeFailure>,
    /// What the restaurant last decided about a cook in a pot (decision, reason): a decision is
    /// recorded only when it changes, and a wake or a claim clears it. An idle sweep tick that decides
    /// the same thing again writes nothing.
    held: HashMap<(Uuid, Uuid), (&'static str, &'static str)>,
}

/// What looks at a pot's cooks: an event, the periodic sweep, or the retry of a failed delivery.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    Event,
    Sweep,
    Retry,
}

/// A moment (on the dispatcher's clock) as a date.
fn wall(at: Instant) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
        + chrono::Duration::from_std(at.saturating_duration_since(Instant::now()))
            .unwrap_or_default()
}

/// The wait before waking a cook again for the same work: the coalescing window, doubled for each
/// wake in a row that led to no claim, at most 30 minutes.
fn backoff(base: Duration, misses: u32) -> Duration {
    base.saturating_mul(1u32 << misses.min(10))
        .min(Duration::from_secs(30 * 60))
        .max(base)
}

/// When a wait asked for by an endpoint ends: at most an hour from now.
fn deadline(wait: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(wait.min(Duration::from_secs(3600)))
        .unwrap_or_else(|| now + Duration::from_secs(3600))
}

const TRIGGERS: &[&str] = &[
    "task.created",
    "task.released",
    "task.lease_expired",
    "task.completed",
    "task.unblocked",
    "task.cancelled",
    "task.restored",
    "task.updated",
    // a parked pull request (failing checks, changes requested, conflicts) is work again
    "forge.checks",
    "forge.review",
    "forge.conflict",
    "forge.pr",
];

/// The event kinds that make the dispatcher look at a pot again. The restaurant's own decisions are never among them.
pub fn wake_triggers() -> &'static [&'static str] {
    TRIGGERS
}

impl Dispatcher {
    fn new(state: AppState) -> (Self, tokio::sync::mpsc::UnboundedReceiver<WakeFailure>) {
        let (failed, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Dispatcher {
                state,
                last_wake: HashMap::new(),
                pending: HashSet::new(),
                cloud: Vec::new(),
                retry: HashMap::new(),
                cloud_wait: HashMap::new(),
                misses: HashMap::new(),
                failed,
                held: HashMap::new(),
            },
            rx,
        )
    }

    async fn run(mut self, mut failures: tokio::sync::mpsc::UnboundedReceiver<WakeFailure>) {
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
        let sweep = self.state.config.cloud_sweep;
        let mut tick = tokio::time::interval(Duration::from_secs(5).min(sweep));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Cloud cooks are fire and forget and no event may come for them: check every project with
        // ready work on a timer too.
        let mut sweeper = tokio::time::interval(sweep);
        sweeper.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        sweeper.tick().await;
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
                Some((target, agent, project, wait)) = failures.recv() => {
                    // a work wake that did not get through: honour a wait it asked for, look again later
                    let at = match wait {
                        Some(w) => {
                            let at = deadline(w);
                            self.cloud_wait.insert(target, at);
                            at
                        }
                        None => deadline(self.state.config.cloud_coalesce),
                    };
                    self.retry.insert((agent, project), at);
                }
                _ = tick.tick() => {
                    self.flush_pending().await;
                    self.flush_retry().await;
                    self.flush_cloud().await;
                }
                _ = sweeper.tick() => self.sweep().await,
            }
        }
    }

    async fn on_event(&mut self, seq: i64) {
        let row: Result<Option<(String, Option<Uuid>, Option<Uuid>, Value)>, _> =
            sqlx::query_as("select kind, project_id, actor_id, payload from events where seq = $1")
                .bind(seq)
                .fetch_optional(&self.state.db)
                .await;
        // A message wake (it has a reason) for a fire-and-forget cook: only its owner's direct
        // message gets here (see questions::wake), and it is batched for one cloud run.
        if let Ok(Some((kind, Some(project), _, payload))) = &row
            && kind == "wake"
            && payload["reason"].is_string()
            && let Some(agent) = payload["agent_id"]
                .as_str()
                .and_then(|a| a.parse::<Uuid>().ok())
        {
            let ff: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "select {}",
                crate::cloud::fire_and_forget_sql("$1")
            )))
            .bind(agent)
            .fetch_one(&self.state.db)
            .await
            .unwrap_or(false);
            // defense in depth: only a direct message on the pass starts a cloud run
            if ff && payload["reason"].as_str() == Some("pass") {
                self.cloud.push((agent, *project, payload.clone()));
            }
            return;
        }
        // a claim means the wakes worked: the next one comes at the normal pace
        if let Ok(Some((kind, _, Some(actor), _))) = &row
            && kind == "task.claimed"
        {
            self.misses.remove(&(*actor, true));
            self.misses.remove(&(*actor, false));
            self.held.retain(|(agent, _), _| agent != actor);
        }
        if let Ok(Some((kind, Some(project), actor, _))) = row
            && TRIGGERS.contains(&kind.as_str())
        {
            // The cook whose own action this was is in the middle of a turn and goes back to
            // claim_next by itself: waking it too would queue a duplicate, stale prompt.
            self.consider_for(project, actor, Via::Event).await;
        }
    }

    /// The restaurant decided not to wake a cook (or to stop): record it as a `restaurant.decided`
    /// event when it differs from what it last decided about this cook in this pot.
    #[allow(clippy::too_many_arguments)]
    async fn hold(
        &mut self,
        project: Uuid,
        agent: Uuid,
        name: &str,
        decision: &'static str,
        reason: &'static str,
        text: String,
        until: Option<Instant>,
        misses: Option<u32>,
    ) {
        if self.held.get(&(agent, project)) == Some(&(decision, reason)) {
            return;
        }
        self.held.insert((agent, project), (decision, reason));
        let mut d = crate::restaurant::Decision::new(Some(project), decision, reason, text)
            .cook(name, agent)
            .until(until.map(wall));
        if let Some(m) = misses {
            d = d.misses(m);
        }
        let done = async {
            let mut tx = self.state.db.begin().await?;
            crate::restaurant::decide(&mut tx, d).await?;
            tx.commit().await?;
            Ok::<(), crate::error::Error>(())
        }
        .await;
        if let Err(e) = done {
            tracing::warn!("recording a decision about {name}: {e}");
        }
    }

    /// Cooks with a wake target that may take ready work in `project`. `cloud_only` (the sweep)
    /// looks at cloud targets alone.
    async fn candidates(
        &self,
        project: Uuid,
        except: Option<Uuid>,
        cloud_only: bool,
    ) -> Vec<Candidate> {
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select wt.id as target_id, a.id as agent_id, a.name as agent, wt.type as typ, wt.config,
                    coalesce(wt.labels, '{{}}') as labels,
                    (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) as busy
             from wake_targets wt
             join principals a on a.id = wt.agent_id and a.disabled_at is null
             join projects pr on pr.id = $1
             where exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = a.id)
               and ($2::uuid is null or a.id <> $2)
               and (not $4 or wt.type = any($3))
               and (wt.projects is null or cardinality(wt.projects) = 0 or pr.key = any(wt.projects))
               and exists (select 1 from tasks t where t.project_id = pr.id
                      and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
                      and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
                      and (wt.labels is null or cardinality(wt.labels) = 0 or t.labels && wt.labels)
                      -- a cook scoped to some repositories is woken only for tasks about them (or about none)
                      and {scope}
                      -- not the task this cook just gave back, nor one waiting on an open PR
                      and {takeable}
                      -- a cloud cook is not woken again for a task it released until someone has touched it
                      and case when wt.type = any($3) then not ({given_back}) else true end)
               and (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) < a.max_leases",
            scope = crate::repos::scope_sql("a.id"),
            takeable = crate::service::takeable_sql("a.id"),
            given_back = crate::service::given_back_sql("a.id"),
        )))
        .bind(project)
        .bind(except)
        .bind(crate::cloud::CLOUD_TYPES)
        .bind(cloud_only)
        .fetch_all(&self.state.db)
        .await
        .unwrap_or_default()
    }

    async fn consider(&mut self, project: Uuid) {
        self.consider_for(project, None, Via::Event).await;
    }

    /// Every project with a ready task is checked for cloud cooks that were not woken for it (no
    /// event may come: a cook that finished one task of several, a target added later, a failed
    /// delivery whose retry was dropped). Failed deliveries come back through [`Self::retry`].
    async fn sweep(&mut self) {
        let projects: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select distinct t.project_id from tasks t where {}",
            crate::service::READY_SQL
        )))
        .fetch_all(&self.state.db)
        .await
        .unwrap_or_default();
        for p in projects {
            self.consider_for(p, None, Via::Sweep).await;
        }
    }

    /// Wake the cooks that may take work in `project`. A sweep (`sweep`, also used for retrying
    /// failed deliveries) only looks at cloud targets, wakes a cook while its misses are below
    /// [`SWEEP_MAX_MISSES`], and holds nothing back for later: the next sweep looks again.
    async fn consider_for(&mut self, project: Uuid, except: Option<Uuid>, via: Via) {
        let sweep = via != Via::Event;
        let cands = self.candidates(project, except, sweep).await;
        let mut by_agent: HashMap<(Uuid, bool), Vec<Candidate>> = HashMap::new();
        for c in cands {
            let cloud = crate::cloud::CLOUD_TYPES.contains(&c.typ.as_str());
            by_agent.entry((c.agent_id, cloud)).or_default().push(c);
        }
        for ((agent, cloud), mut targets) in by_agent {
            let name = targets[0].agent.clone();
            if cloud {
                // a target that asked to wait (HTTP 429) is skipped until then
                let waiting = targets
                    .iter()
                    .filter_map(|t| self.cloud_wait.get(&t.target_id).copied())
                    .filter(|w| *w > Instant::now())
                    .max();
                let n = targets.len();
                targets.retain(|t| {
                    !self
                        .cloud_wait
                        .get(&t.target_id)
                        .is_some_and(|w| *w > Instant::now())
                });
                if targets.len() < n && !sweep {
                    self.pending.insert((agent, project));
                }
                if targets.is_empty() {
                    if let Some(until) = waiting {
                        self.hold(
                            project,
                            agent,
                            &name,
                            "wake.held",
                            "retry_after",
                            format!(
                                "{name}'s cloud target asked to wait (it answered 429): no wake before {}.",
                                crate::restaurant::clock(wall(until))
                            ),
                            Some(until),
                            None,
                        )
                        .await;
                    }
                    continue;
                }
            }
            let window = if cloud {
                self.state.config.cloud_coalesce
            } else {
                LOCAL_COALESCE
            };
            let wakes = self.misses.get(&(agent, cloud)).copied().unwrap_or(0);
            if sweep && wakes >= SWEEP_MAX_MISSES {
                let (reason, by) = match via {
                    Via::Retry => ("retry", "the retry of a failed wake"),
                    _ => ("sweep", "the periodic sweep"),
                };
                self.hold(
                    project,
                    agent,
                    &name,
                    "wake.capped",
                    reason,
                    format!(
                        "{name} was woken {wakes} times in a row without claiming anything: {by} stops waking it until it claims a task or the work changes."
                    ),
                    None,
                    Some(wakes),
                )
                .await;
                continue;
            }
            let wait = backoff(window, wakes.saturating_sub(1));
            let since = self.last_wake.get(&(agent, cloud)).map(|t| t.elapsed());
            let recently = since.is_some_and(|e| e < wait);
            // Cloud agents are never woken while they hold a lease: their session is already running.
            if recently || (cloud && targets[0].busy > 0) {
                if !sweep {
                    self.pending.insert((agent, project));
                }
                if recently {
                    let until = Instant::now() + wait.saturating_sub(since.unwrap_or_default());
                    self.hold(
                        project,
                        agent,
                        &name,
                        "wake.held",
                        "backoff",
                        format!(
                            "{name} was woken a moment ago and has not claimed anything yet ({wakes} wake{} in a row): the next wake waits until {}.",
                            if wakes == 1 { "" } else { "s" },
                            crate::restaurant::clock(wall(until))
                        ),
                        Some(until),
                        Some(wakes),
                    )
                    .await;
                } else {
                    self.hold(
                        project,
                        agent,
                        &name,
                        "wake.held",
                        "busy",
                        format!(
                            "{name} holds a lease right now: its session is already running, so it is not woken."
                        ),
                        None,
                        None,
                    )
                    .await;
                }
                continue;
            }
            // one more wake in a row without a claim
            *self.misses.entry((agent, cloud)).or_default() += 1;
            self.last_wake.insert((agent, cloud), Instant::now());
            self.pending.remove(&(agent, project));
            self.held.remove(&(agent, project));
            let mut stations_busy = false;
            for t in targets {
                stations_busy |= self.wake(project, t).await;
            }
            if stations_busy {
                self.hold(
                    project,
                    agent,
                    &name,
                    "wake.held",
                    "stations_busy",
                    format!(
                        "All of {name}'s sessions here are busy: no wake is sent, they pick the work up themselves."
                    ),
                    None,
                    None,
                )
                .await;
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
            let targets: Vec<(Uuid, String, Value, Vec<String>)> = sqlx::query_as(
                "select id, type, config, coalesce(labels, '{}') from wake_targets where agent_id = $1 and type = any($2)
                   and (projects is null or cardinality(projects) = 0 or $3 = any(projects))",
            )
            .bind(agent)
            .bind(crate::cloud::CLOUD_TYPES)
            .bind(&key)
            .fetch_all(&mut *conn)
            .await
            .unwrap_or_default();
            for (target, typ, config, labels) in targets {
                if self
                    .cloud_wait
                    .get(&target)
                    .is_some_and(|t| *t > Instant::now())
                {
                    // the target asked us to wait: keep these for after
                    keep.extend(wakes.iter().map(|w| (agent, project, w.clone())));
                    continue;
                }
                // this run's station, opened first so the text can name it
                let run =
                    match crate::stations::open_run(&mut conn, agent, project, Some(target), &typ)
                        .await
                    {
                        Ok(r) => Some(r),
                        Err(e) => {
                            tracing::warn!("cloud run station for {name}: {e}");
                            None
                        }
                    };
                let text = crate::cloud::messages_text(
                    &mut conn,
                    agent,
                    &name,
                    project,
                    &key,
                    &wakes,
                    &labels,
                    run.as_ref().map(|r| r.reference.as_str()),
                )
                .await;
                let res = crate::cloud::deliver(
                    &self.state,
                    Some(target),
                    &typ,
                    &config,
                    &name,
                    &key,
                    &text,
                    run.as_ref(),
                )
                .await;
                let deferred = matches!(&res, Err(f) if f.retry_after.is_some());
                if let Err(f) = &res
                    && let Some(wait) = f.retry_after
                {
                    self.cloud_wait.insert(target, deadline(wait));
                    keep.extend(wakes.iter().map(|w| (agent, project, w.clone())));
                }
                if !deferred {
                    // this run is the wake: the sweep must not start a second one within the window
                    self.last_wake.insert((agent, true), Instant::now());
                }
                record_delivery(
                    &mut conn,
                    agent,
                    project,
                    &name,
                    &key,
                    &typ,
                    wakes.len(),
                    run.as_ref(),
                    res,
                )
                .await;
            }
        }
        self.cloud.extend(keep);
    }

    /// Failed work wakes whose wait has passed are looked at again, capped like a sweep.
    async fn flush_retry(&mut self) {
        let now = Instant::now();
        let due: Vec<(Uuid, Uuid)> = self
            .retry
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(k, _)| *k)
            .collect();
        let mut projects: HashSet<Uuid> = HashSet::new();
        for k in due {
            self.retry.remove(&k);
            projects.insert(k.1);
        }
        for p in projects {
            self.consider_for(p, None, Via::Retry).await;
        }
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

    /// Deliver one wake. True when it was held back because every session of the cook is busy.
    async fn wake(&self, project: Uuid, c: Candidate) -> bool {
        let key: String = match sqlx::query_scalar("select key from projects where id = $1")
            .bind(project)
            .fetch_one(&self.state.db)
            .await
        {
            Ok(k) => k,
            Err(_) => return false,
        };
        let mut conn = match self.state.db.acquire().await {
            Ok(c) => c,
            Err(_) => return false,
        };
        match c.typ.as_str() {
            "relay" => {
                // Wake an idle station (that very session) when the cook has any in this pot;
                // when all of them are busy, they pick the work up themselves.
                let station =
                    match crate::stations::pick_for_wake(&mut conn, c.agent_id, project).await {
                        Ok((true, None)) => return true,
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
                    Some(crate::state::RESTAURANT),
                    json!({"agent": c.agent, "agent_id": c.agent_id, "project": key, "station": station}),
                )
                .await;
            }
            "webhook" | "github-comment" | "claude-routine" | "coder" | "devin" => {
                let state = self.state.clone();
                let (typ, config, agent, agent_id, target) = (
                    c.typ.clone(),
                    c.config.clone(),
                    c.agent.clone(),
                    c.agent_id,
                    c.target_id,
                );
                let key2 = key.clone();
                let labels = c.labels.clone();
                let failed = self.failed.clone();
                tokio::spawn(async move {
                    let Ok(mut conn) = state.db.acquire().await else {
                        let _ = failed.send((target, agent_id, project, None));
                        return;
                    };
                    // this run's station, opened first so the text can name it
                    let run = match crate::stations::open_run(
                        &mut conn,
                        agent_id,
                        project,
                        Some(target),
                        &typ,
                    )
                    .await
                    {
                        Ok(r) => Some(r),
                        Err(e) => {
                            tracing::warn!("cloud run station for {agent}: {e}");
                            None
                        }
                    };
                    let text = crate::cloud::work_text(
                        &mut conn,
                        agent_id,
                        &agent,
                        project,
                        &key2,
                        &labels,
                        run.as_ref().map(|r| r.reference.as_str()),
                    )
                    .await;
                    let res = crate::cloud::deliver(
                        &state,
                        Some(target),
                        &typ,
                        &config,
                        &agent,
                        &key2,
                        &text,
                        run.as_ref(),
                    )
                    .await;
                    if let Err(f) = &res {
                        let _ = failed.send((target, agent_id, project, f.retry_after));
                    }
                    record_delivery(
                        &mut conn,
                        agent_id,
                        project,
                        &agent,
                        &key2,
                        &typ,
                        0,
                        run.as_ref(),
                        res,
                    )
                    .await;
                });
            }
            _ => {}
        }
        false
    }
}

/// Note a cloud delivery in the feed (with the session a Claude routine started).
async fn record_delivery(
    c: &mut PgConnection,
    agent_id: Uuid,
    project: Uuid,
    agent: &str,
    key: &str,
    typ: &str,
    messages: usize,
    run: Option<&crate::stations::RunStation>,
    res: Result<crate::cloud::Delivered, crate::cloud::Failed>,
) {
    // the station the run ended up with (a follow-up to a gone run starts a new one)
    let mut settled: Option<(Uuid, Option<String>, Option<String>)> = None;
    let (kind, payload) = match res {
        Ok(d) => {
            crate::usage::record_in(
                c,
                agent_id,
                Some(project),
                crate::usage::WAKE,
                &format!("cloud:{typ}"),
                d.chars,
            )
            .await;
            let station_ref: Option<String> = match (d.station, run) {
                (Some(id), _) => sqlx::query_scalar("select ref from stations where id = $1")
                    .bind(id)
                    .fetch_optional(&mut *c)
                    .await
                    .ok()
                    .flatten(),
                (None, Some(r)) => Some(r.reference.clone()),
                _ => None,
            };
            // a follow-up whose run was gone started a new one: the old station is over
            if let (Some(_), Some(r)) = (d.station, run) {
                let _ = crate::stations::end_if_idle(&mut *c, r.id).await;
            }
            if let Some(id) = d.station.or(run.map(|r| r.id)) {
                settled = Some((id, d.session_id.clone(), d.session_url.clone()));
            }
            (
                "wake.delivered",
                json!({"agent": agent, "project": key, "type": typ, "status": d.status,
                   "session_url": d.session_url, "messages": messages, "station": station_ref}),
            )
        }
        Err(f) => {
            if let Some(r) = run {
                if f.retry_after.is_some() && !r.reused {
                    // told to wait: nothing was sent, so the station was never needed
                    let _ = crate::stations::drop_run(&mut *c, r.id).await;
                } else {
                    // the run may have started all the same (a timeout, an answer without an id):
                    // it keeps its station, which ends unless it holds something; a reused one that
                    // was reopened for this wake goes back to ended
                    let _ = crate::stations::end_if_idle(&mut *c, r.id).await;
                }
            }
            tracing::warn!("cloud wake for {agent} ({typ}) failed: {}", f.error);
            (
                "wake.failed",
                json!({"agent": agent, "project": key, "type": typ, "error": f.error,
                       "retry_in_s": f.retry_after.map(|d| d.as_secs()), "messages": messages}),
            )
        }
    };
    let seq = emit(
        c,
        Some(project),
        None,
        kind,
        Some(crate::state::RESTAURANT),
        payload,
    )
    .await
    .ok();
    if let Some((id, session, url)) = settled
        && let Err(e) =
            crate::stations::settle_run(&mut *c, id, session.as_deref(), url.as_deref(), seq).await
    {
        tracing::warn!("settling the station of a cloud run: {e}");
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    #[test]
    fn wakes_without_a_claim_back_off_to_half_an_hour() {
        let b = Duration::from_secs(30);
        assert_eq!(backoff(b, 0), b);
        assert_eq!(backoff(b, 1), Duration::from_secs(60));
        assert_eq!(backoff(b, 3), Duration::from_secs(240));
        assert_eq!(backoff(b, 6), Duration::from_secs(30 * 60));
        assert_eq!(backoff(b, 40), Duration::from_secs(30 * 60));
        assert_eq!(
            backoff(Duration::from_secs(300), 2),
            Duration::from_secs(1200)
        );
    }
}
