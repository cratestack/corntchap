//! Questions linked to tasks. A question is asked on at most one task (`questions.task_id`, the
//! implicit link); `link_question` ties an existing question, asked by anyone, to any task of its
//! pot. A link that `blocks` keeps the task from being ready (`claim_next`, `wait_for_work`, the
//! dispatcher, the board's ready column) while the question is open. When the question is answered
//! or dismissed, the cooks that care about each linked task are woken (reason `answer`): the cook
//! that linked it, the one that holds or last held the task, and the task's owner.

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result},
    questions::{self, Q},
    reviews,
    service::{emit, task_by_ref},
    state::Principal,
};

/// One row of a link as the notifier reads it: the task and why it cares.
struct Linked {
    task: Uuid,
    tref: String,
    title: String,
    status: String,
    owner: Option<Uuid>,
    linked_by: Option<Uuid>,
    blocks: bool,
}

/// Every link of a task, explicit and implicit (the question it was asked on), for `get_task`:
/// `{id, title, status, blocks, implicit, linked_by, answer, answered_by}`, oldest question first.
pub async fn of_task(c: &mut PgConnection, task: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object(
            'id', qp.key || '-Q' || qq.number, 'title', qq.title, 'status', qq.status,
            'blocks', l.blocks, 'implicit', l.implicit,
            'linked_by', (select name from principals where id = l.linked_by),
            'answer', qq.answer, 'answered_by', (select name from principals where id = qq.answered_by))
            order by qq.created_at, qq.number), '[]'::jsonb)
         from (
           select question_id, blocks, linked_by, false as implicit from question_links where task_id = $1
           union all
           select q2.id, false, q2.asked_by, true from questions q2
            where q2.task_id = $1
              and not exists (select 1 from question_links x where x.question_id = q2.id and x.task_id = $1)
         ) l join questions qq on qq.id = l.question_id join projects qp on qp.id = qq.project_id",
    )
    .bind(task)
    .fetch_one(c)
    .await?)
}

/// Whether `p` may link or unlink on `task`: whoever may edit it (people, cooks granted `plan`), or
/// the cook that holds it (or is parked on it).
async fn may_touch(c: &mut PgConnection, p: &Principal, task: Uuid) -> Result<bool> {
    if p.is_relay() {
        return Ok(false);
    }
    if p.can_plan() {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "select exists (select 1 from tasks where id = $1 and (claimed_by = $2 or reserved_for = $2))",
    )
    .bind(task)
    .bind(p.id)
    .fetch_one(c)
    .await?)
}

/// Link an open question to a task of its pot. `blocks`: the task waits on the answer. Linking
/// again changes `blocks`.
pub async fn link(
    db: &PgPool,
    p: &Principal,
    question: &str,
    task: &str,
    blocks: bool,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let q = questions::load(&mut tx, p, question, true).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    let t = task_by_ref(&mut tx, p, task).await?;
    let tref = format!("{}-{}", t.key, t.number);
    if t.project_id != q.project_id {
        return Err(Error::Invalid(format!(
            "{qref} and {tref} are in different pots: a question links only to a task of its own pot"
        )));
    }
    if matches!(t.status.as_str(), "done" | "cancelled") {
        return Err(Error::Invalid(format!(
            "{tref} is {}: there is nothing left to wait for",
            if t.status == "done" { "done" } else { "tossed" }
        )));
    }
    if q.status != "open" {
        return Err(Error::Conflict(format!(
            "{qref} is already {}: read its answer with get_question",
            q.status
        )));
    }
    if !may_touch(&mut tx, p, t.id).await? {
        return Err(Error::Forbidden(format!(
            "{} may link a question only to a task it holds (its owner can grant it `plan`: corntchap-cli cook grant {} plan)",
            p.name, p.name
        )));
    }
    sqlx::query(
        "insert into question_links (question_id, task_id, linked_by, blocks) values ($1,$2,$3,$4)
         on conflict (question_id, task_id) do update set blocks = excluded.blocks, linked_by = excluded.linked_by",
    )
    .bind(q.id)
    .bind(t.id)
    .bind(p.id)
    .bind(blocks)
    .execute(&mut *tx)
    .await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "question.linked",
        Some(p.id),
        json!({"question": qref, "task": tref, "blocks": blocks}),
    )
    .await?;
    let out = questions::view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// Take a question off a task: whoever may edit the task, its holder, or the one who linked it.
/// The question a task was asked on cannot be unlinked (it is the task's own).
pub async fn unlink(db: &PgPool, p: &Principal, question: &str, task: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let q = questions::load(&mut tx, p, question, true).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    let t = task_by_ref(&mut tx, p, task).await?;
    let tref = format!("{}-{}", t.key, t.number);
    let row: Option<(Option<Uuid>, bool)> = sqlx::query_as(
        "select linked_by, blocks from question_links where question_id = $1 and task_id = $2 for update",
    )
    .bind(q.id)
    .bind(t.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((linked_by, blocks)) = row else {
        return Err(if q.task_id == Some(t.id) {
            Error::Conflict(format!(
                "{qref} was asked on {tref}: that link is its own and stays (dismiss the question to end it)"
            ))
        } else {
            Error::NotFound(format!("{qref} is not linked to {tref}"))
        });
    };
    if linked_by != Some(p.id) && !may_touch(&mut tx, p, t.id).await? {
        return Err(Error::Forbidden(format!(
            "{} may unlink a question only from a task it holds, or one it linked",
            p.name
        )));
    }
    sqlx::query("delete from question_links where question_id = $1 and task_id = $2")
        .bind(q.id)
        .bind(t.id)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "question.unlinked",
        Some(p.id),
        json!({"question": qref, "task": tref, "blocks": blocks}),
    )
    .await?;
    let out = questions::view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// The open blocking questions on a task other than `except`, as `PAY-Q3` refs.
async fn still_blocking(c: &mut PgConnection, task: Uuid, except: Uuid) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "select qp.key || '-Q' || qq.number from question_links ql
           join questions qq on qq.id = ql.question_id join projects qp on qp.id = qq.project_id
         where ql.task_id = $1 and ql.blocks and qq.status = 'open' and qq.id <> $2
         order by qq.number",
    )
    .bind(task)
    .bind(except)
    .fetch_all(c)
    .await?)
}

