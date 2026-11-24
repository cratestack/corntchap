//! The task service: every MCP tool and REST handler calls these functions,
//! so the two surfaces cannot disagree.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    state::Principal,
};

pub const MAX_TTL_SECS: f64 = 86_400.0;

/// Seconds a cook's own release keeps a task out of its `claim_next` (and `wait_for_work`, and
/// the dispatcher's wakes for it): a cook that gives a task back means someone else should take
/// it, so it must not get the same card straight back. `RELEASE_COOLDOWN_MINUTES` sets it; 0 turns
/// it off. `claim_task` by name is never held back.
pub static RELEASE_COOLDOWN_SECS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(900);

pub fn release_cooldown_secs() -> u64 {
    RELEASE_COOLDOWN_SECS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Seconds a holder may go without progress (a comment, a ticked item, an update, a question, a
/// review request, a saved memory, a forge event on the task) before its lease ends:
/// `STALL_MINUTES`, default 120. Bare heartbeats and activity renewals are not progress.
pub static STALL_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(7_200);
/// Seconds one holder may keep a task however much it progresses: `MAX_HOLD_HOURS`, default 8.
pub static MAX_HOLD_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(28_800);

pub fn stall_secs() -> u64 {
    STALL_SECS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn max_hold_secs() -> u64 {
    MAX_HOLD_SECS.load(std::sync::atomic::Ordering::Relaxed)
}

/// SQL expression: the latest a lease on task alias `t` may last, whatever its ttl: the stall
/// limit after the last progress, or the maximum hold after the claim, whichever comes first.
pub fn lease_cap_sql(t: &str) -> String {
    format!(
        "least(coalesce({t}.progress_at, {t}.updated_at) + make_interval(secs => {stall}),
               coalesce({t}.held_since, {t}.updated_at) + make_interval(secs => {max}))",
        stall = stall_secs(),
        max = max_hold_secs(),
    )
}

/// Task events (by the holder, or for `forge.*` by anyone) that count as progress.
pub const PROGRESS_KINDS: &[&str] = &[
    "task.commented",
    "item.checked",
    "task.updated",
    "review.requested",
    "question.asked",
    "memory.saved",
];

/// Give-backs by one cook (releases and lapsed leases) after which `claim_next` skips the task for it.
pub const BOUNCE_LIMIT: i64 = 3;
/// Give-backs by any cooks, with no person touching the task between, after which the task is
/// blocked for a person.
pub const BOUNCE_BLOCK: i64 = 5;

/// SQL predicate on `t` (tasks): no open pull request that is in good shape is linked to the task.
/// A task with a healthy open PR and no live lease is not free work: its author is waiting on CI or
/// review, and forge events wake it. A PR that is failing, has changes requested or has conflicts
/// is parked: the task is work again. `claim_task` by name takes it either way (with a warning).
pub const NO_OPEN_PR_SQL: &str = "not exists (select 1 from forge_prs fp where fp.task_id = t.id and fp.state = 'open' and not fp.unlinked
    and not (fp.checks = 'failure' or cardinality(fp.failing) > 0 or fp.review = 'changes_requested' or fp.mergeable = 'false'))";

/// SQL predicate on `t` (tasks): open, every dependency and subtask finished (ready to claim,
/// before the routing and cooldown rules).
pub const READY_SQL: &str = "t.status = 'open'
  and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled'))
  and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))";

/// SQL predicate on `t` (tasks): what `claim_next`, `wait_for_work` and work wakes hand `agent`
/// beyond the routing rules: not a task it released a moment ago, not one it keeps giving back, not
/// one with a healthy open PR.
pub fn takeable_sql(agent: &str) -> String {
    format!(
        "{} and not {} and {NO_OPEN_PR_SQL}",
        cooldown_sql(agent),
        bounce_sql(agent, BOUNCE_LIMIT)
    )
}

/// The events that count as someone touching a task a cook gave back.
const TOUCH_KINDS: &str =
    "'task.updated', 'task.commented', 'task.unblocked', 'task.restored', 'question.answered'";

/// SQL predicate on `t` (tasks): `agent` has given the task back at least `n` times (released it,
/// or let its lease lapse) with nobody else touching it in between (the touches of
/// [`given_back_sql`]). A release made while a healthy pull request was open does not count: the cook was
/// waiting on CI or review, not bouncing. The cook is not handed the task again until someone
/// touches it: a comment, an edit, an answer, a forge event.
pub fn bounce_sql(agent: &str, n: i64) -> String {
    format!(
        "((select count(*) from (select 1 from events r
           where r.task_id = t.id and r.kind in ('task.released', 'task.lease_expired')
             and ((r.kind = 'task.released' and r.actor_id = {agent})
                  or (r.kind = 'task.lease_expired' and r.payload->>'agent_id' = ({agent})::text))
             and coalesce(r.payload->>'on_pr', 'false') <> 'true'
             and not exists (select 1 from events x
               where x.task_id = t.id and x.seq > r.seq
                 and x.actor_id is distinct from {agent}
                 and (x.kind in ({TOUCH_KINDS}) or x.kind like 'forge.%'))
           limit {n}) capped) >= {n})"
    )
}

/// SQL predicate on `t` (tasks): the task is not one `agent` released within the cooldown.
/// Uncorrelated on the task, so it runs once per query over the recent events (`events_at`).
pub fn cooldown_sql(agent: &str) -> String {
    format!(
        "not (t.id = any(array(select e.task_id from events e
            where e.at > now() - make_interval(secs => {secs}) and e.kind = 'task.released'
              and e.actor_id = {agent} and e.task_id is not null)))",
        secs = release_cooldown_secs(),
    )
}

/// SQL predicate on `t` (tasks): `agent` gave the task back (released it, or let its lease lapse)
/// and nobody else has touched it since (an update, a comment, an unblock or restore, an answer, a
/// forge event by anyone else). A cloud cook is not woken for such a task again: it gave it back,
/// so it would only give it back again. Any give-back with no later touch counts, so an older one
/// that someone has touched since does not hide a newer one.
pub fn given_back_sql(agent: &str) -> String {
    format!(
        "exists (select 1 from events r
           where r.task_id = t.id
             and ((r.kind = 'task.released' and r.actor_id = {agent})
                  or (r.kind = 'task.lease_expired' and r.payload->>'agent_id' = ({agent})::text))
             and not exists (select 1 from events x
               where x.task_id = t.id and x.seq > r.seq
                 and x.actor_id is distinct from {agent}
                 and (x.kind in ({TOUCH_KINDS}) or x.kind like 'forge.%')))"
    )
}

/// SQL expression producing the JSON shape of a task. Needs aliases `t` (tasks) and `pr` (projects).
pub const TASK_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-' || t.number, 'project', pr.key, 'number', t.number,
  'title', t.title, 'body', t.body, 'priority', t.priority, 'labels', to_jsonb(t.labels), 'repos', to_jsonb(t.repos),
  'status', t.status,
  'parent', (select pp.key || '-' || pt.number from tasks pt join projects pp on pp.id = pt.project_id where pt.id = t.parent_id),
  'depends_on', coalesce((select jsonb_agg(dp.key || '-' || dt.number order by dp.key, dt.number)
       from task_deps d join tasks dt on dt.id = d.depends_on join projects dp on dp.id = dt.project_id
       where d.task_id = t.id), '[]'::jsonb),
  'claimed_by', (select name from principals where id = t.claimed_by),
  'owner', (select name from principals where id = t.owner_id),
  'station', case when t.status in ('claimed','blocked') then (select ref from stations where id = t.station_id) end,
  'last_station', case when t.status not in ('claimed','blocked') then (select ref from stations where id = t.station_id) end,
  'lease_epoch', t.lease_epoch, 'lease_expires_at', t.lease_expires_at,
  'lease_live', (t.status = 'claimed' and t.lease_expires_at > now()),
  'ready', (t.status = 'open' and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
       where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))),
  'open_items', (select count(*) from items i where i.task_id = t.id and not i.done),
  'total_items', (select count(*) from items i where i.task_id = t.id),
  'menu', (select m.key from menus m where m.id = t.menu_id),
  'result', t.result, 'created_at', t.created_at, 'updated_at', t.updated_at,
  'pr', (select jsonb_build_object('repo', x.repo, 'number', x.number, 'url', x.url, 'state', x.state,
      'draft', x.draft, 'checks', x.checks, 'review', x.review, 'conflict', x.mergeable = 'false',
      'count', (select count(*) from forge_prs y where y.task_id = t.id))
    from forge_prs x where x.task_id = t.id order by (x.state = 'open') desc, x.updated_at desc limit 1),
  'review', (select jsonb_build_object('id', pr.key || '-R' || rv.number, 'round', rv.round,
      'state', rv.state, 'reviewer', (select name from principals where id = rv.reviewer),
      'reviewed_by', (select name from principals where id = rv.reviewed_by),
      'requested_by', (select name from principals where id = rv.requested_by),
      'at', coalesce(rv.decided_at, rv.created_at))
    from reviews rv where rv.task_id = t.id and rv.state <> 'cancelled'
    order by rv.round desc, (rv.state = 'requested') desc, rv.created_at desc limit 1))"#;

/// Characters of a task's body or result kept in a list (`list_tasks`); `get_task` has the whole text.
pub const LIST_CUT: usize = 280;

/// SQL: the column cut to [`LIST_CUT`] characters, with an ellipsis when something was cut.
fn cut_sql(col: &str) -> String {
    format!(
        "case when char_length({col}) > {LIST_CUT} then left({col}, {LIST_CUT}) || '…' else {col} end"
    )
}

