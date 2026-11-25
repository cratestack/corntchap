//! `my_work`: what the signed-in principal owns, holds and is waited on for, across every pot it
//! can see (a person's "my work" page; a cook's own view of the same).
//!
//! * `owned`: open, claimed and blocked tasks whose owner is me, grouped by pot.
//! * `holding`: tasks I hold with a live lease (for a person: the tasks they claimed).
//! * `waiting_on_me`: questions addressed to me, reviews asked of me (or of anyone on the team, not
//!   by me), and, for cooks, people's comments on tasks they hold that nobody answered (no comment,
//!   completion, release or block by the cook on the task since). People also
//!   see the open questions in their pots that nobody picked (`open_pool`, capped).
//! * `blocked_on_me`: tasks parked on a question I must answer, and tasks that depend on a task I
//!   own that is not done (the top ten).
//! * `counts`: the numbers for a badge. `waiting` is what asks for an answer (questions to me,
//!   reviews, comments); the pool of unaddressed questions is counted apart.
//!
//! `what_now` is the station-level version of this for one cook in one pot.

use serde_json::{Map, Value, json};
use sqlx::PgPool;

use crate::{
    error::Result,
    service::{TASK_JSON, access_sql, project_by_key},
    state::Principal,
};

/// Rows in `owned` (a pot with more owned tasks than this shows the first by recent change).
const OWNED_CAP: i64 = 300;
/// Rows per waiting list and in `blocked_on_me`.
const TOP: i64 = 50;
const BLOCKED_TOP: i64 = 10;
const POOL_TOP: i64 = 10;

/// A task row (needs aliases `t` tasks, `pr` projects, `x` the TASK_JSON lateral).
const WORK_TASK: &str = "jsonb_build_object('id', x.j->'id', 'project', pr.key, 'pot_name', pr.name,
  'title', t.title, 'status', t.status, 'priority', t.priority, 'owner', x.j->'owner',
  'holder', x.j->'claimed_by', 'lease_live', x.j->'lease_live', 'lease_expires_at', x.j->'lease_expires_at',
  'pr', x.j->'pr', 'review', x.j->'review', 'updated_at', t.updated_at)";

fn task_from(sql_tail: &str) -> String {
    format!(
        "from tasks t join projects pr on pr.id = t.project_id
         cross join lateral (select {TASK_JSON} as j) x {sql_tail}"
    )
}

/// A question row of the waiting lists (aliases `q` questions, `pr` projects).
const QUESTION_ROW: &str =
    "jsonb_build_object('id', pr.key || '-Q' || q.number, 'project', pr.key, 'title', q.title,
        'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
        'from', (select name from principals where id = q.asked_by), 'at', q.created_at)";

/// A review row (aliases `r` reviews, `pr` projects).
const REVIEW_ROW: &str = "jsonb_build_object('id', pr.key || '-R' || r.number, 'project', pr.key,
                 'task', (select pr.key || '-' || t.number from tasks t where t.id = r.task_id),
                 'task_title', (select t.title from tasks t where t.id = r.task_id),
                 'from', (select name from principals where id = r.requested_by),
                 'note', r.note, 'round', r.round, 'at', r.created_at)";

/// A comment row (aliases `cm` comments, `t` tasks, `pr` projects).
const COMMENT_ROW: &str = "jsonb_build_object('id', cm.id, 'task', pr.key || '-' || t.number, 'project', pr.key,
                     'author', (select name from principals where id = cm.author_id),
                     'author_kind', (select kind from principals where id = cm.author_id),
                     'excerpt', case when char_length(cm.body) > 160 then left(cm.body, 160) || '…' else cm.body end,
                     'at', cm.created_at)";

/// A review waits on the caller (aliases `r`, `pr`).
const REVIEW_COND: &str = "r.state = 'requested' and (r.reviewer = $2
                    or (r.reviewer is null and r.requested_by is distinct from $2 and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $2)))";

/// A person's comment on a task the cook holds that nobody answered (aliases `cr`, `cm`, `t`).
const COMMENT_COND: &str = "cr.agent_id = $2 and cr.answered_at is null and t.status <> 'cancelled'
                   and not exists (select 1 from events e where e.task_id = cr.task_id and e.actor_id = cr.agent_id
                     and e.seq > cr.since_seq
                     and e.kind in ('task.commented', 'task.completed', 'task.released', 'task.blocked'))";

