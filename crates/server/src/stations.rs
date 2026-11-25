//! Stations: one agent session (OpenCode, Claude Code or Codex) registered for a cook in a pot.
//! `corntchap-cli station scan` registers the sessions found in a checkout and its worktrees;
//! the dispatcher wakes an idle station (that very session) instead of "the cook", and a lease
//! taken with `station` records which session holds the task.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    admin,
    error::{Error, Result, invalid},
    questions,
    service::{project_by_key, task_by_ref},
    state::Principal,
};

pub const RUNTIMES: &[&str] = &[
    "opencode",
    "claude-code",
    "codex",
    "copilot",
    "antigravity",
    "other",
];

const STATION_JSON: &str = r#"jsonb_build_object(
  'id', st.ref, 'cook', a.name, 'project', pr.key, 'runtime', st.runtime, 'session', st.session_ref,
  'directory', st.directory, 'repo', st.repo, 'confined', st.confined, 'label', st.label, 'main', st.main, 'created_at', st.created_at, 'last_woken_at', st.last_woken_at,
  'kind', st.kind, 'session_url', st.session_url, 'ended_at', st.ended_at,
  'target_type', (select wt.type from wake_targets wt where wt.id = st.target_id),
  'holding', coalesce((select jsonb_agg(pr.key || '-' || t.number order by t.number) from tasks t
      where t.station_id = st.id and t.status = 'claimed' and t.lease_expires_at > now()), '[]'::jsonb),
  'waiting', coalesce((select jsonb_agg(pr.key || '-' || t.number order by t.number) from tasks t
      where t.station_id = st.id and t.status = 'blocked' and t.reserved_for is not null), '[]'::jsonb))"#;

fn new_ref() -> String {
    let u = Uuid::new_v4().simple().to_string();
    format!("st_{}", &u[..10])
}

/// The cook a station is registered for: the caller itself (a cook registering its own session),
/// or a cook the caller manages (people; cooks granted `manage` act as their owner).
pub(crate) async fn cook_for(db: &PgPool, p: &Principal, cook: Option<&str>) -> Result<Principal> {
    let name = cook
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty());
    if p.is_agent() && name.as_deref().is_none_or(|n| n == p.name) {
        return Ok(p.clone());
    }
    let Some(name) = name else {
        return invalid("cook is required: the cook (agent) this session works as");
    };
    let who = admin::acting(db, p).await?;
    let row: Option<(Uuid, String, Option<Uuid>, crate::state::Role, i64)> = sqlx::query_as(
        "select id, name, owner_id, role, max_leases from principals where name = $1 and kind = 'agent' and disabled_at is null",
    )
    .bind(&name)
    .fetch_optional(db)
    .await?;
    match row {
        Some((id, name, owner, role, max_leases))
            if who.is_super_admin() || owner == Some(who.id) =>
        {
            Ok(Principal {
                id,
                kind: "agent".into(),
                name,
                owner_id: owner,
                role,
                max_leases,
                runtime: None,
                agent_kind: None,
                token_id: None,
                grants: vec![],
            })
        }
        _ => Err(Error::NotFound(format!(
            "no cook named {name} that you manage"
        ))),
    }
}

pub struct NewStation {
    pub cook: Option<String>,
    pub project: String,
    pub runtime: String,
    pub session: String,
    pub directory: Option<String>,
    pub label: Option<String>,
    /// The pot's repository (alias) the folder is a checkout of, when the CLI matched one.
    pub repo: Option<String>,
    /// The session cannot leave its folder (a sandbox, a harness locked to one directory):
    /// `claim_next` from it hands out only tasks about its `repo` or about none. `None` keeps what
    /// the station had (a new station is not confined).
    pub confined: Option<bool>,
}