/// Project visibility predicate; $N = is_admin, $M = principal id.
pub(crate) fn access_sql(admin: &str, pid: &str) -> String {
    format!(
        "({admin} or exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = {pid}))"
    )
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct ProjectRow {
    pub id: Uuid,
    pub team_id: Uuid,
    pub key: String,
    pub name: String,
    pub repo_url: Option<String>,
    pub ttl_secs: f64,
}

pub async fn project_by_key(c: &mut PgConnection, p: &Principal, key: &str) -> Result<ProjectRow> {
    let sql = format!(
        "select pr.id, pr.team_id, pr.key, pr.name, pr.repo_url, extract(epoch from pr.lease_ttl)::float8 as ttl_secs \
         from projects pr where pr.key = $1 and {}",
        access_sql("$2", "$3")
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(key)
        .bind(p.is_admin)
        .bind(p.id)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::NotFound(format!("project {key} not found")))
}

pub fn parse_task_ref(s: &str) -> Result<(String, i64)> {
    let (k, n) = s
        .trim()
        .rsplit_once('-')
        .ok_or_else(|| Error::Invalid(format!("bad task id {s:?}, expected like PAY-42")))?;
    let n: i64 = n
        .parse()
        .map_err(|_| Error::Invalid(format!("bad task id {s:?}, expected like PAY-42")))?;
    Ok((k.to_uppercase(), n))
}

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct TaskRef {
    pub id: Uuid,
    pub project_id: Uuid,
    pub key: String,
    pub number: i64,
    pub status: String,
    pub claimed_by: Option<Uuid>,
    pub lease_epoch: i64,
    pub ttl_secs: f64,
}

pub async fn task_by_ref(c: &mut PgConnection, p: &Principal, r: &str) -> Result<TaskRef> {
    let (key, number) = parse_task_ref(r)?;
    let sql = format!(
        "select t.id, t.project_id, pr.key, t.number, t.status, t.claimed_by, t.lease_epoch, \
         extract(epoch from pr.lease_ttl)::float8 as ttl_secs \
         from tasks t join projects pr on pr.id = t.project_id \
         where pr.key = $1 and t.number = $2 and {}",
        access_sql("$3", "$4")
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(&key)
        .bind(number)
        .bind(p.is_admin)
        .bind(p.id)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::NotFound(format!("task {r} not found")))
}

pub async fn emit(
    c: &mut PgConnection,
    project: Option<Uuid>,
    task: Option<Uuid>,
    kind: &str,
    actor: Option<Uuid>,
    payload: Value,
) -> Result<i64> {
    emit_to(c, project, task, kind, actor, payload, None).await
}

/// An event only `audience` sees in the feed and the live stream (`None`: everyone in the pot).
pub async fn emit_to(
    c: &mut PgConnection,
    project: Option<Uuid>,
    task: Option<Uuid>,
    kind: &str,
    actor: Option<Uuid>,
    payload: Value,
    audience: Option<&[Uuid]>,
) -> Result<i64> {
    let seq: i64 = sqlx::query_scalar(
        "insert into events (project_id, task_id, kind, actor_id, payload, audience) values ($1,$2,$3,$4,$5,$6) returning seq",
    )
    .bind(project)
    .bind(task)
    .bind(kind)
    .bind(actor)
    .bind(payload)
    .bind(audience)
    .fetch_one(&mut *c)
    .await?;
    if let Some(t) = task {
        note_progress(c, t, kind, actor).await?;
    }
    Ok(seq)
}

/// The one place progress is recorded (every event passes through [`emit_to`]): an event on a
/// claimed task that counts as progress (see [`PROGRESS_KINDS`]: the holder's own; any `forge.*`
/// event) moves `progress_at` to now. The holder's progress also extends a lease that sits at its
/// stall limit, so the stall clock and the lease run on together.
async fn note_progress(
    c: &mut PgConnection,
    task: Uuid,
    kind: &str,
    actor: Option<Uuid>,
) -> Result<()> {
    if kind.starts_with("forge.") {
        sqlx::query("update tasks set progress_at = now() where id = $1 and status = 'claimed' and lease_expires_at > now()")
            .bind(task)
            .execute(c)
            .await?;
    } else if PROGRESS_KINDS.contains(&kind)
        && let Some(actor) = actor
    {
        let sql = format!(
            "update tasks t set progress_at = now(),
               lease_expires_at = case when t.lease_expires_at > now()
                 then greatest(t.lease_expires_at, least(now() + pr.lease_ttl,
                        now() + make_interval(secs => {stall}), coalesce(t.held_since, now()) + make_interval(secs => {max})))
                 else t.lease_expires_at end
             from projects pr where pr.id = t.project_id and t.id = $1 and t.status = 'claimed' and t.claimed_by = $2
               and t.lease_expires_at > now()",
            stall = stall_secs(),
            max = max_hold_secs(),
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(task)
            .bind(actor)
            .execute(c)
            .await?;
    }
    Ok(())
}

pub(crate) async fn task_json(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id where t.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// Task with its items, comments and dependency states.
pub(crate) async fn task_full(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let mut t = task_json(c, id).await?;
    let items: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('position', i.position, 'text', i.text, 'done', i.done,
            'done_by', (select name from principals where id = i.done_by)) order by i.position), '[]'::jsonb)
         from items i where i.task_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let comments: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('author', (select name from principals where id = cm.author_id),
            'body', cm.body, 'at', cm.created_at,
            'station', (select s.ref from stations s where s.id = cm.station_id),
            'station_label', (select coalesce(s.label, s.directory, s.session_ref) from stations s where s.id = cm.station_id))
            order by cm.created_at), '[]'::jsonb)
         from comments cm where cm.task_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let deps: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('id', dp.key || '-' || dt.number, 'title', dt.title, 'status', dt.status)), '[]'::jsonb)
         from task_deps d join tasks dt on dt.id = d.depends_on join projects dp on dp.id = dt.project_id where d.task_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let subtasks: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('id', sp.key || '-' || st.number, 'title', st.title, 'status', st.status) order by st.number), '[]'::jsonb)
         from tasks st join projects sp on sp.id = st.project_id where st.parent_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let files: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('id', fp.key || '-F' || f.number, 'name', f.name,
            'content_type', f.content_type, 'size', f.size, 'status', f.status) order by f.number), '[]'::jsonb)
         from files f join projects fp on fp.id = f.project_id where f.task_id = $1 and f.status <> 'deleted'",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?;
    let o = t.as_object_mut().unwrap();
    o.insert("items".into(), items);
    o.insert("comments".into(), comments);
    o.insert("dependencies".into(), deps);
    o.insert("subtasks".into(), subtasks);
    o.insert("files".into(), files);
    o.insert("prs".into(), crate::forge::link::prs_of(c, id).await?);
    o.insert("reviews".into(), crate::reviews::of_task(c, id).await?);
    Ok(t)
}

pub async fn get_task(db: &PgPool, p: &Principal, r: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let t = task_by_ref(&mut c, p, r).await?;
    task_full(&mut c, t.id).await
}

pub async fn list_projects(db: &PgPool, p: &Principal) -> Result<Value> {
    let sql = format!(
        "select jsonb_build_object('key', pr.key, 'name', pr.name, 'repo_url', pr.repo_url,
            'repos', {repos},
            'team', (select name from teams where id = pr.team_id),
            'lease_ttl_s', extract(epoch from pr.lease_ttl)::float8,
            'guide', pr.guide,
            'open', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'open'),
            'ready', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'open'
                and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))),
            'claimed', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'claimed'),
            'blocked', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'blocked'),
            'done', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'done'),
            'tossed', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'cancelled'),
            'open_questions', (select count(*) from questions q where q.project_id = pr.id and q.status = 'open'),
            'waiting_reviews', (select count(*) from reviews rv where rv.project_id = pr.id and rv.state = 'requested'
                and (rv.reviewer = $2 or (rv.reviewer is null and rv.requested_by is distinct from $2))),
            'unread_pass', (select count(*) from pass_messages m
                where m.project_id = pr.id and m.author_id is distinct from $2 and m.relayed_by is distinct from $2
                  and (m.to_principal is null or m.to_principal = $2)
                  and m.seq > coalesce((select r.last_seq from pass_reads r where r.principal_id = $2 and r.project_id = pr.id
                      and r.conversation = case when m.to_principal is null then 'pot' else m.author_id::text end), 0)))
         from projects pr where {} order by pr.key",
        access_sql("$1", "$2"),
        repos = crate::repos::repos_json_sql("pr.id"),
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.is_admin)
        .bind(p.id)
        .fetch_all(db)
        .await?;
    Ok(Value::Array(rows))
}

pub async fn list_tasks(
    db: &PgPool,
    p: &Principal,
    project: &str,
    status: Option<&str>,
    label: Option<&str>,
    ready_only: bool,
    menu: Option<&str>,
    owner: Option<&str>,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, project).await?;
    // owner: `me`, a handle, or `-` (nobody); empty or `anyone` is no filter
    let owner = match owner.map(str::trim).filter(|o| !o.is_empty()) {
        None => None,
        Some(o) if o.eq_ignore_ascii_case("anyone") => None,
        Some(o) if o.eq_ignore_ascii_case("me") => Some(p.name.clone()),
        Some(o) => Some(o.to_string()),
    };
    // A list is for choosing: bodies and results are cut (get_task has them whole). Agents call this
    // on pots with long histories, where full bodies made one call cost a hundred thousand tokens.
    let sql = format!(
        "select {TASK_JSON} || jsonb_build_object(
                'body', {body}, 'result', {result})
         from tasks t join projects pr on pr.id = t.project_id
         where t.project_id = $1
           and ($2::text is null or t.status = $2)
           and ($3::text is null or $3 = any(t.labels))
           and ($5::text is null or ($5 = '-' and t.menu_id is null)
                or exists (select 1 from menus m where m.id = t.menu_id and m.key = $5))
           and ($6::text is null or ($6 = '-' and t.owner_id is null)
                or t.owner_id = (select id from principals where name = $6))
           and (not $4 or (t.status = 'open' and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                 where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))))
         order by t.priority desc, t.number",
        body = cut_sql("t.body"),
        result = cut_sql("t.result"),
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(status)
        .bind(label)
        .bind(ready_only)
        .bind(menu.map(|m| {
            if m.trim() == "-" {
                "-".to_string()
            } else {
                crate::menus::slug(m)
            }
        }))
        .bind(owner)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows))
}

