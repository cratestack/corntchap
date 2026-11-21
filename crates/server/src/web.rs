//! Browser-facing and device-facing endpoints: GitHub SSO (authkestra OAuth2 flow),
//! sessions, the RFC 8628 device authorization grant, and the read-only board.

use std::{collections::HashMap, time::Duration};

use authkestra::core::{OAuth2Flow, state::OAuth2State};
use authkestra::oidc::OidcProvider;
use axum::{
    Json, Router,
    extract::{Form, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    admin,
    auth::{generate_token, random_hex, sha256},
    error::{Error, Result},
    service,
    state::{AppState, Principal},
};

const SESSION_COOKIE: &str = "ct_session";
const SESSION_TTL_SECS: i64 = 12 * 3600;
const DEVICE_TTL_SECS: i64 = 600;
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/oidc/login", get(oidc_login))
        .route("/auth/oidc/callback", get(oidc_callback))
        .route("/logout", get(logout))
        .route("/oauth/device/code", post(device_code))
        .route("/oauth/token", post(device_token))
        .route("/device", get(device_page).post(device_decision))
        .route("/board", get(board_index))
        .route("/board/{key}", get(board_project))
}

// ---------- helpers ----------

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: &str, refresh: bool) -> Html<String> {
    Html(format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">{}<title>{}</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem;color:#1b1b1b}}\
         code,.code{{font:1.1rem ui-monospace,monospace;background:#f1f1ef;padding:.1rem .4rem;border-radius:4px}}\
         .code{{font-size:2rem;letter-spacing:.15em;display:inline-block;padding:.4rem 1rem}}\
         button{{font:inherit;padding:.5rem 1.2rem;margin-right:.5rem;cursor:pointer}}\
         li.done{{color:#777}}ul{{list-style:none;padding-left:1.2rem}}h2{{margin-top:2rem}}small{{color:#666}}</style></head><body>{}</body></html>",
        if refresh {
            "<meta http-equiv=refresh content=15>"
        } else {
            ""
        },
        esc(title),
        body
    ))
}

fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.contains('\\')
                && !n.chars().any(|c| c.is_control()) =>
        {
            n.to_string()
        }
        _ => "/".to_string(),
    }
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let h = headers.get(header::COOKIE)?.to_str().ok()?;
    h.split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

pub struct Session {
    pub user: Principal,
    pub csrf: String,
}

pub async fn session_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Session>> {
    let Some(tok) = cookie_value(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let row: Option<(Uuid, String, String, Option<Uuid>, bool, i32, String)> = sqlx::query_as(
        "select p.id, p.kind, p.name, p.owner_id, p.is_admin, p.max_leases, s.csrf
         from web_sessions s join principals p on p.id = s.user_id
         where s.id_hash = $1 and s.expires_at > now() and p.disabled_at is null",
    )
    .bind(sha256(tok.as_bytes()))
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(
        |(id, kind, name, owner_id, is_admin, max_leases, csrf)| Session {
            user: Principal {
                id,
                kind,
                name,
                owner_id,
                is_admin,
                max_leases,
                runtime: None,
                agent_kind: None,
            },
            csrf,
        },
    ))
}

/// Session cookie, or a bearer token (so `curl -H "Authorization: Bearer …" /board/PAY` works too).
async fn web_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<(Principal, Option<String>)>> {
    if let Some(s) = session_from_headers(state, headers).await? {
        return Ok(Some((s.user, Some(s.csrf))));
    }
    if headers.contains_key(header::AUTHORIZATION) {
        let p = crate::auth::authenticate_headers(state, headers).await?;
        return Ok(Some((p, None)));
    }
    Ok(None)
}

fn login_redirect(next: &str) -> Response {
    Redirect::to(&format!("/auth/oidc/login?next={}", urlencode(next))).into_response()
}

fn urlencode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

// ---------- OIDC SSO (Keycloak or any OpenID provider) ----------

