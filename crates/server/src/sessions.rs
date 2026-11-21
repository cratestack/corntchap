//! Browser sessions: an authkestra `SessionStore` over our sqlx 0.9 pool.
//!
//! (authkestra-store-sqlx targets sqlx 0.8, which cannot share a graph with cratestack's
//! `=0.9.0` pin, so the three trait methods are implemented here.) The cookie carries a random
//! token; only its SHA-256 is stored. The session's `identity.external_id` is the principal id;
//! the CSRF token and the IdP's id_token (kept as `id_token_hint` for logout) ride in
//! `identity.attributes`.

use async_trait::async_trait;
use authkestra::core::{
    auth::{Identity, Session, SessionStore},
    error::AuthError,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::sha256;

pub const ATTR_CSRF: &str = "csrf";
pub const ATTR_ID_TOKEN: &str = "id_token";

#[derive(Clone)]
pub struct PgSessionStore {
    pub db: PgPool,
}

fn internal(e: impl std::fmt::Display) -> AuthError {
    AuthError::Session(e.to_string())
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn load_session(&self, id: &str) -> Result<Option<Session>, AuthError> {
        let data: Option<serde_json::Value> = sqlx::query_scalar(
            "select s.data from auth_sessions s join principals p on p.id = s.user_id
             where s.id_hash = $1 and s.expires_at > now() and p.disabled_at is null",
        )
        .bind(sha256(id.as_bytes()))
        .fetch_optional(&self.db)
        .await
        .map_err(internal)?;
        data.map(|d| serde_json::from_value(d).map_err(internal))
            .transpose()
    }

    async fn save_session(&self, session: &Session) -> Result<(), AuthError> {
        let user: Uuid = session.identity.external_id.parse().map_err(internal)?;
        sqlx::query(
            "insert into auth_sessions (id_hash, user_id, data, expires_at) values ($1,$2,$3,$4)
             on conflict (id_hash) do update set data = excluded.data, expires_at = excluded.expires_at",
        )
        .bind(sha256(session.id.as_bytes()))
        .bind(user)
        .bind(serde_json::to_value(session).map_err(internal)?)
        .bind(session.expires_at)
        .execute(&self.db)
        .await
        .map_err(internal)?;
        Ok(())
    }

    async fn delete_session(&self, id: &str) -> Result<(), AuthError> {
        sqlx::query("delete from auth_sessions where id_hash = $1")
            .bind(sha256(id.as_bytes()))
            .execute(&self.db)
            .await
            .map_err(internal)?;
        Ok(())
    }
}

/// A fresh session for `user` (the principal id).
pub fn new_session(
    id: String,
    user: Uuid,
    name: &str,
    csrf: String,
    id_token: Option<String>,
    ttl_secs: i64,
) -> Session {
    let mut attributes = std::collections::HashMap::new();
    attributes.insert(ATTR_CSRF.to_string(), csrf);
    if let Some(t) = id_token {
        attributes.insert(ATTR_ID_TOKEN.to_string(), t);
    }
    Session::new(
        id,
        Identity {
            provider_id: "oidc".into(),
            external_id: user.to_string(),
            email: None,
            username: Some(name.to_string()),
            attributes,
        },
        chrono::Utc::now() + chrono::Duration::seconds(ttl_secs),
    )
}
