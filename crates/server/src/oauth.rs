//! OAuth 2.1 for MCP clients, so a repo's MCP config is only a URL: OpenCode, Claude Code, Codex
//! and claude.ai connectors register themselves (RFC 7591), send the person through
//! `/oauth/authorize` (signed in with the usual SSO), and get a cook's token by authorization
//! code + PKCE (S256), refreshed with a refresh token.
//!
//! The consent screen picks the cook (one of yours, or a new one). A client that asked for a pot's
//! MCP URL (`resource` = `{PUBLIC_URL}/mcp/<KEY>`) gets a token bound to that pot: it works on
//! `/mcp/<KEY>` and nowhere else (see `auth::POT_SCOPE`). One for `/mcp` works like any cook token.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Form, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    admin,
    auth::{random_hex, sha256},
    error::{Error, Result},
    pages::{self, Left, esc},
    state::{AppState, Principal},
    web::{self, Session},
};

pub const CODE_GRANT: &str = "authorization_code";
pub const REFRESH_GRANT: &str = "refresh_token";
const REQUEST_TTL_SECS: i64 = 600;
const CODE_TTL_SECS: i64 = 120;
/// Access tokens last a month; clients refresh them (refresh tokens: six months, used once).
const ACCESS_TTL_SECS: i64 = 30 * 86_400;
const REFRESH_TTL_SECS: i64 = 180 * 86_400;

/// RFC 8414 metadata: the device grant (the CLI) and the browser flow (MCP clients).
pub fn metadata(base: &str) -> Value {
    json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "device_authorization_endpoint": format!("{base}/oauth/device/code"),
        "grant_types_supported": [CODE_GRANT, REFRESH_GRANT, crate::web::DEVICE_GRANT],
        "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp"],
    })
}

/// RFC 9728 metadata of a pot's MCP URL.
pub async fn pot_resource_metadata(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Response {
    let base = &state.config.public_url;
    Json(json!({
        "resource": format!("{base}/mcp/{}", key.to_uppercase()),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["mcp"],
        "resource_name": format!("corntchap pot {}", key.to_uppercase()),
    }))
    .into_response()
}

// ---------- registration (RFC 7591) ----------

/// Where a client may be sent back: https, a loopback http port (RFC 8252), or an app's own
/// private-use scheme (`com.example.app:/cb`).
fn redirect_ok(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some(),
        "http" => matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        s => s.contains('.') && !s.starts_with("javascript"),
    }
}

/// Same URI, or the same loopback URI on another port (RFC 8252 section 7.3).
fn redirect_matches(registered: &[String], given: &str) -> bool {
    registered.iter().any(|r| {
        r == given || {
            let (Ok(a), Ok(b)) = (url::Url::parse(r), url::Url::parse(given)) else {
                return false;
            };
            a.scheme() == "http"
                && b.scheme() == "http"
                && matches!(a.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && a.host_str() == b.host_str()
                && a.path() == b.path()
                && a.query() == b.query()
        }
    })
}

fn oauth_json_error(status: StatusCode, code: &str, desc: &str) -> Response {
    (
        status,
        Json(json!({"error": code, "error_description": desc})),
    )
        .into_response()
}

pub async fn register(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let Ok(req) = serde_json::from_slice::<Value>(&body) else {
        return oauth_json_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "a JSON body is required",
        );
    };
    let uris: Vec<String> = req["redirect_uris"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| u.as_str().map(String::from))
        .collect();
    if uris.is_empty() || uris.len() > 10 || uris.iter().any(|u| u.len() > 500 || !redirect_ok(u)) {
        return oauth_json_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris: 1 to 10 https, loopback http or private-use scheme URIs",
        );
    }
    let name: String = req["client_name"]
        .as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("MCP client")
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    let id = format!("cc_{}", random_hex(16));
    // registrations nobody used within a day are forgotten
    let _ = sqlx::query(
        "delete from oauth_clients c where c.created_at < now() - interval '1 day' and c.last_used_at is null",
    )
    .execute(&state.db)
    .await;
    if let Err(e) =
        sqlx::query("insert into oauth_clients (id, name, redirect_uris) values ($1, $2, $3)")
            .bind(&id)
            .bind(&name)
            .bind(&uris)
            .execute(&state.db)
            .await
    {
        return Error::from(e).into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id,
            "client_name": name,
            "redirect_uris": uris,
            "grant_types": [CODE_GRANT, REFRESH_GRANT],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "client_id_issued_at": chrono::Utc::now().timestamp(),
        })),
    )
        .into_response()
}

