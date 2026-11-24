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
        .bind(p.is_admin)
        .bind(p.id)
        .bind(project)
        .fetch_all(db)
        .await?)
}

async fn count(db: &PgPool, sql: String, p: &Principal, project: Option<&str>) -> Result<i64> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.is_admin)
        .bind(p.id)
        .bind(project)
        .fetch_one(db)
        .await?)
}

/// The tasks blocked on me: SQL with the select list `sel` (an object, or `1` for a count).
fn blocked_sql(question_sel: &str, dep_sel: &str, access: &str, tail: &str) -> String {
    format!(
        "select obj from (
           select {question_sel} as obj, t.updated_at as at
           from questions q join tasks t on t.id = q.task_id join projects pr on pr.id = t.project_id
             cross join lateral (select {TASK_JSON} as j) x
           where q.status = 'open' and q.to_principal = $2 and t.status = 'blocked'
             and {access} and ($3::text is null or pr.key = $3)
           union all
           select {dep_sel} as obj, t.updated_at as at
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
    let question =
        "jsonb_build_object('id', pr.key || '-Q' || q.number, 'project', pr.key, 'title', q.title,
        'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
        'from', (select name from principals where id = q.asked_by), 'at', q.created_at)";
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
            "select jsonb_build_object('id', pr.key || '-R' || r.number, 'project', pr.key,
                 'task', (select pr.key || '-' || t.number from tasks t where t.id = r.task_id),
                 'task_title', (select t.title from tasks t where t.id = r.task_id),
                 'from', (select name from principals where id = r.requested_by),
                 'note', r.note, 'round', r.round, 'at', r.created_at)
             from reviews r join projects pr on pr.id = r.project_id
             where r.state = 'requested' and {access} and {scope}
               and (r.reviewer = $2
                    or (r.reviewer is null and r.requested_by is distinct from $2 and {MEMBER}))
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
                "select jsonb_build_object('task', pr.key || '-' || t.number, 'project', pr.key,
                     'author', (select name from principals where id = cm.author_id),
                     'excerpt', case when char_length(cm.body) > 160 then left(cm.body, 160) || '…' else cm.body end,
                     'at', cm.created_at)
                 from comment_replies cr join comments cm on cm.id = cr.comment_id
                   join tasks t on t.id = cr.task_id join projects pr on pr.id = t.project_id
                 where cr.agent_id = $2 and cr.answered_at is null and t.status <> 'cancelled' and {access} and {scope}
                   and not exists (select 1 from events e where e.task_id = cr.task_id and e.actor_id = cr.agent_id
                     and e.seq > cr.since_seq
                     and e.kind in ('task.commented', 'task.completed', 'task.released', 'task.blocked'))
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
            blocked_sql("1", "1", &access, "")
        ),
        p,
        project,
    )
    .await?;

    let waiting = (questions.len() + reviews.len() + comments.len()) as i64;
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