pub struct NewTask {
    pub project: String,
    pub title: String,
    pub body: String,
    pub items: Vec<String>,
    pub parent: Option<String>,
    pub depends_on: Vec<String>,
    pub labels: Vec<String>,
    /// Aliases of the pot's repositories the task is about (empty: any).
    pub repos: Vec<String>,
    pub priority: i64,
    /// Menu key (milestone) to put the task on.
    pub menu: Option<String>,
    /// Handle of the task's owner (a person or cook in the pot's team).
    pub owner: Option<String>,
}

/// The principal a task's owner handle names. Empty: nobody. The owner is in the pot's team.
async fn owner_id(
    c: &mut PgConnection,
    project: Uuid,
    key: &str,
    handle: &str,
) -> Result<Option<Uuid>> {
    let handle = handle.trim().trim_start_matches('@');
    if handle.is_empty() {
        return Ok(None);
    }
    let id: Option<Uuid> = sqlx::query_scalar(
        "select p.id from principals p join projects pr on pr.id = $2
           join team_members tm on tm.principal_id = p.id and tm.team_id = pr.team_id
         where p.name = $1 and p.kind <> 'relay' and p.disabled_at is null",
    )
    .bind(handle)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?;
    match id {
        Some(id) => Ok(Some(id)),
        None => invalid(format!(
            "{handle} is not in the team of {key}: an owner is a person or a cook of the pot's team (list_team_members)"
        )),
    }
}

pub async fn create_task(db: &PgPool, p: &Principal, n: NewTask) -> Result<Value> {
    if n.title.trim().is_empty() {
        return invalid("title is required");
    }
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &n.project).await?;
    let parent = match &n.parent {
        Some(r) => Some(task_by_ref(&mut tx, p, r).await?.id),
        None => None,
    };
    let mut deps = vec![];
    for d in &n.depends_on {
        deps.push(task_by_ref(&mut tx, p, d).await?.id);
    }
    let menu = match n.menu.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => Some(crate::menus::open_menu_id(&mut tx, pr.id, &pr.key, m).await?),
        None => None,
    };
    let repos = crate::repos::check_aliases(&mut tx, pr.id, &pr.key, &n.repos).await?;
    let owner = match n.owner.as_deref() {
        Some(h) => owner_id(&mut tx, pr.id, &pr.key, h).await?,
        None => None,
    };
    let number: i64 = sqlx::query_scalar(
        "update projects set next_number = next_number + 1 where id = $1 returning next_number - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *tx)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into tasks (id, project_id, number, title, body, priority, labels, parent_id, status, created_by, menu_id, repos, owner_id)
         values ($1,$2,$3,$4,$5,$6,$7,$8,'open',$9,$10,$11,$12)",
    )
    .bind(id)
    .bind(pr.id)
    .bind(number)
    .bind(n.title.trim())
    .bind(&n.body)
    .bind(n.priority)
    .bind(&n.labels)
    .bind(parent)
    .bind(p.id)
    .bind(menu)
    .bind(&repos)
    .bind(owner)
    .execute(&mut *tx)
    .await?;
    for (i, text) in n.items.iter().enumerate() {
        sqlx::query("insert into items (task_id, position, text) values ($1,$2,$3)")
            .bind(id)
            .bind(i as i64 + 1)
            .bind(text)
            .execute(&mut *tx)
            .await?;
    }
    for d in deps {
        sqlx::query(
            "insert into task_deps (task_id, depends_on) values ($1,$2) on conflict do nothing",
        )
        .bind(id)
        .bind(d)
        .execute(&mut *tx)
        .await?;
    }
    emit(
        &mut tx,
        Some(pr.id),
        Some(id),
        "task.created",
        Some(p.id),
        json!({"title": n.title.trim(), "labels": n.labels, "repos": repos, "menu": n.menu.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(crate::menus::slug)}),
    )
    .await?;
    if owner.is_some() {
        emit_owner_changed(&mut tx, pr.id, id, p.id, None, owner).await?;
    }
    let out = task_full(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

/// `task.owner_changed` with the old and the new owner (handles).
async fn emit_owner_changed(
    c: &mut PgConnection,
    project: Uuid,
    task: Uuid,
    actor: Uuid,
    old: Option<Uuid>,
    new: Option<Uuid>,
) -> Result<()> {
    let old_name: Option<String> = match old {
        Some(id) => {
            sqlx::query_scalar("select name from principals where id = $1")
                .bind(id)
                .fetch_optional(&mut *c)
                .await?
        }
        None => None,
    };
    let new_name: Option<String> = match new {
        Some(id) => {
            sqlx::query_scalar("select name from principals where id = $1")
                .bind(id)
                .fetch_optional(&mut *c)
                .await?
        }
        None => None,
    };
    emit(
        c,
        Some(project),
        Some(task),
        "task.owner_changed",
        Some(actor),
        json!({"old_owner": old_name, "new_owner": new_name}),
    )
    .await?;
    Ok(())
}

pub struct TaskEdit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    /// Some(empty) clears the repositories.
    pub repos: Option<Vec<String>>,
    pub priority: Option<i64>,
    /// Some("") takes the task off its menu.
    pub menu: Option<String>,
    /// Some(handle) sets the owner (a person or cook of the pot's team); Some("") clears it.
    pub owner: Option<String>,
}

pub async fn update_task(db: &PgPool, p: &Principal, r: &str, e: TaskEdit) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    if !p.can_plan() {
        let live: bool = sqlx::query_scalar(
            "select status = 'claimed' and claimed_by = $2 and lease_expires_at > now() from tasks where id = $1",
        )
        .bind(t.id)
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
        if !live {
            return Err(Error::Forbidden(format!(
                "{} may only edit tasks it currently holds (its owner can grant it `plan`: corntchap-cli cook grant {} plan)",
                p.name, p.name
            )));
        }
    }
    let repos = match &e.repos {
        Some(r) => Some(crate::repos::check_aliases(&mut tx, t.project_id, &t.key, r).await?),
        None => None,
    };
    let (old_owner, new_owner) = match e.owner.as_deref() {
        Some(h) => {
            let new = owner_id(&mut tx, t.project_id, &t.key, h).await?;
            let old: Option<Uuid> = sqlx::query_scalar("select owner_id from tasks where id = $1")
                .bind(t.id)
                .fetch_one(&mut *tx)
                .await?;
            (old, Some(new))
        }
        None => (None, None),
    };
    sqlx::query(
        "update tasks set repos = coalesce($6, repos), title = coalesce($2, title), body = coalesce($3, body), labels = coalesce($4, labels),
            priority = coalesce($5, priority), updated_at = now() where id = $1",
    )
    .bind(t.id)
    .bind(e.title.as_deref())
    .bind(e.body.as_deref())
    .bind(e.labels.as_deref())
    .bind(e.priority)
    .bind(repos.as_deref())
    .execute(&mut *tx)
    .await?;
    if let Some(new) = new_owner
        && new != old_owner
    {
        sqlx::query("update tasks set owner_id = $2 where id = $1")
            .bind(t.id)
            .bind(new)
            .execute(&mut *tx)
            .await?;
        emit_owner_changed(&mut tx, t.project_id, t.id, p.id, old_owner, new).await?;
    }
    if let Some(m) = e.menu.as_deref().map(str::trim) {
        let menu = if m.is_empty() {
            None
        } else {
            Some(crate::menus::open_menu_id(&mut tx, t.project_id, &t.key, m).await?)
        };
        sqlx::query("update tasks set menu_id = $2 where id = $1")
            .bind(t.id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
    }
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.updated",
        Some(p.id),
        json!({}),
    )
    .await?;
    let out = task_full(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}

async fn live_leases(c: &mut PgConnection, agent: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "select count(*) from tasks where claimed_by = $1 and status = 'claimed' and lease_expires_at > now()",
    )
    .bind(agent)
    .fetch_one(c)
    .await?)
}

/// A lease length asked for (seconds), else the pot's: at least 10 seconds, at most a day, and
/// never past the stall limit or the maximum hold.
fn lease_ttl(asked: Option<f64>, pot: f64) -> f64 {
    asked
        .unwrap_or(pot)
        .clamp(10.0, MAX_TTL_SECS)
        .min(stall_secs() as f64)
        .min(max_hold_secs() as f64)
}

/// Less than this much time before a lease must stall, the holder is warned.
const STALL_WARNING_MINS: i64 = 20;

/// When the lease on a held task stalls: the stall limit after its last progress, or the maximum
/// hold after the claim, whichever comes first; and, with under 20 minutes left, what to do.
pub async fn stall_clock(
    c: &mut PgConnection,
    task: Uuid,
) -> Result<(DateTime<Utc>, Option<String>)> {
    let (progress, held): (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "select coalesce(progress_at, updated_at), coalesce(held_since, updated_at) from tasks where id = $1",
    )
    .bind(task)
    .fetch_one(c)
    .await?;
    let stall_at = progress + chrono::Duration::seconds(stall_secs() as i64);
    let max_at = held + chrono::Duration::seconds(max_hold_secs() as i64);
    let fmt = |t: DateTime<Utc>| t.format("%Y-%m-%d %H:%M UTC").to_string();
    let (at, by_hold) = if max_at < stall_at {
        (max_at, true)
    } else {
        (stall_at, false)
    };
    let warning = (at - Utc::now() < chrono::Duration::minutes(STALL_WARNING_MINS)).then(|| {
        if by_hold {
            format!(
                "held since {}: the lease ends at {} whatever you do (maximum hold {}): finish, or \
                 release or block it with `learned`, before then.",
                fmt(held),
                fmt(at),
                hours(max_hold_secs())
            )
        } else {
            format!(
                "no progress since {}: comment or tick a checklist item, or the task is released at {}.",
                fmt(progress),
                fmt(at)
            )
        }
    });
    Ok((at, warning))
}