// ---------- authorization ----------

struct Client {
    name: String,
    redirect_uris: Vec<String>,
}

async fn client(c: &mut PgConnection, id: &str) -> Result<Option<Client>> {
    let row: Option<(String, Vec<String>)> =
        sqlx::query_as("select name, redirect_uris from oauth_clients where id = $1")
            .bind(id)
            .fetch_optional(c)
            .await?;
    Ok(row.map(|(name, redirect_uris)| Client {
        name,
        redirect_uris,
    }))
}

/// `{PUBLIC_URL}/mcp` (no pot) or `{PUBLIC_URL}/mcp/<KEY>` (that pot): `Ok(None)` / `Ok(Some(key))`.
fn resource_pot(base: &str, resource: Option<&str>) -> std::result::Result<Option<String>, ()> {
    let Some(r) = resource.map(|r| r.trim_end_matches('/')) else {
        return Ok(None);
    };
    let Some(rest) = r.strip_prefix(&format!("{base}/mcp")) else {
        return Err(());
    };
    match rest.strip_prefix('/') {
        None if rest.is_empty() => Ok(None),
        Some(k)
            if !k.is_empty() && k.len() <= 16 && k.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            Ok(Some(k.to_uppercase()))
        }
        _ => Err(()),
    }
}

fn with_query(uri: &str, pairs: &[(&str, &str)]) -> String {
    let mut u = url::Url::parse(uri).expect("checked redirect uri");
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
    }
    u.to_string()
}

fn left(session: &Session) -> Left<'_> {
    Left {
        headline: "Connect an agent",
        sub: "An MCP client asks to work in your kitchen as one of your cooks. It gets a token for that cook only.",
        who: Some((
            pages::person(&session.user.name, session.display_name.as_deref()),
            session.csrf.as_str(),
        )),
    }
}

fn fail_page(title: &str, msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        pages::simple(title, &format!("<h1>{}</h1><p>{}</p>", esc(title), msg)),
    )
        .into_response()
}

/// The cook kinds a client can be given at sign-in (OpenCode first: the first-class harness).
const KINDS: [&str; 6] = [
    "opencode",
    "claude-code",
    "codex",
    "copilot",
    "antigravity",
    "other",
];

/// A kind guessed from the client's name, for a new cook.
fn guess_kind(client: &str) -> &'static str {
    let c = client.to_lowercase();
    if c.contains("opencode") {
        "opencode"
    } else if c.contains("codex") {
        "codex"
    } else if c.contains("copilot") || c.contains("visual studio code") || c.contains("vscode") {
        "copilot"
    } else if c.contains("antigravity") {
        "antigravity"
    } else if c.contains("claude") {
        "claude-code"
    } else {
        "other"
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(32).collect()
}

