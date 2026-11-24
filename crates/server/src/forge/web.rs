//! The HTTP side of forge events:
//!
//! * `POST /webhooks/{provider}`: a delivery. Checked against every connection's secret (401 when
//!   none matches), deduplicated by delivery id, answered `202` at once and processed in a spawned
//!   task so the forge's 10 second timeout is never hit.
//! * `GET /forges/github/connect` and `/forges/github/callback`: the GitHub App manifest flow
//!   (admins only, signed in with the browser session).
//! * `/api/v1/forges`: the admin API (list, remove), see `rest.rs`.

use std::collections::HashMap;

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde_json::Value;

use super::{events, for_provider, github, store};
use crate::{
    error::{Error, Result},
    pages::{self, esc},
    state::AppState,
    web::{login_redirect, session_from_headers},
};

pub const DEFAULT_GITHUB_API: &str = "https://api.github.com";
pub const DEFAULT_GITHUB_WEB: &str = "https://github.com";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/webhooks/{provider}", post(webhook))
        .route("/forges/github/connect", get(connect))
        .route("/forges/github/callback", get(callback))
}

fn text(code: StatusCode, msg: &str) -> Response {
    (code, msg.to_string()).into_response()
}

async fn webhook(
    State(st): State<AppState>,
    axum::extract::Path(provider): axum::extract::Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(forge) = for_provider(&provider) else {
        return text(StatusCode::NOT_FOUND, "unknown provider");
    };
    let secrets = match store::secrets(&st.db, forge.provider()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("forge: {e}");
            return text(StatusCode::INTERNAL_SERVER_ERROR, "error");
        }
    };
    let Some(conn) = secrets
        .iter()
        .find(|c| forge.verify(&c.secret, &headers, &body))
    else {
        return text(StatusCode::UNAUTHORIZED, "bad signature");
    };
    let (Some(delivery), Some(event)) = (forge.delivery_id(&headers), forge.event_name(&headers))
    else {
        return text(StatusCode::BAD_REQUEST, "missing delivery or event header");
    };
    let action = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["action"].as_str().map(str::to_string));
    match store::claim_delivery(
        &st.db,
        forge.provider(),
        &delivery,
        conn.id,
        &event,
        action.as_deref(),
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return text(StatusCode::OK, "duplicate"),
        Err(e) => {
            tracing::error!("forge: {e}");
            return text(StatusCode::INTERNAL_SERVER_ERROR, "error");
        }
    }
    let (st2, conn_id) = (st.clone(), conn.id);
    tokio::spawn(async move {
        let (status, detail) = match forge.parse(&event, &body) {
            Err(e) => ("error", e),
            Ok(evs) => match events::process(&st2.db, conn_id, &evs).await {
                Ok(a) => (a.status, a.detail),
                Err(e) => {
                    tracing::error!("forge delivery {delivery}: {e}");
                    ("error", e.to_string())
                }
            },
        };
        store::finish_delivery(&st2.db, forge.provider(), &delivery, status, Some(&detail)).await;
    });
    text(StatusCode::ACCEPTED, "accepted")
}

/// An https or http address without a trailing slash, or None.
fn clean_base(u: &str) -> Option<String> {
    let u = u.trim().trim_end_matches('/');
    (u.starts_with("https://") || u.starts_with("http://")).then(|| u.to_string())
}

/// The admin's session, or a redirect to sign in, or a refusal.
#[allow(clippy::result_large_err)]
async fn admin_session(
    st: &AppState,
    headers: &HeaderMap,
    next: &str,
) -> std::result::Result<crate::web::Session, Response> {
    match session_from_headers(st, headers).await {
        Ok(Some(s)) if s.user.is_super_admin() => Ok(s),
        Ok(Some(_)) => Err((
            StatusCode::FORBIDDEN,
            pages::simple(
                "Super-admins only",
                "<h1>Super-admins only</h1><p>Only a super-admin can connect a git host.</p><div class=actions><a class=\"btn primary\" href=\"/\">Back to the kitchen</a></div>",
            ),
        )
            .into_response()),
        Ok(None) => Err(login_redirect(next)),
        Err(e) => Err(e.into_response()),
    }
}

