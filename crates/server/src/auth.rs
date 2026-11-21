//! Authentication: opaque bearer tokens, checked by an authkestra `Guard`.
//! Tokens are minted by the RFC 8628 device flow (approved through OIDC SSO)
//! or by `corntchap token` / `agent add`; only a SHA-256 of each is stored.

use std::sync::Arc;

use async_trait::async_trait;
use authkestra::core::{
    auth::{SessionStore, strategy::utils::extract_cookie},
    error::AuthError,
    strategy::{AuthenticationStrategy, TokenStrategy, TokenValidator},
};
use authkestra::resource::Guard;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::{
    error::{Error, Result},
    sessions::PgSessionStore,
    state::{AppState, Principal},
};

/// The browser session cookie set by the OIDC callback.
pub const SESSION_COOKIE: &str = "ct_session";
/// The console sends this header on every API call. The session cookie only counts together with it: a
/// cross-site page cannot add a custom header without a CORS preflight, which this server never grants.
pub const CONSOLE_HEADER: &str = "x-corntchap-console";

pub const TOKEN_PREFIX: &str = "ct_";

pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

/// A fresh opaque secret: `ct_` + 32 random bytes in URL-safe base64.
pub fn generate_token() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let bytes: [u8; 32] = rand::random();
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn random_hex(n: usize) -> String {
    let mut out = String::new();
    while out.len() < n * 2 {
        let b: [u8; 16] = rand::random();
        out.push_str(&hex::encode(b));
    }
    out.truncate(n * 2);
    out
}

#[derive(sqlx::FromRow)]
struct PrincipalRow {
    id: uuid::Uuid,
    kind: String,
    name: String,
    owner_id: Option<uuid::Uuid>,
    is_admin: bool,
    max_leases: i64,
    runtime: Option<String>,
    agent_kind: Option<String>,
    grants: Vec<String>,
    token_id: uuid::Uuid,
    stale: bool,
}

pub struct DbTokenValidator {
    db: PgPool,
}

#[async_trait]
impl TokenValidator for DbTokenValidator {
    type Identity = Principal;

    async fn validate(&self, token: &str) -> std::result::Result<Option<Principal>, AuthError> {
        let hash = sha256(token.as_bytes());
        let row: Option<PrincipalRow> = sqlx::query_as(
            "select p.id, p.kind, p.name, p.owner_id, p.is_admin, p.max_leases, p.runtime, p.agent_kind, p.grants,
                    t.id as token_id, (t.last_used_at is null or t.last_used_at < now() - interval '1 minute') as stale
             from tokens t join principals p on p.id = t.principal_id
             where t.token_hash = $1 and t.revoked_at is null and (t.expires_at is null or t.expires_at > now())
               and p.disabled_at is null",
        )
        .bind(hash)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| AuthError::Internal(e.to_string()))?;
        let Some(r) = row else { return Ok(None) };
        if r.stale {
            let _ = sqlx::query("update tokens set last_used_at = now() where id = $1")
                .bind(r.token_id)
                .execute(&self.db)
                .await;
        }
        Ok(Some(Principal {
            id: r.id,
            kind: r.kind,
            name: r.name,
            owner_id: r.owner_id,
            is_admin: r.is_admin,
            max_leases: r.max_leases,
            runtime: r.runtime,
            agent_kind: r.agent_kind,
            token_id: Some(r.token_id),
            grants: r.grants,
        }))
    }
}

/// Cookie-session authentication for the console (`/api/v1` and `/api/v2` see the same `Principal` as with a bearer
/// token). It applies only when the request carries [`CONSOLE_HEADER`]; for unsafe methods the `Origin` header must
/// also equal the origin of `PUBLIC_URL` (CSRF). Anything else yields "no credentials" (401), never a guess.
pub struct SessionCookieStrategy {
    store: PgSessionStore,
    origin: String,
}

/// `https://host:port` of a base URL (no path), for comparing with an `Origin` header.
pub fn origin_of(url: &str) -> String {
    url.split('/').take(3).collect::<Vec<_>>().join("/")
}

