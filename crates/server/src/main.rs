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
            let router = app::build(state)?;
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            tracing::info!("listening on {bind}");
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }
    Ok(())
}

/// Resolves on SIGINT or (on unix) SIGTERM, so Kubernetes can stop the pod cleanly.
async fn shutdown_signal() {
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
    tracing::info!("shutting down");
}
