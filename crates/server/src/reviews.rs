//! Reviews: someone who did the work on a task asks a cook or a person to look at it. A review is
//! not a lane: the task keeps its status. One row is one request; a task reviewed again after
//! changes gets a new row with the next round. Requesting wakes the reviewer cook (reason
//! `review`); a verdict wakes the cook that holds or last held the task (reason `review-done`).
//! A person reviewer sees the request in the pot's reviews (the console's Questions page) and the
//! activity feed. GitHub reviews on a pull request are separate (src/forge): they sit in the
//! task's `pr`, these in its `review`.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    questions::{short, wake},
    service::{emit, project_by_key, task_by_ref},
    state::Principal,
};

pub const STATES: [&str; 5] = [
    "requested",
    "approved",
    "changes_requested",
    "commented",
    "cancelled",
];
const NOTE_MAX: usize = 4000;
const BODY_MAX: usize = 20_000;

/// SQL expression of one review (aliases `rv` reviews, `pr` projects).
pub const REVIEW_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-R' || rv.number, 'project', pr.key,
  'task', (select pr.key || '-' || t.number from tasks t where t.id = rv.task_id),
  'task_title', (select t.title from tasks t where t.id = rv.task_id),
  'round', rv.round, 'state', rv.state,
  'requested_by', (select name from principals where id = rv.requested_by),
  'reviewer', (select name from principals where id = rv.reviewer),
  'reviewed_by', (select name from principals where id = rv.reviewed_by),
  'note', rv.note, 'body', rv.verdict,
  'created_at', rv.created_at, 'decided_at', rv.decided_at, 'updated_at', rv.updated_at)"#;

/// `PAY-R3` -> ("PAY", 3)
pub fn parse_ref(s: &str) -> Result<(String, i64)> {
    let bad = || Error::Invalid(format!("bad review id {s:?}, expected like PAY-R3"));
    let (k, n) = s
        .trim()
        .rsplit_once("-R")
        .or_else(|| s.trim().rsplit_once("-r"))
        .ok_or_else(bad)?;
    let n: i64 = n.parse().map_err(|_| bad())?;
    if k.is_empty() {
        return Err(bad());
    }
    Ok((k.to_uppercase(), n))
}

/// What the reviewer decides: `approve`, `changes` or `comment` (longer spellings work too).
pub fn parse_verdict(v: &str) -> Result<&'static str> {
    match v.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
        "approve" | "approved" | "approval" | "lgtm" => Ok("approved"),
        "changes" | "change" | "changes_requested" | "request_changes" | "reject" | "rejected" => {
            Ok("changes_requested")
        }
        "comment" | "commented" => Ok("commented"),
        _ => invalid("verdict: approve, changes or comment"),
    }
}

struct R {
    id: Uuid,
    project_id: Uuid,
    key: String,
    number: i64,
    task_id: Uuid,
    requested_by: Option<Uuid>,
    reviewer: Option<Uuid>,
    state: String,
    round: i32,
}

impl R {
    fn rref(&self) -> String {
        format!("{}-R{}", self.key, self.number)
    }
}

async fn view(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {REVIEW_JSON} from reviews rv join projects pr on pr.id = rv.project_id where rv.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// Every round of a task's reviews, oldest first (the task detail shows them).
pub async fn of_task(c: &mut PgConnection, task: Uuid) -> Result<Value> {
    let sql = format!(
        "select coalesce(jsonb_agg({REVIEW_JSON} order by rv.round, rv.created_at), '[]'::jsonb)
         from reviews rv join projects pr on pr.id = rv.project_id where rv.task_id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(task)
        .fetch_one(c)
        .await?)
}

async fn load(c: &mut PgConnection, p: &Principal, r: &str, lock: bool) -> Result<R> {
    let (key, number) = parse_ref(r)?;
    let pr = project_by_key(c, p, &key).await?;
    let sql = format!(
        "select id, task_id, requested_by, reviewer, state, round from reviews
         where project_id = $1 and number = $2 {}",
        if lock { "for update" } else { "" }
    );
    let row: Option<(Uuid, Uuid, Option<Uuid>, Option<Uuid>, String, i32)> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(pr.id)
            .bind(number)
            .fetch_optional(&mut *c)
            .await?;
    let Some((id, task_id, requested_by, reviewer, state, round)) = row else {
        return Err(Error::NotFound(format!("no review {key}-R{number}")));
    };
    Ok(R {
        id,
        project_id: pr.id,
        key: pr.key,
        number,
        task_id,
        requested_by,
        reviewer,
        state,
        round,
    })
}

/// The cook responsible for a task: the one holding it, parked on it, or the last that claimed it.
async fn responsible(c: &mut PgConnection, task: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "select coalesce(t.reserved_for, t.claimed_by,
                (select e.actor_id from events e where e.task_id = t.id and e.kind = 'task.claimed'
                 order by e.seq desc limit 1))
         from tasks t where t.id = $1",
    )
    .bind(task)
    .fetch_one(c)
    .await?)
}

