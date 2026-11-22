#![allow(non_snake_case)]
//! The cratestack procedure registry: each MCP tool is a thin adapter onto `service`.

use cratestack::{CratestackContext, CratestackError, Value};
use uuid::Uuid;

use crate::{
    admin,
    error::Error,
    files, menus, pantry, pass, questions,
    schema::cratestack_schema::{self, procedures::*},
    service,
    state::{AppState, Principal},
    stations, stream, views,
};

fn csv(v: &Option<String>) -> Vec<String> {
    v.as_deref()
        .unwrap_or("")
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn lines(v: &Option<String>) -> Vec<String> {
    v.as_deref()
        .unwrap_or("")
        .lines()
        .map(|x| {
            x.trim()
                .trim_start_matches("- [ ]")
                .trim_start_matches("- ")
                .trim()
                .to_string()
        })
        .filter(|x| !x.is_empty())
        .collect()
}

type Db = cratestack_schema::Cratestack;

#[derive(Clone)]
pub struct Procedures {
    pub state: AppState,
}

/// The caller's identity as the `auth Caller` block sees it.
pub fn ctx_fields(p: &Principal) -> Vec<(String, Value)> {
    let mut f = vec![
        ("id".to_string(), Value::String(p.id.to_string())),
        ("name".to_string(), Value::String(p.name.clone())),
        ("kind".to_string(), Value::String(p.kind.clone())),
        ("isAdmin".to_string(), Value::Bool(p.is_admin)),
        ("maxLeases".to_string(), Value::Int(p.max_leases)),
    ];
    if let Some(o) = p.owner_id {
        f.push(("ownerId".to_string(), Value::String(o.to_string())));
    }
    if let Some(r) = &p.runtime {
        f.push(("runtime".to_string(), Value::String(r.clone())));
    }
    if let Some(k) = &p.agent_kind {
        f.push(("agentKind".to_string(), Value::String(k.clone())));
    }
    if !p.grants.is_empty() {
        f.push(("grants".to_string(), Value::String(p.grants.join(","))));
    }
    f
}

fn field_str(ctx: &CratestackContext, k: &str) -> Option<String> {
    match ctx.auth_field(k) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn principal(ctx: &CratestackContext) -> Result<Principal, CratestackError> {
    let bad = || CratestackError::Unauthorized("authentication required".into());
    let p = Principal {
        id: field_str(ctx, "id")
            .and_then(|s| s.parse().ok())
            .ok_or_else(bad)?,
        name: field_str(ctx, "name").ok_or_else(bad)?,
        kind: field_str(ctx, "kind").ok_or_else(bad)?,
        is_admin: matches!(ctx.auth_field("isAdmin"), Some(Value::Bool(true))),
        max_leases: match ctx.auth_field("maxLeases") {
            Some(Value::Int(n)) => *n,
            _ => 1,
        },
        owner_id: field_str(ctx, "ownerId").and_then(|s| s.parse::<Uuid>().ok()),
        runtime: field_str(ctx, "runtime"),
        agent_kind: field_str(ctx, "agentKind"),
        token_id: None,
        grants: field_str(ctx, "grants")
            .map(|g| g.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
    };
    p.worker().map_err(map_err)
}

/// Wake targets as the typed view: `config` (which may hold secrets) becomes a short `target`.
fn wake_target_views(v: serde_json::Value) -> serde_json::Value {
    let rows = v.as_array().cloned().unwrap_or_default();
    serde_json::Value::Array(
        rows.into_iter()
            .map(|mut w| {
                let c = w["config"].take();
                let target = c["url"].as_str().map(str::to_string).or_else(|| {
                    c["repo"].as_str().map(|r| match &c["issue"] {
                        serde_json::Value::Null => r.to_string(),
                        i => format!(
                            "{r}#{}",
                            i.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| i.to_string())
                        ),
                    })
                });
                if let Some(o) = w.as_object_mut() {
                    o.remove("config");
                    o.insert("target".into(), serde_json::json!(target));
                    if let Some(id) = o.get("id").and_then(|x| x.as_str()).map(str::to_string) {
                        o.insert("id".into(), serde_json::json!(id));
                    }
                }
                w
            })
            .collect(),
    )
}

pub fn map_err(e: Error) -> CratestackError {
    match e {
        Error::NotFound(m) => CratestackError::NotFound(m),
        Error::Forbidden(m) => CratestackError::Forbidden(m),
        Error::Invalid(m) => CratestackError::BadRequest(m),
        Error::Conflict(m) => CratestackError::Conflict(m),
        Error::LeaseLost => CratestackError::Conflict(format!("lease_lost: {e}")),
        Error::MaxLeases => CratestackError::Conflict(format!("max_leases: {e}")),
        Error::Unauthorized | Error::Insecure(_) => CratestackError::Unauthorized(e.to_string()),
        Error::Db(_) | Error::Internal(_) => {
            let _ = e.body(); // logs the cause
            CratestackError::Internal("internal error".into())
        }
    }
}

fn typed<T: serde::de::DeserializeOwned>(
    r: crate::error::Result<serde_json::Value>,
) -> Result<T, CratestackError> {
    let v = r.map_err(map_err)?;
    serde_json::from_value(v).map_err(|e| {
        tracing::error!("typed procedure output does not match the schema: {e}");
        CratestackError::Internal("internal error".into())
    })
}

impl ProcedureRegistry for Procedures {
    async fn whoami(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: whoami::Args,
        _z: whoami::Authorized,
    ) -> Result<whoami::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::whoami(&self.state.db, &p).await)
    }

    async fn project_overview(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: project_overview::Args,
        _z: project_overview::Authorized,
    ) -> Result<project_overview::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::list_projects(&self.state.db, &p).await)
    }

    async fn project_tasks(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: project_tasks::Args,
        _z: project_tasks::Authorized,
    ) -> Result<project_tasks::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::list_tasks(
                &self.state.db,
                &p,
                &a.project.to_uppercase(),
                a.status.as_deref(),
                a.label.as_deref(),
                a.readyOnly.unwrap_or(false),
                a.menu.as_deref(),
            )
            .await,
        )
    }

    async fn task_detail(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: task_detail::Args,
        _z: task_detail::Authorized,
    ) -> Result<task_detail::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::get_task(&self.state.db, &p, &a.id).await)
    }

    async fn get_board(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: get_board::Args,
        _z: get_board::Authorized,
    ) -> Result<get_board::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::board_markdown(&self.state.db, &p, &a.project.to_uppercase())
                .await
                .map(|m| serde_json::json!({"markdown": m})),
        )
    }

    async fn get_task_markdown(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: get_task_markdown::Args,
        _z: get_task_markdown::Authorized,
    ) -> Result<get_task_markdown::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::task_markdown_for(&self.state.db, &p, &a.id)
                .await
                .map(|m| serde_json::json!({"markdown": m})),
        )
    }

    async fn task_create(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: task_create::Args,
        _z: task_create::Authorized,
    ) -> Result<task_create::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::create_task(
                &self.state.db,
                &p,
                service::NewTask {
                    project: a.project.to_uppercase(),
                    title: a.title,
                    body: a.body.unwrap_or_default(),
                    items: lines(&a.items),
                    parent: a.parent,
                    depends_on: csv(&a.dependsOn),
                    labels: csv(&a.labels),
                    priority: a.priority.unwrap_or(0),
                    menu: a.menu,
                },
            )
            .await,
        )
    }

    async fn task_edit(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: task_edit::Args,
        _z: task_edit::Authorized,
    ) -> Result<task_edit::Output, CratestackError> {
        let p = principal(ctx)?;
        let labels = a.labels.as_ref().map(|_| csv(&a.labels));
        typed(
            service::update_task(
                &self.state.db,
                &p,
                &a.id,
                service::TaskEdit {
                    title: a.title,
                    body: a.body,
                    labels,
                    priority: a.priority,
                    menu: a.menu,
                },
            )
            .await,
        )
    }

    async fn claim_next(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: claim_next::Args,
        _z: claim_next::Authorized,
    ) -> Result<claim_next::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::claim_next(
                &self.state.db,
                &p,
                &a.project.to_uppercase(),
                &csv(&a.labels),
                a.menu.as_deref(),
                a.station.as_deref(),
            )
            .await,
        )
    }

    async fn claim_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: claim_task::Args,
        _z: claim_task::Authorized,
    ) -> Result<claim_task::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::claim_task(&self.state.db, &p, &a.id, a.station.as_deref()).await)
    }

    async fn heartbeat(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: heartbeat::Args,
        _z: heartbeat::Authorized,
    ) -> Result<heartbeat::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::heartbeat(&self.state.db, &p, &a.id, a.epoch, a.ttlS.map(|n| n as f64)).await,
        )
    }

    async fn check_item(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: check_item::Args,
        _z: check_item::Authorized,
    ) -> Result<check_item::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::check_item(&self.state.db, &p, &a.id, a.epoch, a.position, a.done).await)
    }

    async fn comment(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: comment::Args,
        _z: comment::Authorized,
    ) -> Result<comment::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::comment(&self.state.db, &p, &a.id, &a.body, a.epoch).await)
    }

    async fn complete(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: complete::Args,
        _z: complete::Authorized,
    ) -> Result<complete::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::complete(&self.state.db, &p, &a.id, a.epoch, &a.result).await)
    }

    async fn release(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: release::Args,
        _z: release::Authorized,
    ) -> Result<release::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::release(&self.state.db, &p, &a.id, a.epoch, &a.reason).await)
    }

    async fn block(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: block::Args,
        _z: block::Authorized,
    ) -> Result<block::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::block(&self.state.db, &p, &a.id, a.epoch, &a.reason).await)
    }

    async fn changes_since(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: changes_since::Args,
        _z: changes_since::Authorized,
    ) -> Result<changes_since::Output, CratestackError> {
        let p = principal(ctx)?;
        let project = a.project.map(|k| k.to_uppercase());
        typed(
            service::changes_since(
                &self.state.db,
                &p,
                a.cursor,
                project.as_deref(),
                a.limit.unwrap_or(200),
            )
            .await,
        )
    }

    async fn wait_for_work(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: wait_for_work::Args,
        _z: wait_for_work::Authorized,
    ) -> Result<wait_for_work::Output, CratestackError> {
        let p = principal(ctx)?;
        let project = a.project.map(|k| k.to_uppercase());
        typed(
            stream::wait_for_work(
                &self.state,
                &p,
                project.as_deref(),
                a.timeoutS.unwrap_or(30),
            )
            .await,
        )
    }

    async fn unblock_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: unblock_task::Args,
        _z: unblock_task::Authorized,
    ) -> Result<unblock_task::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::human_transition(&self.state.db, &p, &a.id, "unblock", None).await)
    }

    async fn cancel_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: cancel_task::Args,
        _z: cancel_task::Authorized,
    ) -> Result<cancel_task::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            service::human_transition(&self.state.db, &p, &a.id, "cancel", a.reason.as_deref())
                .await,
        )
    }

    async fn restore_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: restore_task::Args,
        _z: restore_task::Authorized,
    ) -> Result<restore_task::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::human_transition(&self.state.db, &p, &a.id, "restore", None).await)
    }

    async fn memory_recall(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: memory_recall::Args,
        _z: memory_recall::Authorized,
    ) -> Result<memory_recall::Output, CratestackError> {
        let p = principal(ctx)?;
        let r = pantry::Recall {
            project: a.project,
            query: a.query,
            kind: a.kind,
            tags: csv(&a.tags),
            limit: a.limit,
        };
        typed(pantry::recall(&self.state.db, &p, r).await)
    }

    async fn memory_get(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: memory_get::Args,
        _z: memory_get::Authorized,
    ) -> Result<memory_get::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(pantry::get(&self.state.db, &p, &a.project, &a.key).await)
    }

    async fn memory_digest(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: memory_digest::Args,
        _z: memory_digest::Authorized,
    ) -> Result<memory_digest::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            pantry::digest(&self.state.db, &p, &a.project)
                .await
                .map(|m| serde_json::json!({"markdown": m})),
        )
    }

    async fn memory_remember(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: memory_remember::Args,
        _z: memory_remember::Authorized,
    ) -> Result<memory_remember::Output, CratestackError> {
        let p = principal(ctx)?;
        let tags = a.tags.as_ref().map(|_| csv(&a.tags));
        let r = pantry::Remember {
            project: a.project,
            key: a.key,
            title: a.title.unwrap_or_default(),
            body: a.body,
            kind: a.kind,
            tags,
            task: a.task,
            append: a.append.unwrap_or(false),
            revision: a.revision,
        };
        typed(pantry::remember(&self.state.db, &p, r).await)
    }

    async fn memory_forget(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: memory_forget::Args,
        _z: memory_forget::Authorized,
    ) -> Result<memory_forget::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(pantry::forget(&self.state.db, &p, &a.project, &a.key).await)
    }

    async fn menu_list(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: menu_list::Args,
        _z: menu_list::Authorized,
    ) -> Result<menu_list::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            menus::list(
                &self.state.db,
                &p,
                &a.project,
                a.includeServed.unwrap_or(false),
            )
            .await,
        )
    }

    async fn menu_create(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: menu_create::Args,
        _z: menu_create::Authorized,
    ) -> Result<menu_create::Output, CratestackError> {
        let p = principal(ctx)?;
        let n = menus::NewMenu {
            project: a.project,
            name: a.name,
            key: a.key,
            description: a.description,
            due_on: a.dueOn,
            position: a.position,
        };
        typed(menus::create(&self.state.db, &p, n).await)
    }

    async fn menu_update(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: menu_update::Args,
        _z: menu_update::Authorized,
    ) -> Result<menu_update::Output, CratestackError> {
        let p = principal(ctx)?;
        let e = menus::MenuEdit {
            name: a.name,
            description: a.description,
            due_on: a.dueOn,
            position: a.position,
            status: a.status,
        };
        typed(menus::update(&self.state.db, &p, &a.project, &a.key, e).await)
    }

    async fn admin_users(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: admin_users::Args,
        _z: admin_users::Authorized,
    ) -> Result<admin_users::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::list_users(db, &p).await)
    }

    async fn admin_add_user(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_user::Args,
        _z: admin_add_user::Authorized,
    ) -> Result<admin_add_user::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            admin::create_user(db, &p, &a.login, a.isAdmin.unwrap_or(false))
                .await
                .map(|mut v| {
                    v["linked"] = serde_json::json!(false);
                    v
                }),
        )
    }

    async fn admin_teams(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: admin_teams::Args,
        _z: admin_teams::Authorized,
    ) -> Result<admin_teams::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::list_teams(db, &p).await)
    }

    async fn admin_add_team(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_team::Args,
        _z: admin_add_team::Authorized,
    ) -> Result<admin_add_team::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::create_team(db, &p, &a.name).await.map(|mut v| {
            v["id"] = serde_json::json!(
                v["id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v["id"].to_string())
            );
            v
        }))
    }

    async fn admin_team_members(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_team_members::Args,
        _z: admin_team_members::Authorized,
    ) -> Result<admin_team_members::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::list_members(db, &p, &a.team).await)
    }

    async fn admin_add_member(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_member::Args,
        _z: admin_add_member::Authorized,
    ) -> Result<admin_add_member::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            admin::add_member(
                db,
                &p,
                &a.team,
                &a.principal,
                a.role.as_deref().unwrap_or("member"),
            )
            .await,
        )
    }

    async fn admin_remove_member(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_remove_member::Args,
        _z: admin_remove_member::Authorized,
    ) -> Result<admin_remove_member::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::remove_member(db, &p, &a.team, &a.principal).await)
    }

    async fn admin_add_project(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_project::Args,
        _z: admin_add_project::Authorized,
    ) -> Result<admin_add_project::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            admin::create_project(
                db,
                &p,
                &a.team,
                &a.key,
                &a.name,
                a.repoUrl.as_deref(),
                a.leaseTtlS,
            )
            .await,
        )
    }

    async fn admin_cooks(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: admin_cooks::Args,
        _z: admin_cooks::Authorized,
    ) -> Result<admin_cooks::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::list_principals(db, &p, "agent").await)
    }

    async fn admin_fires(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: admin_fires::Args,
        _z: admin_fires::Authorized,
    ) -> Result<admin_fires::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::list_principals(db, &p, "relay").await)
    }

    async fn admin_add_cook(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_cook::Args,
        _z: admin_add_cook::Authorized,
    ) -> Result<admin_add_cook::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            admin::create_principal(
                db,
                &p,
                "agent",
                &a.name,
                a.agentKind.as_deref(),
                a.runtime.as_deref(),
                a.maxLeases,
                a.owner.as_deref(),
            )
            .await,
        )
    }

    async fn admin_add_fire(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_fire::Args,
        _z: admin_add_fire::Authorized,
    ) -> Result<admin_add_fire::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::create_principal(db, &p, "relay", &a.name, None, None, None, None).await)
    }

    async fn admin_rotate(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_rotate::Args,
        _z: admin_rotate::Authorized,
    ) -> Result<admin_rotate::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::rotate(db, &p, &a.name).await)
    }

    async fn admin_set_enabled(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_set_enabled::Args,
        _z: admin_set_enabled::Authorized,
    ) -> Result<admin_set_enabled::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::set_disabled(db, &p, &a.name, !a.enabled).await)
    }

    async fn admin_wake_targets(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_wake_targets::Args,
        _z: admin_wake_targets::Authorized,
    ) -> Result<admin_wake_targets::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            admin::list_wake_targets(db, &p, &a.cook)
                .await
                .map(wake_target_views),
        )
    }

    async fn admin_add_wake_target(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_add_wake_target::Args,
        _z: admin_add_wake_target::Authorized,
    ) -> Result<admin_add_wake_target::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed({
            let mut config = serde_json::Map::new();
            if let Some(u) = a.url {
                config.insert("url".into(), serde_json::json!(u));
            }
            if let Some(r) = a.repo {
                config.insert("repo".into(), serde_json::json!(r));
            }
            if let Some(i) = a.issue {
                config.insert("issue".into(), serde_json::json!(i));
            }
            if let Some(t) = a.token {
                config.insert("token".into(), serde_json::json!(t));
            }
            admin::add_wake_target(
                db,
                &p,
                &a.cook,
                a.r#type.as_deref().unwrap_or("relay"),
                csv(&a.projects),
                csv(&a.labels),
                serde_json::Value::Object(config),
            )
            .await
            .map(|mut v| {
                v["id"] =
                    serde_json::json!(v["id"].as_str().map(str::to_string).unwrap_or_default());
                v
            })
        })
    }

    async fn admin_delete_wake_target(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_delete_wake_target::Args,
        _z: admin_delete_wake_target::Authorized,
    ) -> Result<admin_delete_wake_target::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(match a.id.parse::<Uuid>() {
            Ok(id) => admin::delete_wake_target(db, &p, id).await,
            Err(_) => Err(Error::Invalid(format!("bad wake target id {:?}", a.id))),
        })
    }

    async fn admin_set_grants(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: admin_set_grants::Args,
        _z: admin_set_grants::Authorized,
    ) -> Result<admin_set_grants::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(admin::set_grants(db, &p, &a.name, &csv(&Some(a.grants.clone()))).await)
    }

    async fn station_register(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: station_register::Args,
        _z: station_register::Authorized,
    ) -> Result<station_register::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            stations::register(
                db,
                &p,
                stations::NewStation {
                    cook: a.cook,
                    project: a.project,
                    runtime: a.runtime,
                    session: a.session,
                    directory: a.directory,
                    label: a.label,
                },
            )
            .await,
        )
    }

    async fn station_list(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: station_list::Args,
        _z: station_list::Authorized,
    ) -> Result<station_list::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(stations::list(db, &p, a.project.as_deref(), a.cook.as_deref()).await)
    }

    async fn station_remove(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: station_remove::Args,
        _z: station_remove::Authorized,
    ) -> Result<station_remove::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(stations::remove(db, &p, &a.id).await)
    }

    async fn question_ask(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_ask::Args,
        _z: question_ask::Authorized,
    ) -> Result<question_ask::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(
            questions::ask(
                db,
                &p,
                questions::Ask {
                    project: a.project,
                    body: a.question,
                    task: a.task,
                    options: lines(&a.options),
                    to: a.to,
                    wait: a.wait.unwrap_or(false),
                    station: a.station,
                },
            )
            .await,
        )
    }

    async fn question_list(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_list::Args,
        _z: question_list::Authorized,
    ) -> Result<question_list::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::list(db, &p, &a.project, a.status.as_deref()).await)
    }

    async fn question_mine(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_mine::Args,
        _z: question_mine::Authorized,
    ) -> Result<question_mine::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::mine(db, &p, a.status.as_deref()).await)
    }

    async fn question_get(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_get::Args,
        _z: question_get::Authorized,
    ) -> Result<question_get::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::get(db, &p, &a.id).await)
    }

    async fn question_pick(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_pick::Args,
        _z: question_pick::Authorized,
    ) -> Result<question_pick::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::pick(db, &p, a.project.as_deref()).await)
    }

    async fn question_answer(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_answer::Args,
        _z: question_answer::Authorized,
    ) -> Result<question_answer::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::answer(db, &p, &a.id, &a.answer, false).await)
    }

    async fn question_release(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_release::Args,
        _z: question_release::Authorized,
    ) -> Result<question_release::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::release(db, &p, &a.id).await)
    }

    async fn question_note(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_note::Args,
        _z: question_note::Authorized,
    ) -> Result<question_note::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::note(db, &p, &a.id, &a.note).await)
    }

    async fn question_dismiss(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: question_dismiss::Args,
        _z: question_dismiss::Authorized,
    ) -> Result<question_dismiss::Output, CratestackError> {
        let p = principal(ctx)?;
        let db = &self.state.db;
        typed(questions::answer(db, &p, &a.id, a.reason.as_deref().unwrap_or(""), true).await)
    }

    async fn file_create_upload(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: file_create_upload::Args,
        _z: file_create_upload::Authorized,
    ) -> Result<file_create_upload::Output, CratestackError> {
        let p = principal(ctx)?;
        let (db, st) = (&self.state.db, &self.state);
        typed(
            files::create_upload(
                db,
                st,
                &p,
                files::NewFile {
                    project: a.project,
                    name: a.name,
                    content_type: a.contentType,
                    size: Some(a.size),
                    task: a.task,
                    question: a.question,
                },
            )
            .await,
        )
    }

    async fn file_upload_inline(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: file_upload_inline::Args,
        _z: file_upload_inline::Authorized,
    ) -> Result<file_upload_inline::Output, CratestackError> {
        let p = principal(ctx)?;
        let (db, st) = (&self.state.db, &self.state);
        typed(
            files::upload_inline(
                db,
                st,
                &p,
                files::NewFile {
                    project: a.project,
                    name: a.name,
                    content_type: a.contentType,
                    size: None,
                    task: a.task,
                    question: a.question,
                },
                &a.contentBase64,
            )
            .await,
        )
    }

    async fn file_get(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: file_get::Args,
        _z: file_get::Authorized,
    ) -> Result<file_get::Output, CratestackError> {
        let p = principal(ctx)?;
        let (db, st) = (&self.state.db, &self.state);
        typed(files::get(db, st, &p, &a.id).await)
    }

    async fn file_list(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: file_list::Args,
        _z: file_list::Authorized,
    ) -> Result<file_list::Output, CratestackError> {
        let p = principal(ctx)?;
        let (db, st) = (&self.state.db, &self.state);
        typed(
            files::list(
                db,
                st,
                &p,
                &a.project,
                a.task.as_deref(),
                a.question.as_deref(),
            )
            .await,
        )
    }

    async fn file_delete(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: file_delete::Args,
        _z: file_delete::Authorized,
    ) -> Result<file_delete::Output, CratestackError> {
        let p = principal(ctx)?;
        let (db, st) = (&self.state.db, &self.state);
        typed(files::delete(db, st, &p, &a.id).await)
    }

    async fn pass_post(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: pass_post::Args,
        _z: pass_post::Authorized,
    ) -> Result<pass_post::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            pass::post(
                &self.state.db,
                &p,
                pass::Post {
                    project: a.project,
                    body: a.message,
                    to: a.to,
                    task: a.task,
                    from_owner: a.fromOwner.unwrap_or(false),
                },
            )
            .await,
        )
    }

    async fn pass_read(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: pass_read::Args,
        _z: pass_read::Authorized,
    ) -> Result<pass_read::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            pass::read(
                &self.state.db,
                &p,
                &a.project,
                a.chef.as_deref(),
                a.after,
                a.limit,
            )
            .await,
        )
    }

    async fn view_list(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: view_list::Args,
        _z: view_list::Authorized,
    ) -> Result<view_list::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(views::list(&self.state.db, &p, &a.project).await)
    }

    async fn view_save(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: view_save::Args,
        _z: view_save::Authorized,
    ) -> Result<view_save::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            views::save(
                &self.state.db,
                &p,
                views::Save {
                    project: a.project,
                    name: a.name,
                    key: a.key,
                    description: a.description,
                    layout: a.layout,
                    where_sql: a.r#where,
                    order_sql: a.orderBy,
                },
            )
            .await,
        )
    }

    async fn view_delete(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: view_delete::Args,
        _z: view_delete::Authorized,
    ) -> Result<view_delete::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(views::delete(&self.state.db, &p, &a.project, &a.key).await)
    }

    async fn view_run(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: view_run::Args,
        _z: view_run::Authorized,
    ) -> Result<view_run::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(
            views::run(
                &self.state.db,
                &p,
                &a.project,
                a.key.as_deref(),
                a.r#where.as_deref(),
                a.orderBy.as_deref(),
            )
            .await
            .map(serde_json::Value::Array),
        )
    }
}