async fn oidc_flow(state: &AppState) -> Result<OAuth2Flow<OidcProvider>> {
    let cfg = state.config.oidc.as_ref().ok_or_else(|| {
        Error::Invalid(
            "sign-in is not configured (set OIDC_ISSUER, OIDC_CLIENT_ID and OIDC_CLIENT_SECRET)"
                .into(),
        )
    })?;
    let provider = state
        .oidc
        .get_or_try_init(|| async {
            OidcProvider::discover(
                cfg.client_id.clone(),
                cfg.client_secret.clone(),
                format!("{}/auth/oidc/callback", state.config.public_url),
                &cfg.issuer,
                Duration::from_secs(3600),
            )
            .await
            .map_err(|e| {
                Error::Internal(format!("OIDC discovery against {} failed: {e}", cfg.issuer))
            })
        })
        .await?;
    Ok(OAuth2Flow::new(provider.clone()))
}

/// A claim from the (already signature-checked) ID token's payload.
fn id_token_claim(id_token: &str, key: &str) -> Option<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = id_token.split('.').nth(1)?;
    let raw = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    v.get(key)?.as_str().map(String::from)
}

async fn oidc_login(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let flow = oidc_flow(&state).await?;
    let (url, st) = flow.initiate_login(&[], None);
    let next = safe_next(q.get("next").map(|s| s.as_str()));
    // Bind the attempt to this browser: the callback must present the same nonce cookie (login CSRF).
    let nonce = random_hex(16);
    let mut data = serde_json::to_value(&st).map_err(|e| Error::Internal(e.to_string()))?;
    data["ct_nonce_hash"] = json!(hex::encode(sha256(nonce.as_bytes())));
    sqlx::query("delete from oauth_states where expires_at < now()")
        .execute(&state.db)
        .await?;
    sqlx::query("insert into oauth_states (state, data, next, expires_at) values ($1,$2,$3, now() + interval '10 minutes')")
        .bind(&st.state)
        .bind(data)
        .bind(next)
        .execute(&state.db)
        .await?;
    let secure = if state.config.require_https {
        "; Secure"
    } else {
        ""
    };
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("ct_oauth={nonce}; HttpOnly; SameSite=Lax; Path=/auth; Max-Age=600{secure}")
            .parse()
            .unwrap(),
    );
    Ok(resp)
}

async fn oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    if let Some(err) = q.get("error") {
        return Ok((
            StatusCode::FORBIDDEN,
            page(
                "Sign-in refused",
                &format!("<p>GitHub said: {}</p>", esc(err)),
                false,
            ),
        )
            .into_response());
    }
    let (Some(code), Some(received)) = (q.get("code"), q.get("state")) else {
        return Err(Error::Invalid("missing code or state".into()));
    };
    let row: Option<(Value, String)> = sqlx::query_as(
        "delete from oauth_states where state = $1 and expires_at > now() returning data, next",
    )
    .bind(received)
    .fetch_optional(&state.db)
    .await?;
    let Some((data, next)) = row else {
        return Err(Error::Invalid(
            "unknown or expired sign-in attempt; start again".into(),
        ));
    };
    let want = data["ct_nonce_hash"].as_str().unwrap_or("").to_string();
    let have = cookie_value(&headers, "ct_oauth")
        .map(|n| hex::encode(sha256(n.as_bytes())))
        .unwrap_or_default();
    if want.is_empty() || want != have {
        return Err(Error::Forbidden(
            "this sign-in was not started in this browser; start again".into(),
        ));
    }
    let expected: OAuth2State =
        serde_json::from_value(data).map_err(|e| Error::Internal(e.to_string()))?;
    let flow = oidc_flow(&state).await?;
    let (identity, token, _) = flow
        .finalize_login(code, received, &expected)
        .await
        .map_err(|e| Error::Forbidden(format!("sign-in failed: {e}")))?;
    let login = token
        .id_token
        .as_deref()
        .and_then(|t| id_token_claim(t, "preferred_username"))
        .or_else(|| identity.email.clone())
        .unwrap_or_default()
        .to_lowercase();
    let gh_id = identity.external_id.clone();

    // Match by stable subject first, then link a pre-provisioned login on first sign-in.
    let user: Option<Uuid> = sqlx::query_scalar(
        "update principals set oidc_sub = $1, oidc_login = $2
         where kind = 'user' and disabled_at is null and (oidc_sub = $1 or (oidc_sub is null and oidc_login = $2))
         returning id",
    )
    .bind(&gh_id)
    .bind(&login)
    .fetch_optional(&state.db)
    .await?;
    let Some(user) = user else {
        return Ok((
            StatusCode::FORBIDDEN,
            page(
                "Not provisioned",
                &format!("<h1>Not provisioned</h1><p>Account <code>{}</code> has no user on this instance. Ask an admin to add it.</p>", esc(&login)),
                false,
            ),
        )
            .into_response());
    };
    let session = generate_token();
    sqlx::query("insert into web_sessions (id_hash, user_id, csrf, expires_at) values ($1,$2,$3, now() + make_interval(secs => $4::float8))")
        .bind(sha256(session.as_bytes()))
        .bind(user)
        .bind(random_hex(16))
        .bind(SESSION_TTL_SECS as f64)
        .execute(&state.db)
        .await?;
    let secure = if state.config.require_https {
        "; Secure"
    } else {
        ""
    };
    let mut resp = Redirect::to(&next).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("{SESSION_COOKIE}={session}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_TTL_SECS}{secure}")
            .parse()
            .unwrap(),
    );
    Ok(resp)
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    if let Some(tok) = cookie_value(&headers, SESSION_COOKIE) {
        sqlx::query("delete from web_sessions where id_hash = $1")
            .bind(sha256(tok.as_bytes()))
            .execute(&state.db)
            .await?;
    }
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("{SESSION_COOKIE}=; Max-Age=0; Path=/")
            .parse()
            .unwrap(),
    );
    Ok(resp)
}

