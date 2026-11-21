//! Browser-facing and device-facing endpoints: OIDC SSO (authkestra OAuth2/OIDC flow),
//! sessions, the RFC 8628 device authorization grant. The old server-rendered board now redirects to the console.

use std::{collections::HashMap, time::Duration};

use authkestra::core::{
    OAuth2Flow,
    auth::{SessionStore, session::resolve_redirect_target, strategy::utils::extract_cookie},
    pkce::Pkce,
    state::OAuth2State,
};
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
    auth::{CONSOLE_HEADER, SESSION_COOKIE, generate_token, random_hex, sha256},
    error::{Error, Result},
    pages::{self, Left, esc},
    sessions::{self, PgSessionStore},
    state::{AppState, Principal},
};

/// Sign-in attempt state: an authkestra `OAuth2State`, AES-GCM encrypted into this cookie.
const STATE_COOKIE: &str = "ak_state";
const SESSION_TTL_SECS: i64 = 12 * 3600;
const DEVICE_TTL_SECS: i64 = 600;
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/oidc/login", get(oidc_login))
        .route("/auth/signin", get(signin_page))
        .route("/auth/session", get(session_info))
        .route("/static/fonts/{file}", get(font_file))
        .route("/auth/oidc/callback", get(oidc_callback))
        .route("/logout", get(logout_page).post(logout))
        .route("/oauth/device/code", post(device_code))
        .route("/oauth/token", post(device_token))
        .route("/device", get(device_page).post(device_decision))
        .route("/board", get(|| async { Redirect::to("/") }))
        .route("/board/{key}", get(board_redirect))
}

// ---------- helpers ----------

fn page(title: &str, body: &str) -> Html<String> {
    pages::simple(title, body)
}

fn safe_next(next: Option<&str>) -> String {
    resolve_redirect_target(next, "/")
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    extract_cookie(headers, name)
}

/// The key that seals the sign-in state cookie: stable across restarts and replicas, derived
/// from the OIDC client secret.
fn state_key(state: &AppState) -> [u8; 32] {
    let secret = state
        .config
        .oidc
        .as_ref()
        .map(|c| c.client_secret.as_str())
        .unwrap_or("");
    let mut key = [0u8; 32];
    key.copy_from_slice(&sha256(
        format!("corntchap/oauth-state/{secret}").as_bytes(),
    ));
    key
}

fn store(state: &AppState) -> PgSessionStore {
    PgSessionStore {
        db: state.db.clone(),
    }
}

pub struct Session {
    pub user: Principal,
    pub csrf: String,
    /// The IdP's id_token, used as `id_token_hint` at logout.
    pub id_token: Option<String>,
}