/// The open blocking questions of a task, as `PAY-Q3 "title"` (the `claim_task` warning).
pub async fn open_blocking(c: &mut PgConnection, task: Uuid) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "select qp.key || '-Q' || qq.number || ' \"' || qq.title || '\"' from question_links ql
           join questions qq on qq.id = ql.question_id join projects qp on qp.id = qq.project_id
         where ql.task_id = $1 and ql.blocks and qq.status = 'open' order by qq.number",
    )
    .bind(task)
    .fetch_all(c)
    .await?)
}

/// The tasks a question is linked to (explicit links, and the one it was asked on), not finished.
async fn linked_tasks(c: &mut PgConnection, q: &Q) -> Result<Vec<Linked>> {
    let rows: Vec<(Uuid, String, String, String, Option<Uuid>, Option<Uuid>, bool)> = sqlx::query_as(
        "select t.id, pr.key || '-' || t.number, t.title, t.status, t.owner_id, l.linked_by, l.blocks
         from (
           select task_id, linked_by, blocks from question_links where question_id = $1
           union all
           select $2::uuid, null::uuid, false where $2::uuid is not null
             and not exists (select 1 from question_links x where x.question_id = $1 and x.task_id = $2)
         ) l join tasks t on t.id = l.task_id join projects pr on pr.id = t.project_id
         order by t.number",
    )
    .bind(q.id)
    .bind(q.task_id)
    .fetch_all(c)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(task, tref, title, status, owner, linked_by, blocks)| Linked {
                task,
                tref,
                title,
                status,
                owner,
                linked_by,
                blocks,
            },
        )
        .collect())
}

/// What the cooks of linked tasks are told about each task: whether it is unblocked now.
async fn task_line(c: &mut PgConnection, l: &Linked, qid: Uuid) -> Result<String> {
    let state = if l.blocks {
        let rest = still_blocking(c, l.task, qid).await?;
        if rest.is_empty() {
            "it waited on this question and is unblocked now: claim_next hands it out again once its other conditions hold"
                .to_string()
        } else {
            format!("it still waits on {}", rest.join(", "))
        }
    } else {
        "linked, it did not wait on this question".to_string()
    };
    Ok(format!(
        "- {} \"{}\": {state}",
        l.tref,
        questions::short(&l.title)
    ))
}

/// Told to the cooks of the linked tasks, once each: never one in `skip` (whoever answered, and the
/// asker, who has its own wake), and never a fire-and-forget cook ([`questions::wake`] holds those
/// back). `body` is the answer as the asker reads it ("was answered by X: ...").
pub(crate) async fn notify(
    c: &mut PgConnection,
    q: &Q,
    title: &str,
    skip: &[Uuid],
    body: &str,
) -> Result<()> {
    let links = linked_tasks(c, q).await?;
    if links.is_empty() {
        return Ok(());
    }
    // cook -> the linked tasks it cares about, in task order
    let mut by_cook: Vec<(Uuid, Vec<usize>)> = vec![];
    let add = |cook: Uuid, i: usize, by_cook: &mut Vec<(Uuid, Vec<usize>)>| match by_cook
        .iter_mut()
        .find(|(c, _)| *c == cook)
    {
        Some((_, v)) => {
            if !v.contains(&i) {
                v.push(i)
            }
        }
        None => by_cook.push((cook, vec![i])),
    };
    for (i, l) in links.iter().enumerate() {
        if matches!(l.status.as_str(), "done" | "cancelled") {
            continue;
        }
        let mut who: Vec<Uuid> = vec![];
        who.extend(l.linked_by);
        who.extend(reviews::responsible(c, l.task).await?);
        who.extend(l.owner);
        for w in who {
            if !skip.contains(&w) && reviews::is_agent(c, w).await? {
                add(w, i, &mut by_cook);
            }
        }
    }
    let mut lines: HashMap<usize, String> = HashMap::new();
    for (_, idx) in &by_cook {
        for i in idx {
            if !lines.contains_key(i) {
                lines.insert(*i, task_line(c, &links[*i], q.id).await?);
            }
        }
    }
    for (cook, idx) in by_cook {
        let tasks: Vec<&str> = idx.iter().map(|i| lines[i].as_str()).collect();
        let prompt = format!(
            "Question {} \"{}\" {body}\n\nLinked to {}:\n{}",
            format_args!("{}-Q{}", q.key, q.number),
            questions::short(title),
            if idx.len() == 1 {
                "your task"
            } else {
                "your tasks"
            },
            tasks.join("\n")
        );
        questions::wake(
            c,
            q.project_id,
            &q.key,
            cook,
            None,
            Some(links[idx[0]].task),
            &prompt,
            "answer",
        )
        .await?;
    }
    Ok(())
}