/// Register (or move) a session as a station of a cook in a pot. The same session registered
/// again keeps its station id (and its main flag) and takes the new directory and label; moved to
/// another pot it stops being the main station of the old one.
pub async fn register(db: &PgPool, p: &Principal, n: NewStation) -> Result<Value> {
    let runtime = n.runtime.trim().to_lowercase();
    if !RUNTIMES.contains(&runtime.as_str()) {
        return invalid(format!(
            "runtime {runtime:?}: one of {}",
            RUNTIMES.join(", ")
        ));
    }
    let session = n.session.trim();
    if session.is_empty() || session.len() > 200 {
        return invalid("session is the runtime's session id (1-200 characters)");
    }
    let cook = cook_for(db, p, n.cook.as_deref()).await?;
    let mut tx = db.begin().await?;
    // the cook itself must be able to see the pot (member of its team)
    let pr = project_by_key(&mut tx, &cook, &n.project.to_uppercase())
        .await
        .map_err(|_| {
            Error::NotFound(format!(
                "{} cannot see pot {}: add the cook to the pot's team first",
                cook.name,
                n.project.to_uppercase()
            ))
        })?;
    let repo = match n.repo.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => crate::repos::check_aliases(&mut tx, pr.id, &pr.key, &[r.to_string()])
            .await?
            .pop(),
        None => None,
    };
    let id: Uuid = sqlx::query_scalar(
        "insert into stations (ref, agent_id, project_id, runtime, session_ref, directory, label, created_by, repo, confined)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,coalesce($10, false))
         on conflict (agent_id, runtime, session_ref) do update
           set project_id = excluded.project_id, directory = coalesce(excluded.directory, stations.directory),
               label = coalesce(excluded.label, stations.label),
               repo = case when excluded.project_id = stations.project_id then coalesce(excluded.repo, stations.repo) else excluded.repo end,
               confined = coalesce($10, stations.confined),
               main = stations.main and stations.project_id = excluded.project_id
         returning id",
    )
    .bind(new_ref())
    .bind(cook.id)
    .bind(pr.id)
    .bind(&runtime)
    .bind(session)
    .bind(n.directory.as_deref().map(str::trim).filter(|d| !d.is_empty()))
    .bind(n.label.as_deref().map(str::trim).filter(|l| !l.is_empty()))
    .bind(p.id)
    .bind(repo)
    .bind(n.confined)
    .fetch_one(&mut *tx)
    .await?;
    let mut out = one(&mut tx, id).await?;
    // how we work here: the start of the pot's guide comes with the registration
    out["guide"] = json!(crate::what_now::guide_excerpt(&mut tx, pr.id).await?);
    tx.commit().await?;
    Ok(out)
}

pub struct Start {
    pub cook: Option<String>,
    pub project: String,
    pub task: Option<String>,
    pub prompt: Option<String>,
    pub directory: Option<String>,
    pub label: Option<String>,
}

/// Runtimes the relay can start a fresh session for: it needs to choose the session id up front.
pub const STARTABLE: &[&str] = &["claude-code"];