/// The caller is on the pot's team (aliases `pr`; `$2` is the caller).
const MEMBER: &str =
    "exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $2)";

async fn rows(
    db: &PgPool,
    sql: String,
    p: &Principal,
    project: Option<&str>,
) -> Result<Vec<Value>> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(project)
        .fetch_all(db)
        .await?)
}

async fn count(db: &PgPool, sql: String, p: &Principal, project: Option<&str>) -> Result<i64> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(project)
        .fetch_one(db)
        .await?)
}

/// The tasks blocked on me: SQL with the select list `sel` (an object, or `1` for a count).
fn blocked_sql(question_sel: &str, dep_sel: &str, extra: &str, access: &str, tail: &str) -> String {
    let out = if extra.is_empty() { "obj" } else { "*" };
    format!(
        "select {out} from (
           select {question_sel} as obj, t.updated_at as at{extra}
           from questions q join tasks t on t.id = q.task_id join projects pr on pr.id = t.project_id
             cross join lateral (select {TASK_JSON} as j) x
           where q.status = 'open' and q.to_principal = $2 and t.status = 'blocked'
             and {access} and ($3::text is null or pr.key = $3)
           union all
           select {dep_sel} as obj, t.updated_at as at{extra}
           from tasks ot join projects op on op.id = ot.project_id
             join task_deps d on d.depends_on = ot.id
             join tasks t on t.id = d.task_id join projects pr on pr.id = t.project_id
             cross join lateral (select {TASK_JSON} as j) x
           where ot.owner_id = $2 and ot.status not in ('done','cancelled')
             and t.status not in ('done','cancelled') and t.owner_id is distinct from $2
             and {access} and ($3::text is null or pr.key = $3)
         ) b {tail}"
    )
}

