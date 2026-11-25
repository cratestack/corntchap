//! Saved views of a pot's tasks. A view is a SQL condition (`where`) and an optional order over
//! the pot's task view (one row per task, columns in [`COLUMNS`]), written by a person or by a cook
//! for them, shown as a list or a board.
//!
//! The SQL comes from users and agents, so it is never pasted in as text that could escape: it is
//! parsed as exactly one expression (or a comma-separated order), every node is checked against an
//! allowlist (the view's columns, literals, operators, CASE, casts and a short list of pure
//! functions; no subqueries, no other tables, no placeholders), and it runs in a read-only
//! transaction with a statement timeout, against the rows of that pot only.

use std::ops::ControlFlow;

use serde_json::{Value, json};
use sqlparser::{
    ast::{
        Expr, FunctionArg, FunctionArgExpr, FunctionArguments, InTarget, ObjectNamePart,
        OrderByExpr, Value as SqlValue, visit_expressions, visit_relations,
    },
    dialect::PostgreSqlDialect,
    parser::Parser,
    tokenizer::Token,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    menus::slug,
    service::{TASK_JSON, project_by_key},
    state::Principal,
};

/// The columns a view's SQL may use, with what they hold (shown to agents and in the console).
pub const COLUMNS: &[(&str, &str)] = &[
    ("id", "text: the task id, e.g. PAY-12"),
    ("number", "int: 12 in PAY-12"),
    ("title", "text"),
    ("body", "text"),
    (
        "status",
        "text: open, claimed (on the fire), blocked (waiting), done (served) or cancelled (tossed)",
    ),
    ("priority", "int: higher is picked first"),
    ("labels", "text[]: e.g. 'urgent' = any(labels)"),
    (
        "menu",
        "text: the menu key, e.g. mvp (null when on no menu)",
    ),
    ("claimed_by", "text: the cook holding it"),
    ("created_by", "text: who filed it"),
    ("created_at", "timestamptz"),
    ("updated_at", "timestamptz"),
    (
        "ready",
        "bool: open with every dependency and subtask done (claim_next may hand it out)",
    ),
    ("lease_live", "bool: claimed and the lease has not expired"),
    (
        "waiting_on_question",
        "bool: parked on a question until it is answered",
    ),
    ("open_items", "int: unticked checklist items"),
    ("total_items", "int"),
    ("parent", "text: the parent task id for a subtask"),
];

/// Functions a view may call: pure, cheap, and blind to other tables.
const FUNCTIONS: &[&str] = &[
    "lower",
    "upper",
    "length",
    "char_length",
    "coalesce",
    "nullif",
    "greatest",
    "least",
    "now",
    "current_date",
    "date_trunc",
    "age",
    "cardinality",
    "array_length",
    "array_position",
    "abs",
    "round",
    "starts_with",
    "position",
    "strpos",
    "left",
    "right",
    "trim",
    "btrim",
];

const MAX_ROWS: i64 = 500;

/// Rows of pot `$1` as a view's SQL sees them.
const VIEW_ROWS: &str = r#"select t.id as _tid, pr.key || '-' || t.number as id, t.number, t.title, t.body, t.status,
    t.priority, t.labels,
    (select m.key from menus m where m.id = t.menu_id) as menu,
    (select name from principals where id = t.claimed_by) as claimed_by,
    (select name from principals where id = t.created_by) as created_by,
    t.created_at, t.updated_at,
    (t.status = 'open' and not exists (select 1 from task_deps d join tasks dt on dt.id = d.depends_on
        where d.task_id = t.id and dt.status not in ('done','cancelled'))
      and not exists (select 1 from tasks sc where sc.parent_id = t.id and sc.status not in ('done','cancelled'))) as ready,
    (t.status = 'claimed' and t.lease_expires_at > now()) as lease_live,
    (t.status = 'blocked' and t.reserved_for is not null) as waiting_on_question,
    (select count(*) from items i where i.task_id = t.id and not i.done)::int as open_items,
    (select count(*) from items i where i.task_id = t.id)::int as total_items,
    (select pp.key || '-' || pt.number from tasks pt join projects pp on pp.id = pt.project_id where pt.id = t.parent_id) as parent
  from tasks t join projects pr on pr.id = t.project_id where t.project_id = $1"#;