pub async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response> {
    let get = |k: &str| q.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
    let mut c = state.db.acquire().await?;
    let (Some(client_id), Some(redirect_uri)) = (get("client_id"), get("redirect_uri")) else {
        return Ok(fail_page(
            "Cannot connect",
            "The request names no client or no redirect URI. Start again from your agent.",
        ));
    };
    let Some(cl) = client(&mut c, client_id).await? else {
        return Ok(fail_page(
            "Unknown client",
            "This client is not registered (registrations unused for a day are forgotten). Start again from your agent.",
        ));
    };
    if !redirect_matches(&cl.redirect_uris, redirect_uri) {
        return Ok(fail_page(
            "Cannot connect",
            "The redirect URI is not one this client registered.",
        ));
    }
    let st = get("state").unwrap_or("");
    let back_err = |code: &str, desc: &str| -> Response {
        let mut pairs = vec![("error", code), ("error_description", desc)];
        if !st.is_empty() {
            pairs.push(("state", st));
        }
        Redirect::to(&with_query(redirect_uri, &pairs)).into_response()
    };
    if get("response_type") != Some("code") {
        return Ok(back_err(
            "unsupported_response_type",
            "response_type must be code",
        ));
    }
    let Some(challenge) = get("code_challenge") else {
        return Ok(back_err(
            "invalid_request",
            "PKCE is required: code_challenge with S256",
        ));
    };
    if get("code_challenge_method") != Some("S256") {
        return Ok(back_err(
            "invalid_request",
            "code_challenge_method must be S256",
        ));
    }
    let base = state.config.public_url.clone();
    let Ok(pot) = resource_pot(&base, get("resource")) else {
        return Ok(back_err(
            "invalid_target",
            "resource must be this server's /mcp or /mcp/<POT> URL",
        ));
    };
    let Some(session) = web::session_from_headers(&state, &headers).await? else {
        let here = format!(
            "/oauth/authorize?{}",
            serde_urlencoded::to_string(&q).unwrap_or_default()
        );
        return Ok(web::login_redirect(&here));
    };
    let user = &session.user;
    // the pot, as this person sees it
    let pot_row: Option<(Uuid, String, String, Uuid, bool)> = match &pot {
        Some(k) => sqlx::query_as(
            "select pr.id, pr.key, t.name, t.id,
                    $3 or ($4 and exists (select 1 from team_members m where m.team_id = t.id and m.principal_id = $2 and m.role = 'owner'))
             from projects pr join teams t on t.id = pr.team_id
             where pr.key = $1 and ($3 or exists (select 1 from team_members m where m.team_id = t.id and m.principal_id = $2))",
        )
        .bind(k)
        .bind(user.id)
        .bind(user.is_super_admin())
        .bind(user.can_orchestrate())
        .fetch_optional(&mut *c)
        .await?,
        None => None,
    };
    if pot.is_some() && pot_row.is_none() {
        return Ok(back_err(
            "invalid_target",
            "no such pot, or you are not in its team",
        ));
    }
    let mut cooks: Vec<(String, Option<String>, Vec<String>)> = sqlx::query_as(
        "select a.name, a.agent_kind,
                coalesce(array(select t.name from team_members m join teams t on t.id = m.team_id where m.principal_id = a.id order by t.name), '{}')
         from principals a where a.kind = 'agent' and a.owner_id = $1 and a.disabled_at is null order by a.name",
    )
    .bind(user.id)
    .fetch_all(&mut *c)
    .await?;
    let req_id = random_hex(24);
    let params = json!({
        "client_id": client_id, "redirect_uri": redirect_uri, "state": st,
        "challenge": challenge, "pot": pot,
    });
    sqlx::query(
        "insert into oauth_requests (id, user_id, params, expires_at)
         values ($1, $2, $3, now() + make_interval(secs => $4::float8))",
    )
    .bind(&req_id)
    .bind(user.id)
    .bind(&params)
    .bind(REQUEST_TTL_SECS as f64)
    .execute(&mut *c)
    .await?;
    let _ = sqlx::query("delete from oauth_requests where expires_at < now()")
        .execute(&mut *c)
        .await;

    let back_host = url::Url::parse(redirect_uri)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| {
                format!(
                    "{h}{}",
                    u.port().map(|p| format!(":{p}")).unwrap_or_default()
                )
            })
        })
        .unwrap_or_else(|| redirect_uri.to_string());
    let kind = guess_kind(&cl.name);
    let new_name = {
        let s = slug(&format!("{} {}", cl.name, user.name));
        if s.is_empty() {
            "my-agent".to_string()
        } else {
            s
        }
    };
    let pot_html = match &pot_row {
        Some((_, key, team, _, _)) => format!(
            "<div><dt>Pot</dt><dd><code>{}</code> (team {}) &middot; <small>a cook outside the team cannot see it: add your cook to the team first (any chef of the team can)</small></dd></div>",
            esc(key),
            esc(team),
        ),
        None => "<div><dt>Pots</dt><dd>every pot of the cook's teams</dd></div>".into(),
    };
    let fits = |ak: &Option<String>, teams: &[String]| {
        ak.as_deref() == Some(kind)
            && pot_row
                .as_ref()
                .is_none_or(|(_, _, team, _, _)| teams.iter().any(|t| t == team))
    };
    // the cooks that can work in the pot first (the client's kind before others), then the rest
    let in_pot = |teams: &[String]| {
        pot_row
            .as_ref()
            .is_none_or(|(_, _, team, _, _)| teams.iter().any(|t| t == team))
    };
    cooks.sort_by_key(|(name, ak, teams)| {
        (!in_pot(teams), ak.as_deref() != Some(kind), name.clone())
    });
    let preselect = cooks.iter().position(|(_, ak, t)| fits(ak, t));
    let mut options = String::new();
    for (i, (name, ak, teams)) in cooks.iter().enumerate() {
        let in_team = pot_row
            .as_ref()
            .is_none_or(|(_, _, team, _, _)| teams.iter().any(|t| t == team));
        options.push_str(&format!(
            "<label class=choice><input type=radio name=cook value=\"{n}\"{checked}><code>{n}</code><small>{k}{t}</small></label>",
            n = esc(name),
            k = esc(ak.as_deref().unwrap_or("other")),
            t = if teams.is_empty() {
                " &middot; in no team".to_string()
            } else if !in_team {
                format!(" &middot; not in team {}", esc(&pot_row.as_ref().map(|p| p.2.clone()).unwrap_or_default()))
            } else {
                format!(" &middot; {}", esc(&teams.join(", ")))
            },
            checked = if preselect == Some(i) { " checked" } else { "" },
        ));
    }
    let kinds = KINDS
        .iter()
        .map(|k| {
            format!(
                "<option value=\"{k}\"{}>{k}</option>",
                if *k == kind { " selected" } else { "" }
            )
        })
        .collect::<String>();
    let add_team = match &pot_row {
        Some((_, _, team, _, true)) => format!(
            "<label class=choice><input type=checkbox name=add_team value=1 checked><span>Add the cook to team <b>{}</b> if it is not in it</span></label>",
            esc(team)
        ),
        _ => String::new(),
    };
    let body = format!(
        "<h1>Connect {client}</h1>\
         <dl class=details><div><dt>Client</dt><dd>{client}</dd></div><div><dt>Sends you back to</dt><dd><code>{host}</code></dd></div>{pot_html}</dl>\
         <form method=post action=/oauth/authorize>\
         <input type=hidden name=request value=\"{req}\"><input type=hidden name=csrf value=\"{csrf}\">\
         <div class=label>Work as</div><div class=choices>{options}\
         <label class=choice><input type=radio name=cook value=\"__new\"{new_checked}><span class=newcook>A new cook\
         <input class=plain name=new_name value=\"{new_name}\" aria-label=\"New cook name\" autocomplete=off spellcheck=false>\
         <select name=new_kind aria-label=\"New cook kind\">{kinds}</select></span></label>\
         {add_team}</div>\
         <div class=actions><button class=primary name=action value=approve>Allow</button><button name=action value=deny>Deny</button></div></form>\
         <p style=\"margin-top:18px\"><small>Only allow it if you started this from your own agent. You can cut it off from the cook's card (Rotate token).</small></p>",
        client = esc(&cl.name),
        host = esc(&back_host),
        req = esc(&req_id),
        csrf = esc(&session.csrf),
        new_checked = if preselect.is_none() { " checked" } else { "" },
        new_name = esc(&new_name),
    );
    Ok(pages::render("Connect an agent", left(&session), &body).into_response())
}