pub fn hours_text(secs: u64) -> String {
    hours(secs)
}

fn hours(secs: u64) -> String {
    if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else {
        format!("{} min", secs.div_ceil(60))
    }
}

fn lease_json(epoch: i64, expires: DateTime<Utc>, clock: (DateTime<Utc>, Option<String>)) -> Value {
    let mut l = json!({ "epoch": epoch, "expires_at": expires, "stalls_at": clock.0 });
    if let Some(w) = clock.1 {
        l["warning"] = json!(w);
    }
    l
}

fn lease_view(
    t: Value,
    epoch: i64,
    expires: DateTime<Utc>,
    clock: (DateTime<Utc>, Option<String>),
) -> Value {
    json!({ "task": t, "lease": lease_json(epoch, expires, clock) })
}

/// An agent that was away (see `auth`: 30 minutes of silence) takes no lease until it has been
/// briefed on the pot: its first claim there after the absence returns a `catch_up` instead of a
/// task (never `none`: loops stop on that), records the briefing, and the next claim goes through.
/// what_now, get_board, my_work and changes_since with a project brief too. People are never gated.
async fn catch_up_gate(
    c: &mut PgConnection,
    p: &Principal,
    project: Uuid,
    key: &str,
) -> Result<Option<Value>> {
    if !p.is_agent() {
        return Ok(None);
    }
    let gated: bool = sqlx::query_scalar(
        "select p.back_at is not null and (b.briefed_at is null or b.briefed_at < p.back_at)
         from principals p left join pot_briefings b on b.principal_id = p.id and b.project_id = $2
         where p.id = $1",
    )
    .bind(p.id)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?
    .unwrap_or(false);
    if !gated {
        return Ok(None);
    }
    let cu = crate::what_now::catch_up(c, p, project, key).await?;
    crate::what_now::brief(c, p, &[project]).await?;
    let away = cu["away_for"].as_str().unwrap_or("a while").to_string();
    let hold = cu["holding"]
        .as_array()
        .and_then(|h| h.first())
        .and_then(|t| t["id"].as_str())
        .map(|t| format!(" You still hold {t}: finish it first."))
        .unwrap_or_default();
    Ok(Some(json!({
        "catch_up": cu,
        "next": format!(
            "You were away {away}: read this (what changed, what you hold, what waits on you, the \
             memories that matter), then claim again.{hold}"
        ),
    })))
}

pub async fn claim_next(
    db: &PgPool,
    p: &Principal,
    project: &str,
    labels: &[String],
    menu: Option<&str>,
    station: Option<&str>,
    ttl_s: Option<f64>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, project).await?;
    if let Some(v) = catch_up_gate(&mut tx, p, pr.id, &pr.key).await? {
        tx.commit().await?;
        return Ok(v);
    }
    let ttl = lease_ttl(ttl_s, pr.ttl_secs);
    let st = crate::stations::for_claim(&mut tx, p, station, pr.id).await?;
    let only_repo = crate::stations::confined_repo(&mut tx, st).await?;
    let menu = menu.map(crate::menus::slug).filter(|m| !m.is_empty());
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(p.id.to_string())
        .execute(&mut *tx)
        .await?;
    if live_leases(&mut tx, p.id).await? >= p.max_leases {
        return Err(Error::MaxLeases);
    }
    #[allow(clippy::type_complexity)]
    let row: Option<(Uuid, i64, DateTime<Utc>, String, Option<Uuid>, Option<DateTime<Utc>>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "with next as (
           select t.id, t.status as was_status, t.claimed_by as was_holder, t.lease_expires_at as was_expiry
           from tasks t
           where t.project_id = $1
             and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
             and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                             where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
             and (cardinality($4::text[]) = 0 or t.labels && $4::text[])
             and ($5::text is null or exists (select 1 from menus m where m.id = t.menu_id and m.key = $5))
             and ($7::text is null or cardinality(t.repos) = 0 or $7 = any(t.repos))
             and {scope} and {takeable}
           order by t.priority desc, t.created_at
           for update of t skip locked limit 1)
         update tasks t set status = 'claimed', claimed_by = $2, lease_epoch = t.lease_epoch + 1,
            lease_expires_at = now() + make_interval(secs => $3::float8), updated_at = now(), station_id = $6,
            held_since = now(), progress_at = now()
         from next where t.id = next.id
         returning t.id, t.lease_epoch, t.lease_expires_at, next.was_status, next.was_holder, next.was_expiry",
        scope = crate::repos::scope_sql("$2"),
        takeable = takeable_sql("$2"),
    )))
    .bind(pr.id)
    .bind(p.id)
    .bind(ttl)
    .bind(labels)
    .bind(menu)
    .bind(st)
    .bind(only_repo)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, epoch, exp, was_status, was_holder, was_expiry)) = row else {
        let none = held_back(&mut tx, p, pr.id).await?;
        tx.commit().await?;
        return Ok(none);
    };
    let lapse = lapsed_before(&mut tx, p, id, &was_status, was_holder, was_expiry).await?;
    emit(
        &mut tx,
        Some(pr.id),
        Some(id),
        "task.claimed",
        Some(p.id),
        json!({"agent": p.name, "epoch": epoch}),
    )
    .await?;
    let t = task_full(&mut tx, id).await?;
    let news = crate::pantry::news(&mut tx, p.id, pr.id).await?;
    let clock = stall_clock(&mut tx, id).await?;
    let memories = crate::pantry::relevant(&mut tx, pr.id, Some(id)).await?;
    tx.commit().await?;
    let mut v = with_news(lease_view(t, epoch, exp, clock), news);
    with_memories(&mut v, memories);
    with_lapse(&mut v, lapse);
    Ok(v)
}

/// The memories most relevant to the task, with the claim: read them before you start.
fn with_memories(v: &mut Value, memories: Vec<Value>) {
    if !memories.is_empty() {
        v["memories"] = Value::Array(memories);
    }
}

/// `claim_next` found nothing: say so, and name the open tasks it held back: those this cook
/// released a moment ago (free for any other cook) and those waiting on an open pull request.
/// `claim_task` takes either by name.
async fn held_back(c: &mut PgConnection, p: &Principal, project: Uuid) -> Result<Value> {
    let rows: Vec<(String, bool, bool, Option<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select pr.key || '-' || t.number, not {cooldown}, {bounce},
                case when not {NO_OPEN_PR_SQL} then (select fp.repo || '#' || fp.number from forge_prs fp
                  where fp.task_id = t.id and fp.state = 'open' and not fp.unlinked order by fp.updated_at desc limit 1) end
         from tasks t join projects pr on pr.id = t.project_id
         where t.project_id = $1 and t.status = 'open' and (not {cooldown} or {bounce} or not {NO_OPEN_PR_SQL})
         order by t.priority desc, t.created_at",
        cooldown = cooldown_sql("$2"),
        bounce = bounce_sql("$2", BOUNCE_LIMIT),
    )))
    .bind(project)
    .bind(p.id)
    .fetch_all(&mut *c)
    .await?;
    let released: Vec<&str> = rows.iter().filter(|r| r.1).map(|r| r.0.as_str()).collect();
    let bounced: Vec<&str> = rows.iter().filter(|r| r.2).map(|r| r.0.as_str()).collect();
    let on_pr: Vec<String> = rows
        .iter()
        .filter_map(|r| r.3.as_ref().map(|pr| format!("{} ({pr})", r.0)))
        .collect();
    if released.is_empty() && on_pr.is_empty() && bounced.is_empty() {
        return Ok(json!({"none": true}));
    }
    let mut hint = Vec::new();
    if !bounced.is_empty() {
        hint.push(format!(
            "{} {} you gave back {BOUNCE_LIMIT} times with nobody touching {} since: claim_next skips {} and claim_task refuses {}. \
             Ask a person (comment @owner, or block the task): your own comment does not lift this, one from someone else does, as does an edit or an answer.",
            bounced.join(", "),
            if bounced.len() == 1 { "is a task" } else { "are tasks" },
            if bounced.len() == 1 { "it" } else { "them" },
            if bounced.len() == 1 { "it" } else { "them" },
            if bounced.len() == 1 { "it" } else { "them" },
        ));
    }
    if !released.is_empty() {
        let mins = release_cooldown_secs().div_ceil(60);
        hint.push(format!(
            "{} {} you released in the last {mins} minutes: claim_next leaves {} to the other cooks for now.",
            released.join(", "),
            if released.len() == 1 { "is a task" } else { "are tasks" },
            if released.len() == 1 { "it" } else { "them" },
        ));
    }
    if !on_pr.is_empty() {
        hint.push(format!(
            "{} {} an open pull request: {} waiting on CI or review, not free work.",
            on_pr.join(", "),
            if on_pr.len() == 1 { "has" } else { "have" },
            if on_pr.len() == 1 {
                "it is"
            } else {
                "they are"
            },
        ));
    }
    if !released.is_empty() || !on_pr.is_empty() {
        hint.push("Take one by name with claim_task if you mean to.".into());
    }
    let mut v = json!({"none": true, "hint": hint.join(" ")});
    if !bounced.is_empty() {
        v["bounced"] = json!(bounced.join(","));
    }
    if !released.is_empty() {
        v["released_recently"] = json!(released.join(","));
    }
    if !on_pr.is_empty() {
        v["open_prs"] = json!(on_pr.join(","));
    }
    Ok(v)
}

