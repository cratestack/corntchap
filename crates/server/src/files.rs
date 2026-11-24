//! Files: uploads to S3-compatible storage (RustFS in dev, any S3 in prod), one pot at a time.
//!
//! An agent asks for an upload slot (`create_upload`) and gets a presigned PUT plus ready-made
//! `curl` commands (foreground and background) and the file's permanent link, shareable at
//! once. It uploads straight to storage; nobody has to confirm: the leader's watcher sees the
//! object arrive (HEAD), checks its size and marks it ready (`file.ready`), or failed. Links
//! (`/files/PAY-F12/name`) always go through corntchap's login and the pot's visibility, then
//! redirect to a short-lived presigned GET. Small files can be sent inline (`upload_file`), and
//! the console streams its uploads through the server.

use std::{sync::Arc, time::Duration};

use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use url::Url;
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    service::{emit, parse_task_ref, project_by_key, task_by_ref},
    state::{AppState, Principal},
};

/// How long a presigned upload URL stays valid (the PUT must start before; it may then run
/// for as long as it takes).
const UPLOAD_URL_TTL: Duration = Duration::from_secs(60 * 60);
/// How long a download redirect stays valid.
const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(10 * 60);
/// An upload that never arrived is marked failed after this long.
const UPLOAD_DEADLINE_HOURS: i64 = 24;

#[derive(Clone, Debug)]
pub struct StorageConfig {
    /// Where the server reaches storage (e.g. http://rustfs:9000 inside compose).
    pub endpoint: String,
    /// Where agents and browsers reach it (presigned URLs are signed for this host).
    pub public_endpoint: String,
    pub bucket: String,
    /// Put in front of every object key (`corntchap/files/`), for a bucket shared with other data;
    /// empty or ending in `/` (see [`key_prefix`]).
    pub prefix: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// Path-style URLs (http://host/bucket/key): RustFS, MinIO. Virtual-host style for AWS.
    pub path_style: bool,
    /// Create the bucket at start-up (dev).
    pub create_bucket: bool,
    pub max_bytes: i64,
    pub inline_max_bytes: i64,
}

/// `S3_PREFIX` as a key prefix: slashes at either end and empty segments dropped, then one `/`
/// appended (`/corntchap//files` -> `corntchap/files/`); blank means the bucket root. `.` and `..`
/// segments are refused: URL path resolution would move them out of the prefix, even into another
/// bucket. So are control characters and segments with spaces around them.
pub fn key_prefix(raw: &str) -> anyhow::Result<String> {
    let segs: Vec<&str> = raw.split('/').filter(|s| !s.is_empty()).collect();
    for s in &segs {
        anyhow::ensure!(
            *s != "." && *s != "..",
            "S3_PREFIX {raw:?}: `.` and `..` segments would leave the prefix"
        );
        anyhow::ensure!(
            !s.chars().any(char::is_control) && s.trim() == *s,
            "S3_PREFIX {raw:?}: no control characters or spaces around a segment"
        );
    }
    Ok(if segs.is_empty() {
        String::new()
    } else {
        format!("{}/", segs.join("/"))
    })
}

impl StorageConfig {
    /// File storage from `S3_*` / `FILES_*` (via `get`, which treats empty values as unset); off
    /// unless both `S3_ENDPOINT` and `S3_BUCKET` are set. A bad `S3_PREFIX` stops the server.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        let (Some(endpoint), Some(bucket)) = (get("S3_ENDPOINT"), get("S3_BUCKET")) else {
            return Ok(None);
        };
        let num = |k: &str, d: i64| get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        Ok(Some(Self {
            public_endpoint: get("S3_PUBLIC_ENDPOINT").unwrap_or_else(|| endpoint.clone()),
            endpoint,
            bucket,
            prefix: key_prefix(&get("S3_PREFIX").unwrap_or_default())?,
            region: get("S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            access_key: get("S3_ACCESS_KEY").unwrap_or_default(),
            secret_key: get("S3_SECRET_KEY").unwrap_or_default(),
            path_style: get("S3_PATH_STYLE").is_none_or(|v| v != "false" && v != "0"),
            create_bucket: get("S3_CREATE_BUCKET").is_some_and(|v| v == "true" || v == "1"),
            max_bytes: num("FILES_MAX_BYTES", DEFAULT_MAX_BYTES),
            inline_max_bytes: num("FILES_INLINE_MAX_BYTES", DEFAULT_INLINE_MAX_BYTES),
        }))
    }
}

