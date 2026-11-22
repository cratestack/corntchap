//! The question pool, per pot. A cook asks (optionally parking its task: it stays reserved for
//! that cook and station, waiting); people answer in the console, or a cook in a chat with its
//! owner picks the question, asks the owner and records the answer in the owner's name ("selast,
//! via desktop-wow"). An answer hands a parked task back to the cook with a fresh lease and wakes
//! the very station that asked, with the answer in the prompt.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    service::{emit, parse_task_ref, project_by_key, task_by_ref},
    state::Principal,
    stations,
};

const PICK_MINUTES: i64 = 15;

const QUESTION_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-Q' || q.number, 'project', pr.key,
  'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
  'asked_by', (select name from principals where id = q.asked_by),
  'station', (select ref from stations where id = q.station_id),
  'to', (select name from principals where id = q.to_principal),
  'body', q.body, 'options', to_jsonb(q.options), 'wait', q.wait, 'status', q.status,
  'picked_by', case when q.pick_expires_at > now() then (select name from principals where id = q.picked_by) end,
  'answer', q.answer,
  'answered_by', (select name from principals where id = q.answered_by),
  'relayed_by', (select name from principals where id = q.relayed_by),
  'answered_at', q.answered_at, 'created_at', q.created_at, 'updated_at', q.updated_at,
  'notes', coalesce((select jsonb_agg(jsonb_build_object('author', (select name from principals where id = n.author_id),
      'body', n.body, 'at', n.created_at) order by n.created_at) from question_notes n where n.question_id = q.id), '[]'::jsonb),
  'files', coalesce((select jsonb_agg(jsonb_build_object('id', pr.key || '-F' || f.number, 'name', f.name,
      'content_type', f.content_type, 'size', f.size, 'status', f.status) order by f.number)
      from files f where f.question_id = q.id and f.status <> 'deleted'), '[]'::jsonb))"#;

/// `PAY-Q3` -> ("PAY", 3)
pub fn parse_ref(s: &str) -> Result<(String, i64)> {
    let bad = || Error::Invalid(format!("bad question id {s:?}, expected like PAY-Q3"));
    let (k, n) = s
        .trim()
        .rsplit_once("-Q")
        .or_else(|| s.trim().rsplit_once("-q"))
        .ok_or_else(bad)?;
    let n: i64 = n.parse().map_err(|_| bad())?;
    if k.is_empty() {
        return Err(bad());
    }
    Ok((k.to_uppercase(), n))
}

struct Q {
    id: Uuid,
    project_id: Uuid,
    key: String,
    number: i64,
    team_id: Uuid,
    task_id: Option<Uuid>,
    asked_by: Option<Uuid>,
    station_id: Option<Uuid>,
    to_principal: Option<Uuid>,
    status: String,
    wait: bool,
    picked_by: Option<Uuid>,
    pick_expires_at: Option<DateTime<Utc>>,
}

async fn load(c: &mut PgConnection, p: &Principal, r: &str, lock: bool) -> Result<Q> {
    let (key, number) = parse_ref(r)?;
    let pr = project_by_key(c, p, &key).await?;
    let sql = format!(
        "select id, task_id, asked_by, station_id, to_principal, status, wait, picked_by, pick_expires_at
         from questions where project_id = $1 and number = $2 {}",
        if lock { "for update" } else { "" }
    );
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        String,
        bool,
        Option<Uuid>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(number)
        .fetch_optional(&mut *c)
        .await?;
    let Some((
        id,
        task_id,
        asked_by,
        station_id,
        to_principal,
        status,
        wait,
        picked_by,
        pick_expires_at,
    )) = row
    else {
        return Err(Error::NotFound(format!("no question {key}-Q{number}")));
    };
    Ok(Q {
        id,
        project_id: pr.id,
        key: pr.key,
        number,
        team_id: pr.team_id,
        task_id,
        asked_by,
        station_id,
        to_principal,
        status,
        wait,
        picked_by,
        pick_expires_at,
    })
}