/// A lease that ran out without a release: who held it, from which station, and when it lapsed.
struct Lapse {
    holder: Option<String>,
    station: Option<String>,
    at: DateTime<Utc>,
    mine: bool,
}

/// Was the task's last lease one that lapsed (expired, reaped or not) rather than ended by a
/// release, block or completion? Read before the new claim is recorded.
async fn lapsed_before(
    c: &mut PgConnection,
    p: &Principal,
    task: Uuid,
    was_status: &str,
    was_holder: Option<Uuid>,
    was_expiry: Option<DateTime<Utc>>,
) -> Result<Option<Lapse>> {
    let station: Option<String> = sqlx::query_scalar(
        "select s.ref from tasks t join stations s on s.id = t.station_id where t.id = $1",
    )
    .bind(task)
    .fetch_optional(&mut *c)
    .await?;
    // expired but not reaped yet: the row still names the holder
    if was_status == "claimed" {
        let holder: Option<String> =
            sqlx::query_scalar("select name from principals where id = $1")
                .bind(was_holder)
                .fetch_optional(&mut *c)
                .await?;
        return Ok(Some(Lapse {
            holder,
            station,
            at: was_expiry.unwrap_or_else(Utc::now),
            mine: was_holder == Some(p.id),
        }));
    }
    // reaped: the last lease event says how it ended
    let last: Option<(String, Value, DateTime<Utc>)> = sqlx::query_as(
        "select kind, payload, at from events where task_id = $1
           and kind in ('task.claimed','task.released','task.blocked','task.completed','task.lease_expired',
                        'task.cancelled','task.unblocked','task.restored')
         order by seq desc limit 1",
    )
    .bind(task)
    .fetch_optional(&mut *c)
    .await?;
    let Some((kind, payload, at)) = last else {
        return Ok(None);
    };
    if kind != "task.lease_expired" {
        return Ok(None);
    }
    let holder = payload["agent"].as_str().map(String::from);
    Ok(Some(Lapse {
        mine: holder.as_deref() == Some(p.name.as_str()),
        station: payload["station"].as_str().map(String::from).or(station),
        holder,
        at,
    }))
}

/// Tell the new holder that the last lease lapsed: that work may still sit, uncommitted, in the
/// old holder's branch or worktree.
fn with_lapse(v: &mut Value, lapse: Option<Lapse>) {
    let Some(l) = lapse else { return };
    let id = v["task"]["id"].as_str().unwrap_or("this task").to_string();
    let mins = (Utc::now() - l.at).num_minutes().max(0);
    let ago = if mins < 1 {
        "just now".to_string()
    } else {
        format!("{mins} min ago")
    };
    let at = l
        .station
        .as_deref()
        .map(|s| format!(" (station {s})"))
        .unwrap_or_default();
    let text = if l.mine {
        format!(
            "Your last lease on {id}{at} lapsed {ago} without a release. Look in your own branch \
             or worktree for that work before you start again."
        )
    } else {
        format!(
            "The last lease on {id}, held by {}{at}, lapsed {ago} without a release. Its work may \
             still sit uncommitted in that cook's branch or worktree, or in an open pull request: \
             look for it (or ask that cook on the pass) before you start again.",
            l.holder.as_deref().unwrap_or("a cook that is gone"),
        )
    };
    v["previous_lease"] = json!({
        "holder": l.holder, "station": l.station, "lapsed_at": l.at, "yours": l.mine,
    });
    v["warning"] = match v["warning"].as_str() {
        Some(w) => json!(format!("{w} {text}")),
        None => json!(text),
    };
}

/// The pantry news a cook gets with each claim: read it before starting.
fn with_news(mut v: Value, news: Value) -> Value {
    if news["count"].as_u64().unwrap_or(0) > 0 {
        v["pantry"] = news;
    }
    v
}