pub const DEFAULT_MAX_BYTES: i64 = 1024 * 1024 * 1024;
pub const DEFAULT_INLINE_MAX_BYTES: i64 = 5 * 1024 * 1024;

impl StorageConfig {
    /// The origin browsers fetch presigned URLs from (scheme://host[:port]), for the console's CSP.
    pub fn public_origin(&self) -> Option<String> {
        let u = Url::parse(&self.public_endpoint).ok()?;
        let host = u.host_str()?;
        let host = if self.path_style {
            host.to_string()
        } else {
            format!("{}.{host}", self.bucket)
        };
        Some(match u.port() {
            Some(p) => format!("{}://{host}:{p}", u.scheme()),
            None => format!("{}://{host}", u.scheme()),
        })
    }
}

pub struct Storage {
    pub cfg: StorageConfig,
    internal: Bucket,
    public: Bucket,
    creds: Credentials,
    http: reqwest::Client,
}

impl Storage {
    pub fn new(cfg: StorageConfig) -> anyhow::Result<Self> {
        let style = if cfg.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket = |ep: &str| -> anyhow::Result<Bucket> {
            Ok(Bucket::new(
                Url::parse(ep)?,
                style,
                cfg.bucket.clone(),
                cfg.region.clone(),
            )?)
        };
        Ok(Storage {
            internal: bucket(&cfg.endpoint)?,
            public: bucket(&cfg.public_endpoint)?,
            creds: Credentials::new(cfg.access_key.clone(), cfg.secret_key.clone()),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()?,
            cfg,
        })
    }

    /// Create the bucket if it is missing (dev).
    pub async fn ensure_bucket(&self) -> anyhow::Result<()> {
        let head = self
            .internal
            .head_bucket(Some(&self.creds))
            .sign(Duration::from_secs(60));
        let r = self.http.head(head).send().await?;
        if r.status().is_success() {
            return Ok(());
        }
        let url = self
            .internal
            .create_bucket(&self.creds)
            .sign(Duration::from_secs(60));
        let r = self.http.put(url).send().await?;
        let st = r.status();
        if !st.is_success() && st.as_u16() != 409 {
            anyhow::bail!(
                "creating bucket {}: {st} {}",
                self.cfg.bucket,
                r.text().await.unwrap_or_default()
            );
        }
        tracing::info!("storage bucket {} ready", self.cfg.bucket);
        Ok(())
    }

    /// A presigned PUT for agents (public endpoint), bound to the content type.
    pub fn upload_url(&self, key: &str, content_type: &str) -> String {
        let mut a = self.public.put_object(Some(&self.creds), key);
        a.headers_mut()
            .insert("content-type", content_type.to_string());
        a.sign(UPLOAD_URL_TTL).to_string()
    }

    /// A presigned GET for downloads (public endpoint).
    pub fn download_url(&self, key: &str, name: &str, content_type: &str) -> String {
        let mut a = self.public.get_object(Some(&self.creds), key);
        let inline = content_type.starts_with("image/")
            || content_type == "application/pdf"
            || content_type.starts_with("text/plain")
            || content_type.starts_with("video/")
            || content_type.starts_with("audio/");
        let disposition = format!(
            "{}; filename=\"{}\"",
            if inline { "inline" } else { "attachment" },
            name.replace(['"', '\\', '\r', '\n'], "_")
        );
        a.query_mut()
            .insert("response-content-disposition", disposition);
        a.sign(DOWNLOAD_URL_TTL).to_string()
    }

