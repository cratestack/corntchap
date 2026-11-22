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
    pub oidc: Option<OidcConfig>,
    pub run_background: bool,
    /// S3-compatible file storage (RustFS in dev); files are off without it.
    pub storage: Option<crate::files::StorageConfig>,
    /// Days the pass (chat) is kept; durable knowledge belongs in the pantry.
    pub pass_retention_days: i64,
    /// Minutes a cook has to answer a person's comment before its main station is woken again.
    pub comment_nudge_minutes: i64,
    /// Who a github-comment wake mentions when its target names nobody (`GITHUB_MENTION`).
    pub github_mention: String,
}

pub const DEFAULT_GITHUB_MENTION: &str = "@cook";

/// `codex` and `@codex` both mean `@codex`.
pub fn mention(m: &str) -> String {
    let m = m.trim();
    if m.starts_with('@') {
        m.to_string()
    } else {
        format!("@{m}")
    }
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    /// Issuer URL used for discovery (`<issuer>/.well-known/openid-configuration`).
    /// With Keycloak's backchannel-dynamic hostname this can be an internal URL while the
    /// issuer inside tokens stays the public one.
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// Issuer URL as browsers and API clients see it (defaults to `issuer`). Advertised as the
    /// OAuth authorization server and used to rewrite the end-session URL for the browser.
    pub public_issuer: Option<String>,
    /// IdP role/group that grants admin (`OIDC_ADMIN_ROLE`, default `corntchap-admin`).
    pub admin_role: String,
}

pub const DEFAULT_ADMIN_ROLE: &str = "corntchap-admin";

impl Config {
    pub fn new(database_url: String, public_url: String, bind: String) -> Self {
        let public_url = public_url.trim_end_matches('/').to_string();
        let require_https = public_url.starts_with("https://");
        Config {
            database_url,
            public_url,
            bind,
            require_https,
            oidc: None,
            run_background: true,
            storage: None,
            pass_retention_days: 7,
            comment_nudge_minutes: 10,
            github_mention: DEFAULT_GITHUB_MENTION.into(),
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
    pub max_leases: i64,
    pub runtime: Option<String>,
    pub agent_kind: Option<String>,
    /// The bearer token this request authenticated with (None for sessions and MCP contexts).
    #[serde(skip)]
    pub token_id: Option<Uuid>,
    /// Cooks only: what their owner granted beyond working tasks (plan, manage, admin).
    #[serde(default)]
    pub grants: Vec<String>,
}

impl Principal {
    pub fn is_user(&self) -> bool {
        self.kind == "user"
    }
    pub fn granted(&self, g: &str) -> bool {
        self.grants.iter().any(|x| x == g)
    }
    /// People, and cooks granted `plan`: the people-side task actions (edit or toss any task,
    /// restore, unblock, tick without a lease, forget any memory).
    pub fn can_plan(&self) -> bool {
        self.is_user() || (self.is_agent() && self.granted("plan"))
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
                "this is a fire (relay) token: fires only carry wake-ups and cannot read or change tasks. \
                 Give the agent its cook token instead (corntchap-cli cook rotate <name>, or Cooks > MCP config in the console)"
                    .into(),
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
    /// OIDC provider, discovered lazily so the server can start before the IdP is up.
    pub oidc: tokio::sync::OnceCell<authkestra::oidc::OidcProvider>,
    pub storage: Option<Arc<crate::files::Storage>>,
    /// Flips to true when the server starts shutting down: the endless streams (relay wakes,
    /// the live feed) and long-polls end so the graceful drain finishes at once.
    pub stopping: tokio::sync::watch::Sender<bool>,
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
        let guard = crate::auth::build_guard(db.clone(), &config.public_url);
        let storage = config
            .storage
            .clone()
            .and_then(|c| match crate::files::Storage::new(c) {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    tracing::error!("file storage disabled: {e}");
                    None
                }
            });
        AppState(Arc::new(Inner {
            db,
            config,
            http,
            events,
            guard,
            oidc: tokio::sync::OnceCell::new(),
            storage,
            stopping: tokio::sync::watch::channel(false).0,
        }))
    }

    /// Ask the streams and long-polls to end (the server is shutting down).
    pub fn shut_down(&self) {
        self.stopping.send_replace(true);
    }

    /// Resolves once the server starts shutting down.
    pub async fn stopped(&self) {
        let mut rx = self.stopping.subscribe();
        let _ = rx.wait_for(|s| *s).await;
    }
}
