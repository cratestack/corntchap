//! Repositories that send forge events although no pot follows them (table of migration 0031).
//!
//! A delivery about such a repository is ignored ("in no pot"); it is remembered here (full name,
//! url, installation, last seen, how many) so an admin sees it on the Forges page and can add it to a
//! pot. One that has joined a pot since is no longer listed.

use std::collections::HashMap;

use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::RepoRef;
use crate::error::Result;

/// Remember a delivery about a repository no pot follows.
pub async fn note(
    c: &mut PgConnection,
    provider: &str,
    repo: &RepoRef,
    installation: Option<i64>,
    connection: Uuid,
) -> Result<()> {
    sqlx::query(
        "insert into forge_unfollowed (provider, norm_url, repo, html_url, connection_id, installation_id)
         values ($1,$2,$3,$4,$5,$6)
         on conflict (provider, norm_url) do update set last_seen = now(), events = forge_unfollowed.events + 1,
           repo = excluded.repo, installation_id = coalesce(excluded.installation_id, forge_unfollowed.installation_id)",
    )
    .bind(provider)
    .bind(&repo.norm_url)
    .bind(&repo.full_name)
    .bind(&repo.html_url)
    .bind(connection)
    .bind(installation)
    .execute(c)
    .await?;
    Ok(())
}

/// The repositories that sent events no pot follows, most recent first.
pub async fn list(db: &PgPool) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object('provider', u.provider, 'repo', u.repo, 'url', u.html_url,
            'installation', u.installation_id, 'events', u.events, 'first_seen', u.first_seen, 'last_seen', u.last_seen)
            order by u.last_seen desc), '[]'::jsonb)
         from forge_unfollowed u
         where not exists (select 1 from project_repos r where r.norm_url = u.norm_url)",
    )
    .fetch_one(db)
    .await?)
}

/// A page of the repositories that sent events no pot follows, most recent first (last_seen desc,
/// then norm_url), narrowed by `q` on the repository name (any case).
pub async fn page(
    db: &PgPool,
    q: Option<&str>,
    page: &cratestack::CursorPageInput,
) -> Result<(Vec<Value>, Option<String>)> {
    let paging = crate::paging::paging(
        page,
        "-last_seen,provider,norm_url",
        3,
        crate::paging::DEFAULT_LIMIT,
        crate::paging::MAX_LIMIT,
    )?;
    let rows: Vec<(Value, Vec<String>)> = sqlx::query_as(
        "select jsonb_build_object('provider', u.provider, 'repo', u.repo, 'url', u.html_url,
            'installation', u.installation_id, 'events', u.events, 'first_seen', u.first_seen, 'last_seen', u.last_seen),
            array[u.last_seen::text, u.provider, u.norm_url]
         from forge_unfollowed u
         where not exists (select 1 from project_repos r where r.norm_url = u.norm_url)
           and ($1::text is null or position(lower($1) in lower(u.repo)) > 0)
           and ($2::text is null or u.last_seen < $2::text::timestamptz
                or (u.last_seen = $2::text::timestamptz
                    and (u.provider, u.norm_url) > ($3::text, $4::text)))
         order by u.last_seen desc, u.provider, u.norm_url limit $5",
    )
    .bind(q.map(str::trim).filter(|q| !q.is_empty()))
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

/// `norm_url -> events`, for marking a listing.
pub async fn counts(db: &PgPool) -> Result<HashMap<String, i32>> {
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "select u.norm_url, u.events from forge_unfollowed u
         where not exists (select 1 from project_repos r where r.norm_url = u.norm_url)",
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Forget one (by `owner/name`, any case). False when it was not listed.
pub async fn dismiss(db: &PgPool, repo: &str) -> Result<bool> {
    Ok(
        sqlx::query("delete from forge_unfollowed where lower(repo) = lower($1)")
            .bind(repo.trim())
            .execute(db)
            .await?
            .rows_affected()
            > 0,
    )
}