/// Whether `p` holds the task, or has held it (it claimed it once).
async fn held(c: &mut PgConnection, p: &Principal, task: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "select exists (select 1 from tasks t where t.id = $1 and (t.claimed_by = $2 or t.reserved_for = $2))
             or exists (select 1 from events e where e.task_id = $1 and e.kind = 'task.claimed' and e.actor_id = $2)",
    )
    .bind(task)
    .bind(p.id)
    .fetch_one(c)
    .await?)
}

async fn is_agent(c: &mut PgConnection, id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "select kind = 'agent' and disabled_at is null from principals where id = $1",
    )
    .bind(id)
    .fetch_optional(c)
    .await?
    .unwrap_or(false))
}

/// Ask for a review of a task. The requester holds or has held the task (or is a person, or a cook
/// with the `plan` grant). The reviewer is named (a cook or person of the pot's team, never the
/// requester) or left out for anyone on the team.
pub async fn request(
    db: &PgPool,
    p: &Principal,
    task: &str,
    reviewer: Option<&str>,
    note: Option<&str>,
) -> Result<Value> {
    let note = note.map(str::trim).unwrap_or("");
    if note.chars().count() > NOTE_MAX {
        return invalid(format!(
            "the note is too long ({NOTE_MAX} characters at most)"
        ));
    }
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, task).await?;
    let tref = format!("{}-{}", t.key, t.number);
    if t.status == "cancelled" {
        return invalid(format!("{tref} is tossed: there is nothing to review"));
    }
    if !p.can_plan() && !held(&mut tx, p, t.id).await? {
        return Err(Error::Forbidden(format!(
            "{} neither holds nor held {tref}: only the cook that worked on it (or a person, or a cook granted `plan`) asks for its review",
            p.name
        )));
    }
    let pr = project_by_key(&mut tx, p, &t.key).await?;
    let reviewer_id: Option<Uuid> = match reviewer
        .map(|r| r.trim().trim_start_matches('@').to_lowercase())
        .filter(|r| !r.is_empty())
    {
        None => None,
        Some(name) => {
            let id = crate::pass::chef(&mut tx, pr.team_id, &name)
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("nobody named {name} in {}'s team", pr.key))
                })?;
            if id == p.id {
                return Err(Error::Forbidden(
                    "you cannot review your own work: ask someone else".into(),
                ));
            }
            if crate::cloud::is_fire_and_forget(&mut tx, id).await? {
                return Err(crate::cloud::refusal(&name));
            }
            Some(id)
        }
    };
    let open: bool = sqlx::query_scalar(
        "select exists (select 1 from reviews where task_id = $1 and state = 'requested'
           and reviewer is not distinct from $2)",
    )
    .bind(t.id)
    .bind(reviewer_id)
    .fetch_one(&mut *tx)
    .await?;
    if open {
        return Err(Error::Conflict(format!(
            "{tref} already has a review requested from {}: wait for it or cancel_review it",
            reviewer
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .unwrap_or("the team")
        )));
    }
    // a new round starts after a verdict; a second reviewer asked while one is open shares its round
    let round: i32 = sqlx::query_scalar(
        "select case when exists (select 1 from reviews where task_id = $1 and state = 'requested')
                then (select max(round) from reviews where task_id = $1 and state = 'requested')
                else coalesce((select max(round) from reviews where task_id = $1 and state <> 'cancelled'), 0) + 1 end",
    )
    .bind(t.id)
    .fetch_one(&mut *tx)
    .await?;
    let number: i64 = sqlx::query_scalar(
        "update projects set next_review = next_review + 1 where id = $1 returning next_review - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *tx)
    .await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into reviews (project_id, number, task_id, requested_by, reviewer, note, round)
         values ($1,$2,$3,$4,$5,$6,$7) returning id",
    )
    .bind(pr.id)
    .bind(number)
    .bind(t.id)
    .bind(p.id)
    .bind(reviewer_id)
    .bind(note)
    .bind(round)
    .fetch_one(&mut *tx)
    .await?;
    let rref = format!("{}-R{number}", pr.key);
    let reviewer_name: Option<String> = match reviewer_id {
        Some(r) => {
            sqlx::query_scalar("select name from principals where id = $1")
                .bind(r)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    emit(
        &mut tx,
        Some(pr.id),
        Some(t.id),
        "review.requested",
        Some(p.id),
        json!({"review_id": rref, "task": tref, "round": round, "reviewer": reviewer_name,
               "note": short(note)}),
    )
    .await?;
    if let Some(rv) = reviewer_id
        && is_agent(&mut tx, rv).await?
    {
        let title: String = sqlx::query_scalar("select title from tasks where id = $1")
            .bind(t.id)
            .fetch_one(&mut *tx)
            .await?;
        let prompt = format!(
            "{} asks you to review {tref} \"{}\" (review {rref}, round {round}).\n\n{}\n\nRead the task with get_task {tref} (its comments and pull request tell what changed), then give your verdict with the `review` tool: id {rref}, verdict approve, changes or comment, and a body (say what to fix when you ask for changes).",
            p.name,
            short(&title),
            if note.is_empty() {
                "No note: look at the task's pull request or branch.".to_string()
            } else {
                format!("What to look at:\n{note}")
            }
        );
        wake(
            &mut tx,
            pr.id,
            &pr.key,
            rv,
            None,
            Some(t.id),
            &prompt,
            "review",
        )
        .await?;
    }
    let out = view(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

/// Give a verdict on an open review, by its id (PAY-R3) or by its task (the review waiting on you).
pub async fn give(
    db: &PgPool,
    p: &Principal,
    id: Option<&str>,
    task: Option<&str>,
    verdict: &str,
    body: Option<&str>,
) -> Result<Value> {
    let state = parse_verdict(verdict)?;
    let body = body.map(str::trim).unwrap_or("");
    if body.chars().count() > BODY_MAX {
        return invalid(format!(
            "the body is too long ({BODY_MAX} characters at most)"
        ));
    }
    if state != "approved" && body.is_empty() {
        return invalid(match state {
            "changes_requested" => "say what to change: the body is empty",
            _ => "the comment is empty",
        });
    }
    let mut tx = db.begin().await?;
    let rid = match (
        id.map(str::trim).filter(|i| !i.is_empty()),
        task.map(str::trim).filter(|i| !i.is_empty()),
    ) {
        (Some(i), _) => i.to_string(),
        (None, Some(t)) => {
            let t = task_by_ref(&mut tx, p, t).await?;
            let found: Option<i64> = sqlx::query_scalar(
                "select number from reviews where task_id = $1 and state = 'requested'
                   and (reviewer = $2 or (reviewer is null and requested_by is distinct from $2))
                 order by (reviewer = $2) desc nulls last, created_at limit 1",
            )
            .bind(t.id)
            .bind(p.id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(n) = found else {
                return Err(Error::NotFound(format!(
                    "{}-{} has no review waiting on {}",
                    t.key, t.number, p.name
                )));
            };
            format!("{}-R{n}", t.key)
        }
        (None, None) => return invalid("give the review id (like PAY-R3) or its task"),
    };
    let r = load(&mut tx, p, &rid, true).await?;
    let rref = r.rref();
    if r.state != "requested" {
        return Err(Error::Conflict(format!("{rref} is already {}", r.state)));
    }
    if r.requested_by == Some(p.id) {
        return Err(Error::Forbidden(
            "you cannot review your own request: someone else gives the verdict".into(),
        ));
    }
    match r.reviewer {
        Some(rv) if rv != p.id => {
            return Err(Error::Forbidden(format!(
                "{rref} is addressed to someone else"
            )));
        }
        Some(_) => {}
        // anyone in the team: project_by_key already limits the caller to the team (or an admin)
        None => {}
    }
    if p.is_relay() {
        return Err(Error::Forbidden("relays cannot review".into()));
    }
    sqlx::query(
        "update reviews set state = $2, verdict = $3, reviewed_by = $4, decided_at = now() where id = $1",
    )
    .bind(r.id)
    .bind(state)
    .bind(Some(body).filter(|b| !b.is_empty()))
    .bind(p.id)
    .execute(&mut *tx)
    .await?;
    let (tref, title, status): (String, String, String) = sqlx::query_as(
        "select pr.key || '-' || t.number, t.title, t.status from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
    )
    .bind(r.task_id)
    .fetch_one(&mut *tx)
    .await?;
    emit(
        &mut tx,
        Some(r.project_id),
        Some(r.task_id),
        "review.done",
        Some(p.id),
        json!({"review_id": rref, "task": tref, "round": r.round, "verdict": state,
               "excerpt": short(body)}),
    )
    .await?;
    // tell the cook that holds or last held the task, and the one that asked, when a cook
    let mut targets: Vec<Uuid> = vec![];
    if let Some(c) = responsible(&mut tx, r.task_id).await? {
        targets.push(c);
    }
    if let Some(rq) = r.requested_by
        && !targets.contains(&rq)
    {
        targets.push(rq);
    }
    let what = match state {
        "approved" => "approved",
        "changes_requested" => "asked for changes on",
        _ => "commented on",
    };
    let mut prompt = format!(
        "{} {what} {tref} \"{}\" (review {rref}, round {}).",
        p.name,
        short(&title),
        r.round
    );
    if !body.is_empty() {
        prompt.push_str(&format!("\n\n{body}"));
    }
    prompt.push_str(&match state {
        "approved" => format!(
            "\n\nNext: nothing is left for the review. Carry on with {tref} (complete it when its work is done)."
        ),
        "changes_requested" => format!(
            "\n\nNext: make these changes on {tref}{} then ask again with request_review (task {tref}, reviewer {}, a note on what you changed): that starts round {}.",
            if matches!(status.as_str(), "claimed") {
                ","
            } else {
                " (claim_task it first: it is not claimed now),"
            },
            p.name,
            r.round + 1
        ),
        _ => "\n\nNext: it is a comment, not a decision: nothing is approved or blocked. Ask again with request_review when you want a verdict.".to_string(),
    });
    for t in targets {
        if t == p.id || !is_agent(&mut tx, t).await? {
            continue;
        }
        wake(
            &mut tx,
            r.project_id,
            &r.key,
            t,
            None,
            Some(r.task_id),
            &prompt,
            "review-done",
        )
        .await?;
    }
    let out = view(&mut tx, r.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// Withdraw an open request: the one who made it, or an admin.
pub async fn cancel(db: &PgPool, p: &Principal, id: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let r = load(&mut tx, p, id, true).await?;
    let rref = r.rref();
    if r.state != "requested" {
        return Err(Error::Conflict(format!("{rref} is already {}", r.state)));
    }
    if r.requested_by != Some(p.id) && !p.is_super_admin() {
        return Err(Error::Forbidden(format!(
            "only the one who asked for {rref} cancels it"
        )));
    }
    sqlx::query("update reviews set state = 'cancelled', decided_at = now() where id = $1")
        .bind(r.id)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(r.project_id),
        Some(r.task_id),
        "review.cancelled",
        Some(p.id),
        json!({"review_id": rref, "round": r.round}),
    )
    .await?;
    let out = view(&mut tx, r.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A page of the pot's reviews for the console (`reviewPage`): waiting ones first, then newest
/// first (requested, created_at, number), same filters as [`list`] without the task.
pub async fn page(
    db: &PgPool,
    p: &Principal,
    project: &str,
    mine: bool,
    state: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let state = state
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if mine {
                "requested".into()
            } else {
                "all".into()
            }
        });
    if state != "all" && !STATES.contains(&state.as_str()) {
        return invalid(
            "state: requested, approved, changes_requested, commented, cancelled or all",
        );
    }
    let paging = crate::paging::paging(
        page,
        "-requested,-created_at,-number",
        3,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let sql = format!(
        "select {REVIEW_JSON}, array[(rv.state = 'requested')::int::text, rv.created_at::text, rv.number::text]
         from reviews rv join projects pr on pr.id = rv.project_id
         where rv.project_id = $1 and ($2 = 'all' or rv.state = $2)
           and (not $3 or rv.reviewer = $4 or (rv.reviewer is null and rv.requested_by is distinct from $4))
           and ($5::text is null or ((rv.state = 'requested')::int, rv.created_at, rv.number)
                < ($5::text::int, $6::text::timestamptz, $7::text::bigint))
         order by (rv.state = 'requested') desc, rv.created_at desc, rv.number desc limit $8"
    );
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&state)
        .bind(mine)
        .bind(p.id)
        .bind(paging.key(0))
        .bind(paging.key(1))
        .bind(paging.key(2))
        .bind(paging.fetch())
        .fetch_all(&mut *c)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}

/// The pot's reviews, newest first. `mine`: those waiting on the caller (named to it, or to anyone
/// when it is not the requester). `state`: requested (default for `mine`), or any state, or `all`.
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: &str,
    mine: bool,
    state: Option<&str>,
    task: Option<&str>,
) -> Result<Value> {
    let state = state
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if mine {
                "requested".into()
            } else {
                "all".into()
            }
        });
    if state != "all" && !STATES.contains(&state.as_str()) {
        return invalid(
            "state: requested, approved, changes_requested, commented, cancelled or all",
        );
    }
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let task_id: Option<Uuid> = match task.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Some(task_by_ref(&mut c, p, t).await?.id),
        None => None,
    };
    let sql = format!(
        "select {REVIEW_JSON} from reviews rv join projects pr on pr.id = rv.project_id
         where rv.project_id = $1 and ($2 = 'all' or rv.state = $2)
           and ($3::uuid is null or rv.task_id = $3)
           and (not $4 or rv.reviewer = $5 or (rv.reviewer is null and rv.requested_by is distinct from $5))
         order by (rv.state = 'requested') desc, rv.created_at desc limit 200"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&state)
        .bind(task_id)
        .bind(mine)
        .bind(p.id)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows))
}