fn bad(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// Every node of a view's SQL must be one of these.
fn check_expr(e: &Expr) -> std::result::Result<(), String> {
    use Expr::*;
    match e {
        Identifier(id) => {
            let name = id.value.to_lowercase();
            if COLUMNS.iter().any(|c| c.0 == name) {
                Ok(())
            } else {
                Err(format!(
                    "unknown column {:?}: a view sees {}",
                    id.value,
                    COLUMNS.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")
                ))
            }
        }
        Value(v) => match &v.value {
            SqlValue::Placeholder(_) => Err("placeholders ($1, ?) are not allowed".into()),
            _ => Ok(()),
        },
        In { target, .. } => match target {
            InTarget::List(_) => Ok(()),
            _ => Err("IN takes a list of values here, not a subquery or a table".into()),
        },
        Function(f) => {
            let name = match f.name.0.as_slice() {
                [ObjectNamePart::Identifier(i)] => i.value.to_lowercase(),
                _ => return Err(format!("function {} is not allowed", f.name)),
            };
            if !FUNCTIONS.contains(&name.as_str()) {
                return Err(format!(
                    "function {name} is not allowed (allowed: {})",
                    FUNCTIONS.join(", ")
                ));
            }
            if f.over.is_some() || f.filter.is_some() || !f.within_group.is_empty() {
                return Err(format!("{name}: no window, FILTER or WITHIN GROUP"));
            }
            if !matches!(f.parameters, FunctionArguments::None) {
                return Err(format!("{name}: unexpected parameters"));
            }
            match &f.args {
                FunctionArguments::None => Ok(()),
                FunctionArguments::Subquery(_) => Err("subqueries are not allowed".into()),
                FunctionArguments::List(l) => {
                    if !l.clauses.is_empty() {
                        return Err(format!("{name}: plain arguments only"));
                    }
                    for a in &l.args {
                        match a {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(_)) => {}
                            _ => return Err(format!("{name}: plain arguments only")),
                        }
                    }
                    Ok(())
                }
            }
        }
        IsFalse(_)
        | IsNotFalse(_)
        | IsTrue(_)
        | IsNotTrue(_)
        | IsNull(_)
        | IsNotNull(_)
        | IsUnknown(_)
        | IsNotUnknown(_)
        | IsDistinctFrom(..)
        | IsNotDistinctFrom(..)
        | Between { .. }
        | BinaryOp { .. }
        | Like { .. }
        | ILike { .. }
        | AnyOp { .. }
        | AllOp { .. }
        | UnaryOp { .. }
        | Cast { .. }
        | AtTimeZone { .. }
        | Extract { .. }
        | Nested(_)
        | TypedString(_)
        | Case { .. }
        | Array(_)
        | Interval(_)
        | Substring { .. }
        | Trim { .. }
        | Position { .. }
        | Ceil { .. }
        | Floor { .. }
        | Tuple(_) => Ok(()),
        Subquery(_) | Exists { .. } => Err("subqueries are not allowed".into()),
        other => Err(format!(
            "this kind of expression is not allowed in a view: {other}"
        )),
    }
}

fn check_tree<T: sqlparser::ast::Visit>(node: &T) -> Result<()> {
    if let ControlFlow::Break(r) = visit_relations(node, |r| ControlFlow::Break(r.to_string())) {
        return Err(bad(format!(
            "a view reads only its own rows, not {r} (no subqueries)"
        )));
    }
    if let ControlFlow::Break(msg) = visit_expressions(node, |e| match check_expr(e) {
        Ok(()) => ControlFlow::Continue(()),
        Err(m) => ControlFlow::Break(m),
    }) {
        return Err(bad(msg));
    }
    Ok(())
}