async fn view(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id where q.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// First line, without markdown decoration, cut to 100 characters (event titles).
pub(crate) fn short(s: &str) -> String {
    let first = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let plain: String = first
        .trim()
        .trim_start_matches(['#', '>', '-', '*', ' '])
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect();
    let one = plain.trim();
    if one.chars().count() > 100 {
        format!("{}…", one.chars().take(100).collect::<String>())
    } else {
        one.to_string()
    }
}

pub struct Ask {
    pub project: Option<String>,
    pub body: String,
    pub task: Option<String>,
    pub options: Vec<String>,
    pub to: Option<String>,
    pub wait: bool,
    pub station: Option<String>,
}

pub async fn ask(db: &PgPool, p: &Principal, a: Ask) -> Result<Value> {
    let body = a.body.trim();
    if body.is_empty() {
        return invalid("the question is empty");
    }
    let key = match (&a.project, &a.task) {
        (Some(k), _) => k.trim().to_uppercase(),
        (None, Some(t)) => parse_task_ref(t)?.0,
        (None, None) => return invalid("project (or task) is required"),
    };
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &key).await?;
    let task = match a.task.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = task_by_ref(&mut tx, p, t).await?;
            if t.project_id != pr.id {
                return invalid(format!("{} is not in {}", t.key, pr.key));
            }
            Some(t)
        }
        None => None,
    };
    let to = match a
        .to
        .as_deref()
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
    {
        None => None,
        Some(name) => Some(
            sqlx::query_scalar::<_, Uuid>(
                "select pr.id from principals pr join team_members tm on tm.principal_id = pr.id
                 where pr.name = $1 and tm.team_id = $2 and pr.kind <> 'relay'",
            )
            .bind(&name)
            .bind(pr.team_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| Error::NotFound(format!("nobody named {name} in {}'s team", pr.key)))?,
        ),
    };
    let mut station = stations::for_claim(&mut tx, p, a.station.as_deref(), pr.id).await?;
    if station.is_none()
        && let Some(t) = &task
    {
        // Asked without `station` about a task this cook holds: the session that holds it asked.
        station = sqlx::query_scalar(
            "select station_id from tasks where id = $1 and claimed_by = $2 and status = 'claimed'",
        )
        .bind(t.id)
        .bind(p.id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
    }
    let mut wait = a.wait;
    if wait {
        let Some(t) = &task else {
            return invalid("wait needs the task you are working on");
        };
        if !p.is_agent() {
            wait = false;
        } else if t.status != "claimed" || t.claimed_by != Some(p.id) {
            return invalid(format!(
                "wait parks a task you hold: {} is {} {}",
                t.key,
                t.status,
                if t.claimed_by == Some(p.id) {
                    ""
                } else {
                    "and not yours"
                }
            ));
        }
    }
    let options: Vec<String> = a
        .options
        .iter()
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty())
        .take(8)
        .collect();
    let number: i64 = sqlx::query_scalar(
        "update projects set next_question = next_question + 1 where id = $1 returning next_question - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *tx)
    .await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into questions (project_id, number, task_id, asked_by, station_id, to_principal, body, options, wait)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9) returning id",
    )
    .bind(pr.id)
    .bind(number)
    .bind(task.as_ref().map(|t| t.id))
    .bind(p.id)
    .bind(station)
    .bind(to)
    .bind(body)
    .bind(&options)
    .bind(wait)
    .fetch_one(&mut *tx)
    .await?;
    let qref = format!("{}-Q{number}", pr.key);
    if wait && let Some(t) = &task {
        // Park: the task waits for the answer and stays with this cook (and station).
        sqlx::query(
            "update tasks set status = 'blocked', result = $2, reserved_for = $3,
               station_id = coalesce($4, station_id), lease_expires_at = null, updated_at = now()
             where id = $1",
        )
        .bind(t.id)
        .bind(format!("waiting for an answer to {qref}"))
        .bind(p.id)
        .bind(station)
        .execute(&mut *tx)
        .await?;
        emit(
            &mut tx,
            Some(pr.id),
            Some(t.id),
            "task.blocked",
            Some(p.id),
            json!({"reason": format!("waiting for an answer to {qref}"), "question": qref}),
        )
        .await?;
    }
    emit(
        &mut tx,
        Some(pr.id),
        task.as_ref().map(|t| t.id),
        "question.asked",
        Some(p.id),
        json!({"question": qref, "title": short(body), "to": a.to}),
    )
    .await?;
    let out = view(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

/// The pot's pool: open questions by default (oldest first), or `answered` / `dismissed` / `all`
/// (newest first).
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: &str,
    status: Option<&str>,
) -> Result<Value> {
    let status = status
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "open".into());
    if !["open", "answered", "dismissed", "all"].contains(&status.as_str()) {
        return invalid("status: open, answered, dismissed or all");
    }
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.to_uppercase()).await?;
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id
         where q.project_id = $1 and ($2 = 'all' or q.status = $2)
         order by case when $2 = 'open' then q.created_at end asc, q.created_at desc limit 200"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&status)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows))
}