pub async fn session_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Session>> {
    let Some(tok) = cookie_value(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let Some(sess) = store(state)
        .load_session(tok)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
    else {
        return Ok(None);
    };
    let Ok(uid) = sess.identity.external_id.parse::<Uuid>() else {
        return Ok(None);
    };
    let row: Option<(Uuid, String, String, Option<Uuid>, bool, i64)> = sqlx::query_as(
        "select id, kind, name, owner_id, is_admin, max_leases from principals
         where id = $1 and disabled_at is null",
    )
    .bind(uid)
    .fetch_optional(&state.db)
    .await?;
    Ok(
        row.map(|(id, kind, name, owner_id, is_admin, max_leases)| Session {
            user: Principal {
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
            csrf: sess
                .identity
                .attributes
                .get(sessions::ATTR_CSRF)
                .cloned()
                .unwrap_or_default(),
            id_token: sess
                .identity
                .attributes
                .get(sessions::ATTR_ID_TOKEN)
                .cloned(),
        }),
    )
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

/// A claim (any JSON type) from the (already signature-checked) ID token's payload.
fn id_token_claim(id_token: &str, key: &str) -> Option<Value> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = id_token.split('.').nth(1)?;
    let raw = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    v.get(key).cloned()
}

fn id_token_str(id_token: &str, key: &str) -> Option<String> {
    id_token_claim(id_token, key)?.as_str().map(String::from)
}

/// True if the ID token carries `role` in Keycloak's `realm_access.roles` or a top-level
/// `roles` / `groups` array (group paths like `/corntchap-admin` match too).
fn has_admin_role(id_token: &str, role: &str) -> bool {
    let matches = |v: &Value| {
        v.as_array().is_some_and(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .any(|x| x == role || x.trim_start_matches('/') == role)
        })
    };
    id_token_claim(id_token, "realm_access")
        .and_then(|ra| ra.get("roles").cloned())
        .is_some_and(|r| matches(&r))
        || ["roles", "groups"]
            .iter()
            .any(|k| id_token_claim(id_token, k).is_some_and(|v| matches(&v)))
}

fn csrf_ok(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn error_page(status: StatusCode, title: &str, msg: &str) -> Response {
    (
        status,
        page(title, &format!("<h1>{}</h1><p>{msg}</p>", esc(title))),
    )
        .into_response()
}

/// Drop expired sign-in attempts, sessions and long-dead device requests.
async fn purge_expired(db: &sqlx::PgPool) {
    for q in [
        "delete from auth_sessions where expires_at < now()",
        "delete from device_requests where expires_at < now() - interval '1 day'",
    ] {
        if let Err(e) = sqlx::query(q).execute(db).await {
            tracing::warn!("purge failed: {e}");
        }
    }
}

async fn oidc_login(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let flow = oidc_flow(&state).await?;
    let pkce = Pkce::new();
    let (url, mut st) = flow.initiate_login(&[], Some(&pkce.code_challenge));
    st.code_verifier = Some(pkce.code_verifier);
    st.success_url = Some(safe_next(q.get("next").map(|s| s.as_str())));
    // The attempt lives in an encrypted cookie, so it is bound to this browser (login CSRF)
    // and needs no server-side state.
    let sealed = st
        .encrypt(&state_key(&state))
        .map_err(|e| Error::Internal(e.to_string()))?;
    purge_expired(&state.db).await;
    let secure = if state.config.require_https {
        "; Secure"
    } else {
        ""
    };
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        format!("{STATE_COOKIE}={sealed}; HttpOnly; SameSite=Lax; Path=/auth; Max-Age=600{secure}")
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
                &format!(
                    "<h1>Sign-in refused</h1><p>The identity provider said: <code>{}</code></p><p><a class=\"btn primary\" href=\"/auth/oidc/login\">Try again</a></p>",
                    esc(err)
                ),
            ),
        )
            .into_response());
    }
    let (Some(code), Some(received)) = (q.get("code"), q.get("state")) else {
        return Err(Error::Invalid("missing code or state".into()));
    };
    let not_here =
        || Error::Forbidden("this sign-in was not started in this browser; start again".into());
    let sealed = cookie_value(&headers, STATE_COOKIE).ok_or_else(not_here)?;
    let expected = OAuth2State::decrypt(sealed, &state_key(&state)).map_err(|_| not_here())?;
    let next = safe_next(expected.success_url.as_deref());
    let flow = oidc_flow(&state).await?;
    let (identity, token, _) = flow
        .finalize_login(code, received, &expected)
        .await
        .map_err(|e| Error::Forbidden(format!("sign-in failed: {e}")))?;
    let id_token = token.id_token.clone().unwrap_or_default();
    let cfg = state.config.oidc.as_ref().expect("oidc configured");
    let sub = identity.external_id.clone();
    // The account name: preferred_username, else the local part of a *verified* e-mail address.
    let verified = id_token_claim(&id_token, "email_verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let login = id_token_str(&id_token, "preferred_username")
        .and_then(|n| admin::norm_name(&n).ok())
        .or_else(|| {
            if !verified {
                return None;
            }
            let email = id_token_str(&id_token, "email")?;
            admin::norm_name(email.split('@').next()?).ok()
        });
    let Some(login) = login else {
        return Ok(error_page(
            StatusCode::FORBIDDEN,
            "Cannot create your account",
            "The identity provider sent no usable <code>preferred_username</code> or verified e-mail; ask an admin.",
        ));
    };
    let want_admin = has_admin_role(&id_token, &cfg.admin_role);

    // Match by stable subject first, then link a pre-provisioned login on first sign-in.
    let find = || async {
        sqlx::query_scalar::<_, Uuid>(
            "update principals set oidc_sub = $1, oidc_login = $2
             where kind = 'user' and disabled_at is null and (oidc_sub = $1 or (oidc_sub is null and oidc_login = $2))
             returning id",
        )
        .bind(&sub)
        .bind(&login)
        .fetch_optional(&state.db)
        .await
    };
    let mut user = find().await?;
    if user.is_none() {
        // Just-in-time provisioning: the IdP is the source of truth for who may sign in.
        user = sqlx::query_scalar(
            "insert into principals (id, kind, name, oidc_login, oidc_sub, is_admin, admin_source)
             values ($1,'user',$2,$2,$3,false,'oidc') on conflict do nothing returning id",
        )
        .bind(Uuid::new_v4())
        .bind(&login)
        .bind(&sub)
        .fetch_optional(&state.db)
        .await?;
        if user.is_none() {
            user = find().await?; // lost a race with a concurrent sign-in of the same user
        }
    }
    let Some(user) = user else {
        return Ok(error_page(
            StatusCode::CONFLICT,
            "Name already taken",
            &format!(
                "The name <code>{}</code> already belongs to another account on this instance (an agent, a relay, a disabled user or a user linked to a different sign-in). Ask an admin.",
                esc(&login)
            ),
        ));
    };
    // Admin flag follows the IdP role, except for admins made by `init` or by hand.
    sqlx::query(
        "update principals set
           admin_source = case when $2 and not is_admin then 'oidc' else admin_source end,
           is_admin = case when $2 then true when admin_source = 'oidc' then false else is_admin end
         where id = $1",
    )
    .bind(user)
    .bind(want_admin)
    .execute(&state.db)
    .await?;
    purge_expired(&state.db).await;
    let session = generate_token();
    store(&state)
        .save_session(&sessions::new_session(
            session.clone(),
            user,
            &login,
            random_hex(16),
            (!id_token.is_empty()).then_some(id_token),
            SESSION_TTL_SECS,
        ))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    let secure = if state.config.require_https {
        "; Secure"
    } else {
        ""
    };
    let mut resp = Redirect::to(&next).into_response();
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!("{SESSION_COOKIE}={session}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_TTL_SECS}{secure}")
            .parse()
            .unwrap(),
    );
    resp.headers_mut().append(
        header::SET_COOKIE,
        format!("{STATE_COOKIE}=; Max-Age=0; Path=/auth")
            .parse()
            .unwrap(),
    );
    Ok(resp)
}