pub async fn claim_task(
    db: &PgPool,
    p: &Principal,
    r: &str,
    station: Option<&str>,
    ttl_s: Option<f64>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    if let Some(v) = catch_up_gate(&mut tx, p, t.project_id, &t.key).await? {
        tx.commit().await?;
        return Ok(v);
    }
    let st = crate::stations::for_claim(&mut tx, p, station, t.project_id).await?;
    let ttl = lease_ttl(ttl_s, t.ttl_secs);
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(p.id.to_string())
        .execute(&mut *tx)
        .await?;
    if live_leases(&mut tx, p.id).await? >= p.max_leases {
        return Err(Error::MaxLeases);
    }
    let (was_status, was_holder, was_expiry): (String, Option<Uuid>, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "select status, claimed_by, lease_expires_at from tasks where id = $1 for update",
        )
        .bind(t.id)
        .fetch_one(&mut *tx)
        .await?;
    // people may take a task cooks keep giving back
    let bounced: bool = p.is_agent()
        && sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select {} from tasks t where t.id = $1",
            bounce_sql("$2", BOUNCE_LIMIT)
        )))
        .bind(t.id)
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
    if bounced {
        return Err(Error::Conflict(format!(
            "you have given {r} back {BOUNCE_LIMIT} times and nobody has touched it since: a claim would only \
             bounce it again. Ask a person (comment @owner, or block it), or leave it to someone else; a \
             comment from someone else, an edit or an answer lifts this."
        )));
    }
    let row: Option<(i64, DateTime<Utc>)> = sqlx::query_as(
        "update tasks t set status = 'claimed', claimed_by = $2, lease_epoch = t.lease_epoch + 1,
            lease_expires_at = now() + make_interval(secs => $3::float8), updated_at = now(), station_id = $4,
            held_since = now(), progress_at = now()
         where t.id = $1
           and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
           and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                           where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
         returning t.lease_epoch, t.lease_expires_at",
    )
    .bind(t.id)
    .bind(p.id)
    .bind(ttl)
    .bind(st)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((epoch, exp)) = row else {
        let (status, deps_open): (String, bool) = sqlx::query_as(
            "select status, exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                    where d.task_id = tasks.id and dt.status not in ('done','cancelled')) or exists (select 1 from tasks sc where sc.parent_id = tasks.id and sc.status not in ('done','cancelled')) from tasks where id = $1",
        )
        .bind(t.id)
        .fetch_one(&mut *tx)
        .await?;
        return Err(Error::Conflict(if status == "open" && deps_open {
            format!("{r} is waiting on unfinished dependencies or subtasks")
        } else {
            format!("{r} is not claimable (status {status})")
        }));
    };
    let lapse = lapsed_before(&mut tx, p, t.id, &was_status, was_holder, was_expiry).await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.claimed",
        Some(p.id),
        json!({"agent": p.name, "epoch": epoch}),
    )
    .await?;
    let out = task_full(&mut tx, t.id).await?;
    let task_repos: Vec<String> = out["repos"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let mut warning = crate::repos::scope_warning(&mut tx, p.id, t.project_id, &task_repos).await?;
    let open_pr: Option<(String, i64, String, Option<String>, String)> = sqlx::query_as(
        "select repo, number, url, author, head_branch from forge_prs
         where task_id = $1 and state = 'open' and not unlinked order by updated_at desc limit 1",
    )
    .bind(t.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((repo, n, url, author, branch)) = open_pr {
        let pr = format!(
            "{r} has an open pull request, {repo}#{n}{by} on branch {branch} ({url}): continue on that \
             branch rather than starting over.",
            by = author.map(|a| format!(" by {a}")).unwrap_or_default(),
        );
        warning = Some(match warning {
            Some(w) => format!("{w} {pr}"),
            None => pr,
        });
    }
    let news = crate::pantry::news(&mut tx, p.id, t.project_id).await?;
    let clock = stall_clock(&mut tx, t.id).await?;
    let memories = crate::pantry::relevant(&mut tx, t.project_id, Some(t.id)).await?;
    tx.commit().await?;
    let mut v = with_news(lease_view(out, epoch, exp, clock), news);
    if let Some(w) = warning {
        v["warning"] = json!(w);
    }
    with_memories(&mut v, memories);
    with_lapse(&mut v, lapse);
    Ok(v)
}

/// Fenced lease extension. Returns the new expiry, or lease_lost. The expiry never passes the
/// stall limit (no progress for STALL_MINUTES) or the maximum hold (MAX_HOLD_HOURS); past either,
/// the lease is lost, and the error says which.
async fn touch_lease(
    c: &mut PgConnection,
    p: &Principal,
    t: &TaskRef,
    epoch: i64,
    ttl_s: Option<f64>,
) -> Result<DateTime<Utc>> {
    let ttl = ttl_s.unwrap_or(t.ttl_secs).clamp(1.0, MAX_TTL_SECS);
    let cap = lease_cap_sql("tasks");
    let sql = format!(
        "update tasks set lease_expires_at = least(now() + make_interval(secs => $4::float8), {cap}), updated_at = now()
         where id = $1 and claimed_by = $2 and lease_epoch = $3 and status = 'claimed' and lease_expires_at > now()
           and {cap} > now()
         returning lease_expires_at"
    );
    let exp: Option<DateTime<Utc>> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(t.id)
        .bind(p.id)
        .bind(epoch)
        .bind(ttl)
        .fetch_optional(&mut *c)
        .await?;
    match exp {
        Some(e) => Ok(e),
        None => Err(lost_reason(c, p, t, epoch).await?),
    }
}

/// Why a fenced lease call failed: the stall limit or the maximum hold passed (say so), else the
/// plain lost lease.
async fn lost_reason(
    c: &mut PgConnection,
    p: &Principal,
    t: &TaskRef,
    epoch: i64,
) -> Result<Error> {
    let sql = format!(
        "select coalesce(progress_at, updated_at) + make_interval(secs => {stall}) <= now(),
                coalesce(held_since, updated_at) + make_interval(secs => {max}) <= now(),
                coalesce(progress_at, updated_at), coalesce(held_since, updated_at)
         from tasks where id = $1 and claimed_by = $2 and lease_epoch = $3 and status = 'claimed'",
        stall = stall_secs(),
        max = max_hold_secs(),
    );
    let row: Option<(bool, bool, DateTime<Utc>, DateTime<Utc>)> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(t.id)
            .bind(p.id)
            .bind(epoch)
            .fetch_optional(c)
            .await?;
    Ok(match row {
        Some((_, true, _, held)) => Error::LeaseLostFor(format!(
            "you have held this task since {} (the maximum is {}): the server ends the lease; \
             stop, and report what is done",
            held.format("%Y-%m-%d %H:%M UTC"),
            hours(max_hold_secs())
        )),
        Some((true, _, progress, _)) => Error::LeaseLostFor(format!(
            "no progress since {} (the limit is {} min): the server ends the lease; stop, and \
             report what is done. Comment or tick a checklist item to show progress",
            progress.format("%Y-%m-%d %H:%M UTC"),
            stall_secs().div_ceil(60)
        )),
        _ => Error::LeaseLost,
    })
}

pub async fn heartbeat(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    ttl_s: Option<f64>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    let exp = match touch_lease(&mut tx, p, &t, epoch, ttl_s).await {
        Ok(e) => e,
        Err(e) => {
            // the reason, with the task, outside the failed transaction's rollback
            tx.commit().await?;
            return Err(e);
        }
    };
    let (stalls_at, warning) = stall_clock(&mut tx, t.id).await?;
    tx.commit().await?;
    let mut v = json!({"id": r, "epoch": epoch, "expires_at": exp, "stalls_at": stalls_at});
    if let Some(w) = warning {
        v["warning"] = json!(w);
    }
    Ok(v)
}

pub async fn check_item(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: Option<i64>,
    position: i64,
    done: bool,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    match epoch {
        Some(e) => {
            touch_lease(&mut tx, p, &t, e, None).await?;
        }
        None if p.can_plan() => {}
        None => {
            return invalid(
                "epoch is required: cooks tick items under a live lease (unless granted `plan`)",
            );
        }
    }
    let n = sqlx::query("update items set done = $3, done_by = case when $3 then $4 else null end where task_id = $1 and position = $2")
        .bind(t.id)
        .bind(position)
        .bind(done)
        .bind(p.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(Error::NotFound(format!("{r} has no item {position}")));
    }
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "item.checked",
        Some(p.id),
        json!({"position": position, "done": done}),
    )
    .await?;
    let out = task_full(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn comment(
    db: &PgPool,
    p: &Principal,
    r: &str,
    body: &str,
    epoch: Option<i64>,
    station: Option<&str>,
) -> Result<Value> {
    if body.trim().is_empty() {
        return invalid("comment body is empty");
    }
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    if let Some(e) = epoch {
        // A live lease is extended; a stale epoch still lets the note through.
        let _ = touch_lease(&mut tx, p, &t, e, None).await;
    }
    let comment_id = add_comment(&mut tx, &t, p.id, body).await?;
    // which of the cook's sessions wrote it, when it said
    if let Some(st) = crate::stations::for_claim(&mut tx, p, station, t.project_id).await? {
        sqlx::query("update comments set station_id = $2 where id = $1")
            .bind(comment_id)
            .bind(st)
            .execute(&mut *tx)
            .await?;
    }
    let seq = emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.commented",
        Some(p.id),
        json!({}),
    )
    .await?;
    let (woken, not_woken) = notify_comment(&mut tx, p, &t, body, comment_id, seq).await?;
    tx.commit().await?;
    let mut out = json!({"ok": true, "woken": woken});
    if let Some(n) = crate::pass::not_woken_note(&not_woken) {
        out["note"] = json!(n);
    }
    Ok(out)
}

/// A comment reaches the cook responsible for the task (the one holding it, parked on it waiting
/// for an answer, or the one that last held it) and the cooks it @mentions, on the session that
/// holds the task when there is one. Never the commenter itself. Returns who was woken and the
/// fire-and-forget cooks that were not (they only take ready work).
async fn notify_comment(
    c: &mut PgConnection,
    p: &Principal,
    t: &TaskRef,
    body: &str,
    comment_id: Uuid,
    seq: i64,
) -> Result<(Vec<String>, Vec<String>)> {
    if t.status == "cancelled" {
        return Ok((vec![], vec![]));
    }
    let (title, team, responsible): (String, Uuid, Option<Uuid>) = sqlx::query_as(
        "select t.title, pr.team_id,
                coalesce(t.reserved_for, t.claimed_by,
                         (select e.actor_id from events e where e.task_id = t.id and e.kind = 'task.claimed'
                          order by e.seq desc limit 1))
         from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
    )
    .bind(t.id)
    .fetch_one(&mut *c)
    .await?;
    let tref = format!("{}-{}", t.key, t.number);
    let quote = crate::questions::short(&title);
    let mut targets: Vec<(Uuid, String)> = vec![];
    if let Some(r) = responsible {
        let why = if t.status == "claimed" && t.claimed_by == Some(r) {
            "you hold it"
        } else if t.status == "blocked" {
            "it is parked with you"
        } else {
            "you worked on it"
        };
        targets.push((
            r,
            format!(
                "{} commented on {tref} \"{quote}\" ({why}):\n\n{body}\n\nAnswer with the `comment` tool on {tref} if it needs a reply.",
                p.name
            ),
        ));
    }
    for name in crate::pass::mentions(body) {
        if let Some(id) = crate::pass::chef(&mut *c, team, &name).await? {
            targets.push((
                id,
                format!(
                    "{} mentioned you in a comment on {tref} \"{quote}\":\n\n{body}\n\nAnswer with the `comment` tool on {tref} if it needs a reply.",
                    p.name
                ),
            ));
        }
    }
    let mut woken: Vec<String> = vec![];
    let mut not_woken: Vec<String> = vec![];
    let mut seen = std::collections::BTreeSet::new();
    for (agent, prompt) in targets {
        if agent == p.id || !seen.insert(agent) {
            continue;
        }
        let row: Option<String> = sqlx::query_scalar(
            "select name from principals where id = $1 and kind = 'agent' and disabled_at is null",
        )
        .bind(agent)
        .fetch_optional(&mut *c)
        .await?;
        let Some(name) = row else { continue };
        let w = crate::questions::wake(
            &mut *c,
            t.project_id,
            &t.key,
            agent,
            None,
            Some(t.id),
            &prompt,
            "comment",
        )
        .await?;
        if w["woken"] == false {
            not_woken.push(name);
            continue;
        }
        // A person's comment waits for an answer; unanswered, the cook's main station hears again.
        if p.is_user() {
            sqlx::query(
                "insert into comment_replies (comment_id, agent_id, task_id, project_id, author_id, since_seq)
                 values ($1,$2,$3,$4,$5,$6) on conflict do nothing",
            )
            .bind(comment_id)
            .bind(agent)
            .bind(t.id)
            .bind(t.project_id)
            .bind(p.id)
            .bind(seq)
            .execute(&mut *c)
            .await?;
        }
        woken.push(name);
    }
    Ok((woken, not_woken))
}

async fn add_comment(c: &mut PgConnection, t: &TaskRef, author: Uuid, body: &str) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("insert into comments (id, task_id, author_id, body) values ($1,$2,$3,$4)")
        .bind(id)
        .bind(t.id)
        .bind(author)
        .bind(body)
        .execute(c)
        .await?;
    Ok(id)
}

/// What a cook says it learned, as the memory to save: `None` for nothing, or "nothing new".
fn learned_text(learned: Option<&str>) -> Option<&str> {
    let l = learned.map(str::trim).filter(|l| !l.is_empty())?;
    let said = l.trim_end_matches(['.', '!']).trim();
    (!said.eq_ignore_ascii_case("nothing new")).then_some(l)
}

/// Save what the holder learned on a task into the memory `learned-<REF>` (appended to on a later
/// revision): kind note, tagged with the task's labels and repositories, linked to the task, so the
/// next claim on similar work finds it.
async fn save_learned(
    c: &mut PgConnection,
    p: &Principal,
    t: &TaskRef,
    learned: Option<&str>,
) -> Result<()> {
    let Some(text) = learned_text(learned) else {
        return Ok(());
    };
    let (title, labels, repos): (String, Vec<String>, Vec<String>) =
        sqlx::query_as("select title, labels, repos from tasks where id = $1")
            .bind(t.id)
            .fetch_one(&mut *c)
            .await?;
    let tref = format!("{}-{}", t.key, t.number);
    let mut tags = labels;
    tags.extend(repos);
    let title: String = format!("{tref}: {title}").chars().take(200).collect();
    // a later revision appends to `learned-<REF>`; when that memory is full, to `-2`, `-3`, ...
    // (completing must never fail on what was learned)
    let text: String = text.chars().take(crate::pantry::MAX_BODY).collect();
    let mut key = format!("learned-{tref}").to_lowercase();
    for n in 2.. {
        let used: Option<i64> = sqlx::query_scalar(
            "select char_length(body)::bigint from memories where project_id = $1 and key = $2",
        )
        .bind(t.project_id)
        .bind(&key)
        .fetch_optional(&mut *c)
        .await?;
        match used {
            Some(u) if u as usize + 2 + text.chars().count() > crate::pantry::MAX_BODY => {
                key = format!("learned-{tref}-{n}").to_lowercase();
            }
            _ => break,
        }
    }
    crate::pantry::remember_in(
        c,
        p,
        &crate::pantry::Remember {
            project: t.key.clone(),
            key: Some(key),
            title,
            body: text,
            // None: a new memory is a note, and one that was promoted keeps its kind
            kind: None,
            tags: Some(tags),
            task: Some(tref),
            append: true,
            revision: None,
            notify: false,
            station: None,
        },
    )
    .await?;
    Ok(())
}

