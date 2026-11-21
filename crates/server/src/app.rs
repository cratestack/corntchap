use std::future::Future;

use axum::{Router, middleware, routing::get};
use cratestack::mcp::{ProtectedResource, StreamableHttpServer};
use cratestack::{AuthProvider, CratestackContext, CratestackError, RequestContext};

use crate::{
    auth, jobs, mcp_compat, procs,
    schema::cratestack_schema,
    state::{AppState, Config},
    web,
};

/// cratestack's MCP guard asks this provider who the caller is: same tokens as REST.
#[derive(Clone)]
pub struct TokenAuth {
    pub state: AppState,
}

impl AuthProvider for TokenAuth {
    type Error = CratestackError;

    fn authenticate(
        &self,
        request: &RequestContext<'_>,
    ) -> impl Future<Output = Result<CratestackContext, Self::Error>> + Send {
        let state = self.state.clone();
        let headers = request.headers.clone();
        async move {
            let p = auth::authenticate_headers(&state, &headers)
                .await
                .map_err(procs::map_err)?;
            Ok(CratestackContext::authenticated(procs::ctx_fields(&p)))
        }
    }
}

pub async fn connect(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&config.database_url)
        .await?)
}

pub async fn migrate(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

pub fn build(state: AppState) -> anyhow::Result<Router> {
    let db = cratestack_schema::Cratestack::builder(state.db.clone()).build();
    // The generated REST surface (models + procedure routes) under /api/v2, same bearer tokens.
    let generated = cratestack_schema::axum::router(
        db.clone(),
        procs::Procedures {
            state: state.clone(),
        },
        (),
        cratestack::JsonCodec,
        TokenAuth {
            state: state.clone(),
        },
        cratestack::DEFAULT_BODY_LIMIT_BYTES,
    );
    let registry = procs::Procedures {
        state: state.clone(),
    };
    let compat_tools = cratestack_schema::mcp::tools(db.clone(), registry.clone(), ());
    let tools = cratestack_schema::mcp::tools(db, registry, ());
    // We are our own authorization server: only our opaque tokens (device flow) are accepted,
    // never the IdP's JWTs. RFC 8414 metadata at /.well-known/oauth-authorization-server says so.
    let resource = ProtectedResource::new(
        format!("{}/mcp", state.config.public_url),
        [state.config.public_url.clone()],
    );
    let mcp = StreamableHttpServer::builder(
        tools,
        TokenAuth {
            state: state.clone(),
        },
        [state.config.public_url.clone()],
        resource,
    )
    .build()?;

    // /mcp: handshake-era clients (2025-11-25 and older, no 2026-07-28 header) are answered by
    // the compat layer on the same generated tool table; everything else goes to cratestack.
    let compat = mcp_compat::Compat::new(compat_tools, state.clone());
    let mcp_router =
        Router::new()
            .fallback_service(mcp.service())
            .layer(middleware::from_fn_with_state(
                compat,
                mcp_compat::front_door::<_>,
            ));

    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/.well-known/oauth-authorization-server",
            get(
                |axum::extract::State(st): axum::extract::State<AppState>| async move {
                    let base = &st.config.public_url;
                    axum::Json(serde_json::json!({
                        "issuer": base,
                        "device_authorization_endpoint": format!("{base}/oauth/device/code"),
                        "token_endpoint": format!("{base}/oauth/token"),
                        "grant_types_supported": [crate::web::DEVICE_GRANT],
                        "token_endpoint_auth_methods_supported": ["none"],
                        "response_types_supported": [],
                    }))
                },
            ),
        )
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/board") }),
        )
        .nest("/api/v1", crate::rest::routes())
        .merge(web::routes())
        .with_state(state.clone())
        .nest("/api/v2", generated)
        .nest_service("/mcp", mcp_router)
        .merge(mcp.metadata_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_tls,
        ));

    if state.config.run_background {
        jobs::spawn_listener(state.clone());
        jobs::spawn_leader(state);
    }
    Ok(router)
}