/// Step 1: a page that posts the manifest to GitHub (the browser must do it: GitHub shows the
/// person a "Create GitHub App" screen).
async fn connect(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let org = q.get("org").map(|s| s.trim()).filter(|s| !s.is_empty());
    let mut next = "/forges/github/connect".to_string();
    if let Some(o) = org {
        next.push_str(&format!("?org={}", o.replace(' ', "")));
    }
    let session = match admin_session(&st, &headers, &next).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    // GitHub Enterprise: `host` is the server's name; the API and web hosts follow. Tests and odd
    // setups may name both (`api_url`, `web_url`).
    let (api, web) = match q.get("host").map(|h| h.trim()).filter(|h| !h.is_empty()) {
        Some(h) => (format!("https://{h}/api/v3"), format!("https://{h}")),
        None => (
            q.get("api_url")
                .and_then(|u| clean_base(u))
                .unwrap_or_else(|| DEFAULT_GITHUB_API.into()),
            q.get("web_url")
                .and_then(|u| clean_base(u))
                .unwrap_or_else(|| DEFAULT_GITHUB_WEB.into()),
        ),
    };
    if let Some(o) = org
        && !o
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return text(StatusCode::BAD_REQUEST, "that is not an organization name");
    }
    let state = match store::start_flow(&st.db, org, &api, &web, session.user.id).await {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let name = q
        .get("name")
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("corntchap {}", crate::auth::random_hex(2)));
    let manifest = github::manifest(&name, &st.config.public_url);
    let action = match org {
        Some(o) => format!("{web}/organizations/{o}/settings/apps/new?state={state}"),
        None => format!("{web}/settings/apps/new?state={state}"),
    };
    let who = pages::person(&session.user.name, session.display_name.as_deref());
    let body = format!(
        "<h1>Connect GitHub</h1><p>GitHub will ask you to create the app. It can only read pull requests, checks and reviews.</p>\
         <form id=f method=post action=\"{}\"><input type=hidden name=manifest value=\"{}\">\
         <noscript><div class=actions><button class=primary>Continue to GitHub</button></div></noscript></form>\
         <script>document.getElementById('f').submit()</script>",
        esc(&action),
        esc(&manifest.to_string())
    );
    pages::render(
        "Connect GitHub",
        pages::Left {
            headline: "Connect GitHub",
            sub: "Pull requests, checks and reviews, shown on your tasks.",
            who: Some((who, session.csrf.as_str())),
        },
        &body,
    )
    .into_response()
}

/// Step 2: GitHub sends the browser back with a code; trade it for the app's credentials.
async fn callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (Some(code), Some(state)) = (q.get("code"), q.get("state")) else {
        return text(StatusCode::BAD_REQUEST, "missing code or state");
    };
    let next = format!("/forges/github/callback?code={code}&state={state}");
    let session = match admin_session(&st, &headers, &next).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let flow = match store::take_flow(&st.db, state).await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                pages::simple(
                    "Link expired",
                    "<h1>This link expired</h1><p>Start again from the Forges page.</p><div class=actions><a class=\"btn primary\" href=\"/admin/forges\">Forges</a></div>",
                ),
            )
                .into_response();
        }
        Err(e) => return e.into_response(),
    };
    let (_org, api, web, started_by) = flow;
    if started_by != Some(session.user.id) {
        return text(
            StatusCode::FORBIDDEN,
            "this flow was started by someone else",
        );
    }
    match convert(&st, &api, code).await {
        Ok(app) => {
            let r = store::insert_connection(
                &st.db,
                "github",
                &app,
                &api,
                &web,
                Some(session.user.id),
            )
            .await;
            match r {
                // done: the page lists the app and its install link
                Ok(_) => Redirect::to("/admin/forges?connected=1").into_response(),
                Err(e) => e.into_response(),
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            pages::simple(
                "GitHub said no",
                &format!(
                    "<h1>GitHub did not accept the code</h1><p>{}</p><div class=actions><a class=\"btn primary\" href=\"/admin/forges\">Forges</a></div>",
                    esc(&e.to_string())
                ),
            ),
        )
            .into_response(),
    }
}

/// `POST {api}/app-manifests/{code}/conversions`: the app's id, keys and webhook secret.
async fn convert(st: &AppState, api: &str, code: &str) -> Result<store::NewApp> {
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let resp = st
        .http
        .post(format!("{api}/app-manifests/{code}/conversions"))
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|e| Error::Internal(format!("could not reach {api}: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(Error::Invalid(format!(
            "{api} answered {status}: {}",
            v["message"].as_str().unwrap_or("no message")
        )));
    }
    let s = |k: &str| v[k].as_str().map(str::to_string);
    Ok(store::NewApp {
        app_id: v["id"]
            .as_i64()
            .ok_or_else(|| Error::Invalid("the answer has no app id".into()))?,
        slug: s("slug").ok_or_else(|| Error::Invalid("the answer has no slug".into()))?,
        name: s("name").unwrap_or_else(|| "corntchap".into()),
        html_url: s("html_url"),
        client_id: s("client_id"),
        client_secret: s("client_secret"),
        pem: s("pem"),
        webhook_secret: s("webhook_secret")
            .ok_or_else(|| Error::Invalid("the answer has no webhook secret".into()))?,
        owner: v["owner"]["login"].as_str().map(str::to_string),
    })
}
