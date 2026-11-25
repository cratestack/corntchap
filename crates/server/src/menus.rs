//! Menus: a project's milestones ("MVP", "v1.2"). A task sits on at most one menu; a menu is
//! `open` while it is being cooked and `served` once it shipped. Every member of the project's
//! team (people and cooks) can see, create and edit them; serving and reopening too.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    service::{emit, project_by_key},
    state::Principal,
};

/// SQL for one menu with its progress. Needs aliases `m` (menus) and `pr` (projects).
const MENU_JSON: &str = r#"jsonb_build_object(
  'key', m.key, 'project', pr.key, 'name', m.name, 'description', m.description,
  'due_on', to_char(m.due_on, 'YYYY-MM-DD'), 'status', m.status, 'position', m.position,
  'served_at', m.served_at, 'created_at', m.created_at, 'updated_at', m.updated_at,
  'total', (select count(*) from tasks t where t.menu_id = m.id and t.status <> 'cancelled'),
  'done', (select count(*) from tasks t where t.menu_id = m.id and t.status = 'done'),
  'open', (select count(*) from tasks t where t.menu_id = m.id and t.status = 'open'),
  'claimed', (select count(*) from tasks t where t.menu_id = m.id and t.status = 'claimed'),
  'blocked', (select count(*) from tasks t where t.menu_id = m.id and t.status = 'blocked'))"#;

/// A menu key: lowercase letters, digits, `.`, `_`, `-`, up to 40 characters ("MVP" -> "mvp",
/// "v1.2 launch" -> "v1.2-launch").
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_') {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(40)
        .collect();
    while out.ends_with(['-', '.', '_']) {
        out.pop();
    }
    out
}

fn with_progress(mut m: Value) -> Value {
    let (total, done) = (
        m["total"].as_i64().unwrap_or(0),
        m["done"].as_i64().unwrap_or(0),
    );
    m["progress"] = json!(if total == 0 {
        0.0
    } else {
        done as f64 / total as f64
    });
    m
}

async fn menu_json(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {MENU_JSON} from menus m join projects pr on pr.id = m.project_id where m.id = $1"
    );
    let v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?;
    Ok(with_progress(v))
}

/// The id of an open menu, for putting a task on it.
pub async fn open_menu_id(
    c: &mut PgConnection,
    project: Uuid,
    project_key: &str,
    key: &str,
) -> Result<Uuid> {
    let k = slug(key);
    let row: Option<(Uuid, String)> =
        sqlx::query_as("select id, status from menus where project_id = $1 and key = $2")
            .bind(project)
            .bind(&k)
            .fetch_optional(c)
            .await?;
    match row {
        None => Err(Error::NotFound(format!(
            "no menu {k} in {project_key}: create it first (create_menu), or see list_menus"
        ))),
        Some((_, s)) if s == "served" => Err(Error::Conflict(format!(
            "menu {k} is already served: reopen it (update_menu status=open) to add tasks"
        ))),
        Some((id, _)) => Ok(id),
    }
}

fn check_due(due: Option<&str>) -> Result<Option<String>> {
    match due.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(Some(String::new())),
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map(|_| Some(d.to_string()))
            .map_err(|_| Error::Invalid(format!("due date {d:?}: expected YYYY-MM-DD"))),
    }
}

/// Menus of a project, open ones first (by position, then due date), each with its progress.
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: &str,
    include_served: bool,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.to_uppercase()).await?;
    let sql = format!(
        "select {MENU_JSON} from menus m join projects pr on pr.id = m.project_id
         where m.project_id = $1 and ($2 or m.status = 'open')
         order by m.status = 'served', m.position, m.due_on nulls last, m.created_at"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(include_served)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows.into_iter().map(with_progress).collect()))
}

/// A page of a project's menus (`menuPage`, the `list_menus` tool): open ones first, then served,
/// each by position and key, with its progress. `status` is `open` (default), `served` or `all`;
/// `q` matches the key and the name (any case).
pub async fn page(
    db: &PgPool,
    p: &Principal,
    project: &str,
    status: Option<&str>,
    q: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let status = status
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "open".into());
    if !["open", "served", "all"].contains(&status.as_str()) {
        return invalid("status is open, served or all");
    }
    let paging = crate::paging::paging(
        page,
        "served,position,key",
        3,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let q = q.map(str::trim).filter(|q| !q.is_empty());
    let sql = format!(
        "select {MENU_JSON}, array[(m.status = 'served')::int::text, m.position::text, m.key]
         from menus m join projects pr on pr.id = m.project_id
         where m.project_id = $1 and ($2 = 'all' or m.status = $2)
           and ($3::text is null or position(lower($3) in lower(m.key || ' ' || m.name)) > 0)
           and ($4::text is null or ((m.status = 'served')::int, m.position, m.key)
                > ($4::text::bigint, $5::text::bigint, $6::text))
         order by (m.status = 'served'), m.position, m.key limit $7"
    );
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&status)
        .bind(q)
        .bind(paging.key(0))
        .bind(paging.key(1))
        .bind(paging.key(2))
        .bind(paging.fetch())
        .fetch_all(&mut *c)
        .await?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (with_progress(v), k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}

