//! REST at /api/v1: the CLI, scripts and admin side. Same service functions as the MCP tools.

use authkestra::axum::Auth as Authn;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    admin,
    error::{Error, Result},
    menus, pantry,
    service::{self, NewTask, TaskEdit},
    state::{AppState, Principal},
    stream,
};

type Auth = Authn<Principal>;
type J = Result<Json<Value>>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route("/users", get(users_list).post(users_add))
        .route("/teams", get(teams_list).post(teams_add))
        .route("/teams/{team}/members", get(members_list).post(members_add))
        .route("/teams/{team}/members/{name}", delete(members_remove))
        .route("/projects", get(projects_list).post(projects_add))
        .route("/projects/{key}/tasks", get(tasks_list).post(tasks_add))
        .route("/projects/{key}/claim-next", post(claim_next))
        .route("/projects/{key}/digest", get(digest))
        .route("/projects/{key}/board", get(board))
        .route("/projects/{key}/menus", get(menus_list).post(menus_add))
        .route(
            "/projects/{key}/menus/{mkey}",
            axum::routing::patch(menus_edit),
        )
        .route("/projects/{key}/pantry", get(pantry_list).post(pantry_save))
        .route(
            "/projects/{key}/pantry/{mkey}",
            get(pantry_get).put(pantry_put).delete(pantry_forget),
        )
        .route("/tasks/{id}", get(task_get).patch(task_patch))
        .route("/tasks/{id}/claim", post(task_claim))
        .route("/tasks/{id}/heartbeat", post(task_heartbeat))
        .route("/tasks/{id}/check", post(task_check))
        .route("/tasks/{id}/comment", post(task_comment))
        .route("/tasks/{id}/complete", post(task_complete))
        .route("/tasks/{id}/release", post(task_release))
        .route("/tasks/{id}/block", post(task_block))
        .route("/tasks/{id}/unblock", post(task_unblock))
        .route("/tasks/{id}/cancel", post(task_cancel))
        .route("/changes", get(changes))
        .route("/events/stream", get(stream::events_stream))
        .route("/wait", get(wait))
        .route("/agents", get(agents_list).post(agents_add))
        .route("/agents/{name}/rotate", post(rotate))
        .route("/agents/{name}/disable", post(disable))
        .route("/agents/{name}/enable", post(enable))
        .route(
            "/agents/{name}/wake-targets",
            get(targets_list).post(targets_add),
        )
        .route("/wake-targets/{id}", delete(targets_delete))
        .route("/relays", get(relays_list).post(relays_add))
        .route("/relays/{name}/rotate", post(rotate))
        .route("/relay/stream", get(stream::relay_stream))
        .route("/relay/runs", post(stream::report_run))
}

// ---- tiny body helpers ----
fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string())
}
fn req_s(v: &Value, k: &str) -> Result<String> {
    s(v, k).ok_or_else(|| Error::Invalid(format!("{k} is required")))
}
fn arr(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}
fn req_epoch(v: &Value) -> Result<i64> {
    i(v, "epoch").ok_or_else(|| Error::Invalid("epoch is required".into()))
}
fn ok(v: Value) -> J {
    Ok(Json(v))
}

async fn me(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(service::whoami(&st.db, &p).await?)
}

/// Revoke exactly the bearer token this request used (the CLI's `logout`).
async fn logout(State(st): State<AppState>, Authn(p): Auth) -> J {
    let Some(tid) = p.token_id else {
        return Err(Error::Invalid("no bearer token on this request".into()));
    };
    sqlx::query("update tokens set revoked_at = now() where id = $1 and revoked_at is null")
        .bind(tid)
        .execute(&st.db)
        .await?;
    ok(json!({"ok": true}))
}

async fn users_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(admin::list_users(&st.db, &p).await?)
}
async fn users_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::create_user(
        &st.db,
        &p,
        &req_s(&b, "login")?,
        b["is_admin"].as_bool().unwrap_or(false),
    )
    .await?)
}

async fn teams_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(admin::list_teams(&st.db, &p).await?)
}
async fn teams_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::create_team(&st.db, &p, &req_s(&b, "name")?).await?)
}
async fn members_list(State(st): State<AppState>, Authn(p): Auth, Path(t): Path<String>) -> J {
    ok(admin::list_members(&st.db, &p, &t).await?)
}
async fn members_add(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(t): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(admin::add_member(
        &st.db,
        &p,
        &t,
        &req_s(&b, "principal")?,
        &s(&b, "role").unwrap_or_else(|| "member".into()),
    )
    .await?)
}
async fn members_remove(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((t, n)): Path<(String, String)>,
) -> J {
    ok(admin::remove_member(&st.db, &p, &t, &n).await?)
}