#[derive(Deserialize)]
pub struct Decision {
    request: String,
    csrf: String,
    action: String,
    cook: Option<String>,
    new_name: Option<String>,
    new_kind: Option<String>,
    add_team: Option<String>,
}

pub async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<Decision>,
) -> Result<Response> {
    let Some(session) = web::session_from_headers(&state, &headers).await? else {
        return Ok(web::login_redirect("/"));
    };
    if !web::csrf_ok(&f.csrf, &session.csrf) {
        return Err(Error::Forbidden("bad CSRF token".into()));
    }
    let user = session.user.clone();
    let mut tx = state.db.begin().await?;
    let params: Option<Value> = sqlx::query_scalar(
        "delete from oauth_requests where id = $1 and user_id = $2 and expires_at > now() returning params",
    )
    .bind(&f.request)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(p) = params else {
        return Ok(fail_page(
            "Expired",
            "This request expired or was already answered. Start again from your agent.",
        ));
    };
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    let (client_id, redirect_uri, st) = (s("client_id"), s("redirect_uri"), s("state"));
    let back = |pairs: &[(&str, &str)]| {
        let mut all: Vec<(&str, &str)> = pairs.to_vec();
        if !st.is_empty() {
            all.push(("state", &st));
        }
        Redirect::to(&with_query(&redirect_uri, &all)).into_response()
    };
    if f.action != "approve" {
        tx.commit().await?;
        return Ok(back(&[
            ("error", "access_denied"),
            ("error_description", "the person said no"),
        ]));
    }
    let pot: Option<String> = p["pot"].as_str().map(String::from);
    let pot_row: Option<(Uuid, Uuid, bool)> = match &pot {
        Some(k) => sqlx::query_as(
            "select pr.id, pr.team_id,
                    $3 or ($4 and exists (select 1 from team_members m where m.team_id = pr.team_id and m.principal_id = $2 and m.role = 'owner'))
             from projects pr where pr.key = $1",
        )
        .bind(k)
        .bind(user.id)
        .bind(user.is_super_admin())
        .bind(user.can_orchestrate())
        .fetch_optional(&mut *tx)
        .await?,
        None => None,
    };
    if pot.is_some() && pot_row.is_none() {
        return Ok(fail_page("Cannot connect", "The pot no longer exists."));
    }
    let cook_id = match f.cook.as_deref().unwrap_or("") {
        "" => {
            return Ok(fail_page(
                "Choose a cook",
                "Pick one of your cooks or name a new one. <a href=\"javascript:history.back()\">Back</a>",
            ));
        }
        "__new" => {
            let name = admin::norm_handle_syntax(f.new_name.as_deref().unwrap_or(""))?;
            let kind = f
                .new_kind
                .as_deref()
                .filter(|k| KINDS.contains(k))
                .unwrap_or("other");
            new_cook(&mut tx, &user, &name, kind).await?
        }
        name => {
            let id: Option<Uuid> = sqlx::query_scalar(
                "select id from principals where name = $1 and kind = 'agent' and owner_id = $2 and disabled_at is null",
            )
            .bind(name)
            .bind(user.id)
            .fetch_optional(&mut *tx)
            .await?;
            id.ok_or_else(|| Error::NotFound(format!("no cook {name} of yours")))?
        }
    };
    if let (Some((_, team, true)), Some(_)) = (&pot_row, &f.add_team) {
        sqlx::query(
            "insert into team_members (team_id, principal_id, role) values ($1, $2, 'member') on conflict do nothing",
        )
        .bind(team)
        .bind(cook_id)
        .execute(&mut *tx)
        .await?;
    }
    let code = format!("cc_code_{}", random_hex(24));
    sqlx::query(
        "insert into oauth_codes (code_hash, client_id, redirect_uri, challenge, principal_id, pot_id, approved_by, expires_at)
         values ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(secs => $8::float8))",
    )
    .bind(sha256(code.as_bytes()))
    .bind(&client_id)
    .bind(&redirect_uri)
    .bind(s("challenge"))
    .bind(cook_id)
    .bind(pot_row.map(|r| r.0))
    .bind(user.id)
    .bind(CODE_TTL_SECS as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let iss = state.config.public_url.clone();
    Ok(back(&[("code", &code), ("iss", &iss)]))
}