pub async fn complete(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    result: &str,
    no_propagate: bool,
    learned: &str,
) -> Result<Value> {
    if learned.trim().is_empty() {
        return invalid(
            "learned is required: say what the next agent on similar work must know (a gotcha, a \
             convention, where the code really is), or \"nothing new\"",
        );
    }
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    let n = sqlx::query(
        "update tasks set status = 'done', claimed_by = null, lease_expires_at = null, result = $4, updated_at = now()
         where id = $1 and claimed_by = $2 and lease_epoch = $3 and status = 'claimed' and lease_expires_at > now()
           and not exists (select 1 from tasks c where c.parent_id = tasks.id and c.status not in ('done','cancelled'))",
    )
    .bind(t.id)
    .bind(p.id)
    .bind(epoch)
    .bind(result)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        // Distinguish a lost lease from open subtasks.
        let live: bool = sqlx::query_scalar(
            "select status = 'claimed' and claimed_by = $2 and lease_epoch = $3 and lease_expires_at > now() from tasks where id = $1",
        )
        .bind(t.id)
        .bind(p.id)
        .bind(epoch)
        .fetch_one(&mut *tx)
        .await?;
        return Err(if live {
            Error::Conflict(format!(
                "{r} has subtasks that are not done or cancelled yet"
            ))
        } else {
            Error::LeaseLost
        });
    }
    save_learned(&mut tx, p, &t, Some(learned)).await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.completed",
        Some(p.id),
        json!({"result": result}),
    )
    .await?;
    if !no_propagate {
        crate::repos::propagate(&mut tx, Some(p.id), t.id, result).await?;
    }
    let out = task_json(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}

async fn end_lease(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    new_status: &str,
    kind: &str,
    reason: &str,
    learned: Option<&str>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    let n = sqlx::query(
        "update tasks set status = $4, claimed_by = null, lease_expires_at = null, updated_at = now()
         where id = $1 and claimed_by = $2 and lease_epoch = $3 and status = 'claimed' and lease_expires_at > now()",
    )
    .bind(t.id)
    .bind(p.id)
    .bind(epoch)
    .bind(new_status)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(Error::LeaseLost);
    }
    if !reason.trim().is_empty() {
        add_comment(&mut tx, &t, p.id, &format!("[{kind}] {reason}")).await?;
    }
    save_learned(&mut tx, p, &t, learned).await?;
    // given back while a healthy pull request is open: it waits on CI or review, which is not
    // bouncing. A failing, conflicting or changes-requested PR is work: giving that back counts.
    let on_pr: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select not ({NO_OPEN_PR_SQL}) from tasks t where t.id = $1"
    )))
    .bind(t.id)
    .fetch_one(&mut *tx)
    .await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        kind,
        Some(p.id),
        json!({"reason": reason, "on_pr": on_pr}),
    )
    .await?;
    // its PR merged while it was held: nobody is left to serve it
    if new_status == "open" {
        crate::forge::events::complete_if_merged(&mut tx, t.id, None).await?;
        block_if_bounced(&mut tx, t.id, t.project_id).await?;
    }
    let out = task_json(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn release(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    reason: &str,
    learned: Option<&str>,
) -> Result<Value> {
    end_lease(db, p, r, epoch, "open", "task.released", reason, learned).await
}

pub async fn block(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    reason: &str,
    learned: Option<&str>,
) -> Result<Value> {
    end_lease(db, p, r, epoch, "blocked", "task.blocked", reason, learned).await
}

/// A task given back this many times by any cooks, with no person touching it in between, is not
/// work a cook can do: it is blocked for a person, with the reason. Called after a give-back.
pub async fn block_if_bounced(c: &mut PgConnection, task: Uuid, project: Uuid) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "select count(*) from (select 1 from events r
         where r.task_id = $1 and r.kind in ('task.released', 'task.lease_expired')
           and coalesce(r.payload->>'on_pr', 'false') <> 'true'
           and r.seq > coalesce((select max(x.seq) from events x join principals h on h.id = x.actor_id and h.kind = 'user'
                                   where x.task_id = $1 and x.kind in ('task.updated', 'task.commented', 'task.unblocked',
                                         'task.restored', 'question.answered')), 0)
         limit $2) capped",
    )
    .bind(task)
    .bind(BOUNCE_BLOCK)
    .fetch_one(&mut *c)
    .await?;
    if n < BOUNCE_BLOCK {
        return Ok(false);
    }
    let reason = format!("bounced {BOUNCE_BLOCK}×: needs a person");
    let moved = sqlx::query(
        "update tasks set status = 'blocked', claimed_by = null, lease_expires_at = null, reserved_for = null
         where id = $1 and status = 'open'",
    )
    .bind(task)
    .execute(&mut *c)
    .await?
    .rows_affected();
    if moved == 0 {
        return Ok(false);
    }
    // the owner is who must look: named in the note, shown under "owned" in my_work, and a cook
    // that owns it is woken
    let owner: Option<(Uuid, String, String)> = sqlx::query_as(
        "select o.id, o.name, o.kind from tasks t join principals o on o.id = t.owner_id where t.id = $1",
    )
    .bind(task)
    .fetch_optional(&mut *c)
    .await?;
    let who = match &owner {
        Some((_, name, _)) => format!(" @{name}, this is yours."),
        None => " The task has no owner: a chef of the pot should take it.".into(),
    };
    system_comment(
        c,
        task,
        &format!(
            "[blocked by the server] {reason}: cooks gave this task back {n} times and nobody \
             touched it in between.{who} Look at why (the comments above), then unblock it with a clearer task."
        ),
    )
    .await?;
    if let Some((id, _, kind)) = owner
        && kind == "agent"
    {
        let (tref, key): (String, String) = sqlx::query_as(
            "select pr.key || '-' || t.number, pr.key from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
        )
        .bind(task)
        .fetch_one(&mut *c)
        .await?;
        crate::questions::wake(
            c,
            project,
            &key,
            id,
            None,
            Some(task),
            &format!(
                "{tref} was given back {n} times and is blocked for a person; you own it. Read its comments and unblock it with a clearer task, or say why it cannot be done."
            ),
            "needs-person",
        )
        .await?;
    }
    emit(
        c,
        Some(project),
        Some(task),
        "task.blocked",
        None,
        json!({"reason": reason, "bounced": true}),
    )
    .await?;
    Ok(true)
}

/// A note on a task written by the server itself (no author).
pub async fn system_comment(c: &mut PgConnection, task: Uuid, body: &str) -> Result<()> {
    sqlx::query("insert into comments (task_id, author_id, body) values ($1, null, $2)")
        .bind(task)
        .bind(body)
        .execute(c)
        .await?;
    Ok(())
}

/// Human-only transitions: unblock (blocked -> open) and cancel (open/blocked -> cancelled).
/// People-only moves: `unblock` (blocked -> open), `cancel` = toss (open, blocked or claimed ->
/// cancelled, with an optional reason; a held task's lease ends and its epoch moves on, so the
/// cook's next lease call fails with lease_lost) and `restore` (cancelled -> open).
pub async fn human_transition(
    db: &PgPool,
    p: &Principal,
    r: &str,
    action: &str,
    reason: Option<&str>,
) -> Result<Value> {
    if !p.can_plan() {
        return Err(Error::Forbidden(format!(
            "{} may not {} tasks: a person does it, or its owner grants it `plan` (corntchap-cli cook grant {} plan)",
            p.name,
            if action == "cancel" { "toss" } else { action },
            p.name
        )));
    }
    let (from, to, kind, verb): (&[&str], &str, &str, &str) = match action {
        "unblock" => (&["blocked"], "open", "task.unblocked", "unblocked"),
        "cancel" => (
            &["open", "blocked", "claimed"],
            "cancelled",
            "task.cancelled",
            "tossed",
        ),
        "restore" => (&["cancelled"], "open", "task.restored", "restored"),
        _ => return invalid("unknown action"),
    };
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    let from: Vec<String> = from.iter().map(|s| s.to_string()).collect();
    let n = sqlx::query(
        "update tasks set status = $3, updated_at = now(),
            result = case when $3 = 'cancelled' then $4 when $3 = 'open' and status = 'cancelled' then null else result end,
            lease_epoch = case when status = 'claimed' then lease_epoch + 1 else lease_epoch end,
            claimed_by = case when $3 = 'cancelled' or status = 'blocked' then null else claimed_by end,
            reserved_for = null,
            lease_expires_at = case when $3 = 'cancelled' then null else lease_expires_at end
         where id = $1 and status = any($2)",
    )
    .bind(t.id)
    .bind(&from)
    .bind(to)
    .bind(reason)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(Error::Conflict(format!(
            "{r} is {} and cannot be {verb}",
            t.status
        )));
    }
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        kind,
        Some(p.id),
        match reason {
            Some(why) => json!({"reason": why}),
            None => json!({}),
        },
    )
    .await?;
    let out = task_json(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn whoami(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Value> {
    let teams: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', t.name, 'role', tm.role) from team_members tm join teams t on t.id = tm.team_id
         where tm.principal_id = $1 order by t.name",
    )
    .bind(p.id)
    .fetch_all(db)
    .await?;
    let projects = list_projects(db, p).await?;
    let display_name: Option<String> =
        sqlx::query_scalar("select display_name from principals where id = $1")
            .bind(p.id)
            .fetch_optional(db)
            .await?
            .flatten();
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id
         where t.claimed_by = $1 and t.status = 'claimed' and t.lease_expires_at > now()"
    );
    let leases: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.id)
        .fetch_all(db)
        .await?;
    let guide = match project.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => {
            let mut c = db.acquire().await?;
            let pr = project_by_key(&mut c, p, &k.to_uppercase()).await?;
            crate::what_now::guide_excerpt(&mut c, pr.id).await?
        }
        None => None,
    };
    Ok(json!({
        "guide": guide,
        "name": p.name, "display_name": display_name, "kind": p.kind, "agent_kind": p.agent_kind, "runtime": p.runtime,
        "is_admin": p.is_admin, "max_leases": p.max_leases, "grants": p.grants,
        "teams": teams,
        "projects": projects.as_array().map(|a| a.iter().map(|x| x["key"].clone()).collect::<Vec<_>>()).unwrap_or_default(),
        "current_leases": leases,
    }))
}

