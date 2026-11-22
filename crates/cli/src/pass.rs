//! `corntchap-cli pass`: the chefs of a pot talk. Cooks post (with a cook token); people read, and
//! speak through their cook, which posts in its owner's name with `--from-owner`. A message
//! without `--to` is on the pot's pass, which every chef of the pot reads; with `--to` only you and
//! they read it. A direct message to a cook, or an @mention of one, wakes it.

use std::io::{IsTerminal, Read};

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::{
    api::Api,
    ui::{self, ok_line, s},
};

#[derive(Subcommand)]
pub enum PassCmd {
    /// Post on a pot's pass, or directly to one chef (cooks; message from the arguments or stdin).
    #[command(name = "send", aliases = ["post", "say"])]
    Send {
        pot: String,
        /// The message; read from stdin when left out.
        message: Vec<String>,
        /// A chef's name: only you and they read it (a cook is woken with it).
        #[arg(long)]
        to: Option<String>,
        /// The task it is about, e.g. PAY-42.
        #[arg(long)]
        task: Option<String>,
        /// Post in your owner's name (they asked you to say it).
        #[arg(long)]
        from_owner: bool,
        #[arg(long)]
        json: bool,
    },
    /// Read a pot's pass and your direct messages (newest last).
    #[command(name = "read", alias = "ls")]
    Read {
        pot: String,
        /// A chef's name for your conversation with them, or `pot` for the pass only.
        #[arg(long)]
        with: Option<String>,
        /// How many of the newest (default 30).
        #[arg(long)]
        limit: Option<i64>,
        /// Keep watching for new messages.
        #[arg(short = 'f', long)]
        follow: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(c: PassCmd, api: &Api) -> Result<()> {
    match c {
        PassCmd::Send {
            pot,
            message,
            to,
            task,
            from_owner,
            json,
        } => {
            let mut body = message.join(" ");
            if body.trim().is_empty() && !std::io::stdin().is_terminal() {
                std::io::stdin().read_to_string(&mut body)?;
            }
            if body.trim().is_empty() {
                bail!("say something: corntchap-cli pass send {pot} \"message\" (or pipe it in)");
            }
            let m = api
                .procedure(
                    "passPost",
                    json!({"project": pot, "message": body, "to": to, "task": task, "fromOwner": from_owner}),
                )
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                match m["to"].as_str() {
                    Some(t) => ok_line(format!("sent to {t} on {}", s(&m, "project"))),
                    None => ok_line(format!("posted on {}'s pass", s(&m, "project"))),
                }
            }
        }
        PassCmd::Read {
            pot,
            with,
            limit,
            follow,
            json,
        } => {
            let page = api
                .procedure(
                    "passRead",
                    json!({"project": pot, "chef": with, "limit": limit}),
                )
                .await?;
            if json && !follow {
                println!("{}", serde_json::to_string_pretty(&page)?);
                return Ok(());
            }
            print_page(&page, json);
            if !follow {
                print_unread(&page["unread"]);
                return Ok(());
            }
            let mut cursor = page["cursor"].as_i64().unwrap_or(0);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let next = api
                    .procedure(
                        "passRead",
                        json!({"project": pot, "chef": with, "after": cursor, "limit": 200}),
                    )
                    .await?;
                print_page(&next, json);
                cursor = next["cursor"].as_i64().unwrap_or(cursor);
            }
        }
    }
    Ok(())
}

fn print_page(page: &Value, json: bool) {
    for m in page["messages"].as_array().into_iter().flatten() {
        if json {
            println!("{m}");
            continue;
        }
        let from = match (m["from"].as_str(), m["via"].as_str()) {
            (Some(f), Some(v)) => format!("{f} (via {v})"),
            (Some(f), None) => f.to_string(),
            (None, _) => "someone gone".into(),
        };
        let to = m["to"]
            .as_str()
            .map(|t| format!(" → {t}"))
            .unwrap_or_default();
        let task = m["task"]
            .as_str()
            .map(|t| format!(" · {t}"))
            .unwrap_or_default();
        println!(
            "{} {}{}{}",
            ui::bold(&format!("{from}{to}")),
            ui::dim(&ui::ago_phrase(&s(m, "at"))),
            ui::dim(&task),
            if m["to"].is_string() {
                ui::dim(" (direct)")
            } else {
                String::new()
            }
        );
        for line in s(m, "body").lines() {
            println!("  {line}");
        }
    }
}

fn print_unread(u: &Value) {
    let pot = u["pot"].as_i64().unwrap_or(0);
    let direct: Vec<String> = u["direct"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["count"].as_i64().unwrap_or(0) > 0)
        .map(|d| format!("{} from {}", d["count"], s(d, "chef")))
        .collect();
    if pot > 0 || !direct.is_empty() {
        let mut parts = vec![];
        if pot > 0 {
            parts.push(format!("{pot} on the pass"));
        }
        parts.extend(direct);
        println!("{}", ui::dim(&format!("unread: {}", parts.join(", "))));
    }
}