/// A cook made from the consent screen, like `cook setup`: owned by the person, with a relay
/// wake target so the fire on their machine can wake its sessions.
async fn new_cook(c: &mut PgConnection, user: &Principal, name: &str, kind: &str) -> Result<Uuid> {
    let taken: Option<Option<Uuid>> =
        sqlx::query_scalar("select owner_id from principals where name = $1")
            .bind(name)
            .fetch_optional(&mut *c)
            .await?;
    if taken.is_some() {
        return Err(Error::Conflict(format!(
            "the name {name} is taken: pick your existing cook or another name"
        )));
    }
    admin::ensure_not_a_handle(&mut *c, name).await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into principals (id, kind, name, owner_id, agent_kind, runtime) values ($1, 'agent', $2, $3, $4, 'local')",
    )
    .bind(id)
    .bind(name)
    .bind(user.id)
    .bind(kind)
    .execute(&mut *c)
    .await?;
    sqlx::query(
        "insert into wake_targets (agent_id, type, projects, labels, config) values ($1, 'relay', '{}', '{}', '{}')",
    )
    .bind(id)
    .execute(&mut *c)
    .await?;
    Ok(id)
}

// ---------- token endpoint ----------

fn token_response(access: &str, refresh: &str, pot: Option<&str>) -> Response {
    let mut body = json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_TTL_SECS,
        "refresh_token": refresh,
        "scope": "mcp",
    });
    if let Some(k) = pot {
        body["pot"] = json!(k);
    }
    let mut r = Json(body).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}