/// Start a new session: a new station with a session id chosen here, woken at once. The cook's
/// relay sees a session it has never run (no transcript on disk) and starts it with that id
/// (`claude -p --session-id`). The folder must be one where the cook already has a station in the
/// pot, so nobody can make a relay run an agent in an arbitrary folder.
pub async fn start(db: &PgPool, p: &Principal, s: Start) -> Result<Value> {
    let cook = cook_for(db, p, s.cook.as_deref()).await?;
    let kind: Option<String> =
        sqlx::query_scalar("select agent_kind from principals where id = $1")
            .bind(cook.id)
            .fetch_one(db)
            .await?;
    let runtime = kind.unwrap_or_default();
    if !STARTABLE.contains(&runtime.as_str()) {
        return invalid(format!(
            "{} is a {} cook: only Claude Code sessions can be started for now (others: start the agent yourself, then `corntchap-cli station scan`)",
            cook.name,
            if runtime.is_empty() { "?" } else { &runtime }
        ));
    }
    let key = s.project.trim().to_uppercase();
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, &cook, &key).await.map_err(|_| {
        Error::NotFound(format!(
            "{} cannot see pot {key}: add the cook to the pot's team first",
            cook.name
        ))
    })?;
    let dirs: Vec<String> = sqlx::query_scalar(
        "select directory from stations where agent_id = $1 and project_id = $2 and kind = 'session' and directory is not null
         group by directory order by bool_or(main) desc, max(coalesce(last_woken_at, created_at)) desc",
    )
    .bind(cook.id)
    .bind(pr.id)
    .fetch_all(&mut *tx)
    .await?;
    let want = s
        .directory
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let directory = match want {
        Some(d) if dirs.iter().any(|x| x == d) => d.to_string(),
        Some(d) => {
            return invalid(format!(
                "{} has no station in {d} for {key}: start it in one of {}",
                cook.name,
                if dirs.is_empty() {
                    "(none yet)".into()
                } else {
                    dirs.join(", ")
                }
            ));
        }
        None => match dirs.first() {
            Some(d) => d.clone(),
            None => {
                return invalid(format!(
                    "{} has no station with a folder in {key} yet: run `corntchap-cli station start` (or `station scan`) once in the repo on its machine",
                    cook.name
                ));
            }
        },
    };
    let task = match s.task.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = task_by_ref(&mut tx, &cook, t).await?;
            if t.project_id != pr.id {
                return invalid(format!("task {}-{} is not in pot {key}", t.key, t.number));
            }
            Some(t)
        }
        None => None,
    };
    let prompt = s.prompt.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if prompt.is_some_and(|t| t.len() > 20_000) {
        return invalid("prompt is at most 20000 characters");
    }
    let task_ref = task.as_ref().map(|t| format!("{}-{}", t.key, t.number));
    let base = directory
        .rsplit(['/', '\\'])
        .find(|x| !x.is_empty())
        .unwrap_or(&directory)
        .to_string();
    let label = s
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .unwrap_or_else(|| match &task_ref {
            Some(t) => format!("{base} ({t})"),
            None => format!("{base} (new)"),
        });
    let session = Uuid::new_v4().to_string();
    let id: Uuid = sqlx::query_scalar(
        "insert into stations (ref, agent_id, project_id, runtime, session_ref, directory, label, created_by, repo)
         values ($1,$2,$3,$4,$5,$6,$7,$8,
           (select repo from stations where agent_id = $2 and project_id = $3 and directory = $6 and repo is not null limit 1))
         returning id",
    )
    .bind(new_ref())
    .bind(cook.id)
    .bind(pr.id)
    .bind(&runtime)
    .bind(&session)
    .bind(&directory)
    .bind(&label)
    .bind(p.id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("update stations set last_woken_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let who: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
    let text = start_text(&who, &key, task_ref.as_deref(), prompt);
    questions::wake(
        &mut tx,
        pr.id,
        &key,
        cook.id,
        Some(id),
        task.as_ref().map(|t| t.id),
        &text,
        "start",
    )
    .await?;
    let out = one(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

/// The first prompt of a started session. The wake adds the station hint and pantry news.
pub fn start_text(who: &str, pot: &str, task: Option<&str>, prompt: Option<&str>) -> String {
    let mut t = format!(
        "{who} started this session for you in corntchap pot {pot}. Use the corntchap MCP tools: start with what_now (project {pot}), which briefs you."
    );
    match task {
        Some(r) => t.push_str(&format!(
            " Your task is {r}: read it (get_task), claim it (claim_task with your station), work it, tick its items and complete it (with `learned`: what the next agent must know, or \"nothing new\"). If it is taken, say so on the pass and take the next ready one (claim_next)."
        )),
        None if prompt.is_none() => t.push_str(
            " Take the next ready task (claim_next with your station), work it and complete it (with `learned`); when none is ready, stop.",
        ),
        None => {}
    }
    if let Some(p) = prompt {
        t.push_str(&format!("\n\n{p}"));
    }
    t
}

async fn one(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {STATION_JSON} from stations st join principals a on a.id = st.agent_id
         join projects pr on pr.id = st.project_id where st.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// Stations the caller can see: a cook its own; a person those of the cooks they own (admins:
/// all); optionally one pot or one cook. Without a `kind`: the sessions and the cloud runs that
/// have not ended; the ended runs only with `kind = cloud-run` (newest first, at most `limit`,
/// 100 by default).
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: Option<&str>,
    cook: Option<&str>,
    kind: Option<&str>,
    live: Option<bool>,
    limit: Option<i64>,
) -> Result<Value> {
    if let Some(k) = kind
        && !matches!(k, "session" | "cloud-run")
    {
        return invalid("kind is session or cloud-run");
    }
    let who = admin::acting_or_self(db, p).await?;
    let sql = format!(
        "select {STATION_JSON} from stations st join principals a on a.id = st.agent_id
         join projects pr on pr.id = st.project_id
         where ($1 or a.id = $2 or a.owner_id = $2
                or ($5 and exists (select 1 from team_members m
                                   where m.team_id = pr.team_id and m.principal_id = $2 and m.role = 'owner')))
           and ($3::text is null or pr.key = $3)
           and ($4::text is null or a.name = $4)
           and (case when $6::text is null then (st.kind = 'session' or st.ended_at is null) else st.kind = $6 end)
           and ($7::bool is null or $7 = (st.ended_at is null))
         order by case when $6::text = 'cloud-run' then st.created_at end desc,
                  pr.key, a.name, st.created_at
         limit $8"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(who.is_super_admin())
        .bind(who.id)
        .bind(project.map(|k| k.trim().to_uppercase()))
        .bind(cook.map(|c| c.trim().to_lowercase()))
        .bind(who.can_orchestrate())
        .bind(kind)
        .bind(live)
        .bind(match kind {
            Some("cloud-run") => limit.unwrap_or(100).clamp(1, 500),
            _ => i64::MAX,
        })
        .fetch_all(db)
        .await?;
    Ok(Value::Array(rows))
}

/// Make a station its cook's main one in its pot (the session that answers messages and mentions),
/// or stop it being the main one. The cook itself, its owner (or a cook acting for them) and
/// admins may.
pub async fn set_main(db: &PgPool, p: &Principal, station: &str, main: bool) -> Result<Value> {
    let who = admin::acting_or_self(db, p).await?;
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        "select st.id, st.agent_id, st.project_id, st.kind from stations st join principals a on a.id = st.agent_id
         where st.ref = $1 and ($2 or a.id = $3 or a.owner_id = $3)
         for update of st",
    )
    .bind(station.trim())
    .bind(who.is_super_admin())
    .bind(who.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, agent, project, kind)) = row else {
        return Err(Error::NotFound(format!("no station {station}")));
    };
    if kind != "session" {
        return invalid(format!(
            "{station} is a cloud run: it ends with the run and cannot be the main station"
        ));
    }
    if main {
        sqlx::query(
            "update stations set main = false where agent_id = $1 and project_id = $2 and main and id <> $3",
        )
        .bind(agent)
        .bind(project)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("update stations set main = $2 where id = $1")
        .bind(id)
        .bind(main)
        .execute(&mut *tx)
        .await?;
    let out = one(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn remove(db: &PgPool, p: &Principal, station: &str) -> Result<Value> {
    let who = admin::acting_or_self(db, p).await?;
    let n = sqlx::query(
        "delete from stations st using principals a
         where st.ref = $1 and a.id = st.agent_id and ($2 or a.id = $3 or a.owner_id = $3)",
    )
    .bind(station.trim())
    .bind(who.is_super_admin())
    .bind(who.id)
    .execute(db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(Error::NotFound(format!("no station {station}")));
    }
    Ok(json!({"ok": true}))
}

/// What a call that names a station is about, to pick the cloud run it belongs to when it names
/// none (see [`for_call`]).
#[derive(Default, Clone, Copy)]
pub struct Call {
    /// The call only reads: it never reopens an ended run.
    pub read_only: bool,
    /// The task it concerns: when that task is held by one of the cook's live runs, that run.
    pub task: Option<Uuid>,
}

/// The station a claim is made from: it must be one of the caller's, in the task's pot. An agent
/// whose cook is fire and forget may leave it out, or name one that does not exist: the call is
/// attributed to its most recent cloud run in the pot (a live one first), since a cloud run is
/// started with no way to choose a session. Naming (or falling back to) a cloud run that already
/// ended reopens it.
pub async fn for_claim(
    c: &mut PgConnection,
    p: &Principal,
    station: Option<&str>,
    project: Uuid,
) -> Result<Option<Uuid>> {
    for_call(c, p, station, project, Call::default()).await
}

pub async fn for_call(
    c: &mut PgConnection,
    p: &Principal,
    station: Option<&str>,
    project: Uuid,
    call: Call,
) -> Result<Option<Uuid>> {
    let named = station.map(str::trim).filter(|s| !s.is_empty());
    let row: Option<(Uuid, Uuid, Uuid, String)> = match named {
        Some(s) => {
            sqlx::query_as("select id, agent_id, project_id, kind from stations where ref = $1")
                .bind(s)
                .fetch_optional(&mut *c)
                .await?
        }
        None => None,
    };
    match (named, row) {
        (Some(_), Some((id, agent, pid, kind))) if agent == p.id && pid == project => {
            if kind == "cloud-run" && !call.read_only {
                reopen(c, id).await?;
            }
            Ok(Some(id))
        }
        (Some(s), Some((_, agent, _, _))) if agent == p.id => Err(Error::Invalid(format!(
            "station {s} is registered for another pot"
        ))),
        (Some(s), Some(_)) => Err(Error::NotFound(format!("{} has no station {s}", p.name))),
        (named, _) => {
            // no station, or one that is not there (a run's station that was removed): a fire and
            // forget cook's run cannot choose, so it falls back; anyone else is told
            if !p.is_agent() || !crate::cloud::is_fire_and_forget(&mut *c, p.id).await? {
                return match named {
                    Some(s) => Err(Error::NotFound(format!("{} has no station {s}", p.name))),
                    None => Ok(None),
                };
            }
            let mut id: Option<Uuid> = None;
            if let Some(t) = call.task {
                id = sqlx::query_scalar(
                    "select st.id from tasks t join stations st on st.id = t.station_id
                     where t.id = $1 and st.agent_id = $2 and st.project_id = $3
                       and st.kind = 'cloud-run' and st.ended_at is null",
                )
                .bind(t)
                .bind(p.id)
                .bind(project)
                .fetch_optional(&mut *c)
                .await?;
            }
            if id.is_none() {
                // a read picks a live run or none; a write may reopen the latest ended one
                let sql = format!(
                    "select id from stations where agent_id = $1 and project_id = $2 and kind = 'cloud-run'
                     {} order by (ended_at is null) desc, coalesce(last_woken_at, created_at) desc, created_at desc limit 1",
                    if call.read_only { "and ended_at is null" } else { "" }
                );
                id = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                    .bind(p.id)
                    .bind(project)
                    .fetch_optional(&mut *c)
                    .await?;
            }
            if let Some(id) = id
                && !call.read_only
            {
                reopen(c, id).await?;
            }
            Ok(id)
        }
    }
}

async fn reopen(c: &mut PgConnection, id: Uuid) -> Result<()> {
    sqlx::query("update stations set ended_at = null where id = $1 and ended_at is not null")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}

/// A cloud run's station, opened before the run is started so its ref goes in the wake text.
pub struct RunStation {
    pub id: Uuid,
    pub reference: String,
    /// The cook and pot, to open a replacement when the run turns out to be gone.
    pub agent: Uuid,
    pub project: Uuid,
    pub target: Option<Uuid>,
    pub typ: String,
    /// It is the station of a run that goes on (Coder, Devin follow-ups): kept when a delivery fails.
    pub reused: bool,
}

/// Open the station of one cloud run. A Coder or Devin target follows up the same run for a pot, so
/// its next wake reuses that run's station (and reopens it); every other wake is a new run and
/// gets a new station, with a placeholder session until [`settle_run`] knows the run's own id.
pub async fn open_run(
    c: &mut PgConnection,
    agent: Uuid,
    project: Uuid,
    target: Option<Uuid>,
    typ: &str,
) -> Result<RunStation> {
    if let (Some(t), "coder" | "devin") = (target, typ) {
        let known: Option<(Uuid, String)> = sqlx::query_as(
            "select st.id, st.ref from coder_runs cr join projects pr on pr.key = cr.project
             join stations st on st.id = cr.station_id
             where cr.target_id = $1 and pr.id = $2 and st.agent_id = $3 and st.project_id = $2",
        )
        .bind(t)
        .bind(project)
        .bind(agent)
        .fetch_optional(&mut *c)
        .await?;
        if let Some((id, reference)) = known {
            sqlx::query("update stations set ended_at = null, last_woken_at = now() where id = $1")
                .bind(id)
                .execute(&mut *c)
                .await?;
            return Ok(RunStation {
                id,
                reference,
                agent,
                project,
                target,
                typ: typ.into(),
                reused: true,
            });
        }
    }
    new_run(c, agent, project, target, typ).await
}

/// A new cloud-run station (see [`open_run`]).
pub async fn new_run(
    c: &mut PgConnection,
    agent: Uuid,
    project: Uuid,
    target: Option<Uuid>,
    typ: &str,
) -> Result<RunStation> {
    let reference = new_ref();
    let label = format!(
        "{typ} run {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
    let id: Uuid = sqlx::query_scalar(
        "insert into stations (ref, agent_id, project_id, runtime, session_ref, label, kind, target_id, last_woken_at)
         values ($1,$2,$3,$4,$5,$6,'cloud-run',$7, now()) returning id",
    )
    .bind(&reference)
    .bind(agent)
    .bind(project)
    .bind(typ)
    .bind(format!("run-{reference}"))
    .bind(label)
    .bind(target)
    .fetch_one(c)
    .await?;
    Ok(RunStation {
        id,
        reference,
        agent,
        project,
        target,
        typ: typ.into(),
        reused: false,
    })
}

/// A session page a station may link to: https, and a host its service owns (a Claude routine's
/// run lives on claude.ai, a Devin session on devin.ai). Anything else (a webhook's answer, a
/// look-alike host) is not stored: the answer of an endpoint is not to be trusted with a link
/// the console renders.
pub fn safe_session_url(typ: &str, url: &str) -> Option<String> {
    let domain = match typ {
        "claude-routine" => "claude.ai",
        "devin" => "devin.ai",
        _ => return None,
    };
    let u = url::Url::parse(url.trim()).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    (host == domain || host.ends_with(&format!(".{domain}"))).then(|| u.to_string())
}

/// The run started: its own session id and page replace the placeholder, and the label becomes the
/// page's address (the console shows a short link anyway). The page is kept only when
/// [`safe_session_url`] accepts it for the run's target type.
pub async fn settle_run(
    c: &mut PgConnection,
    id: Uuid,
    session_id: Option<&str>,
    url: Option<&str>,
    seq: Option<i64>,
) -> Result<()> {
    let typ: Option<String> =
        sqlx::query_scalar("select runtime from stations where id = $1 and kind = 'cloud-run'")
            .bind(id)
            .fetch_optional(&mut *c)
            .await?;
    let Some(typ) = typ else { return Ok(()) };
    let url = url.and_then(|u| safe_session_url(&typ, u));
    sqlx::query(
        "update stations set session_ref = coalesce($2, session_ref), session_url = coalesce($3, session_url),
            label = coalesce($3, label), delivery_seq = coalesce($4, delivery_seq)
         where id = $1 and kind = 'cloud-run'",
    )
    .bind(id)
    .bind(session_id)
    .bind(url)
    .bind(seq)
    .execute(c)
    .await?;
    Ok(())
}

/// A delivery that cannot have started a run (the target asked to wait, nothing was sent): the
/// station opened for it goes away again, unless something already points at it, in which case it
/// just ends. Never deletes a station a run may be using.
pub async fn drop_run(c: &mut PgConnection, id: Uuid) -> Result<()> {
    sqlx::query(
        "delete from stations st
         where st.id = $1 and st.kind = 'cloud-run'
           and not exists (select 1 from tasks x where x.station_id = st.id)
           and not exists (select 1 from comments x where x.station_id = st.id)
           and not exists (select 1 from pass_messages x where x.station_id = st.id or x.to_station_id = st.id)
           and not exists (select 1 from questions x where x.station_id = st.id)
           and not exists (select 1 from forge_prs x where x.station_id = st.id)
           and not exists (select 1 from coder_runs x where x.station_id = st.id)",
    )
    .bind(id)
    .execute(&mut *c)
    .await?;
    end_if_idle(c, id).await
}

/// A cloud run is over when its cook holds nothing for it: no live lease and no task parked on a
/// question. Called when a lease ends.
pub async fn end_if_idle(c: &mut PgConnection, id: Uuid) -> Result<()> {
    sqlx::query(
        "update stations st set ended_at = now()
         where st.id = $1 and st.kind = 'cloud-run' and st.ended_at is null
           and not exists (select 1 from tasks t where t.station_id = st.id
                and ((t.status = 'claimed' and t.lease_expires_at > now())
                     or (t.status = 'blocked' and t.reserved_for is not null)))",
    )
    .bind(id)
    .execute(c)
    .await?;
    Ok(())
}

/// Cloud runs that went quiet without releasing anything (the run died, or went on after its
/// lease was gone): no live lease, nothing parked, and no comment or pass from them for the
/// service's stall time.
pub async fn end_stale_runs(db: &PgPool) -> Result<u64> {
    let sql = format!(
        "update stations st set ended_at = now()
         where st.kind = 'cloud-run' and st.ended_at is null
           and not exists (select 1 from tasks t where t.station_id = st.id
                and ((t.status = 'claimed' and t.lease_expires_at > now())
                     or (t.status = 'blocked' and t.reserved_for is not null)))
           and greatest(st.created_at, coalesce(st.last_woken_at, st.created_at),
                 coalesce((select max(cm.created_at) from comments cm where cm.station_id = st.id), st.created_at),
                 coalesce((select max(pm.created_at) from pass_messages pm where pm.station_id = st.id), st.created_at))
               < now() - make_interval(secs => {})",
        crate::service::stall_secs()
    );
    Ok(sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(db)
        .await?
        .rows_affected())
}

/// The repository a confined station is tied to: `claim_next` from it hands out only tasks about
/// that repository or about none. `None` for a station that may leave its folder (or knows no repo).
pub async fn confined_repo(c: &mut PgConnection, station: Option<Uuid>) -> Result<Option<String>> {
    let Some(id) = station else { return Ok(None) };
    let row: Option<Option<String>> =
        sqlx::query_scalar("select repo from stations where id = $1 and confined")
            .bind(id)
            .fetch_optional(c)
            .await?;
    Ok(row.flatten())
}

/// For the dispatcher: an idle station of this cook in this pot (holds no live lease and waits
/// on no question), least recently woken first, marked as woken now. `Ok(None)` with
/// `has_any == true` means every station is busy: do not wake.
pub async fn pick_for_wake(
    c: &mut PgConnection,
    agent: Uuid,
    project: Uuid,
) -> Result<(bool, Option<Value>)> {
    let has_any: bool = sqlx::query_scalar(
        "select exists (select 1 from stations where agent_id = $1 and project_id = $2 and kind = 'session')",
    )
    .bind(agent)
    .bind(project)
    .fetch_one(&mut *c)
    .await?;
    if !has_any {
        return Ok((false, None));
    }
    // Stations share the cook's lease slots: do not wake more sessions than there are free slots,
    // counting stations woken in the last minute that have not claimed yet.
    let full: bool = sqlx::query_scalar(
        "select (select count(*) from tasks t where t.claimed_by = $1 and t.status = 'claimed' and t.lease_expires_at > now())
              + (select count(*) from stations st where st.agent_id = $1 and st.kind = 'session'
                   and st.last_woken_at > now() - interval '60 seconds'
                   and not exists (select 1 from tasks t where t.station_id = st.id and t.status = 'claimed'
                                     and t.lease_expires_at > now() and t.updated_at >= st.last_woken_at))
              >= (select max_leases from principals where id = $1)",
    )
    .bind(agent)
    .fetch_one(&mut *c)
    .await?;
    if full {
        return Ok((true, None));
    }
    let row: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "update stations set last_woken_at = now() where id = (
           select st.id from stations st
           where st.agent_id = $1 and st.project_id = $2 and st.kind = 'session'
             and not exists (select 1 from tasks t where t.station_id = st.id
                  and ((t.status = 'claimed' and t.lease_expires_at > now()) or (t.status = 'blocked' and t.reserved_for is not null)))
             -- a confined station is woken only when work it may take is ready: the others get
             -- the work about other repositories
             and (not st.confined or st.repo is null or exists (
                  select 1 from tasks t where t.project_id = $2 and {ready}
                    and (cardinality(t.repos) = 0 or st.repo = any(t.repos))
                    and {scope} and {takeable}))
           order by st.main desc, st.last_woken_at nulls first, st.created_at limit 1
           for update skip locked)
         returning ref, runtime, session_ref, directory, label",
        ready = crate::service::READY_SQL,
        scope = crate::repos::scope_sql("$1"),
        takeable = crate::service::takeable_sql("$1"),
    )))
    .bind(agent)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?;
    Ok((
        true,
        row.map(|(r, rt, s, d, l)| station_payload(&r, &rt, &s, d, l)),
    ))
}

pub fn station_payload(
    r: &str,
    runtime: &str,
    session: &str,
    directory: Option<String>,
    label: Option<String>,
) -> Value {
    json!({"id": r, "runtime": runtime, "session": session, "directory": directory, "label": label})
}

/// A station by id, for waking it directly (answers to its questions).
pub async fn payload_of(c: &mut PgConnection, id: Uuid) -> Result<Option<Value>> {
    let row: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "select ref, runtime, session_ref, directory, label from stations where id = $1",
    )
    .bind(id)
    .fetch_optional(c)
    .await?;
    Ok(row.map(|(r, rt, s, d, l)| station_payload(&r, &rt, &s, d, l)))
}
