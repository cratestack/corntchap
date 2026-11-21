use std::future::Future;

use axum::{Router, middleware, routing::get};
use cratestack::mcp::{ProtectedResource, StreamableHttpServer};
use cratestack::{AuthProvider, CratestackContext, CratestackError, RequestContext};

use crate::{
    auth, jobs, procs,
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
    let registry = procs::Procedures {
        state: state.clone(),
    };
    let tools = cratestack_schema::mcp::tools(db, registry, ());
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

    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/board") }),
        )
        .nest("/api/v1", crate::rest::routes())
        .merge(web::routes())
        .with_state(state.clone())
        .nest_service("/mcp", mcp.service())
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