fn grant_err(desc: &str) -> Response {
    oauth_json_error(StatusCode::BAD_REQUEST, "invalid_grant", desc)
}

/// Mint an access token (bound to the pot, if any) and its refresh token.
async fn issue(
    c: &mut PgConnection,
    client_id: &str,
    client_name: &str,
    principal: Uuid,
    pot: Option<Uuid>,
) -> Result<(String, String)> {
    let access = admin::mint_token(
        &mut *c,
        principal,
        &format!("oauth: {client_name}"),
        Some(ACCESS_TTL_SECS),
    )
    .await?;
    let tid: Uuid = sqlx::query_scalar(
        "update tokens set pot_id = $2, client_id = $3 where token_hash = $1 returning id",
    )
    .bind(sha256(access.as_bytes()))
    .bind(pot)
    .bind(client_id)
    .fetch_one(&mut *c)
    .await?;
    let refresh = format!("ctr_{}", random_hex(32));
    sqlx::query(
        "insert into oauth_refresh (token_hash, client_id, principal_id, pot_id, access_token_id, expires_at)
         values ($1, $2, $3, $4, $5, now() + make_interval(secs => $6::float8))",
    )
    .bind(sha256(refresh.as_bytes()))
    .bind(client_id)
    .bind(principal)
    .bind(pot)
    .bind(tid)
    .bind(REFRESH_TTL_SECS as f64)
    .execute(&mut *c)
    .await?;
    sqlx::query("update oauth_clients set last_used_at = now() where id = $1")
        .bind(client_id)
        .execute(&mut *c)
        .await?;
    Ok((access, refresh))
}

fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha256(verifier.as_bytes()))
}

/// `grant_type=authorization_code` and `refresh_token` (the device grant stays in `web.rs`).
pub async fn token(state: &AppState, f: &HashMap<String, String>) -> Result<Response> {
    let get = |k: &str| f.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
    let mut tx = state.db.begin().await?;
    let resp = match get("grant_type") {
        Some(CODE_GRANT) => {
            let (Some(code), Some(verifier), Some(client_id)) =
                (get("code"), get("code_verifier"), get("client_id"))
            else {
                return Ok(oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code, code_verifier and client_id are required",
                ));
            };
            let row: Option<(String, String, String, Uuid, Option<Uuid>, bool, bool)> = sqlx::query_as(
                "select client_id, redirect_uri, challenge, principal_id, pot_id, used_at is not null, expires_at < now()
                 from oauth_codes where code_hash = $1 for update",
            )
            .bind(sha256(code.as_bytes()))
            .fetch_optional(&mut *tx)
            .await?;
            let Some((cid, ruri, challenge, principal, pot, used, expired)) = row else {
                return Ok(grant_err("unknown code"));
            };
            if used || expired {
                return Ok(grant_err("the code was used or expired"));
            }
            if cid != client_id || get("redirect_uri").is_some_and(|r| r != ruri) {
                return Ok(grant_err(
                    "the code belongs to another client or redirect URI",
                ));
            }
            if s256(verifier) != challenge {
                return Ok(grant_err("the PKCE verifier does not match"));
            }
            sqlx::query("update oauth_codes set used_at = now() where code_hash = $1")
                .bind(sha256(code.as_bytes()))
                .execute(&mut *tx)
                .await?;
            let name = client(&mut tx, &cid)
                .await?
                .map(|c| c.name)
                .unwrap_or_default();
            let (a, r) = issue(&mut tx, &cid, &name, principal, pot).await?;
            let key = pot_key(&mut tx, pot).await?;
            token_response(&a, &r, key.as_deref())
        }
        Some(REFRESH_GRANT) => {
            let Some(rt) = get("refresh_token") else {
                return Ok(oauth_json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                ));
            };
            let row: Option<(String, Uuid, Option<Uuid>, bool, bool, bool)> = sqlx::query_as(
                "select r.client_id, r.principal_id, r.pot_id, r.used_at is not null or r.expires_at < now(),
                        coalesce(t.revoked_at is not null, true),
                        exists (select 1 from principals p where p.id = r.principal_id and p.disabled_at is null)
                 from oauth_refresh r left join tokens t on t.id = r.access_token_id
                 where r.token_hash = $1 for update of r",
            )
            .bind(sha256(rt.as_bytes()))
            .fetch_optional(&mut *tx)
            .await?;
            let Some((cid, principal, pot, spent, revoked, active)) = row else {
                return Ok(grant_err("unknown refresh token"));
            };
            if get("client_id").is_some_and(|c| c != cid) {
                return Ok(grant_err("the refresh token belongs to another client"));
            }
            // a rotated or disabled cook, or a revoked token, cuts the client off for good
            if spent || revoked || !active {
                return Ok(grant_err(
                    "the refresh token was used, expired or revoked: connect again",
                ));
            }
            sqlx::query("update oauth_refresh set used_at = now() where token_hash = $1")
                .bind(sha256(rt.as_bytes()))
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "update tokens set revoked_at = now() where id = (select access_token_id from oauth_refresh where token_hash = $1)",
            )
            .bind(sha256(rt.as_bytes()))
            .execute(&mut *tx)
            .await?;
            let name = client(&mut tx, &cid)
                .await?
                .map(|c| c.name)
                .unwrap_or_default();
            let (a, r) = issue(&mut tx, &cid, &name, principal, pot).await?;
            let key = pot_key(&mut tx, pot).await?;
            token_response(&a, &r, key.as_deref())
        }
        _ => oauth_json_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "authorization_code, refresh_token or the device code",
        ),
    };
    tx.commit().await?;
    Ok(resp)
}

