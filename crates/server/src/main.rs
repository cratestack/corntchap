use std::time::Duration;

use clap::{Parser, Subcommand};
use corntchap::{
    admin, app,
    state::{AppState, Config, OidcConfig},
};

#[derive(Parser)]
#[command(
    name = "corntchap",
    version,
    about = "Coordination server for coding agents: tasks, leases, MCP, REST"
)]
struct Cli {
    #[arg(long, env = "DATABASE_URL", global = true)]
    database_url: Option<String>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the server (default).
    Serve,
    /// Apply the database schema.
    Migrate,
    /// Create the first admin (an OIDC username, e.g. Keycloak's preferred_username) and print a 7-day bootstrap token.
    Init {
        #[arg(long)]
        admin: String,
    },
    /// Mint a token for an existing user/agent/relay (needs database access).
    Token {
        #[arg(long)]
        name: String,
        /// Lifetime in hours; omit for no expiry.
        #[arg(long)]
        hours: Option<i64>,
    },
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn config(cli: &Cli) -> anyhow::Result<Config> {
    let db = cli
        .database_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required"))?;
    let bind = env("BIND").unwrap_or_else(|| "0.0.0.0:8080".into());
    let public = env("PUBLIC_URL").unwrap_or_else(|| {
        format!(
            "http://localhost:{}",
            bind.rsplit(':').next().unwrap_or("8080")
        )
    });
    let mut c = Config::new(db, public, bind);
    if let (Some(issuer), Some(id), Some(secret)) = (
        env("OIDC_ISSUER"),
        env("OIDC_CLIENT_ID"),
        env("OIDC_CLIENT_SECRET"),
    ) {
        c.oidc = Some(OidcConfig {
            issuer,
            client_id: id,
            client_secret: secret,
            public_issuer: env("OIDC_PUBLIC_ISSUER"),
            admin_role: env("OIDC_ADMIN_ROLE")
                .unwrap_or_else(|| corntchap::state::DEFAULT_ADMIN_ROLE.to_string()),
        });
    }
    if let Some(m) = env("COMMENT_NUDGE_MINUTES").and_then(|v| v.parse::<i64>().ok()) {
        c.comment_nudge_minutes = m.max(1);
    }
    if let Some(d) = env("PASS_RETENTION_DAYS").and_then(|v| v.parse::<i64>().ok()) {
        c.pass_retention_days = d.max(1);
    }
    if let (Some(endpoint), Some(bucket)) = (env("S3_ENDPOINT"), env("S3_BUCKET")) {
        let num = |k: &str, d: i64| env(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        c.storage = Some(corntchap::files::StorageConfig {
            public_endpoint: env("S3_PUBLIC_ENDPOINT").unwrap_or_else(|| endpoint.clone()),
            endpoint,
            bucket,
            region: env("S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            access_key: env("S3_ACCESS_KEY").unwrap_or_default(),
            secret_key: env("S3_SECRET_KEY").unwrap_or_default(),
            path_style: env("S3_PATH_STYLE").is_none_or(|v| v != "false" && v != "0"),
            create_bucket: env("S3_CREATE_BUCKET").is_some_and(|v| v == "true" || v == "1"),
            max_bytes: num("FILES_MAX_BYTES", corntchap::files::DEFAULT_MAX_BYTES),
            inline_max_bytes: num(
                "FILES_INLINE_MAX_BYTES",
                corntchap::files::DEFAULT_INLINE_MAX_BYTES,
            ),
        });
    }
    Ok(c)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // sqlx's "relation _sqlx_migrations already exists, skipping" notice is noise.
                .unwrap_or_else(|_| "info,sqlx::postgres::notice=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let cfg = config(&cli)?;
    if let Some(origin) = cfg.storage.as_ref().and_then(|s| s.public_origin()) {
        corntchap::console::allow_images_from(&origin);
    }
    let pool = app::connect(&cfg).await?;
    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Migrate => {
            app::migrate(&pool).await?;
            println!("schema is up to date");
        }
        Cmd::Init { admin: login } => {
            let (name, token) = admin::init_admin(&pool, &login).await?;
            println!("admin: {name}");
            if let Some(t) = token {
                println!("bootstrap token (shown once, valid 7 days): {t}");
                println!(
                    "sign in:  corntchap-cli login {} --token <token>   (`just init` does this for you)",
                    cfg.public_url
                );
            }
        }
        Cmd::Token { name, hours } => {
            println!(
                "{}",
                admin::operator_token(&pool, &name, hours.map(|h| h * 3600)).await?
            );
        }
        Cmd::Serve => {
            if env("AUTO_MIGRATE").as_deref() == Some("1") {
                app::migrate(&pool).await?;
            }
            if cfg.oidc.is_none() {
                tracing::warn!(
                    "OIDC_ISSUER/OIDC_CLIENT_ID/OIDC_CLIENT_SECRET not set: browser sign-in and device approval are disabled"
                );
            }
            let bind = cfg.bind.clone();
            let state = AppState::new(pool, cfg);
            let router = app::build(state.clone())?;
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("listening on {bind}");
            // The first signal starts a graceful shutdown; a second one force-exits. One task
            // owns the signal streams so the second is never lost to a race with a spawn.
            let (stop, stopped) = tokio::sync::watch::channel(false);
            tokio::spawn({
                let stop = stop.clone();
                async move {
                    next_signal().await;
                    let _ = stop.send(true);
                    // End the endless streams so the drain below has nothing left to wait for.
                    state.shut_down();
                    next_signal().await;
                    tracing::warn!("second signal: forcing exit");
                    std::process::exit(130);
                }
            });
            let mut graceful = stopped.clone();
            let served = axum::serve(listener, router).with_graceful_shutdown(async move {
                let _ = graceful.wait_for(|stopping| *stopping).await;
                tracing::info!("shutting down");
            });
            supervise(SHUTDOWN_GRACE, served.into_future(), stopped).await?;
        }
    }
    Ok(())
}

/// How long a graceful shutdown may spend draining the connections still open.
///
/// `/relay/stream` and `/events/stream` are endless SSE streams by design, so a connected
/// relay would otherwise hold a drain forever. **The bound applies only after a signal has
/// started the shutdown** — never to the server's normal running life.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Run `served` until it stops by itself, or until `stop` reports a signal — then give it
/// `grace` to drain and abandon whatever is still open.
///
/// Nothing bounds the server before the signal: it runs until asked to stop.
async fn supervise<F>(
    grace: Duration,
    served: F,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = std::io::Result<()>>,
{
    let mut served = std::pin::pin!(served);
    let signalled = async move {
        let _ = stop.wait_for(|stopping| *stopping).await;
    };
    tokio::pin!(signalled);
    tokio::select! {
        res = &mut served => return res,
        _ = &mut signalled => {}
    }
    match tokio::time::timeout(grace, &mut served).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!(
                "graceful shutdown did not finish within {grace:?}: abandoning the \
                 connections still open (the SSE endpoints are endless by design)"
            );
            Ok(())
        }
    }
}

/// Resolves on the next SIGINT or SIGTERM.
async fn next_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::watch;

    /// Regression: bounding the whole serve future made every server exit one grace period
    /// after starting, whether or not anything asked it to stop.
    #[tokio::test]
    async fn supervise_keeps_running_without_a_signal() {
        let (_tx, rx) = watch::channel(false);
        let endless = std::future::pending::<std::io::Result<()>>();
        let ran = tokio::time::timeout(
            Duration::from_millis(50),
            supervise(Duration::from_millis(20), endless, rx),
        )
        .await;
        assert!(
            ran.is_err(),
            "the server must keep running until a signal asks it to stop"
        );
    }

    /// The bug the bound exists for: an endless SSE client must not hold the drain forever.
    #[tokio::test]
    async fn supervise_abandons_a_stream_that_never_drains() {
        let (tx, rx) = watch::channel(false);
        let endless = std::future::pending::<std::io::Result<()>>();
        tokio::spawn(async move {
            let _ = tx.send(true);
        });
        let started = std::time::Instant::now();
        supervise(Duration::from_millis(20), endless, rx)
            .await
            .expect("abandoning is not an error");
        assert!(
            started.elapsed() >= Duration::from_millis(20),
            "the drain gets its grace, then is abandoned"
        );
    }

    #[tokio::test]
    async fn supervise_returns_as_soon_as_the_server_stops() {
        let (_tx, rx) = watch::channel(false);
        let done = async { Ok::<(), std::io::Error>(()) };
        tokio::time::timeout(
            Duration::from_secs(5),
            supervise(Duration::from_secs(30), done, rx),
        )
        .await
        .expect("a server that stopped is not waited on")
        .expect("no error");
    }

    #[tokio::test]
    async fn supervise_propagates_a_serve_error() {
        let (_tx, rx) = watch::channel(false);
        let failing = async { Err::<(), std::io::Error>(std::io::Error::other("bind")) };
        let err = supervise(Duration::from_secs(5), failing, rx)
            .await
            .expect_err("io errors surface");
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
    }
}
