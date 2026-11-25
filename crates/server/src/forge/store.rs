//! Connections, installations and deliveries (tables of migration 0026).
//!
//! The secrets of a connection (client secret, private key, webhook secret) live in
//! `forge_connections`, as a wake target's secrets live in its `config`, and are never selected by
//! the functions that feed an API ([`list`] names its columns).

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::Result;

/// What the signature check needs.
pub struct Secret {
    pub id: Uuid,
    pub secret: String,
}

/// The webhook secrets of every connection of a provider.
pub async fn secrets(db: &PgPool, provider: &str) -> Result<Vec<Secret>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "select id, webhook_secret from forge_connections where provider = $1 order by created_at",
    )
    .bind(provider)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, secret)| Secret { id, secret })
        .collect())
}

/// Connections with their installations and last delivery. No secret is selected.
pub async fn list(db: &PgPool) -> Result<Value> {
    let v: Value = sqlx::query_scalar(
        "select coalesce(jsonb_agg(jsonb_build_object(
            'id', c.id, 'provider', c.provider, 'name', c.name, 'api_url', c.api_url, 'web_url', c.web_url,
            'app_id', c.app_id, 'slug', c.slug, 'html_url', c.html_url, 'owner', c.owner_login,
            'install_url', case when c.slug is null then null else c.web_url || '/apps/' || c.slug || '/installations/new' end,
            'created_at', c.created_at,
            'created_by', (select name from principals where id = c.created_by),
            'installations', coalesce((select jsonb_agg(jsonb_build_object('id', i.installation_id, 'account', i.account,
                  'account_type', i.account_type, 'repo_selection', i.repo_selection, 'suspended', i.suspended)
                  order by i.created_at) from forge_installations i where i.connection_id = c.id), '[]'::jsonb),
            'last_delivery', (select jsonb_build_object('event', d.event, 'action', d.action, 'status', d.status,
                  'detail', d.detail, 'at', d.at) from forge_deliveries d where d.connection_id = c.id order by d.at desc, d.id desc limit 1)
         ) order by c.created_at), '[]'::jsonb) from forge_connections c",
    )
    .fetch_one(db)
    .await?;
    Ok(v)
}

/// Remove a connection (its installations and deliveries go with it). The pull requests it told us
/// about stay: they are history.
pub async fn remove(db: &PgPool, id: Uuid) -> Result<bool> {
    super::catalog::forget_listings(id);
    Ok(sqlx::query("delete from forge_connections where id = $1")
        .bind(id)
        .execute(db)
        .await?
        .rows_affected()
        > 0)
}

/// Claim a delivery id. False when it was already received (and did not fail), so a redelivery is
/// answered 200 and not processed twice.
pub async fn claim_delivery(
    db: &PgPool,
    provider: &str,
    delivery: &str,
    connection: Uuid,
    event: &str,
    action: Option<&str>,
) -> Result<bool> {
    let got: Option<i64> = sqlx::query_scalar(
        "insert into forge_deliveries (provider, delivery_id, connection_id, event, action, status)
         values ($1,$2,$3,$4,$5,'received')
         on conflict (provider, delivery_id) do update set status = 'received', at = now(), detail = null
           where forge_deliveries.status = 'error'
         returning id",
    )
    .bind(provider)
    .bind(delivery)
    .bind(connection)
    .bind(event)
    .bind(action)
    .fetch_optional(db)
    .await?;
    Ok(got.is_some())
}

pub async fn finish_delivery(
    db: &PgPool,
    provider: &str,
    delivery: &str,
    status: &str,
    detail: Option<&str>,
) {
    let _ = sqlx::query(
        "update forge_deliveries set status = $3, detail = $4 where provider = $1 and delivery_id = $2",
    )
    .bind(provider)
    .bind(delivery)
    .bind(status)
    .bind(detail.map(|d| d.chars().take(500).collect::<String>()))
    .execute(db)
    .await;
}

