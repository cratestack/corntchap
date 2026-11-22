#![allow(dead_code)]
use axum::{
    Json, Router,
    extract::Query,
    response::Redirect,
    routing::{get, post},
};
use corntchap::state::{AppState, Config, OidcConfig, Principal};
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

/// Who the mock IdP signs in next.
#[derive(Clone)]
pub struct Ident {
    pub sub: String,
    pub username: String,
    pub email: String,
    pub email_verified: bool,
    /// The `name` claim; `None` leaves it out.
    pub name: Option<String>,
    pub admin_role: bool,
}

impl Default for Ident {
    fn default() -> Self {
        Ident {
            sub: "sub-4242".into(),
            username: "Octo".into(),
            email: "octo@example.test".into(),
            email_verified: true,
            name: Some("Octo Cat".into()),
            admin_role: false,
        }
    }
}

pub type Who = std::sync::Arc<std::sync::Mutex<Ident>>;

pub struct TestApp {
    pub base: String,
    pub state: AppState,
    pub http: reqwest::Client,
    pub gh: String,
    pub who: Who,
}

/// A tiny OpenID provider: discovery, authorize (bounces straight back), token (RS256 ID token), JWKS.
pub async fn mock_oidc(who: Who) -> String {
    use axum::extract::State;
    use std::sync::{Arc, Mutex};
    type Codes = Arc<Mutex<HashMap<String, String>>>; // code -> nonce
    #[derive(Clone)]
    struct Idp {
        base: String,
        codes: Codes,
        who: Who,
    }
    async fn discovery(State(i): State<Idp>) -> Json<Value> {
        Json(json!({
            "issuer": i.base,
            "authorization_endpoint": format!("{}/authorize", i.base),
            "token_endpoint": format!("{}/token", i.base),
            "jwks_uri": format!("{}/jwks", i.base),
            "end_session_endpoint": format!("{}/end-session", i.base),
            "response_types_supported": ["code"],
            "id_token_signing_alg_values_supported": ["RS256"],
        }))
    }
    async fn authorize(State(i): State<Idp>, Query(q): Query<HashMap<String, String>>) -> Redirect {
        let code = format!("code-{}", uuid::Uuid::new_v4().simple());
        i.codes
            .lock()
            .unwrap()
            .insert(code.clone(), q.get("nonce").cloned().unwrap_or_default());
        Redirect::to(&format!(
            "{}?code={code}&state={}",
            q["redirect_uri"], q["state"]
        ))
    }
    async fn token(
        State(i): State<Idp>,
        axum::Form(f): axum::Form<HashMap<String, String>>,
    ) -> Json<Value> {
        let nonce = i
            .codes
            .lock()
            .unwrap()
            .remove(&f["code"])
            .expect("known code");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let w = i.who.lock().unwrap().clone();
        let mut claims = json!({
            "iss": i.base, "aud": "cid", "sub": w.sub, "iat": now, "exp": now + 300, "nonce": nonce,
            "preferred_username": w.username, "email": w.email, "email_verified": w.email_verified,
        });
        if let Some(n) = &w.name {
            claims["name"] = json!(n);
        }
        if w.admin_role {
            claims["realm_access"] = json!({"roles": ["offline_access", "corntchap-admin"]});
        }
        let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        h.kid = Some("k1".into());
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("test_rsa.pem")).unwrap();
        let id_token = jsonwebtoken::encode(&h, &claims, &key).unwrap();
        Json(
            json!({"access_token": "at", "token_type": "Bearer", "expires_in": 300, "id_token": id_token}),
        )
    }
    async fn jwks() -> Json<Value> {
        Json(
            json!({"keys": [{"kty": "RSA", "kid": "k1", "use": "sig", "alg": "RS256",
            "n": include_str!("test_rsa_n.txt").trim(), "e": "AQAB"}]}),
        )
    }
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let idp = Idp {
        base: base.clone(),
        codes: Default::default(),
        who,
    };
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/jwks", get(jwks))
        .route("/end-session", get(|| async { "bye" }))
        .with_state(idp);
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

pub async fn start(background: bool) -> TestApp {
    let (pool, url) = fresh_db().await;
    let who: Who = Default::default();
    let gh = mock_oidc(who.clone()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut cfg = Config::new(url, base.clone(), "127.0.0.1:0".into());
    cfg.oidc = Some(OidcConfig {
        issuer: gh.clone(),
        client_id: "cid".into(),
        client_secret: "csecret".into(),
        public_issuer: None,
        admin_role: "corntchap-admin".into(),
    });
    cfg.run_background = background;
    // File storage when an S3 endpoint is configured (RustFS from `just infra`, a service in CI):
    // one throwaway bucket per test app.
    if let (Ok(endpoint), Ok(key), Ok(secret)) = (
        std::env::var("S3_ENDPOINT"),
        std::env::var("S3_ACCESS_KEY"),
        std::env::var("S3_SECRET_KEY"),
    ) {
        cfg.storage = Some(corntchap::files::StorageConfig {
            public_endpoint: endpoint.clone(),
            endpoint,
            bucket: format!("t{}", uuid::Uuid::new_v4().simple()),
            region: "us-east-1".into(),
            access_key: key,
            secret_key: secret,
            path_style: true,
            create_bucket: true,
            max_bytes: 1024 * 1024,
            inline_max_bytes: 64 * 1024,
        });
    }
    let state = AppState::new(pool, cfg);
    if let Some(st) = &state.storage {
        st.ensure_bucket().await.expect("create the test bucket");
    }
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
        who,
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

    /// Choose who the mock IdP signs in next.
    pub fn sign_in_as(&self, sub: &str, username: &str, admin_role: bool) {
        *self.who.lock().unwrap() = Ident {
            sub: sub.into(),
            username: username.into(),
            email: format!("{username}@example.test"),
            admin_role,
            ..Ident::default()
        };
    }

    /// Choose exactly what the mock IdP signs in next.
    pub fn sign_in_with(&self, ident: Ident) {
        *self.who.lock().unwrap() = ident;
    }

    /// A person who exists already (what a first sign-in would have created), with this handle.
    pub async fn add_user(&self, handle: &str) {
        sqlx::query(
            "with p as (insert into principals (kind, name) values ('user', $1) returning id, name)
             insert into handle_claims (name, principal_id) select name, id from p",
        )
        .bind(handle)
        .execute(&self.state.db)
        .await
        .unwrap();
    }

    pub async fn principal(&self, handle: &str) -> Principal {
        let (id, is_admin): (uuid::Uuid, bool) =
            sqlx::query_as("select id, is_admin from principals where name = $1")
                .bind(handle)
                .fetch_one(&self.state.db)
                .await
                .unwrap();
        Principal {
            id,
            kind: "user".into(),
            name: handle.into(),
            owner_id: None,
            is_admin,
            max_leases: 1,
            runtime: None,
            agent_kind: None,
            token_id: None,
            grants: vec![],
        }
    }

    /// The handle of the person with this OIDC subject.
    pub async fn handle_of(&self, sub: &str) -> String {
        sqlx::query_scalar("select name from principals where oidc_sub = $1")
            .bind(sub)
            .fetch_one(&self.state.db)
            .await
            .unwrap()
    }

    /// Sign in through the mock OIDC provider and return the session cookie header value.
    pub async fn browser_login(&self, next: &str) -> String {
        let r = self
            .http
            .get(format!("{}/auth/oidc/login?next={next}", self.base))
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
