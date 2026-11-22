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

/// SQL expression producing the JSON shape of a task. Needs aliases `t` (tasks) and `pr` (projects).
pub const TASK_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-' || t.number, 'project', pr.key, 'number', t.number,
  'title', t.title, 'body', t.body, 'priority', t.priority, 'labels', to_jsonb(t.labels),
  'status', t.status,
  'parent', (select pp.key || '-' || pt.number from tasks pt join projects pp on pp.id = pt.project_id where pt.id = t.parent_id),
  'depends_on', coalesce((select jsonb_agg(dp.key || '-' || dt.number order by dp.key, dt.number)
       from task_deps d join tasks dt on dt.id = d.depends_on join projects dp on dp.id = dt.project_id
       where d.task_id = t.id), '[]'::jsonb),
  'claimed_by', (select name from principals where id = t.claimed_by),
  'station', (select ref from stations where id = t.station_id),
  'lease_epoch', t.lease_epoch, 'lease_expires_at', t.lease_expires_at,
  'lease_live', (t.status = 'claimed' and t.lease_expires_at > now()),
  'ready', (t.status = 'open' and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
       where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))),
  'open_items', (select count(*) from items i where i.task_id = t.id and not i.done),
  'total_items', (select count(*) from items i where i.task_id = t.id),
  'menu', (select m.key from menus m where m.id = t.menu_id),
  'result', t.result, 'created_at', t.created_at, 'updated_at', t.updated_at)"#;

/// Project visibility predicate; $N = is_admin, $M = principal id.
fn access_sql(admin: &str, pid: &str) -> String {
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
    let seq: i64 = sqlx::query_scalar(
        "insert into events (project_id, task_id, kind, actor_id, payload) values ($1,$2,$3,$4,$5) returning seq",
    )
    .bind(project)
    .bind(task)
    .bind(kind)
    .bind(actor)
    .bind(payload)
    .fetch_one(c)
    .await?;
    Ok(seq)
}

async fn task_json(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id where t.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// Task with its items, comments and dependency states.
async fn task_full(c: &mut PgConnection, id: Uuid) -> Result<Value> {
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
            'body', cm.body, 'at', cm.created_at) order by cm.created_at), '[]'::jsonb)
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
            'team', (select name from teams where id = pr.team_id),
            'lease_ttl_s', extract(epoch from pr.lease_ttl)::float8,
            'open', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'open'),
            'ready', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'open'
                and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))),
            'claimed', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'claimed'),
            'blocked', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'blocked'),
            'done', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'done'),
            'tossed', (select count(*) from tasks t where t.project_id = pr.id and t.status = 'cancelled'),
            'open_questions', (select count(*) from questions q where q.project_id = pr.id and q.status = 'open'))
         from projects pr where {} order by pr.key",
        access_sql("$1", "$2")
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
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, project).await?;
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id
         where t.project_id = $1
           and ($2::text is null or t.status = $2)
           and ($3::text is null or $3 = any(t.labels))
           and ($5::text is null or ($5 = '-' and t.menu_id is null)
                or exists (select 1 from menus m where m.id = t.menu_id and m.key = $5))
           and (not $4 or (t.status = 'open' and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                 where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))))
         order by t.priority desc, t.number"
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
    pub priority: i64,
    /// Menu key (milestone) to put the task on.
    pub menu: Option<String>,
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
    let number: i64 = sqlx::query_scalar(
        "update projects set next_number = next_number + 1 where id = $1 returning next_number - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *tx)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into tasks (id, project_id, number, title, body, priority, labels, parent_id, status, created_by, menu_id)
         values ($1,$2,$3,$4,$5,$6,$7,$8,'open',$9,$10)",
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
        json!({"title": n.title.trim(), "labels": n.labels, "menu": n.menu.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(crate::menus::slug)}),
    )
    .await?;
    let out = task_full(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

pub struct TaskEdit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    pub priority: Option<i64>,
    /// Some("") takes the task off its menu.
    pub menu: Option<String>,
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
    sqlx::query(
        "update tasks set title = coalesce($2, title), body = coalesce($3, body), labels = coalesce($4, labels),
            priority = coalesce($5, priority), updated_at = now() where id = $1",
    )
    .bind(t.id)
    .bind(e.title.as_deref())
    .bind(e.body.as_deref())
    .bind(e.labels.as_deref())
    .bind(e.priority)
    .execute(&mut *tx)
    .await?;
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

fn lease_view(t: Value, epoch: i64, expires: DateTime<Utc>) -> Value {
    json!({ "task": t, "lease": { "epoch": epoch, "expires_at": expires } })
}

pub async fn claim_next(
    db: &PgPool,
    p: &Principal,
    project: &str,
    labels: &[String],
    menu: Option<&str>,
    station: Option<&str>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, project).await?;
    let st = crate::stations::for_claim(&mut tx, p, station, pr.id).await?;
    let menu = menu.map(crate::menus::slug).filter(|m| !m.is_empty());
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(p.id.to_string())
        .execute(&mut *tx)
        .await?;
    if live_leases(&mut tx, p.id).await? >= p.max_leases {
        return Err(Error::MaxLeases);
    }
    let row: Option<(Uuid, i64, DateTime<Utc>)> = sqlx::query_as(
        "with next as (
           select t.id from tasks t
           where t.project_id = $1
             and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
             and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                             where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
             and (cardinality($4::text[]) = 0 or t.labels && $4::text[])
             and ($5::text is null or exists (select 1 from menus m where m.id = t.menu_id and m.key = $5))
           order by t.priority desc, t.created_at
           for update skip locked limit 1)
         update tasks t set status = 'claimed', claimed_by = $2, lease_epoch = t.lease_epoch + 1,
            lease_expires_at = now() + make_interval(secs => $3::float8), updated_at = now(), station_id = $6
         from next where t.id = next.id
         returning t.id, t.lease_epoch, t.lease_expires_at",
    )
    .bind(pr.id)
    .bind(p.id)
    .bind(pr.ttl_secs)
    .bind(labels)
    .bind(menu)
    .bind(st)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, epoch, exp)) = row else {
        return Ok(json!({"none": true}));
    };
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
    tx.commit().await?;
    Ok(lease_view(t, epoch, exp))
}

