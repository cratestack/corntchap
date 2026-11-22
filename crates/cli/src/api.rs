//! Thin REST client and credential storage.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Credentials {
    pub server: Option<String>,
    pub token: Option<String>,
    pub relay_token: Option<String>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("CORNTCHAP_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corntchap")
}

fn cred_path() -> PathBuf {
    config_dir().join("credentials.toml")
}

impl Credentials {
    pub fn load() -> Credentials {
        let mut c: Credentials = fs::read_to_string(cred_path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        if let Ok(v) = std::env::var("CORNTCHAP_SERVER") {
            c.server = Some(v);
        }
        if let Ok(v) = std::env::var("CORNTCHAP_TOKEN") {
            c.token = Some(v);
        }
        if let Ok(v) = std::env::var("CORNTCHAP_RELAY_TOKEN") {
            c.relay_token = Some(v);
        }
        c
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        fs::create_dir_all(&dir)?;
        let path = cred_path();
        // Persist only what is stored on disk, not environment overrides.
        let mut disk: Credentials = fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        if self.server.is_some() {
            disk.server = self.server.clone();
        }
        if self.token.is_some() {
            disk.token = self.token.clone();
        }
        if self.relay_token.is_some() {
            disk.relay_token = self.relay_token.clone();
        }
        write_creds(&path, &disk)
    }

    /// Remove stored tokens from credentials.toml (not environment overrides); returns what was on disk.
    pub fn clear(user: bool, relay: bool) -> Result<Credentials> {
        let path = cred_path();
        let mut disk: Credentials = fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        let before = disk.clone();
        if user {
            disk.token = None;
        }
        if relay {
            disk.relay_token = None;
        }
        if path.exists() {
            write_creds(&path, &disk)?;
        }
        Ok(before)
    }

    /// Delete only `relay_token` from credentials.toml (the user's login stays). Returns whether one
    /// was stored. A file that cannot be parsed is left alone and reported rather than overwritten.
    pub fn drop_relay_token() -> Result<bool> {
        let path = cred_path();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut disk: Credentials = toml::from_str(&text).map_err(|e| {
            anyhow!(
                "{} is not valid TOML ({e}); left untouched, remove relay_token by hand",
                path.display()
            )
        })?;
        if disk.relay_token.take().is_none() {
            return Ok(false);
        }
        write_creds(&path, &disk)?;
        Ok(true)
    }
}

/// Write via a temp file and rename, so a crash never leaves a half-written credentials file.
fn write_creds(path: &std::path::Path, c: &Credentials) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(c)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// The server answered 401: the stored credential is expired, revoked or from another database.
#[derive(Debug)]
pub struct Unauthorized(pub String);

impl std::fmt::Display for Unauthorized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unauthorized {}

#[derive(Clone)]
pub struct Api {
    pub relay: bool,
    pub server: String,
    pub token: Option<String>,
    pub http: reqwest::Client,
}

impl Api {
    pub fn new(server: &str, token: Option<String>) -> Api {
        Api {
            relay: false,
            server: server.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::builder()
                .user_agent(concat!("corntchap-cli/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap(),
        }
    }

    /// Credentials from disk/env; `relay` picks the relay token instead of the user's.
    pub fn from_creds(relay: bool) -> Result<Api> {
        let c = Credentials::load();
        let server = c
            .server
            .ok_or_else(|| anyhow!("not logged in: run `corntchap-cli login <server-url>`"))?;
        let token = if relay { c.relay_token } else { c.token };
        if token.is_none() {
            bail!(if relay {
                "no relay token: run `corntchap-cli fire enroll <name>`"
            } else {
                "not logged in: run `corntchap-cli login <server-url>`"
            });
        }
        let mut api = Api::new(&server, token);
        api.relay = relay;
        Ok(api)
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self
            .http
            .request(m, format!("{}/api/v1{}", self.server, path));
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        r
    }

    pub async fn send(&self, r: reqwest::RequestBuilder) -> Result<Value> {
        let resp = r.send().await.context("request failed")?;
        let status = resp.status();
        let text = resp.text().await?;
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({"message": text}));
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let server = &self.server;
            return Err(Unauthorized(if self.relay {
                format!(
                    "the server rejected your relay credential for {server} (expired, revoked, or the dev database was reset). Run `corntchap-cli fire enroll <name>`"
                )
            } else {
                format!(
                    "the server rejected your stored credential for {server} (expired, revoked, or the dev database was reset). Run `corntchap-cli login {server}`"
                )
            })
            .into());
        }
        if !status.is_success() {
            let code = v["error"].as_str().unwrap_or("");
            let msg = v["message"].as_str().unwrap_or(&text);
            if code.is_empty() || code == "error" {
                bail!("{msg} (HTTP {status})");
            }
            bail!("{code}: {msg} (HTTP {status})");
        }
        Ok(v)
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.send(self.req(reqwest::Method::GET, path)).await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.send(self.req(reqwest::Method::POST, path).json(&body))
            .await
    }
    pub async fn patch(&self, path: &str, body: Value) -> Result<Value> {
        self.send(self.req(reqwest::Method::PATCH, path).json(&body))
            .await
    }
    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.send(self.req(reqwest::Method::DELETE, path)).await
    }
    pub fn raw(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.req(m, path)
    }
    /// Call a generated procedure (`/api/v2/$procs/<name>`) with its JSON arguments.
    pub async fn procedure(&self, name: &str, args: Value) -> Result<Value> {
        let mut r = self
            .http
            .post(format!("{}/api/v2/$procs/{name}", self.server))
            .json(&args);
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        self.send(r).await
    }
}

/// RFC 8628 device flow: prints the code, waits for a human to approve it, returns the access token.
pub async fn device_login(
    server: &str,
    kind: &str,
    name: Option<&str>,
    extra: Value,
) -> Result<(String, Value)> {
    let http = reqwest::Client::new();
    let server = server.trim_end_matches('/');
    let mut body = json!({"kind": kind});
    if let Some(n) = name {
        body["name"] = json!(n);
    }
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    let r: Value = http
        .post(format!("{server}/oauth/device/code"))
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("device authorization request")?
        .json()
        .await?;
    let dc = r["device_code"]
        .as_str()
        .ok_or_else(|| anyhow!("bad device response"))?
        .to_string();
    let url = r["verification_uri_complete"].as_str().unwrap_or("");
    eprintln!(
        "\nOpen  {url}\nand approve code  {}\n",
        r["user_code"].as_str().unwrap_or("")
    );
    open_browser(url);
    let mut interval = r["interval"].as_u64().unwrap_or(5);
    let deadline =
        std::time::Instant::now() + Duration::from_secs(r["expires_in"].as_u64().unwrap_or(600));
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if std::time::Instant::now() > deadline {
            bail!("the code expired; run the command again");
        }
        let resp = http
            .post(format!("{server}/oauth/token"))
            .json(&json!({"grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": dc}))
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await?;
        if status.is_success() {
            let t = v["access_token"]
                .as_str()
                .ok_or_else(|| anyhow!("no token in response"))?
                .to_string();
            return Ok((t, v["principal"].clone()));
        }
        match v["error"].as_str().unwrap_or("") {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            "access_denied" => bail!("the request was denied"),
            "expired_token" => bail!("the code expired; run the command again"),
            other => bail!("device login failed: {other}"),
        }
    }
}

fn open_browser(url: &str) {
    if url.is_empty() || std::env::var("CORNTCHAP_NO_BROWSER").is_ok() {
        return;
    }
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        // not `cmd /C start`: cmd would reinterpret `&` in the URL
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = cmd
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