pub fn console_request_allowed(
    method: &http::Method,
    headers: &http::HeaderMap,
    origin: &str,
) -> bool {
    if headers
        .get(CONSOLE_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.trim().is_empty())
    {
        return false;
    }
    let safe = matches!(
        *method,
        http::Method::GET | http::Method::HEAD | http::Method::OPTIONS
    );
    safe || headers
        .get(http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o.trim_end_matches('/') == origin)
}

#[async_trait]
impl AuthenticationStrategy<Principal> for SessionCookieStrategy {
    async fn authenticate(
        &self,
        parts: &http::request::Parts,
    ) -> std::result::Result<Option<Principal>, AuthError> {
        if !console_request_allowed(&parts.method, &parts.headers, &self.origin) {
            return Ok(None);
        }
        let Some(cookie) = extract_cookie(&parts.headers, SESSION_COOKIE) else {
            return Ok(None);
        };
        let Some(session) = self.store.load_session(cookie).await? else {
            return Ok(None);
        };
        let Ok(uid) = session.identity.external_id.parse::<uuid::Uuid>() else {
            return Ok(None);
        };
        let row: Option<(uuid::Uuid, String, String, Option<uuid::Uuid>, bool, i64)> =
            sqlx::query_as(
                "select id, kind, name, owner_id, is_admin, max_leases from principals
                 where id = $1 and kind = 'user' and disabled_at is null",
            )
            .bind(uid)
            .fetch_optional(&self.store.db)
            .await
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(row.map(
            |(id, kind, name, owner_id, is_admin, max_leases)| Principal {
                id,
                kind,
                name,
                owner_id,
                is_admin,
                max_leases,
                runtime: None,
                agent_kind: None,
                token_id: None,
                grants: vec![],
            },
        ))
    }
}

pub fn build_guard(db: PgPool, public_url: &str) -> Arc<Guard<Principal>> {
    Arc::new(
        Guard::builder()
            .strategy(TokenStrategy::new(DbTokenValidator { db: db.clone() }))
            .strategy(SessionCookieStrategy {
                store: PgSessionStore { db },
                origin: origin_of(public_url),
            })
            .build(),
    )
}

/// Authenticate from a bare header map with a bearer token only (the MCP endpoint: cookies never count there).
pub async fn authenticate_headers(
    state: &AppState,
    headers: &http::HeaderMap,
) -> Result<Principal> {
    let mut h = headers.clone();
    h.remove(http::header::COOKIE);
    authenticate_request(state, &http::Method::POST, &h).await
}

/// Authenticate a request: bearer token, or (for the console) the session cookie, see [`SessionCookieStrategy`].
pub async fn authenticate_request(
    state: &AppState,
    method: &http::Method,
    headers: &http::HeaderMap,
) -> Result<Principal> {
    check_transport_headers(state, headers)?;
    let mut req = http::Request::new(());
    *req.method_mut() = method.clone();
    *req.headers_mut() = headers.clone();
    let (parts, _) = req.into_parts();
    match state.guard.authenticate(&parts).await {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(Error::Unauthorized),
        Err(AuthError::Internal(m)) => Err(Error::Internal(m)),
        Err(_) => Err(Error::Unauthorized),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let h = host.trim();
    let h = if let Some(stripped) = h.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        h.split(':').next().unwrap_or("")
    };
    matches!(h, "localhost" | "127.0.0.1" | "::1")
}

pub fn check_transport_headers(state: &AppState, headers: &http::HeaderMap) -> Result<()> {
    if !state.config.require_https {
        return Ok(());
    }
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if is_loopback_host(host) {
        return Ok(());
    }
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if proto.eq_ignore_ascii_case("https") {
        Ok(())
    } else {
        Err(Error::Insecure(
            "plain HTTP is refused: bearer tokens must travel over https".into(),
        ))
    }
}

/// Middleware: refuse plain HTTP (except loopback) when the instance is configured for https.
pub async fn require_tls(State(state): State<AppState>, req: Request, next: Next) -> Response {
    use axum::response::IntoResponse;
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }
    if let Err(e) = check_transport_headers(&state, req.headers()) {
        return e.into_response();
    }
    next.run(req).await
}