async fn logout_page(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let Some(session) = session_from_headers(&state, &headers).await? else {
        return Ok(Redirect::to("/").into_response());
    };
    Ok(pages::render(
        "Log out",
        Left {
            headline: "Leaving the kitchen?",
            sub: "Your session ends here and at your identity provider. Cooks and relays keep running.",
            who: None,
        },
        &format!(
            "<h1>Log out?</h1><p>Signed in as <b>{}</b>.</p><form method=post action=/logout><input type=hidden name=csrf value=\"{}\"><div class=actions><button class=primary>Log out</button><a class=btn href=\"/\">Stay</a></div></form>",
            esc(&session.user.name),
            esc(&session.csrf)
        ),
    )
    .into_response())
}

#[derive(Deserialize)]
struct LogoutForm {
    csrf: String,
}

/// The IdP's end_session_endpoint (from discovery), rewritten to the public issuer if set.
async fn end_session_url(state: &AppState, id_token: Option<&str>) -> Option<String> {
    let cfg = state.config.oidc.as_ref()?;
    let doc: Value = state
        .http
        .get(format!(
            "{}/.well-known/openid-configuration",
            cfg.issuer.trim_end_matches('/')
        ))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let mut ep = doc.get("end_session_endpoint")?.as_str()?.to_string();
    if let Some(public) = &cfg.public_issuer {
        for internal in [Some(cfg.issuer.as_str()), doc["issuer"].as_str()]
            .into_iter()
            .flatten()
        {
            if let Some(rest) = ep.strip_prefix(internal.trim_end_matches('/')) {
                ep = format!("{}{rest}", public.trim_end_matches('/'));
                break;
            }
        }
    }
    let mut url = format!(
        "{ep}{}post_logout_redirect_uri={}/",
        if ep.contains('?') { '&' } else { '?' },
        urlencode(&state.config.public_url)
    );
    match id_token {
        Some(t) => url.push_str(&format!("&id_token_hint={}", urlencode(t))),
        None => url.push_str(&format!("&client_id={}", urlencode(&cfg.client_id))),
    }
    Some(url)
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<LogoutForm>,
) -> Result<Response> {
    let mut target = "/".to_string();
    if let Some(tok) = cookie_value(&headers, SESSION_COOKIE) {
        let session = session_from_headers(&state, &headers).await?;
        if let Some(s) = &session
            && !csrf_ok(&f.csrf, &s.csrf)
        {
            return Err(Error::Forbidden("bad CSRF token".into()));
        }
        let id_token = session.as_ref().and_then(|s| s.id_token.clone());
        store(&state)
            .delete_session(tok)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        if let Some(u) = end_session_url(&state, id_token.as_deref()).await {
            target = u;
        }
    }
    let mut resp = Redirect::to(&target).into_response();
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
    purge_expired(&state.db).await;
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

fn device_left<'a>(session: &'a Session) -> Left<'a> {
    Left {
        headline: "Approve a device",
        sub: "A terminal, an agent or a relay asked to join your kitchen. Check the code matches before you say yes.",
        who: Some((session.user.name.as_str(), session.csrf.as_str())),
    }
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
        return Ok(pages::render(
            "Connect a device",
            device_left(&session),
            "<h1>Enter the code</h1><p>It is shown in the terminal that asked you to connect.</p>\
             <form method=get action=/device><label class=label for=user_code>Device code</label>\
             <input id=user_code name=user_code autofocus autocomplete=off spellcheck=false placeholder=\"ABCD-EFGH\">\
             <div class=actions><button class=primary>Continue</button></div></form>",
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
            pages::render(
                "Unknown code",
                device_left(&session),
                "<h1>Unknown or expired code</h1><p>Codes last ten minutes and work once. Start again from your terminal.</p><div class=actions><a class=\"btn primary\" href=\"/device\">Enter another code</a></div>",
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
        .map(|(o,)| format!("<div class=warn><b>Heads up:</b> this name already exists (owner: {}). Approving <b>replaces its credentials</b>; the old token stops working.</div>", esc(&o)))
        .unwrap_or_default();
    let row = |k: &str, v: &str| format!("<div><dt>{k}</dt><dd>{v}</dd></div>");
    let mut details = String::new();
    match kind.as_str() {
        "user" => {
            details.push_str(&row("Action", "Sign in the CLI"));
            details.push_str(&row("As", &esc(&session.user.name)));
        }
        "agent" => {
            details.push_str(&row("Action", "Enroll a cook"));
            details.push_str(&row(
                "Name",
                &format!("<code>{}</code>", esc(name.as_deref().unwrap_or(""))),
            ));
            details.push_str(&row(
                "Runs in",
                &esc(agent_kind.as_deref().unwrap_or("other")),
            ));
            details.push_str(&row("Runtime", &esc(runtime.as_deref().unwrap_or("local"))));
            details.push_str(&row("Owner", &esc(&session.user.name)));
        }
        _ => {
            details.push_str(&row("Action", "Enroll a fire (relay)"));
            details.push_str(&row(
                "Name",
                &format!("<code>{}</code>", esc(name.as_deref().unwrap_or(""))),
            ));
            details.push_str(&row("Owner", &esc(&session.user.name)));
        }
    }
    Ok(pages::render(
        "Approve device",
        device_left(&session),
        &format!(
            "<div class=label>Device code</div><div class=code>{c}</div>\
             <dl class=details>{details}</dl>{warn}\
             <form method=post action=/device><input type=hidden name=user_code value=\"{c}\"><input type=hidden name=csrf value=\"{csrf}\">\
             <div class=actions><button class=primary name=action value=approve>Approve</button><button name=action value=deny>Deny</button></div></form>\
             <p style=\"margin-top:18px\"><small>Only approve if you started this from your own terminal.</small></p>",
            c = esc(&code),
            csrf = esc(&session.csrf),
        ),
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
    if !csrf_ok(&f.csrf, &session.csrf) {
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
            page(
                "Unknown code",
                "<h1>Unknown or expired code</h1><p>Start again from your terminal.</p>",
            ),
        )
            .into_response());
    };
    if f.action != "approve" {
        sqlx::query("update device_requests set status = 'denied' where id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(pages::render(
            "Denied",
            device_left(&session),
            "<h1 class=bad>Denied</h1><p>The device was not connected. Nothing was created.</p>",
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
    Ok(pages::render(
        "Approved",
        device_left(&session),
        "<h1 class=ok>Approved</h1><p>You can return to your terminal. It picks the credential up within a few seconds.</p>",
    )
    .into_response())
}

// ---------- console hand-offs ----------

/// The old read-only board lives in the console now.
async fn board_redirect(Path(key): Path<String>) -> Redirect {
    Redirect::to(&format!("/pots/{}", urlencode(&key.to_uppercase())))
}

/// The sign-in landing (design: Sign-in, split screen): one button into the OIDC flow.
async fn signin_page(Query(q): Query<HashMap<String, String>>) -> Response {
    let next = safe_next(q.get("next").map(|s| s.as_str()));
    pages::render(
        "Sign in",
        Left {
            headline: "Tasks for the cooks you run",
            sub: "Claim, tick, serve. Humans and coding agents share one pot without stepping on each other.",
            who: None,
        },
        &format!(
            "<h1>Welcome back</h1><p>Sign in with your organisation's single sign-on to open the kitchen.</p>\
             <div class=actions><a class=\"btn primary\" href=\"/auth/oidc/login?next={}\">Continue with single sign-on</a></div>",
            urlencode(&next)
        ),
    )
    .into_response()
}

/// Who the console is signed in as, plus the CSRF token for the server-rendered `POST /logout`.
async fn session_info(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    if headers.get(CONSOLE_HEADER).is_none() {
        return Err(Error::Unauthorized);
    }
    let Some(s) = session_from_headers(&state, &headers).await? else {
        return Err(Error::Unauthorized);
    };
    let mut resp = Json(json!({
        "id": s.user.id,
        "name": s.user.name,
        "kind": s.user.kind,
        "is_admin": s.user.is_admin,
        "csrf": s.csrf,
    }))
    .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(resp)
}

async fn font_file(Path(file): Path<String>) -> Response {
    match pages::font(&file) {
        Some(bytes) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
