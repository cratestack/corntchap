//! `corntchap-cli pantry`: the project's shared memory (conventions, gotchas, decisions, how-tos).

use std::io::{IsTerminal, Read, Write};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Map, Value, json};

use crate::{
    api::Api,
    ui::{self, ok_line, s},
};

#[derive(Subcommand)]
pub enum PantryCmd {
    /// List what is in a pot's pantry (best matches first with --query).
    #[command(name = "list", alias = "ls")]
    List {
        /// Project key (case-insensitive).
        pot: String,
        /// Full-text search.
        #[arg(short = 'q', long = "query")]
        query: Option<String>,
        /// note | convention | gotcha | decision | howto | fact
        #[arg(long)]
        kind: Option<String>,
        /// Only memories with this tag (repeat for several; all must match).
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// Search a pot's pantry (same as `list --query`).
    Search {
        pot: String,
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// Show one memory in full.
    Show {
        pot: String,
        key: String,
        #[arg(long)]
        json: bool,
    },
    /// Put a memory on the shelf: creates it, or updates the one with the same key (body from --body, --file, or stdin).
    #[command(name = "stock", aliases = ["add", "remember"])]
    Stock {
        pot: String,
        /// Short headline; the key is derived from it unless --key is given.
        #[arg(long)]
        title: Option<String>,
        /// Stable key (lowercase letters, digits, . _ -). Saving the same key again updates it.
        #[arg(long)]
        key: Option<String>,
        /// note | convention | gotcha | decision | howto | fact
        #[arg(long)]
        kind: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Task this memory came from, e.g. PAY-1.
        #[arg(long)]
        task: Option<String>,
        /// Add to the end of the existing body instead of replacing it.
        #[arg(long)]
        append: bool,
        /// Fail with a conflict unless the memory is still at this revision.
        #[arg(long)]
        revision: Option<i64>,
        /// The body text.
        #[arg(long, conflicts_with = "file")]
        body: Option<String>,
        /// Read the body from a file ("-" for stdin).
        #[arg(long)]
        file: Option<String>,
    },
    /// Throw a memory out of the pantry (asks first on a terminal).
    #[command(name = "toss", aliases = ["rm", "forget"])]
    Toss {
        pot: String,
        key: String,
        /// Do not ask.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Print the pantry as one markdown digest.
    #[command(name = "digest", alias = "read")]
    Digest { pot: String },
}

/// Where a memory body comes from.
#[derive(Debug, PartialEq)]
pub enum BodySource {
    Inline(String),
    File(String),
    Stdin,
}

pub fn body_source(body: Option<String>, file: Option<String>) -> BodySource {
    match (body, file) {
        (Some(b), _) => BodySource::Inline(b),
        (None, Some(f)) if f != "-" => BodySource::File(f),
        _ => BodySource::Stdin,
    }
}

fn read_body(src: BodySource) -> Result<String> {
    match src {
        BodySource::Inline(b) => Ok(b),
        BodySource::File(p) => std::fs::read_to_string(&p).with_context(|| format!("reading {p}")),
        BodySource::Stdin => {
            let mut stdin = std::io::stdin();
            if stdin.is_terminal() {
                eprintln!("reading the body from stdin; finish with Ctrl-D");
            }
            let mut b = String::new();
            stdin.read_to_string(&mut b).context("reading stdin")?;
            Ok(b)
        }
    }
}

/// Query string pairs for list/search; the pot is not part of it.
pub fn list_params(
    query: Option<&str>,
    kind: Option<&str>,
    tags: &[String],
    limit: Option<u32>,
) -> Vec<(&'static str, String)> {
    let mut p = vec![];
    if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
        p.push(("q", q.to_string()));
    }
    if let Some(k) = kind {
        p.push(("kind", k.to_string()));
    }
    if !tags.is_empty() {
        p.push(("tags", tags.join(",")));
    }
    if let Some(l) = limit {
        p.push(("limit", l.to_string()));
    }
    p
}

/// Everything a save can carry; unset options are left out of the request so the server keeps what it has.
pub struct Stock<'a> {
    pub title: Option<&'a str>,
    pub body: &'a str,
    pub key: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub tags: &'a [String],
    pub task: Option<&'a str>,
    pub append: bool,
    pub revision: Option<i64>,
}

impl Stock<'_> {
    pub fn to_body(&self) -> Value {
        let mut m = Map::new();
        m.insert("title".into(), json!(self.title.unwrap_or("")));
        m.insert("body".into(), json!(self.body));
        if let Some(k) = self.key {
            m.insert("key".into(), json!(k));
        }
        if let Some(k) = self.kind {
            m.insert("kind".into(), json!(k));
        }
        if !self.tags.is_empty() {
            m.insert("tags".into(), json!(self.tags));
        }
        if let Some(t) = self.task {
            m.insert("task".into(), json!(t));
        }
        if self.append {
            m.insert("append".into(), json!(true));
        }
        if let Some(r) = self.revision {
            m.insert("revision".into(), json!(r));
        }
        Value::Object(m)
    }
}

const EMPTY_HINT: &str =
    "the pantry is empty: stock it with `pantry stock <POT> --title \"...\" --body \"...\"`";