/// Parse and check a view's condition; returns it normalized (as the parser prints it).
pub fn check_where(sql: &str) -> Result<String> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Ok("true".into());
    }
    if sql.len() > 2000 {
        return invalid("a view's condition holds 2000 characters at most");
    }
    let dialect = PostgreSqlDialect {};
    let mut p = Parser::new(&dialect)
        .try_with_sql(sql)
        .map_err(|e| bad(format!("cannot read the condition: {e}")))?;
    let e = p
        .parse_expr()
        .map_err(|e| bad(format!("cannot read the condition: {e}")))?;
    if p.peek_token().token != Token::EOF {
        return invalid(format!(
            "the condition must be a single expression; unexpected {} after it",
            p.peek_token().token
        ));
    }
    check_tree(&e)?;
    Ok(e.to_string())
}

/// Parse and check a view's order (`priority desc, created_at`); returns it normalized.
pub fn check_order(sql: &str) -> Result<Option<String>> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Ok(None);
    }
    if sql.len() > 500 {
        return invalid("a view's order holds 500 characters at most");
    }
    let dialect = PostgreSqlDialect {};
    let mut p = Parser::new(&dialect)
        .try_with_sql(sql)
        .map_err(|e| bad(format!("cannot read the order: {e}")))?;
    let items: Vec<OrderByExpr> = p
        .parse_comma_separated(Parser::parse_order_by_expr)
        .map_err(|e| bad(format!("cannot read the order: {e}")))?;
    if p.peek_token().token != Token::EOF {
        return invalid(format!(
            "the order is a comma-separated list of columns; unexpected {}",
            p.peek_token().token
        ));
    }
    for i in &items {
        check_tree(&i.expr)?;
    }
    Ok(Some(
        items
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

/// Run a checked condition and order over a pot, read-only, at most 500 tasks.
async fn run_checked(
    db: &PgPool,
    project: Uuid,
    where_sql: &str,
    order_sql: Option<&str>,
) -> Result<Vec<Value>> {
    let mut tx = db.begin().await?;
    sqlx::query("set transaction read only")
        .execute(&mut *tx)
        .await?;
    sqlx::query("set local statement_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    let order = order_sql.unwrap_or("priority desc, created_at");
    let sql = format!(
        "with v as ({VIEW_ROWS}),
              sel as (select _tid, row_number() over (order by {order}, number) as rn
                      from v where ({where_sql}) order by rn limit {MAX_ROWS})
         select {TASK_JSON} from sel join tasks t on t.id = sel._tid join projects pr on pr.id = t.project_id
         order by sel.rn"
    );
    let rows: std::result::Result<Vec<Value>, sqlx::Error> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(project)
            .fetch_all(&mut *tx)
            .await;
    // Read-only: nothing to commit either way.
    tx.rollback().await.ok();
    rows.map_err(|e| match e {
        sqlx::Error::Database(d) => bad(format!("the view does not run: {}", d.message())),
        other => other.into(),
    })
}

const VIEW_JSON: &str = r#"jsonb_build_object('key', v.key, 'name', v.name, 'description', v.description,
  'layout', v.layout, 'where', v.where_sql, 'order', v.order_sql, 'position', v.position,
  'created_by', (select name from principals where id = v.created_by), 'updated_at', v.updated_at)"#;

pub async fn list(db: &PgPool, p: &Principal, project: &str) -> Result<Value> {
    let p = p.clone().worker()?;
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, &p, &project.trim().to_uppercase()).await?;
    let sql = format!(
        "select {VIEW_JSON} from pot_views v where v.project_id = $1 order by v.position, v.created_at"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows))
}

pub struct Save {
    pub project: String,
    pub name: String,
    pub key: Option<String>,
    pub description: Option<String>,
    pub layout: Option<String>,
    pub where_sql: Option<String>,
    pub order_sql: Option<String>,
}