async fn projects_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(service::list_projects(&st.db, &p.worker()?).await?)
}
async fn projects_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::create_project(
        &st.db,
        &p,
        &req_s(&b, "team")?,
        &req_s(&b, "key")?,
        &req_s(&b, "name")?,
        s(&b, "repo_url").as_deref(),
        i(&b, "lease_ttl_s"),
    )
    .await?)
}

async fn tasks_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let ready = q
        .get("ready_only")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    ok(service::list_tasks(
        &st.db,
        &p.worker()?,
        &k.to_uppercase(),
        q.get("status").map(|x| x.as_str()),
        q.get("label").map(|x| x.as_str()),
        ready,
        q.get("menu").map(|x| x.as_str()),
    )
    .await?)
}
async fn tasks_add(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::create_task(
        &st.db,
        &p.worker()?,
        NewTask {
            project: k.to_uppercase(),
            title: req_s(&b, "title")?,
            body: s(&b, "body").unwrap_or_default(),
            items: arr(&b, "items"),
            parent: s(&b, "parent"),
            depends_on: arr(&b, "depends_on"),
            labels: arr(&b, "labels"),
            priority: i(&b, "priority").unwrap_or(0),
            menu: s(&b, "menu"),
        },
    )
    .await?)
}
async fn claim_next(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    body: Option<Json<Value>>,
) -> J {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let menu = s(&b, "menu");
    ok(service::claim_next(
        &st.db,
        &p.worker()?,
        &k.to_uppercase(),
        &arr(&b, "labels"),
        menu.as_deref(),
    )
    .await?)
}
async fn digest(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let since = q.get("since").and_then(|v| v.parse().ok()).unwrap_or(0);
    ok(service::digest(&st.db, &p.worker()?, &k.to_uppercase(), since).await?)
}
async fn board(State(st): State<AppState>, Authn(p): Auth, Path(k): Path<String>) -> J {
    ok(json!({"markdown": service::board_markdown(&st.db, &p.worker()?, &k.to_uppercase()).await?}))
}

// ---- menus (milestones) ----
/// `?all=true` includes served menus.
async fn menus_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let all = q.get("all").is_some_and(|v| v == "true" || v == "1");
    ok(menus::list(&st.db, &p.worker()?, &k, all).await?)
}
async fn menus_add(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let n = menus::NewMenu {
        project: k,
        name: req_s(&b, "name")?,
        key: s(&b, "key"),
        description: s(&b, "description"),
        due_on: s(&b, "due_on"),
        position: i(&b, "position"),
    };
    ok(menus::create(&st.db, &p.worker()?, n).await?)
}
async fn menus_edit(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, mk)): Path<(String, String)>,
    Json(b): Json<Value>,
) -> J {
    let e = menus::MenuEdit {
        name: s(&b, "name"),
        description: s(&b, "description"),
        due_on: s(&b, "due_on"),
        position: i(&b, "position"),
        status: s(&b, "status"),
    };
    ok(menus::update(&st.db, &p.worker()?, &k, &mk, e).await?)
}

// ---- the pantry (shared memory per project) ----
/// `?q=&kind=&tags=a,b&limit=`; `?format=markdown` returns the digest `{markdown}`.
async fn pantry_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let p = p.worker()?;
    if q.get("format").map(String::as_str) == Some("markdown") {
        return ok(json!({"markdown": pantry::digest(&st.db, &p, &k).await?}));
    }
    let r = pantry::Recall {
        project: k,
        query: q.get("q").cloned(),
        kind: q.get("kind").cloned(),
        tags: q
            .get("tags")
            .map(|t| t.split(',').map(|x| x.trim().to_string()).collect())
            .unwrap_or_default(),
        limit: q.get("limit").and_then(|v| v.parse().ok()),
    };
    ok(pantry::recall(&st.db, &p, r).await?)
}
fn remember_body(project: String, key: Option<String>, b: &Value) -> Result<pantry::Remember> {
    Ok(pantry::Remember {
        project,
        key: key.or_else(|| s(b, "key")),
        title: s(b, "title").unwrap_or_default(),
        body: req_s(b, "body")?,
        kind: s(b, "kind"),
        tags: b.get("tags").map(|_| arr(b, "tags")),
        task: s(b, "task"),
        append: b["append"].as_bool().unwrap_or(false),
        revision: i(b, "revision"),
    })
}
async fn pantry_save(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(pantry::remember(&st.db, &p.worker()?, remember_body(k, None, &b)?).await?)
}
async fn pantry_put(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, mk)): Path<(String, String)>,
    Json(b): Json<Value>,
) -> J {
    ok(pantry::remember(&st.db, &p.worker()?, remember_body(k, Some(mk), &b)?).await?)
}
async fn pantry_get(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, mk)): Path<(String, String)>,
) -> J {
    ok(pantry::get(&st.db, &p.worker()?, &k, &mk).await?)
}
async fn pantry_forget(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, mk)): Path<(String, String)>,
) -> J {
    ok(pantry::forget(&st.db, &p.worker()?, &k, &mk).await?)
}