const NO_MATCH_HINT: &str =
    "nothing in the pantry matches (try fewer words, or `pantry list` to see everything)";

fn path(pot: &str) -> String {
    format!("/projects/{}/pantry", pot.to_uppercase())
}

async fn list(
    api: &Api,
    pot: &str,
    params: Vec<(&'static str, String)>,
    json: bool,
    searching: bool,
) -> Result<()> {
    let r = api.raw(reqwest::Method::GET, &path(pot));
    let v = api.send(r.query(&params)).await?;
    if json {
        crate::out(&v);
    } else if v.as_array().is_none_or(Vec::is_empty) {
        println!("{}", if searching { NO_MATCH_HINT } else { EMPTY_HINT });
    } else {
        println!("{}", ui::memories_table(&v));
    }
    Ok(())
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut a = String::new();
    std::io::stdin().read_line(&mut a).is_ok() && matches!(a.trim(), "y" | "Y" | "yes")
}

pub async fn run(cmd: PantryCmd, api: &Api) -> Result<()> {
    match cmd {
        PantryCmd::List {
            pot,
            query,
            kind,
            tags,
            limit,
            json,
        } => {
            let p = list_params(query.as_deref(), kind.as_deref(), &tags, limit);
            list(api, &pot, p, json, query.is_some()).await
        }
        PantryCmd::Search {
            pot,
            query,
            kind,
            tags,
            limit,
            json,
        } => {
            let p = list_params(Some(&query.join(" ")), kind.as_deref(), &tags, limit);
            list(api, &pot, p, json, true).await
        }
        PantryCmd::Show { pot, key, json } => {
            let v = api.get(&format!("{}/{key}", path(&pot))).await?;
            crate::show(&v, json, ui::memory_detail);
            Ok(())
        }
        PantryCmd::Stock {
            pot,
            title,
            key,
            kind,
            tags,
            task,
            append,
            revision,
            body,
            file,
        } => {
            if title.is_none() && key.is_none() {
                bail!("give the memory a --title (or a --key to update an existing one)");
            }
            let text = read_body(body_source(body, file))?;
            if text.trim().is_empty() {
                bail!("the body is empty: say what you learned");
            }
            let b = Stock {
                title: title.as_deref(),
                body: &text,
                key: key.as_deref(),
                kind: kind.as_deref(),
                tags: &tags,
                task: task.as_deref(),
                append,
                revision,
            }
            .to_body();
            let v = api.post(&path(&pot), b).await?;
            ok_line(format!(
                "stocked {} in {} (revision {})",
                s(&v, "key"),
                pot.to_uppercase(),
                v["revision"]
            ));
            Ok(())
        }
        PantryCmd::Toss { pot, key, yes } => {
            if !yes
                && std::io::stdin().is_terminal()
                && !confirm(&format!(
                    "Toss {key} from the {} pantry?",
                    pot.to_uppercase()
                ))
            {
                println!("kept");
                return Ok(());
            }
            api.delete(&format!("{}/{key}", path(&pot))).await?;
            ok_line(format!("tossed {key} from {}", pot.to_uppercase()));
            Ok(())
        }
        PantryCmd::Digest { pot } => {
            let r = api.raw(reqwest::Method::GET, &path(&pot));
            let v = api.send(r.query(&[("format", "markdown")])).await?;
            println!("{}", s(&v, "markdown").trim_end());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_source_prefers_inline_then_file_then_stdin() {
        assert_eq!(
            body_source(Some("x".into()), None),
            BodySource::Inline("x".into())
        );
        assert_eq!(
            body_source(None, Some("a.md".into())),
            BodySource::File("a.md".into())
        );
        assert_eq!(body_source(None, Some("-".into())), BodySource::Stdin);
        assert_eq!(body_source(None, None), BodySource::Stdin);
    }

    #[test]
    fn list_params_skip_unset_and_join_tags() {
        assert!(list_params(None, None, &[], None).is_empty());
        assert!(list_params(Some("  "), None, &[], None).is_empty());
        let p = list_params(
            Some("retry 429"),
            Some("gotcha"),
            &["a".into(), "b".into()],
            Some(5),
        );
        assert_eq!(
            p,
            vec![
                ("q", "retry 429".to_string()),
                ("kind", "gotcha".to_string()),
                ("tags", "a,b".to_string()),
                ("limit", "5".to_string())
            ]
        );
    }

    #[test]
    fn stock_body_omits_unset_fields() {
        let s = Stock {
            title: Some("T"),
            body: "body",
            key: None,
            kind: None,
            tags: &[],
            task: None,
            append: false,
            revision: None,
        };
        assert_eq!(s.to_body(), json!({"title": "T", "body": "body"}));
        let tags = ["x".to_string()];
        let s = Stock {
            title: None,
            body: "more",
            key: Some("k"),
            kind: Some("howto"),
            tags: &tags,
            task: Some("PAY-1"),
            append: true,
            revision: Some(2),
        };
        assert_eq!(
            s.to_body(),
            json!({"title":"","body":"more","key":"k","kind":"howto","tags":["x"],
                   "task":"PAY-1","append":true,"revision":2})
        );
    }
}
