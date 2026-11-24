//! Linking a pull request to a task by hand (`link_pr`, `unlink_pr`), and how tasks show their PRs.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    repos::normalize_git_url,
    service,
    state::{AppState, Principal},
};

/// A task's PRs, newest first.
pub async fn prs_of(c: &mut PgConnection, task: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('provider', x.provider, 'repo', x.repo, 'number', x.number,
            'url', x.url, 'title', x.title, 'author', x.author, 'state', x.state, 'draft', x.draft,
            'head_branch', x.head_branch, 'checks', x.checks, 'failing', to_jsonb(x.failing), 'checks_url', x.checks_url,
            'review', x.review, 'mergeable', x.mergeable, 'explicit', x.explicit, 'updated_at', x.updated_at)
            order by (x.state = 'open') desc, x.updated_at desc), '[]'::jsonb)
         from forge_prs x where x.task_id = $1",
    )
    .bind(task)
    .fetch_one(c)
    .await?)
}

/// `(provider, owner/repo, number)` of a PR's web address: `https://github.com/acme/shop/pull/12`.
/// The host must be github.com or the web host of a connection.
pub(super) async fn parse_url(c: &mut PgConnection, url: &str) -> Result<(String, String, i64)> {
    let bad = || {
        Error::Invalid(format!(
            "{url:?} is not a pull request address (like https://github.com/acme/shop/pull/12)"
        ))
    };
    let u = url::Url::parse(url.trim()).map_err(|_| bad())?;
    let host = u.host_str().ok_or_else(bad)?.to_lowercase();
    let seg: Vec<&str> = u.path_segments().ok_or_else(bad)?.collect();
    let (owner, repo, kind, n) = match seg.as_slice() {
        [o, r, k, n, ..] if !o.is_empty() && !r.is_empty() => (*o, *r, *k, *n),
        _ => return Err(bad()),
    };
    if !matches!(kind, "pull" | "pulls") {
        return Err(bad());
    }
    let number: i64 = n.parse().map_err(|_| bad())?;
    let known: bool = host == "github.com"
        || sqlx::query_scalar(
            "select exists (select 1 from forge_connections where provider = 'github' and lower(web_url) like $1)",
        )
        .bind(format!("%//{host}%"))
        .fetch_one(&mut *c)
        .await?;
    if !known {
        return invalid(format!(
            "{host} is not a git host corntchap knows: connect it first (admin: Forges)"
        ));
    }
    Ok(("github".into(), format!("{owner}/{repo}"), number))
}

/// Link a PR to a task by hand. It wins over the branch and title rules, and replaces a link made
/// by them. The PR is followed even if its repository is in no pot.
/// Then the PR is read from GitHub at once (title, state, branch, checks, reviews), when an
/// installed app covers its repository; otherwise the answer carries a `warning` saying why the row
/// stays empty. Linking an already linked PR again refreshes it.
pub async fn link_pr(st: &AppState, p: &Principal, task: &str, url: &str) -> Result<Value> {
    let db = &st.db;
    let mut tx = db.begin().await?;
    let t = service::task_by_ref(&mut tx, p, task).await?;
    let (provider, repo, number) = parse_url(&mut tx, url).await?;
    let host = url::Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default();
    let norm = normalize_git_url(&format!("https://{host}/{repo}"));
    sqlx::query(
        "insert into forge_prs (provider, repo, norm_url, number, url) values ($1,$2,$3,$4,$5)
         on conflict (provider, lower(repo), number) do nothing",
    )
    .bind(&provider)
    .bind(&repo)
    .bind(&norm)
    .bind(number)
    .bind(url.trim())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "update forge_prs set task_id = $4, explicit = true, unlinked = false, updated_at = now()
         where provider = $1 and lower(repo) = lower($2) and number = $3",
    )
    .bind(&provider)
    .bind(&repo)
    .bind(number)
    .bind(t.id)
    .execute(&mut *tx)
    .await?;
    service::emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "forge.linked",
        Some(p.id),
        json!({"provider": provider, "repo": repo, "number": number, "url": url.trim(), "via": "explicit",
               "task": format!("{}-{}", t.key, t.number)}),
    )
    .await?;
    tx.commit().await?;
    let warning = super::sync::sync_pr(st, &host, &repo, number).await?;
    let mut c = db.acquire().await?;
    let mut out = service::task_full(&mut c, t.id).await?;
    out["warning"] = json!(warning);
    Ok(out)
}

/// Read the task's pull requests (or the one at `url`) from GitHub again.
pub async fn refresh_pr(
    st: &AppState,
    p: &Principal,
    task: &str,
    url: Option<&str>,
) -> Result<Value> {
    let mut c = st.db.acquire().await?;
    let t = service::task_by_ref(&mut c, p, task).await?;
    let only = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => {
            let (_, repo, number) = parse_url(&mut c, u).await?;
            Some((repo, number))
        }
        None => None,
    };
    drop(c);
    let warnings = super::sync::refresh_task(st, t.id, only).await?;
    let mut c = st.db.acquire().await?;
    let mut out = service::task_full(&mut c, t.id).await?;
    out["warning"] = if warnings.is_empty() {
        Value::Null
    } else {
        json!(warnings.join(" "))
    };
    Ok(out)
}

/// Take a PR off a task (all of its PRs when `url` is empty). The branch and title rules leave it
/// unlinked from then on.
pub async fn unlink_pr(db: &PgPool, p: &Principal, task: &str, url: Option<&str>) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = service::task_by_ref(&mut tx, p, task).await?;
    let filter = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => {
            let (provider, repo, number) = parse_url(&mut tx, u).await?;
            Some((provider, repo, number))
        }
        None => None,
    };
    let gone = sqlx::query(
        "update forge_prs set task_id = null, explicit = false, unlinked = true, updated_at = now()
         where task_id = $1 and ($2::text is null or (provider = $2 and lower(repo) = lower($3) and number = $4))",
    )
    .bind(t.id)
    .bind(filter.as_ref().map(|f| f.0.clone()))
    .bind(filter.as_ref().map(|f| f.1.clone()))
    .bind(filter.as_ref().map(|f| f.2))
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if gone == 0 {
        return Err(Error::NotFound(format!("{task} has no such pull request")));
    }
    service::emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "forge.unlinked",
        Some(p.id),
        json!({"count": gone, "task": format!("{}-{}", t.key, t.number)}),
    )
    .await?;
    let out = service::task_full(&mut tx, t.id).await?;
    tx.commit().await?;
    Ok(out)
}