    /// Size of a stored object, or None when it is not there (yet).
    pub async fn size_of(&self, key: &str) -> anyhow::Result<Option<i64>> {
        let url = self
            .internal
            .head_object(Some(&self.creds), key)
            .sign(Duration::from_secs(60));
        let r = self.http.head(url).send().await?;
        match r.status().as_u16() {
            200 => Ok(r
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())),
            404 => Ok(None),
            s => anyhow::bail!("HEAD {key}: {s}"),
        }
    }

    /// Store bytes (inline uploads) or a stream with a known length (console uploads).
    pub async fn put(
        &self,
        key: &str,
        content_type: &str,
        length: u64,
        body: reqwest::Body,
    ) -> anyhow::Result<()> {
        let mut a = self.internal.put_object(Some(&self.creds), key);
        a.headers_mut()
            .insert("content-type", content_type.to_string());
        let url = a.sign(Duration::from_secs(60 * 60));
        let r = self
            .http
            .put(url)
            .header("content-type", content_type)
            .header("content-length", length)
            .body(body)
            .send()
            .await?;
        if !r.status().is_success() {
            anyhow::bail!(
                "PUT {key}: {} {}",
                r.status(),
                r.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let url = self
            .internal
            .delete_object(Some(&self.creds), key)
            .sign(Duration::from_secs(60));
        let r = self.http.delete(url).send().await?;
        if !r.status().is_success() && r.status().as_u16() != 404 {
            anyhow::bail!("DELETE {key}: {}", r.status());
        }
        Ok(())
    }
}

fn storage(state: &AppState) -> Result<&Arc<Storage>> {
    state.storage.as_ref().ok_or_else(|| {
        Error::Invalid(
            "file storage is not configured on this server (S3_ENDPOINT, S3_BUCKET, …)".into(),
        )
    })
}

/// `PAY-F12` -> ("PAY", 12)
pub fn parse_ref(s: &str) -> Result<(String, i64)> {
    let bad = || Error::Invalid(format!("bad file id {s:?}, expected like PAY-F12"));
    let (k, n) = s
        .trim()
        .rsplit_once("-F")
        .or_else(|| s.trim().rsplit_once("-f"))
        .ok_or_else(bad)?;
    let n: i64 = n.parse().map_err(|_| bad())?;
    if k.is_empty() {
        return Err(bad());
    }
    Ok((k.to_uppercase(), n))
}

/// A file name safe for keys, links and headers: no paths, no control characters.
pub fn clean_name(raw: &str) -> Result<String> {
    let base = raw.trim().rsplit(['/', '\\']).next().unwrap_or("").trim();
    let name: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '*' | '?') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_string();
    if name.is_empty() {
        return invalid("file name is empty");
    }
    if name.chars().count() > 200 {
        return invalid("file name is longer than 200 characters");
    }
    Ok(name)
}

fn guess_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn url_path_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The permanent link (behind corntchap's login).
pub fn link(public_url: &str, fref: &str, name: &str) -> String {
    format!(
        "{}/files/{fref}/{}",
        public_url.trim_end_matches('/'),
        url_path_segment(name)
    )
}

const FILE_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-F' || f.number, 'project', pr.key, 'name', f.name, 'content_type', f.content_type,
  'size', f.size, 'size_expected', f.size_expected, 'status', f.status,
  'task', (select pr.key || '-' || t.number from tasks t where t.id = f.task_id),
  'question', (select pr.key || '-Q' || q.number from questions q where q.id = f.question_id),
  'created_by', (select name from principals where id = f.created_by),
  'created_at', f.created_at, 'ready_at', f.ready_at, 'error', f.error)"#;

async fn view(c: &mut PgConnection, state: &AppState, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {FILE_JSON} from files f join projects pr on pr.id = f.project_id where f.id = $1"
    );
    let mut v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?;
    with_url(state, &mut v);
    Ok(v)
}

fn with_url(state: &AppState, v: &mut Value) {
    let url = link(
        &state.config.public_url,
        v["id"].as_str().unwrap_or(""),
        v["name"].as_str().unwrap_or(""),
    );
    v["url"] = json!(url);
}

pub struct NewFile {
    pub project: Option<String>,
    pub name: String,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub task: Option<String>,
    pub question: Option<String>,
}

struct Created {
    id: Uuid,
    key: String,
    content_type: String,
    project_id: Uuid,
    fref: String,
}

