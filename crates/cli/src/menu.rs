//! `corntchap-cli menu`: a pot's menus (milestones) and the tasks on them.

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::{Map, Value, json};

use crate::{
    api::Api,
    ui::{self, ok_line, s},
};

#[derive(Subcommand)]
pub enum MenuCmd {
    /// List a pot's menus with their progress (served ones only with --all).
    #[command(name = "ls", alias = "list")]
    Ls {
        /// Project key (case-insensitive).
        pot: String,
        /// Include served menus.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Put a new menu on the pot.
    Add {
        pot: String,
        name: String,
        /// Short key tasks refer to (derived from the name when left out).
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Due date, YYYY-MM-DD.
        #[arg(long)]
        due: Option<String>,
        /// Sort position (lower first).
        #[arg(long)]
        position: Option<i64>,
    },
    /// Change a menu's name, description, due date or position.
    Edit {
        pot: String,
        key: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Due date, YYYY-MM-DD; "" clears it.
        #[arg(long)]
        due: Option<String>,
        #[arg(long)]
        position: Option<i64>,
    },
    /// Serve the menu: the milestone is done.
    Serve { pot: String, key: String },
    /// Bring a served menu back to the pass.
    Reopen { pot: String, key: String },
}

pub fn path(pot: &str) -> String {
    format!("/projects/{}/menus", pot.to_uppercase())
}

pub fn list_path(pot: &str, all: bool) -> String {
    if all {
        format!("{}?all=true", path(pot))
    } else {
        path(pot)
    }
}

/// Request body of `menu add`; unset options are left out.
pub fn add_body(
    name: &str,
    key: Option<&str>,
    description: Option<&str>,
    due: Option<&str>,
    position: Option<i64>,
) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), json!(name));
    if let Some(k) = key {
        m.insert("key".into(), json!(k));
    }
    if let Some(d) = description {
        m.insert("description".into(), json!(d));
    }
    if let Some(d) = due {
        m.insert("due_on".into(), json!(d));
    }
    if let Some(p) = position {
        m.insert("position".into(), json!(p));
    }
    Value::Object(m)
}

/// Request body of `menu edit`; unset options are left out, `--due ""` is sent to clear the date.
pub fn edit_body(
    name: Option<&str>,
    description: Option<&str>,
    due: Option<&str>,
    position: Option<i64>,
    status: Option<&str>,
) -> Value {
    let mut m = Map::new();
    if let Some(n) = name {
        m.insert("name".into(), json!(n));
    }
    if let Some(d) = description {
        m.insert("description".into(), json!(d));
    }
    if let Some(d) = due {
        m.insert("due_on".into(), json!(d));
    }
    if let Some(p) = position {
        m.insert("position".into(), json!(p));
    }
    if let Some(st) = status {
        m.insert("status".into(), json!(st));
    }
    Value::Object(m)
}

pub async fn run(cmd: MenuCmd, api: &Api) -> Result<()> {
    match cmd {
        MenuCmd::Ls { pot, all, json } => {
            let v = api.get(&list_path(&pot, all)).await?;
            crate::show(&v, json, ui::menus_table);
        }
        MenuCmd::Add {
            pot,
            name,
            key,
            description,
            due,
            position,
        } => {
            let b = add_body(
                &name,
                key.as_deref(),
                description.as_deref(),
                due.as_deref(),
                position,
            );
            let v = api.post(&path(&pot), b).await?;
            ok_line(format!(
                "menu {} ({}) added to {}",
                s(&v, "key"),
                s(&v, "name"),
                pot.to_uppercase()
            ));
        }
        MenuCmd::Edit {
            pot,
            key,
            name,
            description,
            due,
            position,
        } => {
            let b = edit_body(
                name.as_deref(),
                description.as_deref(),
                due.as_deref(),
                position,
                None,
            );
            if b.as_object().is_some_and(Map::is_empty) {
                bail!("nothing to change: pass --name, --description, --due or --position");
            }
            let v = api.patch(&format!("{}/{key}", path(&pot)), b).await?;
            ok_line(format!("menu {} updated", s(&v, "key")));
        }
        MenuCmd::Serve { pot, key } => {
            let v = set_status(api, &pot, &key, "served").await?;
            ok_line(format!("served {}", s(&v, "name")));
        }
        MenuCmd::Reopen { pot, key } => {
            let v = set_status(api, &pot, &key, "open").await?;
            ok_line(format!("reopened {}", s(&v, "name")));
        }
    }
    Ok(())
}