/// The pot key a page is limited to, upper-cased; an unknown or unseen pot is an error, not an empty page.
async fn scoped(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Option<String>> {
    let project = project
        .map(|k| k.trim().to_uppercase())
        .filter(|k| !k.is_empty());
    if let Some(k) = &project {
        let mut c = db.acquire().await?;
        project_by_key(&mut c, p, k).await?;
    }
    Ok(project)
}

/// What the principal owns, holds and is waited on for (see the module text).
pub async fn my_work(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Value> {
    let project = project
        .map(|k| k.trim().to_uppercase())
        .filter(|k| !k.is_empty());
    if let Some(k) = &project {
        // an unknown or unseen pot is an error, not an empty page
        let mut c = db.acquire().await?;
        project_by_key(&mut c, p, k).await?;
    }
    let project = project.as_deref();
    // naming a pot is being briefed on it; the unscoped board check across every pot is not
    if p.is_agent()
        && let Some(k) = project
    {
        let mut c = db.acquire().await?;
        let id = project_by_key(&mut c, p, k).await?.id;
        crate::what_now::brief(&mut c, p, &[id]).await?;
    }
    let access = access_sql("$1", "$2");
    let scope = "($3::text is null or pr.key = $3)";

    // owned, grouped by pot
    let owned_rows = rows(
        db,
        format!(
            "select {WORK_TASK} {} where t.owner_id = $2 and t.status in ('open','claimed','blocked')
               and {access} and {scope}
             order by pr.key, t.updated_at desc limit {OWNED_CAP}",
            task_from("")
        ),
        p,
        project,
    )
    .await?;
    let owned_total = count(
        db,
        format!(
            "select count(*) from tasks t join projects pr on pr.id = t.project_id
             where t.owner_id = $2 and t.status in ('open','claimed','blocked') and {access} and {scope}"
        ),
        p,
        project,
    )
    .await?;
    let mut owned: Vec<Value> = vec![];
    for mut row in owned_rows {
        let pot = row["project"].as_str().unwrap_or_default().to_string();
        let name = row
            .as_object_mut()
            .and_then(|o| o.remove("pot_name"))
            .unwrap_or(Value::Null);
        match owned.last_mut() {
            Some(g) if g["project"] == json!(pot) => {
                if let Some(a) = g["tasks"].as_array_mut() {
                    a.push(row);
                }
            }
            _ => owned.push(json!({"project": pot, "name": name, "tasks": [row]})),
        }
    }

    // holding: a live lease
    let strip = |rows: Vec<Value>| -> Vec<Value> {
        rows.into_iter()
            .map(|mut r| {
                if let Some(o) = r.as_object_mut() {
                    o.remove("pot_name");
                }
                r
            })
            .collect()
    };
    let holding = strip(
        rows(
            db,
            format!(
                "select {WORK_TASK} {} where t.claimed_by = $2 and t.status = 'claimed'
                   and t.lease_expires_at > now() and {access} and {scope}
                 order by t.lease_expires_at",
                task_from("")
            ),
            p,
            project,
        )
        .await?,
    );

    // waiting on me
    let question = QUESTION_ROW;
    let questions = rows(
        db,
        format!(
            "select {question} from questions q join projects pr on pr.id = q.project_id
             where q.status = 'open' and q.to_principal = $2 and {access} and {scope}
             order by q.created_at limit {TOP}"
        ),
        p,
        project,
    )
    .await?;
    let reviews = rows(
        db,
        format!(
            "select {REVIEW_ROW}
             from reviews r join projects pr on pr.id = r.project_id
             where {REVIEW_COND} and {access} and {scope}
             order by r.reviewer is null, r.created_at limit {TOP}"
        ),
        p,
        project,
    )
    .await?;
    let comments = if p.is_agent() {
        rows(
            db,
            format!(
                "select {COMMENT_ROW}
                 from comment_replies cr join comments cm on cm.id = cr.comment_id
                   join tasks t on t.id = cr.task_id join projects pr on pr.id = t.project_id
                 where {COMMENT_COND} and {access} and {scope}
                 order by cm.created_at limit {TOP}"
            ),
            p,
            project,
        )
        .await?
    } else {
        vec![]
    };
    let pool_where = format!(
        "q.status = 'open' and q.to_principal is null and q.asked_by is distinct from $2
         and (q.picked_by is null or q.pick_expires_at < now()) and {MEMBER} and {scope}"
    );
    let (open_pool, open_pool_total) = if p.is_user() {
        (
            rows(
                db,
                format!(
                    "select {question} from questions q join projects pr on pr.id = q.project_id
                     where {pool_where} order by q.created_at limit {POOL_TOP}"
                ),
                p,
                project,
            )
            .await?,
            count(
                db,
                format!(
                    "select count(*) from questions q join projects pr on pr.id = q.project_id where {pool_where}"
                ),
                p,
                project,
            )
            .await?,
        )
    } else {
        (vec![], 0)
    };

    // blocked on me
    let blocked_on_me = strip(
        rows(
            db,
            blocked_sql(
                &format!(
                    "{WORK_TASK} || jsonb_build_object('via', 'question', 'because', pr.key || '-Q' || q.number)"
                ),
                &format!(
                    "{WORK_TASK} || jsonb_build_object('via', 'dependency', 'because', op.key || '-' || ot.number)"
                ),
                "",
                &access,
                &format!("order by at desc limit {BLOCKED_TOP}"),
            ),
            p,
            project,
        )
        .await?,
    );
    let blocked_total = count(
        db,
        format!(
            "select count(*) from ({}) c",
            blocked_sql("1", "1", "", &access, "")
        ),
        p,
        project,
    )
    .await?;

    // the true totals: the lists above are the first rows only
    let waiting = count(
        db,
        format!(
            "select (select count(*) from questions q join projects pr on pr.id = q.project_id
                     where q.status = 'open' and q.to_principal = $2 and {access} and {scope})
                  + (select count(*) from reviews r join projects pr on pr.id = r.project_id
                     where {REVIEW_COND} and {access} and {scope})
                  + (select count(*) from comment_replies cr join comments cm on cm.id = cr.comment_id
                       join tasks t on t.id = cr.task_id join projects pr on pr.id = t.project_id
                     where {COMMENT_COND} and {access} and {scope})"
        ),
        p,
        project,
    )
    .await?;
    let mut counts = Map::new();
    counts.insert("owned".into(), json!(owned_total));
    counts.insert("holding".into(), json!(holding.len()));
    counts.insert("waiting".into(), json!(waiting));
    counts.insert("blocked".into(), json!(blocked_total));
    counts.insert("open_pool".into(), json!(open_pool_total));

    Ok(json!({
        "principal": p.name,
        "kind": p.kind,
        "project": project,
        "owned": owned,
        "holding": holding,
        "waiting_on_me": {
            "questions": questions, "reviews": reviews, "comments": comments,
            "open_pool": open_pool, "open_pool_total": open_pool_total,
        },
        "blocked_on_me": blocked_on_me,
        "counts": Value::Object(counts),
    }))
}

/// A page of one section of "my work" (`myWorkPage`): `owned` (open, claimed and blocked tasks I
/// own), `holding` (tasks I hold with a live lease) or `blocked` (tasks parked on a question I must
/// answer, or depending on a task I own). Highest priority first, then pot key and number; a task
/// that is blocked both ways shows once, as a question. `project` limits it to one pot.
pub async fn page(
    db: &PgPool,
    p: &Principal,
    section: &str,
    project: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let section = section.trim().to_lowercase();
    if !["owned", "holding", "blocked"].contains(&section.as_str()) {
        return crate::error::invalid("section is owned, holding or blocked");
    }
    let paging = crate::paging::paging(
        page,
        &format!("-priority,key,number:{section}"),
        3,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let project = scoped(db, p, project).await?;
    let access = access_sql("$1", "$2");
    let scope = "($3::text is null or pr.key = $3)";
    // after the last row: higher priority, or the same priority and a later (pot, number)
    let after = "($4::text is null or t.priority < $4::text::bigint
        or (t.priority = $4::text::bigint and (pr.key, t.number) > ($5::text, $6::text::bigint)))";
    let sql = match section.as_str() {
        "blocked" => {
            let inner = blocked_sql(
                &format!(
                    "{WORK_TASK} || jsonb_build_object('via', 'question', 'because', pr.key || '-Q' || q.number)"
                ),
                &format!(
                    "{WORK_TASK} || jsonb_build_object('via', 'dependency', 'because', op.key || '-' || ot.number)"
                ),
                ", t.id as tid, t.priority as pri, pr.key as pkey, t.number as num",
                &access,
                "",
            );
            format!(
                "select (u.obj - 'pot_name'), array[u.pri::text, u.pkey, u.num::text]
                 from (select distinct on (b.tid) b.* from ({inner}) b order by b.tid, b.obj->>'via' desc) u
                 where ($4::text is null or u.pri < $4::text::bigint
                        or (u.pri = $4::text::bigint and (u.pkey, u.num) > ($5::text, $6::text::bigint)))
                 order by u.pri desc, u.pkey, u.num limit $7"
            )
        }
        _ => {
            let cond = if section == "owned" {
                "t.owner_id = $2 and t.status in ('open','claimed','blocked')"
            } else {
                "t.claimed_by = $2 and t.status = 'claimed' and t.lease_expires_at > now()"
            };
            format!(
                "select ({WORK_TASK} - 'pot_name'), array[t.priority::text, pr.key, t.number::text]
                 {} where {cond} and {access} and {scope} and {after}
                 order by t.priority desc, pr.key, t.number limit $7",
                task_from("")
            )
        }
    };
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(project.as_deref())
        .bind(paging.key(0))
        .bind(paging.key(1))
        .bind(paging.key(2))
        .bind(paging.fetch())
        .fetch_all(db)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}

/// Run a keyset page query of the waiting lists: `$1` super admin, `$2` me, `$3` pot, `$4` and `$5`
/// the cursor keys, `$6` the row count.
async fn waiting_rows(
    db: &PgPool,
    p: &Principal,
    sql: String,
    project: Option<&str>,
    paging: &crate::paging::Paging,
) -> Result<(Vec<Value>, Option<String>)> {
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(project)
        .bind(paging.key(0))
        .bind(paging.key(1))
        .bind(paging.fetch())
        .fetch_all(db)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, paging))
}