async fn insert(
    c: &mut PgConnection,
    state: &AppState,
    p: &Principal,
    n: &NewFile,
    status: &str,
) -> Result<Created> {
    let st = storage(state)?;
    let name = clean_name(&n.name)?;
    if let Some(s) = n.size {
        if s < 0 {
            return invalid("size must be the file's size in bytes");
        }
        if s > st.cfg.max_bytes {
            return invalid(format!(
                "{name} is {} bytes; the limit is {} bytes",
                s, st.cfg.max_bytes
            ));
        }
    }
    let key = match (&n.project, &n.task, &n.question) {
        (Some(k), _, _) => k.trim().to_uppercase(),
        (None, Some(t), _) => parse_task_ref(t)?.0,
        (None, None, Some(q)) => crate::questions::parse_ref(q)?.0,
        _ => return invalid("project (or task / question) is required"),
    };
    let pr = project_by_key(c, p, &key).await?;
    let task_id = match n.task.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = task_by_ref(c, p, t).await?;
            if t.project_id != pr.id {
                return invalid(format!("{} is not in {}", t.key, pr.key));
            }
            Some(t.id)
        }
        None => None,
    };
    let question_id = match n
        .question
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        Some(q) => {
            let (qk, num) = crate::questions::parse_ref(q)?;
            if qk != pr.key {
                return invalid(format!("{q} is not in {}", pr.key));
            }
            Some(
                sqlx::query_scalar::<_, Uuid>(
                    "select id from questions where project_id = $1 and number = $2",
                )
                .bind(pr.id)
                .bind(num)
                .fetch_optional(&mut *c)
                .await?
                .ok_or_else(|| Error::NotFound(format!("no question {q}")))?,
            )
        }
        None => None,
    };
    let content_type = n
        .content_type
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() < 200 && !t.contains(['\r', '\n']))
        .unwrap_or_else(|| guess_type(&name))
        .to_string();
    let number: i64 = sqlx::query_scalar(
        "update projects set next_file = next_file + 1 where id = $1 returning next_file - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *c)
    .await?;
    let id = Uuid::new_v4();
    let key = format!("{}pots/{}/{}/{}", st.cfg.prefix, pr.id, id, name);
    sqlx::query(
        "insert into files (id, project_id, number, name, content_type, size_expected, storage_key, status,
            created_by, task_id, question_id, upload_expires_at)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11, now() + make_interval(hours => $12::int))",
    )
    .bind(id)
    .bind(pr.id)
    .bind(number)
    .bind(&name)
    .bind(&content_type)
    .bind(n.size)
    .bind(&key)
    .bind(status)
    .bind(p.id)
    .bind(task_id)
    .bind(question_id)
    .bind(UPLOAD_DEADLINE_HOURS as i32)
    .execute(&mut *c)
    .await?;
    Ok(Created {
        id,
        key,
        content_type,
        project_id: pr.id,
        fref: format!("{}-F{number}", pr.key),
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// An upload slot: the file's permanent link (shareable now) and a presigned PUT to send the
/// bytes to, with ready-to-run commands. The watcher marks the file ready once it arrives.
pub async fn create_upload(
    db: &PgPool,
    state: &AppState,
    p: &Principal,
    n: NewFile,
) -> Result<Value> {
    if n.size.is_none() {
        return invalid("size (in bytes) is required, e.g. from `stat -c %s <file>`");
    }
    let st = storage(state)?.clone();
    let mut tx = db.begin().await?;
    let f = insert(&mut tx, state, p, &n, "uploading").await?;
    let upload_url = st.upload_url(&f.key, &f.content_type);
    let file = view(&mut tx, state, f.id).await?;
    emit(
        &mut tx,
        Some(f.project_id),
        None,
        "file.created",
        Some(p.id),
        json!({"file": f.fref, "title": file["name"]}),
    )
    .await?;
    tx.commit().await?;
    let local = format!("./{}", file["name"].as_str().unwrap_or("file"));
    let put = format!(
        "curl --fail --silent --show-error --retry 5 --retry-all-errors -X PUT -H {} --upload-file {} {}",
        shell_quote(&format!("Content-Type: {}", f.content_type)),
        shell_quote(&local),
        shell_quote(&upload_url)
    );
    let log = format!("/tmp/corntchap-upload-{}.log", f.fref);
    Ok(json!({
        "file": file,
        "upload_url": upload_url,
        "method": "PUT",
        "content_type": f.content_type,
        "expires_in_s": UPLOAD_URL_TTL.as_secs(),
        "command": put,
        "background_command": format!("nohup {put} > {log} 2>&1 &"),
        "note": format!(
            "Replace {local} with the file's path. The link works now; it serves the file once the upload lands \
             (get_file shows the status). Start the upload within {} minutes.",
            UPLOAD_URL_TTL.as_secs() / 60
        ),
    }))
}

/// Small files sent inline (base64), for agents without a shell.
pub async fn upload_inline(
    db: &PgPool,
    state: &AppState,
    p: &Principal,
    mut n: NewFile,
    base64: &str,
) -> Result<Value> {
    use base64::Engine;
    let st = storage(state)?.clone();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64.trim().replace(['\n', '\r', ' '], ""))
        .map_err(|_| Error::Invalid("contentBase64 is not valid base64".into()))?;
    if bytes.len() as i64 > st.cfg.inline_max_bytes {
        return invalid(format!(
            "{} bytes is over the inline limit of {} bytes: use create_upload and curl instead",
            bytes.len(),
            st.cfg.inline_max_bytes
        ));
    }
    n.size = Some(bytes.len() as i64);
    let mut tx = db.begin().await?;
    let f = insert(&mut tx, state, p, &n, "uploading").await?;
    tx.commit().await?;
    let len = bytes.len() as u64;
    st.put(&f.key, &f.content_type, len, reqwest::Body::from(bytes))
        .await
        .map_err(|e| Error::Internal(format!("storing {}: {e}", f.fref)))?;
    mark_ready(db, state, f.id, len as i64).await?;
    let mut c = db.acquire().await?;
    view(&mut c, state, f.id).await
}

/// The console's upload: the request body streamed to storage.
pub async fn upload_stream(
    db: &PgPool,
    state: &AppState,
    p: &Principal,
    mut n: NewFile,
    length: u64,
    body: reqwest::Body,
) -> Result<Value> {
    let st = storage(state)?.clone();
    if length as i64 > st.cfg.max_bytes {
        return invalid(format!("the limit is {} bytes", st.cfg.max_bytes));
    }
    n.size = Some(length as i64);
    let mut tx = db.begin().await?;
    let f = insert(&mut tx, state, p, &n, "uploading").await?;
    tx.commit().await?;
    if let Err(e) = st.put(&f.key, &f.content_type, length, body).await {
        sqlx::query("update files set status = 'failed', error = $2 where id = $1")
            .bind(f.id)
            .bind("the upload was interrupted")
            .execute(db)
            .await?;
        return Err(Error::Internal(format!("storing {}: {e}", f.fref)));
    }
    mark_ready(db, state, f.id, length as i64).await?;
    let mut c = db.acquire().await?;
    view(&mut c, state, f.id).await
}

async fn mark_ready(db: &PgPool, state: &AppState, id: Uuid, size: i64) -> Result<()> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, Option<Uuid>, String, i64, String, Option<Uuid>)> = sqlx::query_as(
        "update files f set status = 'ready', size = $2, ready_at = now() from projects pr
         where f.id = $1 and f.status = 'uploading' and pr.id = f.project_id
         returning f.project_id, f.task_id, pr.key, f.number, f.name, f.created_by",
    )
    .bind(id)
    .bind(size)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((project, task, key, number, name, by)) = row {
        let fref = format!("{key}-F{number}");
        emit(
            &mut tx,
            Some(project),
            task,
            "file.ready",
            by,
            json!({"file": fref, "title": name, "url": link(&state.config.public_url, &fref, &name)}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn mark_failed(db: &PgPool, id: Uuid, why: &str) -> Result<()> {
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, Option<Uuid>, String, i64, String, Option<Uuid>)> = sqlx::query_as(
        "update files f set status = 'failed', error = $2 from projects pr
         where f.id = $1 and f.status = 'uploading' and pr.id = f.project_id
         returning f.project_id, f.task_id, pr.key, f.number, f.name, f.created_by",
    )
    .bind(id)
    .bind(why)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((project, task, key, number, name, by)) = row {
        emit(
            &mut tx,
            Some(project),
            task,
            "file.failed",
            by,
            json!({"file": format!("{key}-F{number}"), "title": name, "reason": why}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The leader's watcher: has each pending upload landed? Ready when its size is right; failed
/// when it is too big, the wrong size, or never arrived.
pub async fn watch(state: &AppState) -> anyhow::Result<()> {
    let Some(st) = state.storage.clone() else {
        return Ok(());
    };
    let pending: Vec<(Uuid, String, Option<i64>, bool)> = sqlx::query_as(
        "select id, storage_key, size_expected, upload_expires_at < now() from files
         where status = 'uploading' order by created_at limit 200",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, key, expected, expired) in pending {
        match st.size_of(&key).await {
            Ok(Some(size)) => {
                if size > st.cfg.max_bytes {
                    let _ = st.delete(&key).await;
                    mark_failed(
                        &state.db,
                        id,
                        &format!("{size} bytes is over the limit of {}", st.cfg.max_bytes),
                    )
                    .await?;
                } else if expected.is_some_and(|e| e != size) {
                    // a PUT is atomic: a different size is a different file, not a partial one
                    let _ = st.delete(&key).await;
                    mark_failed(
                        &state.db,
                        id,
                        &format!("expected {} bytes, got {size}", expected.unwrap_or(0)),
                    )
                    .await?;
                } else {
                    mark_ready(&state.db, state, id, size).await?;
                }
            }
            Ok(None) if expired => {
                mark_failed(&state.db, id, "the upload never arrived").await?;
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("file watcher: {e}"),
        }
    }
    Ok(())
}

struct Row {
    id: Uuid,
    key: String,
    name: String,
    content_type: String,
    status: String,
    created_by: Option<Uuid>,
    project_id: Uuid,
    fref: String,
}

async fn load(c: &mut PgConnection, p: &Principal, r: &str) -> Result<Row> {
    let (key, number) = parse_ref(r)?;
    let pr = project_by_key(c, p, &key).await?;
    let row: Option<(Uuid, String, String, String, String, Option<Uuid>)> = sqlx::query_as(
        "select id, storage_key, name, content_type, status, created_by from files
         where project_id = $1 and number = $2 and status <> 'deleted'",
    )
    .bind(pr.id)
    .bind(number)
    .fetch_optional(c)
    .await?;
    let (id, skey, name, content_type, status, created_by) =
        row.ok_or_else(|| Error::NotFound(format!("no file {key}-F{number}")))?;
    Ok(Row {
        id,
        key: skey,
        name,
        content_type,
        status,
        created_by,
        project_id: pr.id,
        fref: format!("{key}-F{number}"),
    })
}

/// One file, with a short-lived download URL when it is ready.
pub async fn get(db: &PgPool, state: &AppState, p: &Principal, r: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let f = load(&mut c, p, r).await?;
    let file = view(&mut c, state, f.id).await?;
    let (download, expires) = if f.status == "ready" {
        let st = storage(state)?;
        (
            Some(st.download_url(&f.key, &f.name, &f.content_type)),
            Some(DOWNLOAD_URL_TTL.as_secs()),
        )
    } else {
        (None, None)
    };
    Ok(json!({"file": file, "download_url": download, "expires_in_s": expires}))
}

/// The redirect behind `/files/PAY-F12/name`.
pub async fn download_redirect(
    db: &PgPool,
    state: &AppState,
    p: &Principal,
    r: &str,
) -> Result<std::result::Result<String, (u16, String)>> {
    let mut c = db.acquire().await?;
    let f = load(&mut c, p, r).await?;
    Ok(match f.status.as_str() {
        "ready" => Ok(storage(state)?.download_url(&f.key, &f.name, &f.content_type)),
        "uploading" => Err((
            409,
            format!("{} is still uploading; try again in a moment.", f.fref),
        )),
        _ => Err((410, format!("{} failed to upload.", f.fref))),
    })
}

/// A pot's files (optionally one task's or one question's), newest first.
pub async fn list(
    db: &PgPool,
    state: &AppState,
    p: &Principal,
    project: &str,
    task: Option<&str>,
    question: Option<&str>,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let task_id = match task.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Some(task_by_ref(&mut c, p, t).await?.id),
        None => None,
    };
    let qnum = match question.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => Some(crate::questions::parse_ref(q)?.1),
        None => None,
    };
    let sql = format!(
        "select {FILE_JSON} from files f join projects pr on pr.id = f.project_id
         where f.project_id = $1 and f.status <> 'deleted'
           and ($2::uuid is null or f.task_id = $2)
           and ($3::bigint is null or f.question_id = (select id from questions where project_id = $1 and number = $3))
         order by f.created_at desc limit 500"
    );
    let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(task_id)
        .bind(qnum)
        .fetch_all(&mut *c)
        .await?;
    for r in rows.iter_mut() {
        with_url(state, r);
    }
    Ok(Value::Array(rows))
}

/// Delete a file: its uploader, people (and cooks granted `plan`) in the pot, or admins.
pub async fn delete(db: &PgPool, state: &AppState, p: &Principal, r: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let f = load(&mut tx, p, r).await?;
    if f.created_by != Some(p.id) && !p.can_plan() && !p.is_super_admin() {
        return Err(Error::Forbidden(format!(
            "{} was uploaded by someone else: a person deletes it, or its owner grants {} `plan`",
            f.fref, p.name
        )));
    }
    sqlx::query("update files set status = 'deleted' where id = $1")
        .bind(f.id)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(f.project_id),
        None,
        "file.deleted",
        Some(p.id),
        json!({"file": f.fref, "title": f.name}),
    )
    .await?;
    tx.commit().await?;
    if let Some(st) = &state.storage
        && let Err(e) = st.delete(&f.key).await
    {
        tracing::warn!("deleting {} from storage: {e}", f.fref);
    }
    Ok(json!({"ok": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_prefixes() {
        for (raw, want) in [
            ("", ""),
            ("/", ""),
            ("corntchap/files", "corntchap/files/"),
            ("/corntchap//files/", "corntchap/files/"),
            ("corntchap", "corntchap/"),
            ("a.b/..c/x..", "a.b/..c/x../"),
        ] {
            assert_eq!(key_prefix(raw).unwrap(), want, "{raw:?}");
        }
        for bad in [
            "..",
            ".",
            "../tf-states",
            "corntchap/../tf-states",
            "corntchap/./files",
            "corntchap/ files",
            "  ",
            "corn\nchap",
            "corn\u{7f}",
        ] {
            assert!(key_prefix(bad).is_err(), "{bad:?} should be refused");
        }
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn storage_from_env() {
        // off unless both the endpoint and the bucket are set
        assert!(StorageConfig::from_env(env_of(&[])).unwrap().is_none());
        assert!(
            StorageConfig::from_env(env_of(&[("S3_ENDPOINT", "https://s3")]))
                .unwrap()
                .is_none()
        );
        let c = StorageConfig::from_env(env_of(&[
            ("S3_ENDPOINT", "https://nbg1.example"),
            ("S3_BUCKET", "shared"),
            ("S3_PREFIX", "corntchap/files"),
            ("S3_REGION", "nbg1"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            (&*c.bucket, &*c.prefix, &*c.region, &*c.public_endpoint),
            ("shared", "corntchap/files/", "nbg1", "https://nbg1.example")
        );
        assert!(c.path_style && !c.create_bucket);
        let root = StorageConfig::from_env(env_of(&[("S3_ENDPOINT", "e"), ("S3_BUCKET", "b")]))
            .unwrap()
            .unwrap();
        assert_eq!(root.prefix, "");
        // a prefix that would leave itself stops the server instead of writing elsewhere
        assert!(
            StorageConfig::from_env(env_of(&[
                ("S3_ENDPOINT", "e"),
                ("S3_BUCKET", "b"),
                ("S3_PREFIX", "corntchap/../tf-states"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn refs_names_links() {
        assert_eq!(parse_ref("PAY-F12").unwrap(), ("PAY".into(), 12));
        assert!(parse_ref("PAY-12").is_err());
        assert_eq!(clean_name("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(
            clean_name("C:\\Users\\me\\shot 1.png").unwrap(),
            "shot 1.png"
        );
        assert_eq!(clean_name("a\"b<c>.txt").unwrap(), "a_b_c_.txt");
        assert!(clean_name(" .. ").is_err());
        assert_eq!(
            link("https://c.example/", "PAY-F1", "shot 1.png"),
            "https://c.example/files/PAY-F1/shot%201.png"
        );
        assert_eq!(guess_type("A.PNG"), "image/png");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
