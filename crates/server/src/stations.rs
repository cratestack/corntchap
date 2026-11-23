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
  'directory', st.directory, 'label', st.label, 'main', st.main, 'created_at', st.created_at, 'last_woken_at', st.last_woken_at,
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
    let row: Option<(Uuid, String, Option<Uuid>, bool, i64)> = sqlx::query_as(
        "select id, name, owner_id, is_admin, max_leases from principals where name = $1 and kind = 'agent' and disabled_at is null",
    )
    .bind(&name)
    .fetch_optional(db)
    .await?;
    match row {
        Some((id, name, owner, is_admin, max_leases)) if who.is_admin || owner == Some(who.id) => {
            Ok(Principal {
                id,
                kind: "agent".into(),
                name,
                owner_id: owner,
                is_admin,
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
    let id: Uuid = sqlx::query_scalar(
        "insert into stations (ref, agent_id, project_id, runtime, session_ref, directory, label, created_by)
         values ($1,$2,$3,$4,$5,$6,$7,$8)
         on conflict (agent_id, runtime, session_ref) do update
           set project_id = excluded.project_id, directory = coalesce(excluded.directory, stations.directory),
               label = coalesce(excluded.label, stations.label),
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
    .fetch_one(&mut *tx)
    .await?;
    let out = one(&mut tx, id).await?;
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
        "select directory from stations where agent_id = $1 and project_id = $2 and directory is not null
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
        "insert into stations (ref, agent_id, project_id, runtime, session_ref, directory, label, created_by)
         values ($1,$2,$3,$4,$5,$6,$7,$8) returning id",
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
        "{who} started this session for you in corntchap pot {pot}. Use the corntchap MCP tools."
    );
    match task {
        Some(r) => t.push_str(&format!(
            " Your task is {r}: read it (get_task), claim it (claim_task with your station), work it, tick its items and complete it. If it is taken, say so on the pass and take the next ready one (claim_next)."
        )),
        None if prompt.is_none() => t.push_str(
            " Take the next ready task (claim_next with your station), work it and complete it; when none is ready, stop.",
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
/// all); optionally one pot or one cook.
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: Option<&str>,
    cook: Option<&str>,
) -> Result<Value> {
    let who = admin::acting_or_self(db, p).await?;
    let sql = format!(
        "select {STATION_JSON} from stations st join principals a on a.id = st.agent_id
         join projects pr on pr.id = st.project_id
         where ($1 or a.id = $2 or a.owner_id = $2)
           and ($3::text is null or pr.key = $3)
           and ($4::text is null or a.name = $4)
         order by pr.key, a.name, st.created_at"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(who.is_admin)
        .bind(who.id)
        .bind(project.map(|k| k.trim().to_uppercase()))
        .bind(cook.map(|c| c.trim().to_lowercase()))
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
    let row: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "select st.id, st.agent_id, st.project_id from stations st join principals a on a.id = st.agent_id
         where st.ref = $1 and ($2 or a.id = $3 or a.owner_id = $3)
         for update of st",
    )
    .bind(station.trim())
    .bind(who.is_admin)
    .bind(who.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, agent, project)) = row else {
        return Err(Error::NotFound(format!("no station {station}")));
    };
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
    .bind(who.is_admin)
    .bind(who.id)
    .execute(db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(Error::NotFound(format!("no station {station}")));
    }
    Ok(json!({"ok": true}))
}

/// The station a claim is made from: it must be one of the caller's, in the task's pot.
pub async fn for_claim(
    c: &mut PgConnection,
    p: &Principal,
    station: Option<&str>,
    project: Uuid,
) -> Result<Option<Uuid>> {
    let Some(s) = station.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let row: Option<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("select id, agent_id, project_id from stations where ref = $1")
            .bind(s)
            .fetch_optional(c)
            .await?;
    match row {
        Some((id, agent, pid)) if agent == p.id && pid == project => Ok(Some(id)),
        Some((_, agent, _)) if agent == p.id => Err(Error::Invalid(format!(
            "station {s} is registered for another pot"
        ))),
        _ => Err(Error::NotFound(format!("{} has no station {s}", p.name))),
    }
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
        "select exists (select 1 from stations where agent_id = $1 and project_id = $2)",
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
              + (select count(*) from stations st where st.agent_id = $1
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
    let row: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "update stations set last_woken_at = now() where id = (
           select st.id from stations st
           where st.agent_id = $1 and st.project_id = $2
             and not exists (select 1 from tasks t where t.station_id = st.id
                  and ((t.status = 'claimed' and t.lease_expires_at > now()) or (t.status = 'blocked' and t.reserved_for is not null)))
           order by st.main desc, st.last_woken_at nulls first, st.created_at limit 1
           for update skip locked)
         returning ref, runtime, session_ref, directory, label",
    )
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