/// Keep what the events tell about where the app is installed.
pub async fn upsert_installation(
    db: &PgPool,
    connection: Uuid,
    installation: i64,
    account: Option<&str>,
    account_type: Option<&str>,
    repo_selection: Option<&str>,
) -> Result<()> {
    super::catalog::forget_listings(connection);
    sqlx::query(
        "insert into forge_installations (connection_id, installation_id, account, account_type, repo_selection)
         values ($1,$2,$3,$4,$5)
         on conflict (connection_id, installation_id) do update set
           account = coalesce(excluded.account, forge_installations.account),
           account_type = coalesce(excluded.account_type, forge_installations.account_type),
           repo_selection = coalesce(excluded.repo_selection, forge_installations.repo_selection)",
    )
    .bind(connection)
    .bind(installation)
    .bind(account)
    .bind(account_type)
    .bind(repo_selection)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn remove_installation(db: &PgPool, connection: Uuid, installation: i64) -> Result<()> {
    super::catalog::forget_listings(connection);
    sqlx::query(
        "delete from forge_installations where connection_id = $1 and installation_id = $2",
    )
    .bind(connection)
    .bind(installation)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn suspend_installation(
    db: &PgPool,
    connection: Uuid,
    installation: i64,
    suspended: bool,
) -> Result<()> {
    sqlx::query(
        "update forge_installations set suspended = $3 where connection_id = $1 and installation_id = $2",
    )
    .bind(connection)
    .bind(installation)
    .bind(suspended)
    .execute(db)
    .await?;
    Ok(())
}

/// Start a manifest flow: a random `state` the callback must bring back within an hour.
pub async fn start_flow(
    db: &PgPool,
    org: Option<&str>,
    api_url: &str,
    web_url: &str,
    by: Uuid,
) -> Result<String> {
    let state = crate::auth::random_hex(24);
    sqlx::query("delete from forge_states where expires_at < now()")
        .execute(db)
        .await?;
    sqlx::query(
        "insert into forge_states (state, provider, org, api_url, web_url, created_by, expires_at)
         values ($1,'github',$2,$3,$4,$5, now() + interval '1 hour')",
    )
    .bind(&state)
    .bind(org)
    .bind(api_url)
    .bind(web_url)
    .bind(by)
    .execute(db)
    .await?;
    Ok(state)
}

/// Finish a flow: the state is single use. Returns (org, api_url, web_url, started_by).
pub async fn take_flow(
    db: &PgPool,
    state: &str,
) -> Result<Option<(Option<String>, String, String, Option<Uuid>)>> {
    Ok(sqlx::query_as(
        "delete from forge_states where state = $1 and expires_at > now()
         returning org, api_url, web_url, created_by",
    )
    .bind(state)
    .fetch_optional(db)
    .await?)
}

/// What GitHub returned for a manifest (`POST /app-manifests/{code}/conversions`).
pub struct NewApp {
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub pem: Option<String>,
    pub webhook_secret: String,
    pub owner: Option<String>,
}

pub async fn insert_connection(
    db: &PgPool,
    provider: &str,
    app: &NewApp,
    api_url: &str,
    web_url: &str,
    by: Option<Uuid>,
) -> Result<Value> {
    let id: Uuid = sqlx::query_scalar(
        "insert into forge_connections (provider, name, api_url, web_url, app_id, slug, html_url, client_id,
           client_secret, private_key, webhook_secret, owner_login, created_by)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) returning id",
    )
    .bind(provider)
    .bind(&app.name)
    .bind(api_url)
    .bind(web_url)
    .bind(app.app_id)
    .bind(&app.slug)
    .bind(&app.html_url)
    .bind(&app.client_id)
    .bind(&app.client_secret)
    .bind(&app.pem)
    .bind(&app.webhook_secret)
    .bind(&app.owner)
    .bind(by)
    .fetch_one(db)
    .await?;
    Ok(json!({"id": id, "name": app.name, "slug": app.slug}))
}
