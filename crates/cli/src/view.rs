//! `corntchap-cli view`: a pot's saved views, a SQL condition and order over its tasks (shown as a
//! list or a board tab in the console). The server checks the SQL: one expression over the view's
//! columns, no subqueries or other tables.

use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::{
    api::Api,
    ui::{self, ok_line, s},
};

#[derive(Subcommand)]
pub enum ViewCmd {
    /// List a pot's saved views.
    #[command(name = "list", alias = "ls")]
    List {
        pot: String,
        #[arg(long)]
        json: bool,
    },
    /// Show the tasks of a saved view, or of a condition you are trying out (--where).
    Run {
        pot: String,
        /// A saved view's key.
        key: Option<String>,
        /// A SQL condition to try, e.g. "'urgent' = any(labels) and status <> 'done'".
        #[arg(long = "where")]
        where_sql: Option<String>,
        /// A SQL order, e.g. "priority desc, created_at".
        #[arg(long = "order")]
        order_sql: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Create a view, or update the one with the same key.
    Save {
        pot: String,
        /// What people see on the tab.
        name: String,
        #[arg(long = "where")]
        where_sql: Option<String>,
        #[arg(long = "order")]
        order_sql: Option<String>,
        /// list (default) or board.
        #[arg(long, value_parser = ["list", "board"])]
        layout: Option<String>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete a view.
    #[command(name = "rm", alias = "delete")]
    Rm { pot: String, key: String },
}

pub async fn run(c: ViewCmd, api: &Api) -> Result<()> {
    match c {
        ViewCmd::List { pot, json } => {
            let v = all_pages(api, "savedViewPage", json!({"project": pot})).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", views_table(&v));
            }
        }
        ViewCmd::Run {
            pot,
            key,
            where_sql,
            order_sql,
            json,
        } => {
            let v = all_pages(
                api,
                "viewPage",
                json!({"project": pot, "key": key, "where": where_sql, "orderBy": order_sql}),
            )
            .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", ui::tasks_table(&v));
            }
        }
        ViewCmd::Save {
            pot,
            name,
            where_sql,
            order_sql,
            layout,
            key,
            description,
        } => {
            let v = api
                .procedure(
                    "viewSave",
                    json!({"project": pot, "name": name, "where": where_sql, "orderBy": order_sql,
                           "layout": layout, "key": key, "description": description}),
                )
                .await?;
            ok_line(format!(
                "view {} ({}) saved: {}",
                s(&v, "key"),
                s(&v, "layout"),
                s(&v, "where")
            ));
        }
        ViewCmd::Rm { pot, key } => {
            api.procedure("viewDelete", json!({"project": pot, "key": key}))
                .await?;
            ok_line(format!("view {key} deleted"));
        }
    }
    Ok(())
}

/// Every item of a paged procedure, page after page (at most `MAX_ROWS`).
async fn all_pages(api: &Api, name: &str, args: Value) -> Result<Value> {
    const MAX_ROWS: usize = 500;
    let mut items: Vec<Value> = vec![];
    let mut cursor: Option<String> = None;
    loop {
        let mut a = args.clone();
        a["page"] = json!({"limit": 200, "cursor": cursor});
        let page = api.procedure(name, a).await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        cursor = page["nextCursor"].as_str().map(String::from);
        if cursor.is_none() || items.len() >= MAX_ROWS {
            break;
        }
    }
    items.truncate(MAX_ROWS);
    Ok(Value::Array(items))
}

fn views_table(v: &Value) -> String {
    let rows = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            vec![
                ui::plain(s(x, "key")),
                ui::plain(s(x, "name")),
                ui::plain(s(x, "layout")),
                ui::plain(s(x, "where")),
                ui::dim_cell(x["order"].as_str().unwrap_or("-")),
                ui::dim_cell(x["created_by"].as_str().unwrap_or("-")),
            ]
        })
        .collect();
    ui::table(
        &["key", "name", "layout", "where", "order", "by"],
        rows,
        "no saved views (the pot shows its board; save one with `view save`)",
    )
}
