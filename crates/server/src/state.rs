use std::{ops::Deref, sync::Arc, time::Duration};

use authkestra::resource::Guard;
use axum::extract::FromRef;
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub public_url: String,
    pub bind: String,
    /// Refuse non-loopback requests that did not arrive over TLS (`X-Forwarded-Proto: https`).
    pub require_https: bool,
    pub github: Option<GithubConfig>,
    pub run_background: bool,
}

#[derive(Clone, Debug)]
pub struct GithubConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Overridable so tests (and GitHub Enterprise) can point elsewhere: authorize, token, user.
    pub authorize_url: String,
    pub token_url: String,
    pub user_url: String,
}

impl GithubConfig {
    pub fn new(client_id: String, client_secret: String) -> Self {
        GithubConfig {
            client_id,
            client_secret,
            authorize_url: "https://github.com/login/oauth/authorize".into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            user_url: "https://api.github.com/user".into(),
        }
    }
}

impl Config {
    pub fn new(database_url: String, public_url: String, bind: String) -> Self {
        let public_url = public_url.trim_end_matches('/').to_string();
        let require_https = public_url.starts_with("https://");
        Config {
            database_url,
            public_url,
            bind,
            require_https,
            github: None,
            run_background: true,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Principal {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub owner_id: Option<Uuid>,
    pub is_admin: bool,
    pub max_leases: i32,
    pub runtime: Option<String>,
    pub agent_kind: Option<String>,
}

impl Principal {
    pub fn is_user(&self) -> bool {
        self.kind == "user"
    }
    pub fn is_agent(&self) -> bool {
        self.kind == "agent"
    }
    pub fn is_relay(&self) -> bool {
        self.kind == "relay"
    }
    /// Users and agents work tasks; relays only listen for wakes.
    pub fn worker(self) -> crate::error::Result<Principal> {
        if self.is_relay() {
            Err(crate::error::Error::Forbidden(
                "relays cannot read or change tasks".into(),
            ))
        } else {
            Ok(self)
        }
    }
}

pub struct Inner {
    pub db: PgPool,
    pub config: Config,
    pub http: reqwest::Client,
    /// Fan-out of event sequence numbers (from Postgres LISTEN/NOTIFY).
    pub events: broadcast::Sender<i64>,
    pub guard: Arc<Guard<Principal>>,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl FromRef<AppState> for Arc<Guard<Principal>> {
    fn from_ref(s: &AppState) -> Self {
        s.guard.clone()
    }
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> Self {
        let (events, _) = broadcast::channel(1024);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("corntchap")
            .build()
            .expect("http client");
        let guard = crate::auth::build_guard(db.clone());
        AppState(Arc::new(Inner {
            db,
            config,
            http,
            events,
            guard,
        }))
    }
}
