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
pub const WAKE_PROMPT: &str = "New work is ready in {project}. Call claim_next with project {project}, finish the task, then call it again until it returns none.";
pub const CLOUD_WAKE_PROMPT: &str = "New work is ready in {project}. Call claim_next with project {project}, finish the task, and call complete or release before your session ends. Call claim_next again until it returns none.";

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
            n += 1;
            if n % 240 == 1
                && let Err(e) = retain(&s).await
            {
                tracing::warn!("retention: {e}");
            }
        }
    };
    tokio::select! {
        _ = reaper => {},
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

/// Keep 90 days of events, rolling older ones into per-day counts.
pub async fn retain(state: &AppState) -> Result<(), sqlx::Error> {
    sqlx::query(
        "with old as (delete from events where at < now() - interval '90 days' and project_id is not null returning project_id, kind, at::date as day),
              agg as (select day, project_id, kind, count(*) as n from old group by 1,2,3)
         insert into event_daily_counts (day, project_id, kind, count) select day, project_id, kind, n from agg
         on conflict (day, project_id, kind) do update set count = event_daily_counts.count + excluded.count",
    )
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
                _ = tick.tick() => self.flush_pending().await,
            }
        }
    }

    async fn on_event(&mut self, seq: i64) {
        let row: Result<Option<(String, Option<Uuid>)>, _> =
            sqlx::query_as("select kind, project_id from events where seq = $1")
                .bind(seq)
                .fetch_optional(&self.state.db)
                .await;
        if let Ok(Some((kind, Some(project)))) = row
            && TRIGGERS.contains(&kind.as_str())
        {
            self.consider(project).await;
        }
    }

    async fn candidates(&self, project: Uuid) -> Vec<Candidate> {
        sqlx::query_as(
            "select wt.id as target_id, a.id as agent_id, a.name as agent, a.runtime, wt.type as typ, wt.config,
                    (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) as busy
             from wake_targets wt
             join principals a on a.id = wt.agent_id and a.disabled_at is null
             join projects pr on pr.id = $1
             where exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = a.id)
               and (wt.projects is null or cardinality(wt.projects) = 0 or pr.key = any(wt.projects))
               and exists (select 1 from tasks t where t.project_id = pr.id
                      and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
                      and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
                      and (wt.labels is null or cardinality(wt.labels) = 0 or t.labels && wt.labels))
               and (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()) < a.max_leases",
        )
        .bind(project)
        .fetch_all(&self.state.db)
        .await
        .unwrap_or_default()
    }

    async fn consider(&mut self, project: Uuid) {
        let cands = self.candidates(project).await;
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
            "webhook" | "github-comment" => {
                let state = self.state.clone();
                let cloud = c.runtime.as_deref() == Some("cloud");
                let (typ, config, agent) = (c.typ.clone(), c.config.clone(), c.agent.clone());
                let key2 = key.clone();
                tokio::spawn(async move {
                    let res = deliver(&state, &typ, &config, &agent, &key2, cloud).await;
                    let (kind, payload) = match res {
                        Ok(status) => (
                            "wake.delivered",
                            json!({"agent": agent, "project": key2, "type": typ, "status": status}),
                        ),
                        Err(e) => {
                            tracing::warn!("wake delivery to {agent} failed: {e}");
                            (
                                "wake.failed",
                                json!({"agent": agent, "project": key2, "type": typ}),
                            )
                        }
                    };
                    if let Ok(mut c) = state.db.acquire().await {
                        let _ = emit(&mut c, Some(project), None, kind, None, payload).await;
                    }
                });
            }
            _ => {}
        }
    }
}

/// POST the wake. The text is always a fixed template, never task content.
async fn deliver(
    state: &AppState,
    typ: &str,
    config: &Value,
    agent: &str,
    project: &str,
    cloud: bool,
) -> Result<u16, String> {
    let template = if cloud {
        CLOUD_WAKE_PROMPT
    } else {
        WAKE_PROMPT
    };
    let prompt = template.replace("{project}", project);
    let (url, body, mut req_headers): (String, Value, Vec<(String, String)>) = if typ == "webhook" {
        let url = config["url"].as_str().ok_or("missing url")?.to_string();
        let body = config
            .get("body")
            .cloned()
            .unwrap_or_else(|| json!({"text": prompt, "agent": agent, "project": project}));
        let mut h = vec![];
        if let Some(t) = config["token"].as_str() {
            h.push(("authorization".to_string(), format!("Bearer {t}")));
        }
        for (k, v) in config["headers"].as_object().into_iter().flatten() {
            if let Some(v) = v.as_str() {
                h.push((k.clone(), v.to_string()));
            }
        }
        (url, body, h)
    } else {
        let repo = config["repo"].as_str().ok_or("missing repo")?;
        let issue = config["issue"].as_i64().ok_or("issue must be a number")?;
        let token = config["token"].as_str().ok_or("missing token")?;
        let base = config["api_url"]
            .as_str()
            .unwrap_or("https://api.github.com");
        let mention = config["mention"].as_str().unwrap_or("@codex");
        (
            format!("{base}/repos/{repo}/issues/{issue}/comments"),
            json!({"body": format!("{mention} {prompt}")}),
            vec![
                ("authorization".to_string(), format!("Bearer {token}")),
                (
                    "accept".to_string(),
                    "application/vnd.github+json".to_string(),
                ),
            ],
        )
    };
    let mut req = state.http.post(&url).json(&body);
    for (k, v) in req_headers.drain(..) {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("HTTP {status}"))
    }
}