pub struct NewMenu {
    pub project: String,
    pub name: String,
    pub key: Option<String>,
    pub description: Option<String>,
    pub due_on: Option<String>,
    pub position: Option<i64>,
}

pub async fn create(db: &PgPool, p: &Principal, n: NewMenu) -> Result<Value> {
    let name = n.name.trim();
    if name.is_empty() {
        return invalid("name is required, e.g. MVP");
    }
    let key = slug(n.key.as_deref().unwrap_or(name));
    if key.is_empty() {
        return invalid("menu key is empty: use letters, digits, '.', '_' or '-'");
    }
    let due = check_due(n.due_on.as_deref())?.filter(|d| !d.is_empty());
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &n.project.to_uppercase()).await?;
    let position = match n.position {
        Some(x) => x,
        None => {
            sqlx::query_scalar(
                "select coalesce(max(position), 0) + 1 from menus where project_id = $1",
            )
            .bind(pr.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let id = Uuid::new_v4();
    let inserted = sqlx::query(
        "insert into menus (id, project_id, key, name, description, due_on, position, created_by)
         values ($1,$2,$3,$4,$5,$6::date,$7,$8) on conflict (project_id, key) do nothing",
    )
    .bind(id)
    .bind(pr.id)
    .bind(&key)
    .bind(name)
    .bind(n.description.as_deref().unwrap_or("").trim())
    .bind(due)
    .bind(position)
    .bind(p.id)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(Error::Conflict(format!(
            "{} already has a menu {key}",
            pr.key
        )));
    }
    emit(
        &mut tx,
        Some(pr.id),
        None,
        "menu.created",
        Some(p.id),
        json!({"menu": key, "title": name}),
    )
    .await?;
    let out = menu_json(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

pub struct MenuEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    /// "" clears it.
    pub due_on: Option<String>,
    pub position: Option<i64>,
    /// "open" or "served".
    pub status: Option<String>,
}

pub async fn update(
    db: &PgPool,
    p: &Principal,
    project: &str,
    key: &str,
    e: MenuEdit,
) -> Result<Value> {
    let status = match e.status.as_deref().map(|s| s.trim().to_lowercase()) {
        None => None,
        Some(s) if s == "open" || s == "served" => Some(s),
        Some(s) => return invalid(format!("status {s:?}: open or served")),
    };
    if e.name.as_deref().is_some_and(|n| n.trim().is_empty()) {
        return invalid("name cannot be empty");
    }
    let due = check_due(e.due_on.as_deref())?;
    let key = slug(key);
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &project.to_uppercase()).await?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "select id, status from menus where project_id = $1 and key = $2 for update",
    )
    .bind(pr.id)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, old_status)) = row else {
        return Err(Error::NotFound(format!("no menu {key} in {}", pr.key)));
    };
    sqlx::query(
        "update menus set name = coalesce($2, name), description = coalesce($3, description),
           due_on = case when $4::text is null then due_on when $4 = '' then null else $4::date end,
           position = coalesce($5, position), status = coalesce($6, status),
           served_at = case when $6 = 'served' and status <> 'served' then now()
                            when $6 = 'open' then null else served_at end
         where id = $1",
    )
    .bind(id)
    .bind(e.name.as_deref().map(str::trim))
    .bind(e.description.as_deref().map(str::trim))
    .bind(due)
    .bind(e.position)
    .bind(status.as_deref())
    .execute(&mut *tx)
    .await?;
    let kind = match status.as_deref() {
        Some("served") if old_status != "served" => "menu.served",
        Some("open") if old_status == "served" => "menu.reopened",
        _ => "menu.updated",
    };
    let out = menu_json(&mut tx, id).await?;
    emit(
        &mut tx,
        Some(pr.id),
        None,
        kind,
        Some(p.id),
        json!({"menu": key, "title": out["name"]}),
    )
    .await?;
    tx.commit().await?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn menu_keys() {
        assert_eq!(slug("MVP"), "mvp");
        assert_eq!(slug("v1.2 launch!"), "v1.2-launch");
        assert_eq!(slug("  Beta / Public "), "beta-public");
        assert_eq!(slug(&"x".repeat(60)).len(), 40);
    }
}
