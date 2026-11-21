#![allow(non_snake_case)]
//! The cratestack procedure registry: each MCP tool is a thin adapter onto `service`.

use cratestack::{CratestackContext, CratestackError, Json, Value};
use uuid::Uuid;

use crate::{
    error::Error,
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
type Res = Result<Json<Value>, CratestackError>;

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
        ("maxLeases".to_string(), Value::Int(p.max_leases as i64)),
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
            Some(Value::Int(n)) => *n as i32,
            _ => 1,
        },
        owner_id: field_str(ctx, "ownerId").and_then(|s| s.parse::<Uuid>().ok()),
        runtime: field_str(ctx, "runtime"),
        agent_kind: field_str(ctx, "agentKind"),
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

fn out(r: crate::error::Result<serde_json::Value>) -> Res {
    r.map(|v| Json(Value::from_plain_json(v))).map_err(map_err)
}

impl ProcedureRegistry for Procedures {
    async fn whoami(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: whoami::Args,
        _z: whoami::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::whoami(&self.state.db, &p).await)
    }

    async fn list_projects(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        _a: list_projects::Args,
        _z: list_projects::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::list_projects(&self.state.db, &p).await)
    }

    async fn list_tasks(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: list_tasks::Args,
        _z: list_tasks::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::list_tasks(
            &self.state.db,
            &p,
            &a.project.to_uppercase(),
            a.status.as_deref(),
            a.label.as_deref(),
            a.readyOnly.unwrap_or(false),
        )
        .await)
    }

    async fn get_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: get_task::Args,
        _z: get_task::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::get_task(&self.state.db, &p, &a.id).await)
    }

    async fn get_board(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: get_board::Args,
        _z: get_board::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(
            service::board_markdown(&self.state.db, &p, &a.project.to_uppercase())
                .await
                .map(|m| serde_json::json!({"markdown": m})),
        )
    }

    async fn create_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: create_task::Args,
        _z: create_task::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::create_task(
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
                priority: a.priority.unwrap_or(0) as i32,
            },
        )
        .await)
    }

    async fn update_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: update_task::Args,
        _z: update_task::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        let labels = a.labels.as_ref().map(|_| csv(&a.labels));
        out(service::update_task(
            &self.state.db,
            &p,
            &a.id,
            service::TaskEdit {
                title: a.title,
                body: a.body,
                labels,
                priority: a.priority.map(|n| n as i32),
            },
        )
        .await)
    }

    async fn claim_next(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: claim_next::Args,
        _z: claim_next::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::claim_next(
            &self.state.db,
            &p,
            &a.project.to_uppercase(),
            &csv(&a.labels),
        )
        .await)
    }

    async fn claim_task(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: claim_task::Args,
        _z: claim_task::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::claim_task(&self.state.db, &p, &a.id).await)
    }

    async fn heartbeat(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: heartbeat::Args,
        _z: heartbeat::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::heartbeat(&self.state.db, &p, &a.id, a.epoch, a.ttlS.map(|n| n as f64)).await)
    }

    async fn check_item(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: check_item::Args,
        _z: check_item::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::check_item(
            &self.state.db,
            &p,
            &a.id,
            Some(a.epoch),
            a.position as i32,
            a.done,
        )
        .await)
    }

    async fn comment(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: comment::Args,
        _z: comment::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::comment(&self.state.db, &p, &a.id, &a.body, a.epoch).await)
    }

    async fn complete(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: complete::Args,
        _z: complete::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::complete(&self.state.db, &p, &a.id, a.epoch, &a.result).await)
    }

    async fn release(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: release::Args,
        _z: release::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::release(&self.state.db, &p, &a.id, a.epoch, &a.reason).await)
    }

    async fn block(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: block::Args,
        _z: block::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        out(service::block(&self.state.db, &p, &a.id, a.epoch, &a.reason).await)
    }

    async fn changes_since(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: changes_since::Args,
        _z: changes_since::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        let project = a.project.map(|k| k.to_uppercase());
        out(service::changes_since(
            &self.state.db,
            &p,
            a.cursor,
            project.as_deref(),
            a.limit.unwrap_or(200),
        )
        .await)
    }

    async fn wait_for_work(
        &self,
        _db: &Db,
        ctx: &CratestackContext,
        a: wait_for_work::Args,
        _z: wait_for_work::Authorized,
    ) -> Res {
        let p = principal(ctx)?;
        let project = a.project.map(|k| k.to_uppercase());
        out(stream::wait_for_work(
            &self.state,
            &p,
            project.as_deref(),
            a.timeoutS.unwrap_or(30),
        )
        .await)
    }
}
