//! The repositories a connected GitHub App can see, for picking a pot's repositories.
//!
//! GitHub REST endpoints used (verified against docs.github.com on 2026-11-24):
//! * App JWT: RS256, `iss` the app id (the client id also works), `iat` 60 seconds in the past
//!   (clock drift), `exp` at most 10 minutes ahead ("Generating a JWT for a GitHub App").
//! * `POST {api}/app/installations/{installation_id}/access_tokens` with the JWT as Bearer: `201`
//!   and `{token, expires_at, ...}`; the token lives one hour ("Create an installation access token").
//! * `GET {api}/installation/repositories?per_page=100` with the installation token as Bearer:
//!   `{total_count, repository_selection, repositories: [...]}`, `per_page` at most 100, more pages
//!   through the `Link: <...>; rel="next"` header ("List repositories accessible to the app
//!   installation").
//!
//! GitHub Enterprise: every call goes to the connection's stored `api_url`.
//!
//! The installation token is cached in memory until a minute before it expires; it is never
//! stored in the database and never logged.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    error::{Error, Result},
    repos::normalize_git_url,
    state::AppState,
};

/// One connection that can list repositories.
pub(super) struct Conn {
    pub id: Uuid,
    pub api_url: String,
    pub web_url: String,
    pub app_id: i64,
    pub pem: String,
}

/// A repository the app can see.
#[derive(Clone, Debug)]
pub struct Visible {
    pub full_name: String,
    pub html_url: String,
    pub clone_url: String,
    pub default_branch: Option<String>,
    pub private: bool,
    pub archived: bool,
    pub description: Option<String>,
    pub pushed_at: Option<String>,
    pub account: String,
    pub installation: i64,
}

impl Visible {
    pub fn norm_url(&self) -> String {
        normalize_git_url(if self.html_url.is_empty() {
            &self.clone_url
        } else {
            &self.html_url
        })
    }
}

type Cache = Mutex<HashMap<(Uuid, i64), (String, DateTime<Utc>)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The app's JWT, valid for 9 minutes.
pub fn app_jwt(app_id: i64, pem: &str, now: DateTime<Utc>) -> Result<String> {
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|_| Error::Invalid("the app's private key is not a valid RSA key".into()))?;
    let claims = json!({
        "iss": app_id.to_string(),
        "iat": (now - Duration::seconds(60)).timestamp(),
        "exp": (now + Duration::minutes(9)).timestamp(),
    });
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &key,
    )
    .map_err(|e| Error::Internal(format!("could not sign the app token: {e}")))
}

pub(super) async fn installation_token(
    st: &AppState,
    c: &Conn,
    installation: i64,
) -> Result<String> {
    let key = (c.id, installation);
    let now = Utc::now();
    if let Some((t, exp)) = cache().lock().unwrap().get(&key)
        && *exp - Duration::minutes(1) > now
    {
        return Ok(t.clone());
    }
    let jwt = app_jwt(c.app_id, &c.pem, now)?;
    let resp = st
        .http
        .post(format!(
            "{}/app/installations/{installation}/access_tokens",
            c.api_url
        ))
        .bearer_auth(jwt)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|e| Error::Internal(format!("could not reach {}: {e}", c.api_url)))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(Error::Invalid(format!(
            "GitHub answered {status} for installation {installation}: {}",
            v["message"].as_str().unwrap_or("no message")
        )));
    }
    let token = v["token"]
        .as_str()
        .ok_or_else(|| Error::Invalid("GitHub's answer has no token".into()))?
        .to_string();
    let exp = v["expires_at"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now + Duration::minutes(55));
    cache().lock().unwrap().insert(key, (token.clone(), exp));
    Ok(token)
}

