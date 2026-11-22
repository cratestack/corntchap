//! What corntchap sends to agents, counted per day (`usage_daily`): MCP tool results, tool
//! listings and wake prompts. It is an estimate of the text corntchap injects (tokens are
//! ceil(chars / 4)); what an agent itself uses is not visible here.
//!
//! Recording never fails or slows a response: the MCP paths record from a spawned task, the wake
//! paths in a savepoint, and every error is logged and dropped.

use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{admin, error::Result, state::AppState, state::Principal};

/// How the figures are made, shown with every report.
pub const NOTE: &str = "An estimate: characters of text corntchap sent to agents (tool results, \
tool listings, wake prompts) divided by 4. The agent's own token usage is not visible to corntchap.";

/// Days of usage kept.
pub const RETAIN_DAYS: i32 = 90;

pub const TOOL: &str = "tool";
pub const LIST: &str = "list";
pub const WAKE: &str = "wake";

/// Estimated tokens for some text: ceil(chars / 4).
pub fn tokens(chars: i64) -> i64 {
    (chars.max(0) + 3) / 4
}

const UPSERT: &str = "insert into usage_daily (day, principal_id, project_id, kind, name, calls, chars)
     values ((now() at time zone 'utc')::date, $1,
             coalesce($2::uuid, (select id from projects where key = upper($3::text))), $4, $5, 1, $6)
     on conflict on constraint usage_daily_key
     do update set calls = usage_daily.calls + 1, chars = usage_daily.chars + excluded.chars";

/// Count one send, from a spawned task (the MCP paths).
pub fn record(
    state: &AppState,
    principal: Uuid,
    project_key: Option<String>,
    kind: &'static str,
    name: &str,
    chars: u64,
) {
    let db = state.db.clone();
    let name = name.to_string();
    tokio::spawn(async move {
        let r = sqlx::query(UPSERT)
            .bind(principal)
            .bind(None::<Uuid>)
            .bind(project_key)
            .bind(kind)
            .bind(name)
            .bind(chars as i64)
            .execute(&db)
            .await;
        if let Err(e) = r {
            tracing::warn!("usage not recorded: {e}");
        }
    });
}

/// Count one send inside the caller's connection or transaction. A savepoint keeps a failed
/// insert from aborting the caller's transaction.
pub async fn record_in(
    c: &mut PgConnection,
    principal: Uuid,
    project: Option<Uuid>,
    kind: &str,
    name: &str,
    chars: usize,
) {
    use sqlx::Connection;
    let run = async {
        let mut sp = c.begin().await?;
        sqlx::query(UPSERT)
            .bind(principal)
            .bind(project)
            .bind(None::<String>)
            .bind(kind)
            .bind(name)
            .bind(chars as i64)
            .execute(&mut *sp)
            .await?;
        sp.commit().await
    };
    if let Err(e) = run.await {
        tracing::warn!("usage not recorded: {e}");
    }
}

/// Delete usage older than [`RETAIN_DAYS`] days.
pub async fn prune(state: &AppState) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("delete from usage_daily where day < (now() at time zone 'utc')::date - $1::int")
        .bind(RETAIN_DAYS)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Row {
    day: String,
    cook: String,
    kind: String,
    name: String,
    calls: i64,
    chars: i64,
}

#[derive(Default, Clone, Copy)]
struct Sum {
    calls: i64,
    chars: i64,
}

impl Sum {
    fn add(&mut self, r: &Row) {
        self.calls += r.calls;
        self.chars += r.chars;
    }
    fn json(self) -> Value {
        json!({"calls": self.calls, "chars": self.chars, "tokens": tokens(self.chars)})
    }
}

/// The usage of the cooks you may see (yours; all for admins; a cook sees itself, or its owner's
/// view with the `manage` grant) for the last `days` days (default 7, at most 90).
pub async fn report(
    db: &sqlx::PgPool,
    p: &Principal,
    project: Option<&str>,
    cook: Option<&str>,
    days: Option<i64>,
) -> Result<Value> {
    let who = admin::acting_or_self(db, p).await?;
    let days = days.unwrap_or(7).clamp(1, RETAIN_DAYS as i64);
    let today = Utc::now().date_naive();
    let first = today - Duration::days(days - 1);
    let rows: Vec<Row> = sqlx::query_as(
        "select u.day::text as day, a.name as cook, u.kind, u.name,
                sum(u.calls)::bigint as calls, sum(u.chars)::bigint as chars
         from usage_daily u
         join principals a on a.id = u.principal_id
         left join projects pr on pr.id = u.project_id
         where u.day >= $1::text::date
           and ($2 or a.id = $3 or a.owner_id = $3)
           and ($4::text is null or pr.key = $4)
           and ($5::text is null or a.name = $5)
         group by 1, 2, 3, 4",
    )
    .bind(first.to_string())
    .bind(who.is_admin)
    .bind(who.id)
    .bind(project.map(|k| k.trim().to_uppercase()))
    .bind(cook.map(|c| c.trim().to_lowercase()))
    .fetch_all(db)
    .await?;

    let mut total = Sum::default();
    let mut by_day: std::collections::BTreeMap<String, Sum> = (0..days)
        .map(|i| ((first + Duration::days(i)).to_string(), Sum::default()))
        .collect();
    let mut by_cook: std::collections::BTreeMap<String, Sum> = Default::default();
    let mut top: std::collections::BTreeMap<(String, String), Sum> = Default::default();
    for r in &rows {
        total.add(r);
        by_day.entry(r.day.clone()).or_default().add(r);
        by_cook.entry(r.cook.clone()).or_default().add(r);
        top.entry((r.kind.clone(), r.name.clone()))
            .or_default()
            .add(r);
    }
    let with = |mut v: Value, extra: Value| {
        if let (Some(a), Some(b)) = (v.as_object_mut(), extra.as_object()) {
            a.extend(b.clone());
        }
        v
    };
    let mut cooks: Vec<(String, Sum)> = by_cook.into_iter().collect();
    cooks.sort_by(|a, b| b.1.chars.cmp(&a.1.chars).then(a.0.cmp(&b.0)));
    let mut tops: Vec<((String, String), Sum)> = top.into_iter().collect();
    tops.sort_by(|a, b| b.1.chars.cmp(&a.1.chars).then(a.0.cmp(&b.0)));
    Ok(json!({
        "days": days,
        "note": NOTE,
        "totals": total.json(),
        "by_day": by_day.into_iter().map(|(d, s)| with(s.json(), json!({"day": d}))).collect::<Vec<_>>(),
        "by_cook": cooks.into_iter().map(|(c, s)| with(s.json(), json!({"cook": c}))).collect::<Vec<_>>(),
        "top": tops.into_iter().take(10).map(|((k, n), s)| with(s.json(), json!({"kind": k, "name": n}))).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn tokens_round_up() {
        assert_eq!([0, 1, 4, 5, 8, 9].map(tokens), [0, 1, 1, 2, 2, 3]);
    }
}