/// Create a view, or update the one with the same key. The SQL is checked and run once first.
pub async fn save(db: &PgPool, p: &Principal, a: Save) -> Result<Value> {
    let p = p.clone().worker()?;
    let name = a.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return invalid("a view needs a name (80 characters at most)");
    }
    let key = slug(a.key.as_deref().unwrap_or(name));
    if key.is_empty() {
        return invalid("a view's key needs letters or digits");
    }
    let layout = a.layout.as_deref().map(str::trim).unwrap_or("list");
    if !matches!(layout, "list" | "board") {
        return invalid("layout is list or board");
    }
    let where_sql = check_where(a.where_sql.as_deref().unwrap_or(""))?;
    let order_sql = check_order(a.order_sql.as_deref().unwrap_or(""))?;
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, &p, &a.project.trim().to_uppercase()).await?;
    drop(c);
    run_checked(db, pr.id, &where_sql, order_sql.as_deref()).await?;
    let mut tx = db.begin().await?;
    let sql = format!(
        "insert into pot_views as v (project_id, key, name, description, layout, where_sql, order_sql, position, created_by)
         values ($1,$2,$3,$4,$5,$6,$7, (select coalesce(max(position), 0) + 1 from pot_views where project_id = $1), $8)
         on conflict (project_id, key) do update set name = excluded.name, description = excluded.description,
           layout = excluded.layout, where_sql = excluded.where_sql, order_sql = excluded.order_sql, updated_at = now()
         returning {VIEW_JSON}"
    );
    let out: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&key)
        .bind(name)
        .bind(
            a.description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        )
        .bind(layout)
        .bind(&where_sql)
        .bind(&order_sql)
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        Some(pr.id),
        None,
        "view.saved",
        Some(p.id),
        json!({"view": key, "title": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(out)
}

/// Delete a view: people, and the cook that made it.
pub async fn delete(db: &PgPool, p: &Principal, project: &str, key: &str) -> Result<Value> {
    let p = p.clone().worker()?;
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, &p, &project.trim().to_uppercase()).await?;
    let row: Option<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "select id, created_by, name from pot_views where project_id = $1 and key = $2",
    )
    .bind(pr.id)
    .bind(key.trim().to_lowercase())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, by, name)) = row else {
        return Err(Error::NotFound(format!("{} has no view {key}", pr.key)));
    };
    if p.is_agent() && by != Some(p.id) && !p.can_plan() {
        return Err(Error::Forbidden(
            "a cook deletes only the views it made (or with the plan grant)".into(),
        ));
    }
    sqlx::query("delete from pot_views where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        Some(pr.id),
        None,
        "view.deleted",
        Some(p.id),
        json!({"view": key, "title": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"ok": true}))
}

/// Run a saved view (`key`), or a condition and order to try one out.
pub async fn run(
    db: &PgPool,
    p: &Principal,
    project: &str,
    key: Option<&str>,
    where_sql: Option<&str>,
    order_sql: Option<&str>,
) -> Result<Vec<Value>> {
    let (project, w, o) = resolve(db, p, project, key, where_sql, order_sql).await?;
    run_checked(db, project, &w, o.as_deref()).await
}

/// The pot and the checked condition and order a `run` or `page` call stands for.
async fn resolve(
    db: &PgPool,
    p: &Principal,
    project: &str,
    key: Option<&str>,
    where_sql: Option<&str>,
    order_sql: Option<&str>,
) -> Result<(Uuid, String, Option<String>)> {
    let p = p.clone().worker()?;
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, &p, &project.trim().to_uppercase()).await?;
    let (w, o) = match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => {
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "select where_sql, order_sql from pot_views where project_id = $1 and key = $2",
            )
            .bind(pr.id)
            .bind(k.to_lowercase())
            .fetch_optional(&mut *c)
            .await?;
            let (w, o) =
                row.ok_or_else(|| Error::NotFound(format!("{} has no view {k}", pr.key)))?;
            // stored SQL was checked when saved; check again in case the rules tightened since
            (check_where(&w)?, check_order(o.as_deref().unwrap_or(""))?)
        }
        None => (
            check_where(where_sql.unwrap_or(""))?,
            check_order(order_sql.unwrap_or(""))?,
        ),
    };
    Ok((pr.id, w, o))
}