/// The `rel="next"` target of a `Link` header.
fn next_link(h: &http::HeaderMap) -> Option<String> {
    let l = h.get("link")?.to_str().ok()?;
    l.split(',').find_map(|part| {
        let (url, rest) = part.split_once(';')?;
        rest.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

async fn installation_repos(
    st: &AppState,
    c: &Conn,
    installation: i64,
    account: &str,
) -> Result<Vec<Visible>> {
    let token = installation_token(st, c, installation).await?;
    let mut url = Some(format!(
        "{}/installation/repositories?per_page=100",
        c.api_url
    ));
    let mut out = vec![];
    let mut pages = 0;
    while let Some(u) = url.take() {
        pages += 1;
        // never send the token anywhere but the connection's own API host
        if !u.starts_with(&c.api_url) || pages > 100 {
            break;
        }
        let resp = st
            .http
            .get(&u)
            .bearer_auth(&token)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .send()
            .await
            .map_err(|e| Error::Internal(format!("could not reach {}: {e}", c.api_url)))?;
        let status = resp.status();
        if status == http::StatusCode::UNAUTHORIZED {
            cache().lock().unwrap().remove(&(c.id, installation));
        }
        let next = next_link(resp.headers());
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(Error::Invalid(format!(
                "GitHub answered {status} listing installation {installation}: {}",
                v["message"].as_str().unwrap_or("no message")
            )));
        }
        for r in v["repositories"].as_array().into_iter().flatten() {
            let Some(full_name) = r["full_name"].as_str() else {
                continue;
            };
            let html_url = r["html_url"].as_str().unwrap_or_default().to_string();
            let clone_url = r["clone_url"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| html_url.clone());
            out.push(Visible {
                full_name: full_name.to_string(),
                html_url,
                clone_url,
                default_branch: r["default_branch"].as_str().map(str::to_string),
                private: r["private"].as_bool().unwrap_or(false),
                archived: r["archived"].as_bool().unwrap_or(false),
                description: r["description"].as_str().map(str::to_string),
                pushed_at: r["pushed_at"].as_str().map(str::to_string),
                account: account.to_string(),
                installation,
            });
        }
        url = next;
    }
    Ok(out)
}

/// What [`visible`] found: the repositories, a message per installation that failed, and whether any
/// app that can list repositories (it has a private key) is connected at all.
pub struct Catalog {
    pub connected: bool,
    pub repos: Vec<Visible>,
    pub errors: Vec<String>,
}

type InstallationRow = (Uuid, String, i64, String, i64, Option<String>);

/// Every repository every installation can see (`installation`: only that one), recently pushed
/// first, archived ones last. One broken installation does not fail the rest: it is named in
/// `errors`.
pub async fn visible(st: &AppState, installation: Option<i64>) -> Result<Catalog> {
    let apps: i64 = sqlx::query_scalar(
        "select count(*) from forge_connections
          where provider = 'github' and app_id is not null and private_key is not null",
    )
    .fetch_one(&st.db)
    .await?;
    let rows: Vec<InstallationRow> = sqlx::query_as(
        "select c.id, c.api_url, c.app_id, c.private_key, i.installation_id, i.account
           from forge_connections c join forge_installations i on i.connection_id = c.id
          where c.provider = 'github' and c.app_id is not null and c.private_key is not null
            and not i.suspended
          order by c.created_at, i.created_at",
    )
    .fetch_all(&st.db)
    .await?;
    let mut cat = Catalog {
        connected: apps > 0,
        repos: vec![],
        errors: vec![],
    };
    let jobs = rows
        .into_iter()
        .filter(|r| installation.is_none_or(|i| i == r.4))
        .map(|(id, api_url, app_id, pem, inst, account)| async move {
            let c = Conn {
                id,
                api_url: api_url.trim_end_matches('/').to_string(),
                web_url: String::new(),
                app_id,
                pem,
            };
            let name = account.unwrap_or_else(|| format!("installation {inst}"));
            installation_repos(st, &c, inst, &name)
                .await
                .map_err(|e| format!("{name}: {e}"))
        });
    for r in futures::future::join_all(jobs).await {
        match r {
            Ok(mut v) => cat.repos.append(&mut v),
            Err(e) => cat.errors.push(e),
        }
    }
    // the same repository can be seen through two apps
    let mut seen = std::collections::HashSet::new();
    cat.repos.retain(|r| seen.insert(r.norm_url()));
    cat.repos.sort_by(|a, b| {
        a.archived
            .cmp(&b.archived)
            .then_with(|| b.pushed_at.cmp(&a.pushed_at))
            .then_with(|| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()))
    });
    Ok(cat)
}

/// Every connection that can call the API (an app id and a private key).
pub(super) async fn connections(st: &AppState) -> Result<Vec<Conn>> {
    let rows: Vec<(Uuid, String, String, i64, String)> = sqlx::query_as(
        "select id, api_url, web_url, app_id, private_key from forge_connections
          where provider = 'github' and app_id is not null and private_key is not null order by created_at",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, api, web, app_id, pem)| Conn {
            id,
            api_url: api.trim_end_matches('/').to_string(),
            web_url: web.trim_end_matches('/').to_string(),
            app_id,
            pem,
        })
        .collect())
}

/// One GET against a connection's API: the status, the JSON body (null when there is none) and the
/// `Link` header's next page. `bearer` is the installation token, or the app JWT for the few
/// endpoints that want it. The url must be on the connection's API host.
pub(super) async fn get_json(
    st: &AppState,
    c: &Conn,
    bearer: &str,
    url: &str,
) -> Result<(http::StatusCode, Value, Option<String>)> {
    if !url.starts_with(&c.api_url) {
        return Err(Error::Internal("refusing to call a foreign host".into()));
    }
    let resp = st
        .http
        .get(url)
        .bearer_auth(bearer)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .send()
        .await
        .map_err(|e| Error::Internal(format!("could not reach {}: {e}", c.api_url)))?;
    let status = resp.status();
    let next = next_link(resp.headers());
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    Ok((status, v, next))
}