async fn pot_key(c: &mut PgConnection, pot: Option<Uuid>) -> Result<Option<String>> {
    Ok(match pot {
        Some(id) => {
            sqlx::query_scalar("select key from projects where id = $1")
                .bind(id)
                .fetch_optional(c)
                .await?
        }
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_are_https_loopback_or_private_schemes() {
        assert!(redirect_ok("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_ok("http://127.0.0.1:19876/mcp/oauth/callback"));
        assert!(redirect_ok("http://localhost:5555/cb"));
        assert!(redirect_ok("com.example.app:/oauth"));
        assert!(!redirect_ok("http://evil.example.com/cb"));
        assert!(!redirect_ok("javascript:alert(1)"));
        assert!(!redirect_ok("https://ok.example/cb#frag"));
        assert!(!redirect_ok("not a url"));
    }

    #[test]
    fn loopback_redirects_match_on_any_port() {
        let reg = vec!["http://127.0.0.1:1000/cb".to_string()];
        assert!(redirect_matches(&reg, "http://127.0.0.1:2000/cb"));
        assert!(!redirect_matches(&reg, "http://127.0.0.1:2000/other"));
        let reg = vec!["https://claude.ai/cb".to_string()];
        assert!(redirect_matches(&reg, "https://claude.ai/cb"));
        assert!(!redirect_matches(&reg, "https://claude.ai/cb2"));
    }

    #[test]
    fn resources_name_mcp_or_a_pot() {
        let b = "https://c.example";
        assert_eq!(resource_pot(b, None), Ok(None));
        assert_eq!(resource_pot(b, Some("https://c.example/mcp")), Ok(None));
        assert_eq!(resource_pot(b, Some("https://c.example/mcp/")), Ok(None));
        assert_eq!(
            resource_pot(b, Some("https://c.example/mcp/anr")),
            Ok(Some("ANR".into()))
        );
        assert!(resource_pot(b, Some("https://other.example/mcp")).is_err());
        assert!(resource_pot(b, Some("https://c.example/mcp/a/b")).is_err());
    }

    #[test]
    fn pkce_s256_is_base64url_sha256() {
        // computed independently: printf %s <verifier> | openssl dgst -sha256 -binary | base64url
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gXFWw9-WE"),
            "fPAX_DWTmWmwD8yVEq_nNVrLGeqWlfeNu2xaBU2JW1w"
        );
    }

    #[test]
    fn new_cook_names_and_kinds_come_from_the_client() {
        assert_eq!(guess_kind("OpenCode"), "opencode");
        assert_eq!(guess_kind("Claude Code (corntchap)"), "claude-code");
        assert_eq!(guess_kind("Visual Studio Code"), "copilot");
        assert_eq!(guess_kind("GitHub Copilot CLI"), "copilot");
        assert_eq!(guess_kind("Antigravity"), "antigravity");
        assert_eq!(guess_kind("legacy tool"), "other");
        assert_eq!(slug("Claude Code  selast"), "claude-code-selast");
    }
}
