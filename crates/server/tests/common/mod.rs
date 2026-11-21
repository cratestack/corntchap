#![allow(dead_code)]
use axum::{
    Json, Router,
    extract::Query,
    response::Redirect,
    routing::{get, post},
};
use corntchap::state::{AppState, Config, GithubConfig};
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn admin_url() -> String {
    std::env::var("TEST_DATABASE_ADMIN_URL")
        .unwrap_or_else(|_| "postgres://corn:corn@127.0.0.1/postgres".into())
}

/// A fresh, migrated database per test.
pub async fn fresh_db() -> (sqlx::PgPool, String) {
    let admin = sqlx::PgPool::connect(&admin_url())
        .await
        .expect("admin connection (set TEST_DATABASE_ADMIN_URL)");
    let name = format!("ct_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let url = admin_url().rsplit_once('/').unwrap().0.to_string() + "/" + &name;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(30)
        .connect(&url)
        .await
        .unwrap();
    corntchap::app::migrate(&pool).await.unwrap();
    (pool, url)
}

pub struct TestApp {
    pub base: String,
    pub state: AppState,
    pub http: reqwest::Client,
    pub gh: String,
}

/// A tiny stand-in for GitHub: authorize bounces straight back, token and user are canned.
pub async fn mock_github() -> String {
    async fn authorize(Query(q): Query<HashMap<String, String>>) -> Redirect {
        Redirect::to(&format!(
            "{}?code=mockcode&state={}",
            q["redirect_uri"], q["state"]
        ))
    }
    let app = Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(|| async { Json(json!({"access_token": "gho_mock", "token_type": "bearer", "scope": "user:email"})) }))
        .route("/user", get(|| async { Json(json!({"id": 4242, "login": "Octo", "email": null})) }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

pub async fn start(background: bool) -> TestApp {
    let (pool, url) = fresh_db().await;
    let gh = mock_github().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut cfg = Config::new(url, base.clone(), "127.0.0.1:0".into());
    let mut g = GithubConfig::new("cid".into(), "csecret".into());
    g.authorize_url = format!("{gh}/authorize");
    g.token_url = format!("{gh}/token");
    g.user_url = format!("{gh}/user");
    cfg.github = Some(g);
    cfg.run_background = background;
    let state = AppState::new(pool, cfg);
    let app = corntchap::app::build(state.clone()).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    TestApp {
        base,
        state,
        http,
        gh,
    }
}

impl TestApp {
    pub async fn rest(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut r = self
            .http
            .request(method, format!("{}/api/v1{path}", self.base))
            .bearer_auth(token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.unwrap();
        let st = resp.status().as_u16();
        (st, resp.json().await.unwrap_or(Value::Null))
    }
    pub async fn post(&self, token: &str, path: &str, body: Value) -> (u16, Value) {
        self.rest(token, reqwest::Method::POST, path, Some(body))
            .await
    }
    pub async fn get(&self, token: &str, path: &str) -> (u16, Value) {
        self.rest(token, reqwest::Method::GET, path, None).await
    }

    /// One MCP tools/call (2026-07-28 stateless revision). Returns the `result` object.
    pub async fn mcp(&self, token: &str, tool: &str, args: Value) -> Value {
        let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}});
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args, "_meta": meta}});
        let r = self
            .http
            .post(format!("{}/mcp", self.base))
            .bearer_auth(token)
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call")
            .header("Mcp-Name", tool)
            .header("accept", "application/json, text/event-stream")
            .json(&body)
            .send()
            .await
            .unwrap();
        let v: Value = r.json().await.unwrap();
        v["result"].clone()
    }

    /// Parsed JSON payload of a successful tool result (or the error text).
    pub fn payload(result: &Value) -> Value {
        let text = result["content"][0]["text"].as_str().unwrap_or("null");
        serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
    }

    /// Sign in through the mock GitHub and return the session cookie header value.
    pub async fn browser_login(&self, next: &str) -> String {
        let r = self
            .http
            .get(format!("{}/auth/github/login?next={next}", self.base))
            .send()
            .await
            .unwrap();
        let nonce = r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let to_gh = r.headers()["location"].to_str().unwrap().to_string();
        let r = self.http.get(&to_gh).send().await.unwrap();
        let back = r.headers()["location"].to_str().unwrap().to_string();
        let r = self
            .http
            .get(&back)
            .header("cookie", &nonce)
            .send()
            .await
            .unwrap();
        assert_eq!(
            r.status().as_u16(),
            303,
            "callback should redirect, got {}",
            r.status()
        );
        let cookie = r.headers()["set-cookie"].to_str().unwrap();
        cookie.split(';').next().unwrap().to_string()
    }
}