/// A page of a view's tasks for the console (`viewPage`): the same rows in the same order as
/// [`run`], without its 500-row cap. The cursor holds the last row number; it is tied to the
/// condition and order it was minted for.
pub async fn page(
    db: &PgPool,
    p: &Principal,
    project: &str,
    key: Option<&str>,
    where_sql: Option<&str>,
    order_sql: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    use sha2::{Digest, Sha256};
    let (project, w, o) = resolve(db, p, project, key, where_sql, order_sql).await?;
    let order = o.as_deref().unwrap_or("priority desc, created_at");
    let sig = hex::encode(&Sha256::digest(format!("{w}\n{order}").as_bytes())[..6]);
    let sort = format!("rn:{sig}");
    let paging = crate::paging::paging(
        page,
        &sort,
        1,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let after: i64 = match paging.key(0) {
        Some(k) => k.parse().map_err(|_| bad("invalid cursor".to_string()))?,
        None => 0,
    };
    let mut tx = db.begin().await?;
    sqlx::query("set transaction read only")
        .execute(&mut *tx)
        .await?;
    sqlx::query("set local statement_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    let sql = format!(
        "with v as ({VIEW_ROWS}),
              sel as (select _tid, row_number() over (order by {order}, number) as rn
                      from v where ({w}))
         select {TASK_JSON}, array[sel.rn::text]
         from sel join tasks t on t.id = sel._tid join projects pr on pr.id = t.project_id
         where sel.rn > $2 order by sel.rn limit $3"
    );
    let rows: std::result::Result<Vec<(Value, Vec<String>)>, sqlx::Error> =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(project)
            .bind(after)
            .bind(paging.fetch())
            .fetch_all(&mut *tx)
            .await;
    tx.rollback().await.ok();
    let rows = rows.map_err(|e| match e {
        sqlx::Error::Database(d) => bad(format!("the view does not run: {}", d.message())),
        other => other.into(),
    })?;
    let rows = rows
        .into_iter()
        .map(|(v, k)| (v, k.into_iter().map(Some).collect()))
        .collect();
    Ok(crate::paging::finish(rows, &paging))
}

/// The columns a view sees, for the tool descriptions and the console.
pub fn columns_doc() -> String {
    COLUMNS
        .iter()
        .map(|(n, d)| format!("{n} ({d})"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_conditions_over_the_view() {
        for ok in [
            "status = 'open' and 'urgent' = any(labels)",
            "menu = 'mvp' or priority > 3",
            "lower(title) like '%refund%'",
            "created_at > now() - interval '7 days'",
            "claimed_by in ('doug', 'c1') and not ready",
            "coalesce(menu, '') = '' and open_items > 0",
            "case when priority > 5 then true else false end",
            "",
        ] {
            assert!(check_where(ok).is_ok(), "{ok}: {:?}", check_where(ok));
        }
        assert_eq!(
            check_order("priority desc, created_at").unwrap().as_deref(),
            Some("priority DESC, created_at")
        );
    }

    #[test]
    fn refuses_escapes_and_other_tables() {
        for bad in [
            "true) union select token_hash from tokens --",
            "true; drop table tasks",
            "exists (select 1 from principals)",
            "id in (select key from projects)",
            "token_hash is not null",
            "pr.key = 'PAY'",
            "pg_sleep(10) is null",
            "set_config('x', 'y', false) = 'y'",
            "status = $1",
            "(select count(*) from tokens) > 0",
            "title = 'a' -- and then",
        ] {
            let r = check_where(bad);
            // a trailing comment is harmless (it is dropped when normalized); everything else fails
            if bad.contains("--") && !bad.contains("union") {
                assert!(
                    r.as_deref().is_ok_and(|s| !s.contains("--")),
                    "{bad}: {r:?}"
                );
            } else {
                assert!(r.is_err(), "{bad} must be refused, got {r:?}");
            }
        }
        assert!(check_order("priority desc; drop table tasks").is_err());
        assert!(check_order("(select 1)").is_err());
    }
}
