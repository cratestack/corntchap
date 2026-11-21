#![allow(non_snake_case)]
//! The cratestack procedure registry: each MCP tool is a thin adapter onto `service`.

use cratestack::{CratestackContext, CratestackError, Value};
use uuid::Uuid;

use crate::{
    error::Error,
    pantry,
    schema::cratestack_schema::{self, procedures::*},
    service,
    state::{AppState, Principal},
    stream,
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
    };
    p.worker().map_err(map_err)
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
        typed(service::claim_task(&self.state.db, &p, &a.id).await)
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
        typed(
            service::check_item(&self.state.db, &p, &a.id, Some(a.epoch), a.position, a.done).await,
        )
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
        typed(service::human_transition(&self.state.db, &p, &a.id, "unblock").await)
    }

    async fn cancel_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: cancel_task::Args,
        _z: cancel_task::Authorized,
    ) -> Result<cancel_task::Output, CratestackError> {
        let p = principal(ctx)?;
        typed(service::human_transition(&self.state.db, &p, &a.id, "cancel").await)
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
}