/// Questions the caller asked (a cook: its own), open ones first.
pub async fn mine(db: &PgPool, p: &Principal, status: Option<&str>) -> Result<Value> {
    let status = status
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "all".into());
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id
         where q.asked_by = $1 and ($2 = 'all' or q.status = $2)
         order by q.status <> 'open', q.created_at desc limit 100"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.id)
        .bind(&status)
        .fetch_all(db)
        .await?;
    Ok(Value::Array(rows))
}

pub async fn get(db: &PgPool, p: &Principal, r: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let q = load(&mut c, p, r, false).await?;
    view(&mut c, q.id).await
}

/// The person a cook relays for: its owner, who must be in the pot's team.
async fn owner_in_team(c: &mut PgConnection, p: &Principal, team: Uuid) -> Result<Uuid> {
    let owner = p
        .owner_id
        .ok_or_else(|| Error::Forbidden(format!("{} has no owner to ask", p.name)))?;
    let member: bool = sqlx::query_scalar(
        "select exists (select 1 from team_members where team_id = $1 and principal_id = $2)",
    )
    .bind(team)
    .bind(owner)
    .fetch_one(c)
    .await?;
    if !member {
        return Err(Error::Forbidden(format!(
            "{}'s owner is not in this pot's team, so {} cannot relay its questions",
            p.name, p.name
        )));
    }
    Ok(owner)
}

/// For a cook in a chat with its owner: take the next open question it may put to its owner
/// (unaddressed or addressed to the owner), or one addressed to the cook itself, and hold it for
/// a few minutes so no other chat asks the same thing.
pub async fn pick(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Value> {
    if !p.is_agent() {
        return invalid(
            "pick_question is for cooks relaying to their owner; people answer in the console",
        );
    }
    let mut tx = db.begin().await?;
    let pid = match project {
        Some(k) if !k.trim().is_empty() => Some(
            project_by_key(&mut tx, p, &k.trim().to_uppercase())
                .await?
                .id,
        ),
        _ => None,
    };
    let row: Option<Uuid> = sqlx::query_scalar(
        "select q.id from questions q join projects pr on pr.id = q.project_id
         where q.status = 'open' and ($1::uuid is null or q.project_id = $1)
           and (q.picked_by is null or q.picked_by = $2 or q.pick_expires_at < now())
           and (
             q.to_principal = $2
             or ((q.to_principal is null or q.to_principal = $3)
                 and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $3)
                 and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $2)))
         order by (q.to_principal = $2) desc nulls last, q.created_at
         for update of q skip locked limit 1",
    )
    .bind(pid)
    .bind(p.id)
    .bind(p.owner_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = row else {
        tx.commit().await?;
        return Ok(json!({"none": true}));
    };
    sqlx::query(
        "update questions set picked_by = $2, pick_expires_at = now() + make_interval(mins => $3::int) where id = $1",
    )
    .bind(id)
    .bind(p.id)
    .bind(PICK_MINUTES as i32)
    .execute(&mut *tx)
    .await?;
    let out = view(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"question": out}))
}