pub async fn claim_task(
    db: &PgPool,
    p: &Principal,
    r: &str,
    station: Option<&str>,
) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, r).await?;
    let st = crate::stations::for_claim(&mut tx, p, station, t.project_id).await?;
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(p.id.to_string())
        .execute(&mut *tx)
        .await?;
    if live_leases(&mut tx, p.id).await? >= p.max_leases {
        return Err(Error::MaxLeases);
    }
    let row: Option<(i64, DateTime<Utc>)> = sqlx::query_as(
        "update tasks t set status = 'claimed', claimed_by = $2, lease_epoch = t.lease_epoch + 1,
            lease_expires_at = now() + make_interval(secs => $3::float8), updated_at = now(), station_id = $4
         where t.id = $1
           and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
           and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
                           where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
         returning t.lease_epoch, t.lease_expires_at",
    )
    .bind(t.id)
    .bind(p.id)
    .bind(t.ttl_secs)
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
    tx.commit().await?;
    Ok(lease_view(out, epoch, exp))
}

/// Fenced lease extension. Returns the new expiry, or lease_lost.
async fn touch_lease(
    c: &mut PgConnection,
    p: &Principal,
    t: &TaskRef,
    epoch: i64,
    ttl_s: Option<f64>,
) -> Result<DateTime<Utc>> {
    let ttl = ttl_s.unwrap_or(t.ttl_secs).clamp(1.0, MAX_TTL_SECS);
    sqlx::query_scalar(
        "update tasks set lease_expires_at = now() + make_interval(secs => $4::float8), updated_at = now()
         where id = $1 and claimed_by = $2 and lease_epoch = $3 and status = 'claimed' and lease_expires_at > now()
         returning lease_expires_at",
    )
    .bind(t.id)
    .bind(p.id)
    .bind(epoch)
    .bind(ttl)
    .fetch_optional(c)
    .await?
    .ok_or(Error::LeaseLost)
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
    let exp = touch_lease(&mut tx, p, &t, epoch, ttl_s).await?;
    tx.commit().await?;
    Ok(json!({"id": r, "epoch": epoch, "expires_at": exp}))
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
    add_comment(&mut tx, &t, p.id, body).await?;
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.commented",
        Some(p.id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"ok": true}))
}

async fn add_comment(c: &mut PgConnection, t: &TaskRef, author: Uuid, body: &str) -> Result<()> {
    sqlx::query("insert into comments (id, task_id, author_id, body) values ($1,$2,$3,$4)")
        .bind(Uuid::new_v4())
        .bind(t.id)
        .bind(author)
        .bind(body)
        .execute(c)
        .await?;
    Ok(())
}

pub async fn complete(
    db: &PgPool,
    p: &Principal,
    r: &str,
    epoch: i64,
    result: &str,
) -> Result<Value> {
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
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.completed",
        Some(p.id),
        json!({"result": result}),
    )
    .await?;
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
    emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        kind,
        Some(p.id),
        json!({"reason": reason}),
    )
    .await?;
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
) -> Result<Value> {
    end_lease(db, p, r, epoch, "open", "task.released", reason).await
}

pub async fn block(db: &PgPool, p: &Principal, r: &str, epoch: i64, reason: &str) -> Result<Value> {
    end_lease(db, p, r, epoch, "blocked", "task.blocked", reason).await
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

pub async fn whoami(db: &PgPool, p: &Principal) -> Result<Value> {
    let teams: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', t.name, 'role', tm.role) from team_members tm join teams t on t.id = tm.team_id
         where tm.principal_id = $1 order by t.name",
    )
    .bind(p.id)
    .fetch_all(db)
    .await?;
    let projects = list_projects(db, p).await?;
    let sql = format!(
        "select {TASK_JSON} from tasks t join projects pr on pr.id = t.project_id
         where t.claimed_by = $1 and t.status = 'claimed' and t.lease_expires_at > now()"
    );
    let leases: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.id)
        .fetch_all(db)
        .await?;
    Ok(json!({
        "name": p.name, "kind": p.kind, "agent_kind": p.agent_kind, "runtime": p.runtime,
        "is_admin": p.is_admin, "max_leases": p.max_leases, "grants": p.grants,
        "teams": teams,
        "projects": projects.as_array().map(|a| a.iter().map(|x| x["key"].clone()).collect::<Vec<_>>()).unwrap_or_default(),
        "current_leases": leases,
    }))
}

pub async fn changes_since(
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
            'actor', (select name from principals where id = e.actor_id), 'at', e.at, 'payload', e.payload)
         from events e join projects pr on pr.id = e.project_id
         where e.seq > $1 and ($2::uuid is null or e.project_id = $2) and e.kind not like 'wake%' and {}
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
            'priority', t.priority, 'labels', to_jsonb(t.labels))
         from tasks t join projects pr on pr.id = t.project_id
         where ($1::uuid is null or pr.id = $1) and {}
           and (t.status = 'open' or (t.status = 'claimed' and t.lease_expires_at < now()))
           and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on where d.task_id = t.id and dt.status not in ('done','cancelled')) and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))
         order by t.priority desc, t.created_at limit 20",
        access_sql("$2", "$3")
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
        "select kind, count(*) from events where project_id = $1 and seq > $2 and kind not like 'wake%' group by kind order by kind",
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