/// A page of the questions waiting on me (`myWorkQuestionPage`), oldest first: those addressed to
/// me, or with `pool` (people only) the open ones in my pots that nobody picked.
pub async fn question_page(
    db: &PgPool,
    p: &Principal,
    project: Option<&str>,
    pool: bool,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    if pool && !p.is_user() {
        return Ok((vec![], None));
    }
    let order = if pool {
        "created_at,id:pool"
    } else {
        "created_at,id:mine"
    };
    let paging = crate::paging::paging(
        page,
        order,
        2,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let project = scoped(db, p, project).await?;
    let access = access_sql("$1", "$2");
    let cond = if pool {
        format!(
            "q.status = 'open' and q.to_principal is null and q.asked_by is distinct from $2
             and (q.picked_by is null or q.pick_expires_at < now()) and {MEMBER}"
        )
    } else {
        "q.status = 'open' and q.to_principal = $2".to_string()
    };
    // the id is compared as text (a uuid's canonical text sorts as the uuid does)
    let sql = format!(
        "select {QUESTION_ROW}, array[q.created_at::text, q.id::text]
         from questions q join projects pr on pr.id = q.project_id
         where {cond} and {access} and ($3::text is null or pr.key = $3)
           and ($4::text is null or q.created_at > $4::text::timestamptz
                or (q.created_at = $4::text::timestamptz and q.id::text collate \"C\" > $5::text collate \"C\"))
         order by q.created_at, q.id limit $6"
    );
    waiting_rows(db, p, sql, project.as_deref(), &paging).await
}

/// A page of the reviews waiting on me (`myWorkReviewPage`): those named to me first, then those
/// asked of anyone on the team, each oldest first.
pub async fn review_page(
    db: &PgPool,
    p: &Principal,
    project: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let paging = crate::paging::paging(
        page,
        "rn,created_at,id:reviews",
        3,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let project = scoped(db, p, project).await?;
    let access = access_sql("$1", "$2");
    // the cursor keys are (anyone-flag, created_at, id): `$4` flag, `$5` created_at, `$7` id
    let sql = format!(
        "select {REVIEW_ROW}, array[(r.reviewer is null)::int::text, r.created_at::text, r.id::text]
         from reviews r join projects pr on pr.id = r.project_id
         where {REVIEW_COND} and {access} and ($3::text is null or pr.key = $3)
           and ($4::text is null
                or (r.reviewer is null)::int > $4::text::bigint
                or ((r.reviewer is null)::int = $4::text::bigint
                    and (r.created_at > $5::text::timestamptz
                         or (r.created_at = $5::text::timestamptz and r.id::text collate \"C\" > $7::text collate \"C\"))))
         order by (r.reviewer is null)::int, r.created_at, r.id limit $6"
    );
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(p.is_super_admin())
        .bind(p.id)
        .bind(project.as_deref())
        .bind(paging.key(0))
        .bind(paging.key(1))
        .bind(paging.fetch())
        .bind(paging.key(2))
        .fetch_all(db)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}

/// A page of the people's comments a cook has not answered (`myWorkCommentPage`), oldest first;
/// empty for a person.
pub async fn comment_page(
    db: &PgPool,
    p: &Principal,
    project: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let paging = crate::paging::paging(
        page,
        "created_at,id:comments",
        2,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let project = scoped(db, p, project).await?;
    if !p.is_agent() {
        return Ok((vec![], None));
    }
    let access = access_sql("$1", "$2");
    let sql = format!(
        "select {COMMENT_ROW}, array[cm.created_at::text, cm.id::text]
         from comment_replies cr join comments cm on cm.id = cr.comment_id
           join tasks t on t.id = cr.task_id join projects pr on pr.id = t.project_id
         where {COMMENT_COND} and {access} and ($3::text is null or pr.key = $3)
           and ($4::text is null or cm.created_at > $4::text::timestamptz
                or (cm.created_at = $4::text::timestamptz and cm.id::text collate \"C\" > $5::text collate \"C\"))
         order by cm.created_at, cm.id limit $6"
    );
    waiting_rows(db, p, sql, project.as_deref(), &paging).await
}
