//! `corntchap-cli file`: share files (screenshots, logs, build artifacts, up to 1 GiB) in a pot.
//!
//! `put` asks the server for an upload slot, prints the file's permanent link at once, and sends
//! the bytes straight to object storage with the presigned PUT, in the foreground or detached
//! (`--background`). The server notices when the upload lands and marks the file ready; links
//! stay behind corntchap's login.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    api::{Api, config_dir},
    ui::{self, ok_line, s},
};

#[derive(Subcommand)]
pub enum FileCmd {
    /// Upload files to a pot (optionally attached to a task or a question); prints each link at once.
    #[command(name = "put", alias = "upload")]
    Put {
        /// Local files to upload.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
        /// Pot key; implied by --task or --question.
        #[arg(long, alias = "project")]
        pot: Option<String>,
        /// Attach to this task, e.g. PAY-42.
        #[arg(long)]
        task: Option<String>,
        /// Attach to this question, e.g. PAY-Q3.
        #[arg(long)]
        question: Option<String>,
        /// Name to store it under (one file only); defaults to the file's name.
        #[arg(long)]
        name: Option<String>,
        /// MIME type; guessed from the name when omitted.
        #[arg(long = "type")]
        content_type: Option<String>,
        /// Return as soon as the links exist; the bytes go up in a detached process.
        #[arg(short = 'b', long)]
        background: bool,
        #[arg(long)]
        json: bool,
    },
    /// Download a file (to its own name in the current directory, or -o PATH; `-o -` for stdout).
    #[command(name = "get", alias = "download")]
    Get {
        /// File id, e.g. PAY-F12.
        id: String,
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
        /// Print the file's status and a short-lived download URL instead of downloading.
        #[arg(long)]
        url: bool,
        #[arg(long)]
        json: bool,
    },
    /// List a pot's files, newest first.
    #[command(name = "list", alias = "ls")]
    List {
        pot: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        question: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Delete a file.
    #[command(name = "rm", alias = "delete")]
    Rm { id: String },
    /// (internal) Send one file's bytes to its presigned PUT; used by `put --background`.
    #[command(name = "_send", hide = true)]
    Send {
        path: PathBuf,
        url: String,
        content_type: String,
        file_id: String,
    },
}

pub async fn run(c: FileCmd, api: &Api) -> Result<()> {
    match c {
        FileCmd::Put {
            paths,
            pot,
            task,
            question,
            name,
            content_type,
            background,
            json,
        } => {
            if name.is_some() && paths.len() > 1 {
                bail!("--name works with one file only");
            }
            let pot = pot
                .or_else(|| task.as_deref().and_then(pot_of))
                .or_else(|| question.as_deref().and_then(pot_of))
                .ok_or_else(|| anyhow!("which pot? pass --pot KEY (or --task / --question)"))?;
            let mut out = Vec::new();
            for path in &paths {
                let meta = tokio::fs::metadata(path)
                    .await
                    .with_context(|| format!("reading {}", path.display()))?;
                if !meta.is_file() {
                    bail!("{} is not a file", path.display());
                }
                let fname = name.clone().unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into())
                });
                let slot = api
                    .post(
                        &format!("/projects/{}/uploads", urlenc(&pot)),
                        json!({"name": fname, "size": meta.len(), "content_type": content_type,
                               "task": task, "question": question}),
                    )
                    .await?;
                let file = &slot["file"];
                let (id, url) = (s(file, "id"), s(file, "url"));
                let (put, ctype) = (s(&slot, "upload_url"), s(&slot, "content_type"));
                if background {
                    let log = spawn_send(path, &put, &ctype, &id)?;
                    if !json {
                        ok_line(format!(
                            "{id} {fname} ({}): uploading in the background (log: {})",
                            human(meta.len()),
                            log.display()
                        ));
                        println!("  {url}");
                    }
                } else {
                    send(path, &put, &ctype, meta.len(), !json).await?;
                    if !json {
                        ok_line(format!("{id} {fname} ({}) uploaded", human(meta.len())));
                        println!("  {url}");
                    }
                }
                out.push(file.clone());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&Value::Array(out))?);
            }
        }
        FileCmd::Get {
            id,
            output,
            url,
            json,
        } => {
            let v = api.get(&format!("/files/{}", urlenc(&id))).await?;
            let f = &v["file"];
            if url || json {
                if json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    println!("{}  {}", s(f, "status"), s(f, "name"));
                    println!("link      {}", s(f, "url"));
                    if let Some(d) = v["download_url"].as_str() {
                        println!(
                            "download  {d}  (valid {}s)",
                            v["expires_in_s"].as_i64().unwrap_or(0)
                        );
                    }
                }
                return Ok(());
            }
            let Some(dl) = v["download_url"].as_str() else {
                bail!(
                    "{id} is {}{}",
                    s(f, "status"),
                    f["error"]
                        .as_str()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                );
            };
            let to = output.unwrap_or_else(|| PathBuf::from(safe_name(&s(f, "name"))));
            download(&api.http, dl, &to).await?;
            if to.as_os_str() != "-" {
                ok_line(format!("{id} saved to {}", to.display()));
            }
        }
        FileCmd::List {
            pot,
            task,
            question,
            json,
        } => {
            let mut q = Vec::new();
            if let Some(t) = &task {
                q.push(format!("task={}", urlenc(t)));
            }
            if let Some(x) = &question {
                q.push(format!("question={}", urlenc(x)));
            }
            let path = format!(
                "/projects/{}/files{}{}",
                urlenc(&pot),
                if q.is_empty() { "" } else { "?" },
                q.join("&")
            );
            let v = api.get(&path).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", files_table(&v));
            }
        }
        FileCmd::Rm { id } => {
            api.delete(&format!("/files/{}", urlenc(&id))).await?;
            ok_line(format!("{id} deleted"));
        }
        FileCmd::Send {
            path,
            url,
            content_type,
            file_id,
        } => {
            let len = tokio::fs::metadata(&path).await?.len();
            match send(&path, &url, &content_type, len, false).await {
                Ok(()) => println!("{file_id}: uploaded {} ({})", path.display(), human(len)),
                Err(e) => {
                    println!("{file_id}: upload of {} failed: {e:#}", path.display());
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

/// PAY-42 / PAY-Q3 → PAY.
fn pot_of(r: &str) -> Option<String> {
    r.rsplit_once('-')
        .map(|(k, _)| k.to_string())
        .filter(|k| !k.is_empty())
}

fn urlenc(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A downloaded name never escapes the current directory.
fn safe_name(n: &str) -> String {
    let base = n.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if base.is_empty() || base == "." || base == ".." {
        "download".into()
    } else {
        base.to_string()
    }
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Stream a file to a presigned PUT (no buffering of the whole file), with progress on a terminal.
async fn send(path: &Path, url: &str, content_type: &str, len: u64, progress: bool) -> Result<()> {
    let f = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let show = progress && std::io::IsTerminal::is_terminal(&std::io::stderr()) && len > 0;
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // 1 MiB chunks, reporting progress as they go out.
    let chunks = futures::stream::unfold((f, 0u64), move |(mut f, sent)| {
        let label = label.clone();
        async move {
            let mut buf = vec![0u8; 1 << 20];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    let sent = sent + n as u64;
                    if show {
                        eprint!("\r  {label}: {:>3}% of {}   ", sent * 100 / len, human(len));
                    }
                    Some((Ok::<_, std::io::Error>(buf), (f, sent)))
                }
                Err(e) => Some((Err(e), (f, sent))),
            }
        }
    });
    let r = reqwest::Client::new()
        .put(url)
        .header("content-type", content_type)
        .header("content-length", len)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await
        .context("upload failed")?;
    if show {
        eprintln!();
    }
    let status = r.status();
    if !status.is_success() {
        let text = r.text().await.unwrap_or_default();
        bail!(
            "storage refused the upload (HTTP {status}): {}",
            text.chars().take(300).collect::<String>()
        );
    }
    Ok(())
}

/// Re-run this binary detached (`file _send`), logging to the config dir; returns the log path.
fn spawn_send(path: &Path, url: &str, content_type: &str, id: &str) -> Result<PathBuf> {
    let dir = config_dir().join("uploads");
    std::fs::create_dir_all(&dir)?;
    let log = dir.join(format!("{}.log", safe_name(id)));
    let out = std::fs::File::create(&log)?;
    let abs = std::fs::canonicalize(path)?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.args(["file", "_send"])
        .arg(&abs)
        .args([url, content_type, id])
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // a closed terminal or Ctrl-C does not stop the upload
    }
    cmd.spawn().context("starting the background upload")?;
    Ok(log)
}

async fn download(http: &reqwest::Client, url: &str, to: &Path) -> Result<()> {
    let r = http.get(url).send().await?;
    let status = r.status();
    if !status.is_success() {
        bail!("storage refused the download (HTTP {status})");
    }
    let mut body = r.bytes_stream();
    if to.as_os_str() == "-" {
        let mut out = tokio::io::stdout();
        while let Some(chunk) = body.next().await {
            out.write_all(&chunk?).await?;
        }
        out.flush().await?;
        return Ok(());
    }
    let part = to.with_extension(format!(
        "{}part",
        to.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    let mut out = tokio::fs::File::create(&part)
        .await
        .with_context(|| format!("creating {}", part.display()))?;
    while let Some(chunk) = body.next().await {
        out.write_all(&chunk?).await?;
    }
    out.flush().await?;
    drop(out);
    tokio::fs::rename(&part, to).await?;
    Ok(())
}

fn files_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let size = x["size"]
                .as_u64()
                .or_else(|| x["size_expected"].as_u64())
                .map(human)
                .unwrap_or_else(|| "-".into());
            let on = x["task"]
                .as_str()
                .or_else(|| x["question"].as_str())
                .unwrap_or("-");
            vec![
                ui::plain(s(x, "id")),
                ui::plain(s(x, "name")),
                ui::status_cell(&s(x, "status")),
                ui::plain(size),
                ui::plain(on),
                ui::plain(x["created_by"].as_str().unwrap_or("-")),
                ui::dim_cell(ui::age(&s(x, "created_at"))),
            ]
        })
        .collect();
    ui::table(
        &["id", "name", "status", "size", "on", "by", "age"],
        rows,
        "no files (upload with `file put PATH --pot KEY`)",
    )
}