/// The feed of events since a cursor, for a project when one is named. For an agent that names a
/// project, reading it counts as being briefed on that pot (see [`catch_up_gate`]).
pub async fn changes_since(
    db: &PgPool,
    p: &Principal,
    cursor: i64,
    project: Option<&str>,
    limit: i64,
) -> Result<Value> {
    let out = changes_feed(db, p, cursor, project, limit).await?;
    if project.is_some() && p.is_agent() {
        let mut c = db.acquire().await?;
        let pr = project_by_key(&mut c, p, project.unwrap_or_default()).await?;
        crate::what_now::brief(&mut c, p, &[pr.id]).await?;
    }
    Ok(out)
}

/// [`changes_since`] without the briefing: the live feed polls it all day.
pub async fn changes_feed(
    db: &PgPool,
    p: &Principal,
    cursor: i64,
    project: Option<&str>,
    limit: i64,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pid = match project {
        Some(k) => Some(project_by_key(&mut c, p, k).await?.id),
        None => None,
    };
    let sql = format!(
        "select e.seq, jsonb_build_object('seq', e.seq, 'kind', e.kind, 'project', pr.key,
            'task', (select pp.key || '-' || tt.number from tasks tt join projects pp on pp.id = tt.project_id where tt.id = e.task_id),
            'actor', coalesce((select name from principals where id = e.actor_id), e.payload->>'actor_name'), 'at', e.at, 'payload', e.payload)
         from events e join projects pr on pr.id = e.project_id
         where e.seq > $1 and ($2::uuid is null or e.project_id = $2) and e.kind not like 'wake%' and e.kind <> 'activity'
           and (e.audience is null or $5 = any(e.audience)) and {}
         order by e.seq limit $3",
        access_sql("$4", "$5")
    );
    let rows: Vec<(i64, Value)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(cursor)
        .bind(pid)
        .bind(limit.clamp(1, 500))
        .bind(p.is_admin)
        .bind(p.id)
        .fetch_all(&mut *c)
        .await?;
    let next = rows.last().map(|r| r.0).unwrap_or(cursor);
    Ok(json!({"events": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(), "next_cursor": next}))
}

/// Ready tasks this principal could take right now (empty when it already holds its maximum leases).
pub async fn ready_for(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Vec<Value>> {
    let mut c = db.acquire().await?;
    let pid = match project {
        Some(k) => Some(project_by_key(&mut c, p, k).await?.id),
        None => None,
    };
    if live_leases(&mut c, p.id).await? >= p.max_leases {
        return Ok(vec![]);
    }
    let sql = format!(
        "select jsonb_build_object('id', pr.key || '-' || t.number, 'project', pr.key, 'title', t.title,
            'priority', t.priority, 'labels', to_jsonb(t.labels), 'repos', to_jsonb(t.repos))
         from tasks t join projects pr on pr.id = t.project_id
         where ($1::uuid is null or pr.id = $1) and {} and {} and {}
           and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
           and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
         order by t.priority desc, t.created_at limit 20",
        access_sql("$2", "$3"),
        crate::repos::scope_sql("$3"),
        takeable_sql("$3"),
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pid)
        .bind(p.is_admin)
        .bind(p.id)
        .fetch_all(&mut *c)
        .await?)
}

/// Markdown board for a project, with checkboxes per task.
pub async fn board_markdown(db: &PgPool, p: &Principal, key: &str) -> Result<String> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, key).await?;
    if p.is_agent() {
        crate::what_now::brief(&mut c, p, &[pr.id]).await?;
    }
    let tasks = list_tasks_conn(&mut c, pr.id).await?;
    let mut out = format!("# {} — {}\n", pr.key, pr.name);
    for (heading, statuses) in [
        ("In progress", &["claimed"][..]),
        ("Ready / open", &["open"][..]),
        ("Blocked", &["blocked"][..]),
        ("Done", &["done"][..]),
        ("Cancelled", &["cancelled"][..]),
    ] {
        let group: Vec<&Value> = tasks
            .iter()
            .filter(|t| statuses.contains(&t["status"].as_str().unwrap_or("")))
            .collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {heading}\n\n"));
        for t in group {
            out.push_str(&task_markdown(t));
        }
    }
    Ok(out)
}

async fn list_tasks_conn(c: &mut PgConnection, project_id: Uuid) -> Result<Vec<Value>> {
    let sql = format!(
        "select {TASK_JSON} || jsonb_build_object('items', coalesce((select jsonb_agg(jsonb_build_object('position', i.position,
            'text', i.text, 'done', i.done) order by i.position) from items i where i.task_id = t.id), '[]'::jsonb))
         from tasks t join projects pr on pr.id = t.project_id where t.project_id = $1 order by t.priority desc, t.number"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(project_id)
        .fetch_all(c)
        .await?)
}

fn task_markdown(t: &Value) -> String {
    let status = t["status"].as_str().unwrap_or("");
    let mark = if status == "done" { "x" } else { " " };
    let mut s = format!(
        "- [{mark}] **{}** {}",
        t["id"].as_str().unwrap_or(""),
        t["title"].as_str().unwrap_or("")
    );
    let mut meta = vec![];
    if let Some(by) = t["claimed_by"].as_str() {
        meta.push(format!("claimed by {by}"));
    }
    if let Some(m) = t["menu"].as_str() {
        meta.push(format!("menu {m}"));
    }
    if t["priority"].as_i64().unwrap_or(0) != 0 {
        meta.push(format!("priority {}", t["priority"]));
    }
    if let Some(l) = t["labels"].as_array().filter(|l| !l.is_empty()) {
        meta.push(format!(
            "labels {}",
            l.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some(r) = t["repos"].as_array().filter(|l| !l.is_empty()) {
        meta.push(format!(
            "repos {}",
            r.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some(d) = t["depends_on"].as_array().filter(|l| !l.is_empty()) {
        meta.push(format!(
            "after {}",
            d.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !meta.is_empty() {
        s.push_str(&format!(" _({})_", meta.join("; ")));
    }
    s.push('\n');
    if let Some(items) = t["items"].as_array() {
        for i in items {
            let m = if i["done"].as_bool().unwrap_or(false) {
                "x"
            } else {
                " "
            };
            s.push_str(&format!("  - [{m}] {}\n", i["text"].as_str().unwrap_or("")));
        }
    }
    s
}

pub async fn task_markdown_for(db: &PgPool, p: &Principal, r: &str) -> Result<String> {
    let t = get_task(db, p, r).await?;
    let mut s = format!(
        "# {} {}\n\nstatus: {}\n\n{}\n\n",
        t["id"].as_str().unwrap_or(""),
        t["title"].as_str().unwrap_or(""),
        t["status"].as_str().unwrap_or(""),
        t["body"].as_str().unwrap_or("")
    );
    if let Some(items) = t["items"].as_array() {
        for i in items {
            let m = if i["done"].as_bool().unwrap_or(false) {
                "x"
            } else {
                " "
            };
            s.push_str(&format!("- [{m}] {}\n", i["text"].as_str().unwrap_or("")));
        }
    }
    if let Some(cm) = t["comments"].as_array().filter(|c| !c.is_empty()) {
        s.push_str("\n## Comments\n\n");
        for c in cm {
            s.push_str(&format!(
                "- **{}**: {}\n",
                c["author"].as_str().unwrap_or("?"),
                c["body"].as_str().unwrap_or("")
            ));
        }
    }
    Ok(s)
}

/// What changed in a project since an event cursor: counts and touched tasks.
pub async fn digest(db: &PgPool, p: &Principal, key: &str, since: i64) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, key).await?;
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "select kind, count(*) from events where project_id = $1 and seq > $2 and kind not like 'wake%' and kind <> 'activity' and audience is null group by kind order by kind",
    )
    .bind(pr.id)
    .bind(since)
    .fetch_all(&mut *c)
    .await?;
    let cursor: i64 =
        sqlx::query_scalar("select coalesce(max(seq), $2) from events where project_id = $1")
            .bind(pr.id)
            .bind(since)
            .fetch_one(&mut *c)
            .await?;
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id
         where t.id in (select task_id from events where project_id = $1 and seq > $2 and task_id is not null)
         order by t.number"
    );
    let tasks: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(since)
        .fetch_all(&mut *c)
        .await?;
    let counts: serde_json::Map<String, Value> =
        counts.into_iter().map(|(k, n)| (k, json!(n))).collect();
    Ok(json!({"project": key, "since": since, "cursor": cursor, "counts": counts, "tasks": tasks}))
}

/// Project title and every task (with items) for the read-only board page.
pub async fn board_data(db: &PgPool, p: &Principal, key: &str) -> Result<(String, Vec<Value>)> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, key).await?;
    let tasks = list_tasks_conn(&mut c, pr.id).await?;
    Ok((format!("{} — {}", pr.key, pr.name), tasks))
}