// ---------- RFC 8628 device authorization grant ----------

#[derive(Deserialize, Default)]
struct DeviceCodeReq {
    kind: Option<String>,
    name: Option<String>,
    agent_kind: Option<String>,
    runtime: Option<String>,
}

fn user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";
    let b: [u8; 8] = rand::random();
    let s: String = b
        .iter()
        .map(|x| ALPHABET[*x as usize % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &s[..4], &s[4..])
}

/// Accept either a JSON or a form body (RFC 8628 specifies form encoding).
async fn parse_body<T: serde::de::DeserializeOwned + Default>(
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<T> {
    if body.is_empty() {
        return Ok(T::default());
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.starts_with("application/json") {
        serde_json::from_slice(&body).map_err(|e| Error::Invalid(e.to_string()))
    } else {
        serde_urlencoded_from(&body)
    }
}

fn serde_urlencoded_from<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T> {
    serde_urlencoded::from_bytes(body).map_err(|e| Error::Invalid(e.to_string()))
}

async fn device_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>> {
    let req: DeviceCodeReq = parse_body(&headers, body).await?;
    let kind = req.kind.unwrap_or_else(|| "user".into());
    if !["user", "agent", "relay"].contains(&kind.as_str()) {
        return Err(Error::Invalid("kind must be user, agent or relay".into()));
    }
    let name = match (kind.as_str(), req.name.as_deref()) {
        ("user", _) => None,
        (_, Some(n)) if !n.is_empty() => Some(admin::norm_name(n)?),
        _ => {
            return Err(Error::Invalid(
                "name is required to enroll an agent or relay".into(),
            ));
        }
    };
    sqlx::query("delete from device_requests where expires_at < now() - interval '1 day'")
        .execute(&state.db)
        .await?;
    let device_code = generate_token();
    let code = user_code();
    sqlx::query(
        "insert into device_requests (device_hash, user_code, kind, name, runtime, agent_kind, expires_at)
         values ($1,$2,$3,$4,$5,$6, now() + make_interval(secs => $7::float8))",
    )
    .bind(sha256(device_code.as_bytes()))
    .bind(&code)
    .bind(&kind)
    .bind(name)
    .bind(req.runtime.filter(|r| ["cloud", "local"].contains(&r.as_str())))
    .bind(req.agent_kind.filter(|k| ["claude-code", "codex", "opencode", "other"].contains(&k.as_str())))
    .bind(DEVICE_TTL_SECS as f64)
    .execute(&state.db)
    .await?;
    let base = &state.config.public_url;
    Ok(Json(json!({
        "device_code": device_code,
        "user_code": code,
        "verification_uri": format!("{base}/device"),
        "verification_uri_complete": format!("{base}/device?user_code={code}"),
        "expires_in": DEVICE_TTL_SECS,
        "interval": 5,
    })))
}

#[derive(sqlx::FromRow)]
struct DeviceRow {
    id: Uuid,
    kind: String,
    name: Option<String>,
    runtime: Option<String>,
    agent_kind: Option<String>,
    status: String,
    approved_by: Option<Uuid>,
    interval_s: i32,
    expired: bool,
    too_fast: bool,
}

fn oauth_err(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

#[derive(Deserialize, Default)]
struct TokenReq {
    grant_type: Option<String>,
    device_code: Option<String>,
}

async fn device_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response> {
    let req: TokenReq = parse_body(&headers, body).await?;
    if req.grant_type.as_deref() != Some(DEVICE_GRANT) {
        return Ok(oauth_err(StatusCode::BAD_REQUEST, "unsupported_grant_type"));
    }
    let Some(dc) = req.device_code else {
        return Ok(oauth_err(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let hash = sha256(dc.as_bytes());
    let mut tx = state.db.begin().await?;
    let row: Option<DeviceRow> = sqlx::query_as(
        "select id, kind, name, runtime, agent_kind, status, approved_by, interval_s, expires_at < now() as expired,
                coalesce(last_polled_at > now() - make_interval(secs => interval_s::float8), false) as too_fast
         from device_requests where device_hash = $1 for update",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(r) = row else {
        return Ok(oauth_err(StatusCode::BAD_REQUEST, "invalid_grant"));
    };
    if r.expired {
        return Ok(oauth_err(StatusCode::BAD_REQUEST, "expired_token"));
    }
    sqlx::query("update device_requests set last_polled_at = now() where id = $1")
        .bind(r.id)
        .execute(&mut *tx)
        .await?;
    let resp = match r.status.as_str() {
        "pending" if r.too_fast => {
            sqlx::query("update device_requests set interval_s = interval_s + 5 where id = $1")
                .bind(r.id)
                .execute(&mut *tx)
                .await?;
            oauth_err(StatusCode::BAD_REQUEST, "slow_down")
        }
        "pending" => oauth_err(StatusCode::BAD_REQUEST, "authorization_pending"),
        "denied" => oauth_err(StatusCode::BAD_REQUEST, "access_denied"),
        "consumed" => oauth_err(StatusCode::BAD_REQUEST, "invalid_grant"),
        "approved" => {
            let approver = r
                .approved_by
                .ok_or_else(|| Error::Internal("approved without approver".into()))?;
            let target: Uuid = if r.kind == "user" {
                approver
            } else {
                sqlx::query_scalar("select id from principals where name = $1 and kind = $2")
                    .bind(r.name.as_deref().unwrap_or(""))
                    .bind(&r.kind)
                    .fetch_one(&mut *tx)
                    .await?
            };
            // Agents and relays hold a long-lived credential; a user's CLI login lasts 90 days.
            let ttl = if r.kind == "user" {
                Some(90 * 86_400)
            } else {
                None
            };
            let token = admin::mint_token(&mut tx, target, "device", ttl).await?;
            sqlx::query("update device_requests set status = 'consumed' where id = $1")
                .bind(r.id)
                .execute(&mut *tx)
                .await?;
            let (name, kind): (String, String) =
                sqlx::query_as("select name, kind from principals where id = $1")
                    .bind(target)
                    .fetch_one(&mut *tx)
                    .await?;
            let mut r = Json(json!({"access_token": token, "token_type": "Bearer", "principal": {"name": name, "kind": kind}})).into_response();
            r.headers_mut()
                .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            r
        }
        _ => oauth_err(StatusCode::BAD_REQUEST, "invalid_grant"),
    };
    let _ = (r.interval_s, r.runtime, r.agent_kind);
    tx.commit().await?;
    Ok(resp)
}

async fn device_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let code = q
        .get("user_code")
        .map(|s| s.trim().to_uppercase())
        .unwrap_or_default();
    let Some(session) = session_from_headers(&state, &headers).await? else {
        let next = if code.is_empty() {
            "/device".to_string()
        } else {
            format!("/device?user_code={}", urlencode(&code))
        };
        return Ok(login_redirect(&next));
    };
    if code.is_empty() {
        return Ok(page(
            "Connect a device",
            "<h1>Connect a device</h1><form method=get action=/device><p>Enter the code shown in your terminal:</p>\
             <p><input name=user_code autofocus placeholder=\"ABCD-EFGH\" style=\"font:1.4rem ui-monospace,monospace\"></p><button>Continue</button></form>",
            false,
        )
        .into_response());
    }
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "select kind, name, runtime, agent_kind from device_requests where user_code = $1 and status = 'pending' and expires_at > now()",
    )
    .bind(&code)
    .fetch_optional(&state.db)
    .await?;
    let Some((kind, name, runtime, agent_kind)) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            page(
                "Unknown code",
                "<h1>Unknown or expired code</h1><p>Start again from your terminal.</p>",
                false,
            ),
        )
            .into_response());
    };
    let existing: Option<(String,)> = match (kind.as_str(), name.as_deref()) {
        ("user", _) | (_, None) => None,
        (k, Some(n)) => sqlx::query_as("select coalesce((select name from principals where id = p.owner_id), '?') from principals p where p.name = $1 and p.kind = $2")
            .bind(n)
            .bind(k)
            .fetch_optional(&state.db)
            .await?,
    };
    let warn = existing
        .map(|(o,)| format!("<p><b>Warning:</b> this name already exists (owner: {}). Approving <b>replaces its credentials</b>; the old token stops working.</p>", esc(&o)))
        .unwrap_or_default();
    let what = match kind.as_str() {
        "user" => format!(
            "sign in <b>the CLI</b> as <b>{}</b>",
            esc(&session.user.name)
        ),
        "agent" => format!(
            "enroll the <b>{}</b> agent <code>{}</code> ({}) owned by <b>{}</b>",
            esc(agent_kind.as_deref().unwrap_or("other")),
            esc(name.as_deref().unwrap_or("")),
            esc(runtime.as_deref().unwrap_or("local")),
            esc(&session.user.name)
        ),
        _ => format!(
            "enroll the relay <code>{}</code> owned by <b>{}</b>",
            esc(name.as_deref().unwrap_or("")),
            esc(&session.user.name)
        ),
    };
    Ok(page(
        "Approve device",
        &format!(
            "<h1>Approve this device?</h1><p>Code <span class=code>{c}</span></p><p>This will {what}.</p>{warn}\
             <form method=post action=/device><input type=hidden name=user_code value=\"{c}\"><input type=hidden name=csrf value=\"{csrf}\">\
             <button name=action value=approve>Approve</button><button name=action value=deny>Deny</button></form>\
             <p><small>Only approve if you started this from your own terminal. Signed in as {u}.</small></p>",
            c = esc(&code),
            warn = warn,
            csrf = esc(&session.csrf),
            u = esc(&session.user.name)
        ),
        false,
    )
    .into_response())
}

#[derive(Deserialize)]
struct DeviceDecision {
    user_code: String,
    csrf: String,
    action: String,
}

async fn device_decision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<DeviceDecision>,
) -> Result<Response> {
    let Some(session) = session_from_headers(&state, &headers).await? else {
        return Ok(login_redirect("/device"));
    };
    if f.csrf != session.csrf {
        return Err(Error::Forbidden("bad CSRF token".into()));
    }
    let code = f.user_code.trim().to_uppercase();
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, String, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "select id, kind, name, runtime, agent_kind from device_requests
         where user_code = $1 and status = 'pending' and expires_at > now() for update",
        )
        .bind(&code)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((id, kind, name, runtime, agent_kind)) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            page("Unknown code", "<h1>Unknown or expired code</h1>", false),
        )
            .into_response());
    };
    if f.action != "approve" {
        sqlx::query("update device_requests set status = 'denied' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(page(
            "Denied",
            "<h1>Denied</h1><p>The device was not connected.</p>",
            false,
        )
        .into_response());
    }
    if kind != "user" {
        let name = name.unwrap_or_default();
        let existing: Option<(Option<Uuid>,)> =
            sqlx::query_as("select owner_id from principals where name = $1 and kind = $2")
                .bind(&name)
                .bind(&kind)
                .fetch_optional(&mut *tx)
                .await?;
        match existing {
            Some((owner,)) if owner == Some(session.user.id) || session.user.is_admin => {
                sqlx::query("update tokens set revoked_at = now() where principal_id = (select id from principals where name = $1) and revoked_at is null")
                    .bind(&name)
                    .execute(&mut *tx)
                    .await?; // re-enrolling replaces the previous credential
            }
            Some(_) => {
                return Err(Error::Conflict(format!(
                    "{name} already exists and belongs to someone else"
                )));
            }
            None => {
                let pid = Uuid::new_v4();
                let (ak, rt) = if kind == "agent" {
                    (
                        Some(agent_kind.unwrap_or_else(|| "other".into())),
                        Some(runtime.unwrap_or_else(|| "local".into())),
                    )
                } else {
                    (None, None)
                };
                sqlx::query("insert into principals (id, kind, name, owner_id, agent_kind, runtime) values ($1,$2,$3,$4,$5,$6)")
                    .bind(pid)
                    .bind(&kind)
                    .bind(&name)
                    .bind(session.user.id)
                    .bind(ak)
                    .bind(rt)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    sqlx::query("update device_requests set status = 'approved', approved_by = $2 where id = $1")
        .bind(id)
        .bind(session.user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(page(
        "Approved",
        "<h1>Approved</h1><p>You can return to your terminal.</p>",
        false,
    )
    .into_response())
}

// ---------- board ----------

async fn board_index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let Some((p, _)) = web_principal(&state, &headers).await? else {
        return Ok(login_redirect("/board"));
    };
    let projects = service::list_projects(&state.db, &p).await?;
    let mut body = format!(
        "<h1>Boards</h1><p><small>Signed in as {}</small></p><ul>",
        esc(&p.name)
    );
    for pr in projects.as_array().into_iter().flatten() {
        let key = pr["key"].as_str().unwrap_or("");
        body.push_str(&format!(
            "<li><a href=\"/board/{k}\"><b>{k}</b></a> {n} — {r} ready, {c} in progress, {d} done</li>",
            k = esc(key),
            n = esc(pr["name"].as_str().unwrap_or("")),
            r = pr["ready"],
            c = pr["claimed"],
            d = pr["done"]
        ));
    }
    body.push_str("</ul>");
    Ok(page("Boards", &body, false).into_response())
}

async fn board_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Response> {
    let Some((p, _)) = web_principal(&state, &headers).await? else {
        return Ok(login_redirect(&format!("/board/{key}")));
    };
    let (title, tasks) = service::board_data(&state.db, &p, &key.to_uppercase()).await?;
    let mut body = format!(
        "<p><a href=/board>&larr; boards</a></p><h1>{}</h1>",
        esc(&title)
    );
    for (heading, status) in [
        ("In progress", "claimed"),
        ("Ready / open", "open"),
        ("Blocked", "blocked"),
        ("Done", "done"),
        ("Cancelled", "cancelled"),
    ] {
        let group: Vec<&Value> = tasks.iter().filter(|t| t["status"] == status).collect();
        if group.is_empty() {
            continue;
        }
        body.push_str(&format!("<h2>{heading}</h2><ul>"));
        for t in group {
            let mark = if status == "done" { "☑" } else { "☐" };
            let by = t["claimed_by"]
                .as_str()
                .map(|b| format!(" <small>· {}</small>", esc(b)))
                .unwrap_or_default();
            body.push_str(&format!(
                "<li>{mark} <b>{}</b> {}{by}<ul>",
                esc(t["id"].as_str().unwrap_or("")),
                esc(t["title"].as_str().unwrap_or(""))
            ));
            for i in t["items"].as_array().into_iter().flatten() {
                let done = i["done"].as_bool().unwrap_or(false);
                body.push_str(&format!(
                    "<li class=\"{}\">{} {}</li>",
                    if done { "done" } else { "" },
                    if done { "☑" } else { "☐" },
                    esc(i["text"].as_str().unwrap_or(""))
                ));
            }
            body.push_str("</ul></li>");
        }
        body.push_str("</ul>");
    }
    Ok(page(&title, &body, true).into_response())
}