async fn set_status(api: &Api, pot: &str, key: &str, status: &str) -> Result<Value> {
    api.patch(
        &format!("{}/{key}", path(pot)),
        edit_body(None, None, None, None, Some(status)),
    )
    .await
}

/// `task edit` body: only what was asked for; `--no-menu` (or `--menu ""`) takes the task off its menu.
pub fn task_edit_body(
    title: Option<&str>,
    body: Option<&str>,
    labels: &[String],
    priority: Option<i64>,
    menu: Option<&str>,
    no_menu: bool,
) -> Value {
    let mut m = Map::new();
    if let Some(t) = title {
        m.insert("title".into(), json!(t));
    }
    if let Some(b) = body {
        m.insert("body".into(), json!(b));
    }
    if !labels.is_empty() {
        m.insert("labels".into(), json!(labels));
    }
    if let Some(p) = priority {
        m.insert("priority".into(), json!(p));
    }
    if no_menu {
        m.insert("menu".into(), json!(""));
    } else if let Some(k) = menu {
        m.insert("menu".into(), json!(k));
    }
    Value::Object(m)
}

/// Query string pairs of `task ls`; `--menu -` (tasks on no menu) is passed through for the server.
pub fn task_ls_params(
    status: Option<&str>,
    label: Option<&str>,
    menu: Option<&str>,
    ready: bool,
) -> Vec<(&'static str, String)> {
    let mut p = vec![];
    if let Some(s) = status {
        p.push(("status", s.to_string()));
    }
    if let Some(l) = label {
        p.push(("label", l.to_string()));
    }
    if let Some(m) = menu {
        p.push(("menu", m.to_string()));
    }
    if ready {
        p.push(("ready_only", "true".to_string()));
    }
    p
}

/// `task scoop` body: labels, plus the menu to take work from when given.
pub fn claim_next_body(labels: &[String], menu: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("labels".into(), json!(labels));
    if let Some(k) = menu {
        m.insert("menu".into(), json!(k));
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_path_adds_all_only_when_asked() {
        assert_eq!(list_path("pay", false), "/projects/PAY/menus");
        assert_eq!(list_path("pay", true), "/projects/PAY/menus?all=true");
    }

    #[test]
    fn add_body_omits_unset_fields() {
        assert_eq!(
            add_body("MVP", None, None, None, None),
            json!({"name":"MVP"})
        );
        assert_eq!(
            add_body("MVP", Some("mvp"), Some("d"), Some("2026-12-01"), Some(2)),
            json!({"name":"MVP","key":"mvp","description":"d","due_on":"2026-12-01","position":2})
        );
    }

    #[test]
    fn edit_body_keeps_empty_due_to_clear_it() {
        assert_eq!(edit_body(None, None, None, None, None), json!({}));
        assert_eq!(
            edit_body(None, None, Some(""), None, None),
            json!({"due_on":""})
        );
        assert_eq!(
            edit_body(Some("N"), None, None, Some(1), Some("served")),
            json!({"name":"N","position":1,"status":"served"})
        );
    }

    #[test]
    fn task_edit_body_sets_and_clears_the_menu() {
        assert_eq!(
            task_edit_body(None, None, &[], None, None, false),
            json!({})
        );
        assert_eq!(
            task_edit_body(Some("T"), None, &["a".into()], Some(3), Some("mvp"), false),
            json!({"title":"T","labels":["a"],"priority":3,"menu":"mvp"})
        );
        assert_eq!(
            task_edit_body(None, None, &[], None, None, true),
            json!({"menu":""})
        );
        assert_eq!(
            task_edit_body(None, None, &[], None, Some(""), false),
            json!({"menu":""})
        );
    }

    #[test]
    fn task_ls_params_include_menu_and_dash() {
        assert!(task_ls_params(None, None, None, false).is_empty());
        assert_eq!(
            task_ls_params(Some("open"), None, Some("-"), true),
            vec![
                ("status", "open".to_string()),
                ("menu", "-".to_string()),
                ("ready_only", "true".to_string())
            ]
        );
    }

    #[test]
    fn claim_next_body_names_the_menu_only_when_given() {
        assert_eq!(claim_next_body(&[], None), json!({"labels":[]}));
        assert_eq!(
            claim_next_body(&["x".into()], Some("mvp")),
            json!({"labels":["x"],"menu":"mvp"})
        );
    }
}