async fn task_get(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::get_task(&st.db, &p.worker()?, &id).await?)
}
async fn task_patch(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let labels = b.get("labels").map(|_| arr(&b, "labels"));
    ok(service::update_task(
        &st.db,
        &p.worker()?,
        &id,
        TaskEdit {
            title: s(&b, "title"),
            body: s(&b, "body"),
            labels,
            priority: i(&b, "priority"),
            menu: s(&b, "menu"),
        },
    )
    .await?)
}
async fn task_claim(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::claim_task(&st.db, &p.worker()?, &id).await?)
}
async fn task_heartbeat(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::heartbeat(
        &st.db,
        &p.worker()?,
        &id,
        req_epoch(&b)?,
        i(&b, "ttl_s").map(|n| n as f64),
    )
    .await?)
}
async fn task_check(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let pos = i(&b, "position").ok_or_else(|| Error::Invalid("position is required".into()))?;
    ok(service::check_item(
        &st.db,
        &p.worker()?,
        &id,
        i(&b, "epoch"),
        pos,
        b["done"].as_bool().unwrap_or(true),
    )
    .await?)
}
async fn task_comment(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::comment(
        &st.db,
        &p.worker()?,
        &id,
        &req_s(&b, "body")?,
        i(&b, "epoch"),
    )
    .await?)
}
async fn task_complete(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::complete(
        &st.db,
        &p.worker()?,
        &id,
        req_epoch(&b)?,
        &s(&b, "result").unwrap_or_default(),
    )
    .await?)
}
async fn task_release(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::release(
        &st.db,
        &p.worker()?,
        &id,
        req_epoch(&b)?,
        &s(&b, "reason").unwrap_or_default(),
    )
    .await?)
}
async fn task_block(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(service::block(
        &st.db,
        &p.worker()?,
        &id,
        req_epoch(&b)?,
        &s(&b, "reason").unwrap_or_default(),
    )
    .await?)
}
async fn task_unblock(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::human_transition(&st.db, &p.worker()?, &id, "unblock").await?)
}
async fn task_cancel(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::human_transition(&st.db, &p.worker()?, &id, "cancel").await?)
}

async fn changes(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    // `since` is accepted too: the digest takes it, and agents guess it.
    let cursor = q
        .get("cursor")
        .or_else(|| q.get("since"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(200);
    let project = q.get("project").map(|k| k.to_uppercase());
    ok(service::changes_since(&st.db, &p.worker()?, cursor, project.as_deref(), limit).await?)
}
async fn wait(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let t = q
        .get("timeout_s")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let project = q.get("project").map(|k| k.to_uppercase());
    ok(stream::wait_for_work(&st, &p.worker()?, project.as_deref(), t).await?)
}

async fn agents_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(admin::list_principals(&st.db, &p, "agent").await?)
}
async fn agents_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::create_principal(
        &st.db,
        &p,
        "agent",
        &req_s(&b, "name")?,
        s(&b, "kind").as_deref(),
        s(&b, "runtime").as_deref(),
        i(&b, "max_leases"),
        s(&b, "owner").as_deref(),
    )
    .await?)
}
async fn relays_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    ok(admin::list_principals(&st.db, &p, "relay").await?)
}
async fn relays_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::create_principal(
        &st.db,
        &p,
        "relay",
        &req_s(&b, "name")?,
        None,
        None,
        None,
        s(&b, "owner").as_deref(),
    )
    .await?)
}
async fn rotate(State(st): State<AppState>, Authn(p): Auth, Path(n): Path<String>) -> J {
    ok(admin::rotate(&st.db, &p, &n).await?)
}
async fn disable(State(st): State<AppState>, Authn(p): Auth, Path(n): Path<String>) -> J {
    ok(admin::set_disabled(&st.db, &p, &n, true).await?)
}
async fn enable(State(st): State<AppState>, Authn(p): Auth, Path(n): Path<String>) -> J {
    ok(admin::set_disabled(&st.db, &p, &n, false).await?)
}
async fn targets_list(State(st): State<AppState>, Authn(p): Auth, Path(n): Path<String>) -> J {
    ok(admin::list_wake_targets(&st.db, &p, &n).await?)
}
async fn targets_add(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(n): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(admin::add_wake_target(
        &st.db,
        &p,
        &n,
        &req_s(&b, "type")?,
        arr(&b, "projects"),
        arr(&b, "labels"),
        b.get("config").cloned().unwrap_or(json!({})),
    )
    .await?)
}
async fn targets_delete(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<Uuid>) -> J {
    ok(admin::delete_wake_target(&st.db, &p, id).await?)
}