pub async fn release(db: &PgPool, p: &Principal, r: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, true).await?;
    if q.picked_by != Some(p.id) {
        return Err(Error::Conflict(format!(
            "{} is not picked by {}",
            r.trim(),
            p.name
        )));
    }
    sqlx::query("update questions set picked_by = null, pick_expires_at = null where id = $1")
        .bind(q.id)
        .execute(&mut *tx)
        .await?;
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// Answer (or dismiss) a question. People answer as themselves. A cook answers a question
/// addressed to it as itself; any other question it can only relay from its owner (recorded as
/// answered by the owner, relayed by the cook).
pub async fn answer(
    db: &PgPool,
    p: &Principal,
    r: &str,
    text: &str,
    dismiss: bool,
) -> Result<Value> {
    let text = text.trim();
    if text.is_empty() && !dismiss {
        return invalid("the answer is empty");
    }
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, true).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    if q.status != "open" {
        return Err(Error::Conflict(format!("{qref} is already {}", q.status)));
    }
    // People answer as themselves, and so do the asker (dismissing) and a cook a question is
    // addressed to; any other cook can only relay its owner's answer.
    let as_self =
        p.is_user() || (dismiss && q.asked_by == Some(p.id)) || q.to_principal == Some(p.id);
    let (answered_by, relayed_by) = if as_self {
        (p.id, None)
    } else if p.is_agent() {
        let owner = owner_in_team(&mut tx, p, q.team_id).await?;
        if q.to_principal.is_some_and(|to| to != owner) {
            return Err(Error::Forbidden(format!(
                "{qref} is addressed to someone else"
            )));
        }
        if q.picked_by.is_some_and(|by| by != p.id)
            && q.pick_expires_at.is_some_and(|t| t > Utc::now())
        {
            return Err(Error::Conflict(format!(
                "{qref} is being asked in another chat right now"
            )));
        }
        (owner, Some(p.id))
    } else {
        return Err(Error::Forbidden("relays cannot answer questions".into()));
    };
    let status = if dismiss { "dismissed" } else { "answered" };
    let stored = if dismiss {
        if text.is_empty() { None } else { Some(text) }
    } else {
        Some(text)
    };
    sqlx::query(
        "update questions set status = $2, answer = $3, answered_by = $4, relayed_by = $5, answered_at = now(),
           picked_by = null, pick_expires_at = null where id = $1",
    )
    .bind(q.id)
    .bind(status)
    .bind(stored)
    .bind(answered_by)
    .bind(relayed_by)
    .execute(&mut *tx)
    .await?;
    let names: (String, Option<String>) = sqlx::query_as(
        "select (select name from principals where id = $1), (select name from principals where id = $2)",
    )
    .bind(answered_by)
    .bind(relayed_by)
    .fetch_one(&mut *tx)
    .await?;
    let who = match &names.1 {
        Some(via) => format!("{}, via {via}", names.0),
        None => names.0.clone(),
    };
    emit(
        &mut tx,
        Some(q.project_id),
        q.task_id,
        if dismiss {
            "question.dismissed"
        } else {
            "question.answered"
        },
        Some(p.id),
        json!({"question": qref, "answer": stored.map(short), "relayed_by": names.1}),
    )
    .await?;

    // Hand a parked task back to the cook that asked, with a fresh lease.
    let mut resumed: Option<(String, i64)> = None;
    if q.wait
        && let (Some(tid), Some(asker)) = (q.task_id, q.asked_by)
    {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "update tasks t set status = 'claimed', claimed_by = $2, reserved_for = null, result = null,
               lease_epoch = t.lease_epoch + 1, updated_at = now(),
               lease_expires_at = now() + (select lease_ttl from projects where id = t.project_id)
             where t.id = $1 and t.status = 'blocked' and t.reserved_for = $2
             returning t.lease_epoch, t.number",
        )
        .bind(tid)
        .bind(asker)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((epoch, number)) = row {
            let asker_name: String =
                sqlx::query_scalar("select name from principals where id = $1")
                    .bind(asker)
                    .fetch_one(&mut *tx)
                    .await?;
            emit(
                &mut tx,
                Some(q.project_id),
                Some(tid),
                "task.claimed",
                Some(asker),
                json!({"agent": asker_name, "epoch": epoch, "reason": format!("{qref} {status}")}),
            )
            .await?;
            resumed = Some((format!("{}-{number}", q.key), epoch));
        }
    }

    // Wake the asking cook (its very station when it asked from one) with the answer.
    if let Some(asker) = q.asked_by
        && asker != p.id
    {
        let is_agent: bool =
            sqlx::query_scalar("select kind = 'agent' from principals where id = $1")
                .bind(asker)
                .fetch_one(&mut *tx)
                .await?;
        if is_agent {
            let mut prompt = if dismiss {
                format!(
                    "Your question {qref} was dismissed by {who}{}.",
                    stored.map(|s| format!(": {s}")).unwrap_or_default()
                )
            } else {
                format!("Your question {qref} was answered by {who}:\n\n{text}\n")
            };
            if let Some((task, epoch)) = &resumed {
                prompt.push_str(&format!(
                    "\nYou hold {task} again with lease epoch {epoch}: continue it (pass that epoch to check_item, complete, ...)."
                ));
            }
            wake(
                &mut tx,
                q.project_id,
                &q.key,
                asker,
                q.station_id,
                q.task_id,
                &prompt,
                "answer",
            )
            .await?;
        }
    }
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A follow-up in a question's thread. A person's note to an open question wakes the cook that
/// asked, so it can clarify.
pub async fn note(db: &PgPool, p: &Principal, r: &str, body: &str) -> Result<Value> {
    let body = body.trim();
    if body.is_empty() {
        return invalid("the note is empty");
    }
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, false).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    sqlx::query("insert into question_notes (question_id, author_id, body) values ($1,$2,$3)")
        .bind(q.id)
        .bind(p.id)
        .bind(body)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(q.project_id),
        q.task_id,
        "question.noted",
        Some(p.id),
        json!({"question": qref, "title": short(body)}),
    )
    .await?;
    if q.status == "open"
        && let Some(asker) = q.asked_by
        && asker != p.id
    {
        let is_agent: bool =
            sqlx::query_scalar("select kind = 'agent' from principals where id = $1")
                .bind(asker)
                .fetch_one(&mut *tx)
                .await?;
        if is_agent {
            let prompt = format!(
                "{} replied on your question {qref}:\n\n{body}\n\nAnswer with comment_question {qref} if they need more from you.",
                p.name
            );
            wake(
                &mut tx,
                q.project_id,
                &q.key,
                asker,
                q.station_id,
                q.task_id,
                &prompt,
                "note",
            )
            .await?;
        }
    }
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A direct wake for one cook (and station), carrying its own prompt text.
pub async fn wake(
    c: &mut PgConnection,
    project: Uuid,
    key: &str,
    agent: Uuid,
    station: Option<Uuid>,
    task: Option<Uuid>,
    text: &str,
    reason: &str,
) -> Result<()> {
    let name: String = sqlx::query_scalar("select name from principals where id = $1")
        .bind(agent)
        .fetch_one(&mut *c)
        .await?;
    let station = match station {
        Some(s) => Some(s),
        None => infer_station(&mut *c, project, agent, task).await?,
    };
    let st = match station {
        Some(s) => stations::payload_of(&mut *c, s).await?,
        None => None,
    };
    let news = crate::pantry::news(&mut *c, agent, project).await?;
    let text = match news["count"].as_u64().unwrap_or(0) {
        0 => text.to_string(),
        n => format!(
            "{text}\n\n(Pantry: {n} memor{} new or changed since you last read it: {}. Read with get_memory or read_pantry.)",
            if n == 1 { "y" } else { "ies" },
            news["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["key"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let text = text.as_str();
    // A cook can run several sessions: tell this one who it is, so its answer says so.
    let text = match st.as_ref().and_then(|s| s["id"].as_str()) {
        Some(id) => format!(
            "{text}\n\n(You are station {id}: pass `station: \"{id}\"` to pass, comment, ask and claim_next, so people see which session answers.)"
        ),
        None => text.to_string(),
    };
    emit(
        c,
        Some(project),
        None,
        "wake",
        None,
        json!({"agent": name, "agent_id": agent, "project": key, "station": st, "text": text, "reason": reason}),
    )
    .await?;
    Ok(())
}

/// The station to wake when none is named (answers, notes, pass messages and mentions): the
/// station that holds or is parked on the task in question (it has the context), else the cook's
/// main station in this pot, else its station woken last, else none.
async fn infer_station(
    c: &mut PgConnection,
    project: Uuid,
    agent: Uuid,
    task: Option<Uuid>,
) -> Result<Option<Uuid>> {
    if let Some(t) = task {
        let on_task: Option<Uuid> = sqlx::query_scalar(
            "select t.station_id from tasks t join stations s on s.id = t.station_id
             where t.id = $1 and s.agent_id = $2",
        )
        .bind(t)
        .bind(agent)
        .fetch_optional(&mut *c)
        .await?
        .flatten();
        if on_task.is_some() {
            return Ok(on_task);
        }
    }
    // the cook's main station in the pot, else the one woken last
    Ok(sqlx::query_scalar(
        "select id from stations where agent_id = $1 and project_id = $2
         order by main desc, last_woken_at desc nulls last, created_at desc limit 1",
    )
    .bind(agent)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::{parse_ref, short};

    #[test]
    fn titles_are_plain() {
        assert_eq!(
            short("**Round** `down` or _up_?\nmore"),
            "Round down or up?"
        );
        assert_eq!(short("\n## Which currency?"), "Which currency?");
    }

    #[test]
    fn question_refs() {
        assert_eq!(parse_ref("PAY-Q3").unwrap(), ("PAY".into(), 3));
        assert_eq!(parse_ref("ai-helm-q12").unwrap(), ("AI-HELM".into(), 12));
        assert!(parse_ref("PAY-3").is_err());
        assert!(parse_ref("-Q3").is_err());
    }
}
