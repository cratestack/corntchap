//! REST at /api/v1: the CLI, scripts and admin side. Same service functions as the MCP tools.

use authkestra::axum::Auth as Authn;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, patch, post, put},
};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    admin,
    error::{Error, Result},
    files, menus, pantry, questions, reviews,
    service::{self, NewTask, TaskEdit},
    state::{AppState, Principal},
    stations, stream,
};

type Auth = Authn<Principal>;
type J = Result<Json<Value>>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", get(me).patch(me_rename))
        .route("/me/work", get(me_work))
        .route("/logout", post(logout))
        .route("/users", get(users_list))
        .route("/users/{name}", patch(users_update))
        .route("/teams", get(teams_list).post(teams_add))
        .route("/teams/{team}/members", get(members_list).post(members_add))
        .route("/teams/{team}/members/{name}", delete(members_remove))
        .route("/projects", get(projects_list).post(projects_add))
        .route("/projects/{key}/tasks", get(tasks_list).post(tasks_add))
        .route("/projects/{key}/claim-next", post(claim_next))
        .route("/projects/{key}/what-now", get(what_now))
        .route("/projects/{key}/guide", put(set_guide))
        .route("/projects/{key}/repos", post(repos_add))
        .route("/projects/{key}/repos/bulk", post(repos_add_many))
        .route("/projects/{key}/repos/{name}", delete(repos_remove))
        .route(
            "/projects/{key}/repo-links",
            post(repos_link).delete(repos_unlink),
        )
        .route("/projects/{key}/cook-repos", post(cook_repos_set))
        .route("/projects/{key}/digest", get(digest))
        .route("/projects/{key}/board", get(board))
        .route("/projects/{key}/menus", get(menus_list).post(menus_add))
        .route("/projects/{key}/menus/{mkey}", patch(menus_edit))
        .route("/projects/{key}/pantry", get(pantry_list).post(pantry_save))
        .route(
            "/projects/{key}/pantry/{mkey}",
            get(pantry_get).put(pantry_put).delete(pantry_forget),
        )
        .route("/tasks/{id}", get(task_get).patch(task_patch))
        .route("/tasks/{id}/claim", post(task_claim))
        .route("/tasks/{id}/split-by-repo", post(task_split))
        .route("/tasks/{id}/link-pr", post(task_link_pr))
        .route("/tasks/{id}/unlink-pr", post(task_unlink_pr))
        .route("/tasks/{id}/refresh-pr", post(task_refresh_pr))
        .route("/forges", get(forges_list))
        .route("/forges/repos", get(forges_repos))
        .route("/forges/unfollowed/dismiss", post(forges_dismiss))
        .route("/forges/{id}", delete(forges_remove))
        .route("/tasks/{id}/heartbeat", post(task_heartbeat))
        .route("/tasks/{id}/check", post(task_check))
        .route("/tasks/{id}/comment", post(task_comment))
        .route("/tasks/{id}/complete", post(task_complete))
        .route("/tasks/{id}/release", post(task_release))
        .route("/tasks/{id}/block", post(task_block))
        .route("/tasks/{id}/unblock", post(task_unblock))
        .route("/tasks/{id}/cancel", post(task_cancel))
        .route("/tasks/{id}/toss", post(task_cancel))
        .route("/tasks/{id}/restore", post(task_restore))
        .route("/changes", get(changes))
        .route(
            "/projects/{key}/files",
            get(files_list)
                .post(files_upload)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/projects/{key}/uploads", post(files_create_upload))
        .route("/files/{id}", get(files_get).delete(files_delete))
        .route("/stations", get(stations_list).post(stations_add))
        .route("/stations/{id}", delete(stations_remove))
        .route("/stations/{id}/main", post(stations_main))
        .route(
            "/projects/{key}/questions",
            get(questions_list).post(questions_ask),
        )
        .route("/questions/mine", get(questions_mine))
        .route("/questions/{id}", get(questions_get))
        .route("/questions/{id}/answer", post(questions_answer))
        .route("/questions/{id}/note", post(questions_note))
        .route("/questions/{id}/dismiss", post(questions_dismiss))
        .route("/questions/{id}/release", post(questions_release))
        .route("/questions/pick", post(questions_pick))
        .route("/projects/{key}/reviews", get(reviews_list))
        .route("/tasks/{id}/review-request", post(reviews_request))
        .route("/tasks/{id}/review", post(reviews_verdict_task))
        .route("/reviews/{id}/verdict", post(reviews_verdict))
        .route("/reviews/{id}/cancel", post(reviews_cancel))
        .route("/events/stream", get(stream::events_stream))
        .route("/wait", get(wait))
        .route("/agents", get(agents_list).post(agents_add))
        .route("/agents/{name}/rotate", post(rotate))
        .route("/agents/{name}/disable", post(disable))
        .route("/agents/{name}/enable", post(enable))
        .route("/agents/{name}/grants", post(grants_set))
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

async fn me(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(service::whoami(&st.db, &p, q.get("project").map(|k| k.as_str())).await?)
}

/// Change your own handle: `{"name": "<new handle>"}`. People only.
async fn me_rename(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(admin::rename_self(&st.db, &p, &req_s(&b, "name")?).await?)
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
/// Change a person (admins only): `{"is_admin": bool}` and/or `{"disabled": bool}`. An admin setting is manual: the
/// IdP role no longer changes it. A disabled person's tokens and sessions stop working.
async fn users_update(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(name): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let (admin_flag, disabled) = (b["is_admin"].as_bool(), b["disabled"].as_bool());
    if admin_flag.is_none() && disabled.is_none() {
        return Err(Error::Invalid(
            "is_admin and/or disabled (true or false) is required".into(),
        ));
    }
    let mut out = json!({"name": name.trim().trim_start_matches('@').to_lowercase()});
    if let Some(on) = admin_flag {
        out = admin::set_admin(&st.db, &p, &name, on).await?;
    }
    if let Some(d) = disabled {
        let v = admin::set_user_disabled(&st.db, &p, &name, d).await?;
        out["disabled"] = v["disabled"].clone();
    }
    ok(out)
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
        q.get("owner").map(|x| x.as_str()),
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
            repos: arr(&b, "repos"),
            priority: i(&b, "priority").unwrap_or(0),
            menu: s(&b, "menu"),
            owner: s(&b, "owner"),
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
    let station = s(&b, "station");
    ok(service::claim_next(
        &st.db,
        &p.worker()?,
        &k.to_uppercase(),
        &arr(&b, "labels"),
        menu.as_deref(),
        station.as_deref(),
        b["ttl_s"].as_f64(),
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

/// `{"url": "...", "name": "backend", "default_branch": "main"}`: add a repository to the pot.
async fn repos_add(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let url = req_s(&b, "url")?;
    let r = crate::repos::add_repo(
        &st.db,
        &p,
        &k,
        &url,
        s(&b, "name").as_deref(),
        s(&b, "default_branch").as_deref(),
    )
    .await?;
    crate::forge::sync::spawn_backfill(&st, &k, vec![url]);
    ok(r)
}
/// `{"repos": "acme/api, acme/web"}`: add several repositories (GitHub full names or urls) at once.
async fn repos_add_many(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::forge::pick::add_many(&st, &p, &k, &req_s(&b, "repos")?).await?)
}
async fn repos_remove(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, name)): Path<(String, String)>,
) -> J {
    ok(crate::repos::remove_repo(&st.db, &p, &k, &name).await?)
}
/// `{"cook": "name" (default: you), "repos": ["backend"]}`: which of the pot's repositories a cook handles (empty: all).
async fn cook_repos_set(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(
        crate::repos::set_cook_repos(&st.db, &p, s(&b, "cook").as_deref(), &k, &arr(&b, "repos"))
            .await?,
    )
}
/// `{"repo": "app", "on": "backend", "mode": "notify"|"task"}`: `app` depends on `backend`.
async fn repos_link(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::repos::link_repo(
        &st.db,
        &p,
        &k,
        &req_s(&b, "repo")?,
        &req_s(&b, "on")?,
        s(&b, "mode").as_deref(),
    )
    .await?)
}
async fn repos_unlink(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::repos::unlink_repo(&st.db, &p, &k, &req_s(&b, "repo")?, &req_s(&b, "on")?).await?)
}
async fn task_split(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(crate::repos::split_by_repo(&st.db, &p.worker()?, &id).await?)
}

/// `{"url": "https://github.com/acme/shop/pull/12"}`: link a pull request to the task by hand.
async fn task_link_pr(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let url = b["url"].as_str().unwrap_or("");
    ok(crate::forge::link::link_pr(&st, &p.worker()?, &id, url).await?)
}

/// `{"url": ...}` unlinks that pull request; no url unlinks all of the task's.
async fn task_unlink_pr(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::forge::link::unlink_pr(&st.db, &p.worker()?, &id, b["url"].as_str()).await?)
}

/// Connected git hosts with their installations and last delivery (admins only). No secrets.
async fn forges_list(State(st): State<AppState>, Authn(p): Auth) -> J {
    if !p.is_user() || !p.is_admin {
        return Err(Error::Forbidden(
            "only admins see the connected git hosts".into(),
        ));
    }
    ok(json!({
        "public_url": st.config.public_url,
        "connections": crate::forge::store::list(&st.db).await?,
        "unfollowed": crate::forge::unfollowed::list(&st.db).await?
    }))
}

/// `?project=PAY&q=api&installation=1`: the repositories the connected GitHub Apps can see.
async fn forges_repos(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(crate::forge::pick::list(
        &st,
        &p,
        q.get("project").map(String::as_str),
        q.get("q").map(String::as_str),
        q.get("installation").and_then(|v| v.parse().ok()),
    )
    .await?)
}

/// `{"repo": "acme/api"}`: stop listing a repository that sends events no pot follows (admins).
async fn forges_dismiss(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    ok(crate::forge::pick::dismiss(&st, &p, &req_s(&b, "repo")?).await?)
}

async fn task_refresh_pr(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::forge::link::refresh_pr(&st, &p.worker()?, &id, b["url"].as_str()).await?)
}

async fn forges_remove(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<Uuid>) -> J {
    if !p.is_user() || !p.is_admin {
        return Err(Error::Forbidden(
            "only admins remove a connected git host".into(),
        ));
    }
    if !crate::forge::store::remove(&st.db, id).await? {
        return Err(Error::NotFound("no such connection".into()));
    }
    ok(json!({"removed": id}))
}

/// `{"grants": ["plan", "manage"]}`: what a cook may do beyond working its tasks (people only).
async fn grants_set(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(name): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(admin::set_grants(&st.db, &p, &name, &arr(&b, "grants")).await?)
}

// ---- files ----
fn new_file(
    project: Option<String>,
    q: &HashMap<String, String>,
    b: Option<&Value>,
) -> files::NewFile {
    let get = |k: &str| b.and_then(|b| s(b, k)).or_else(|| q.get(k).cloned());
    files::NewFile {
        project,
        name: get("name").unwrap_or_default(),
        content_type: get("content_type"),
        size: b
            .and_then(|b| i(b, "size"))
            .or_else(|| q.get("size").and_then(|v| v.parse().ok())),
        task: get("task"),
        question: get("question"),
    }
}
async fn files_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(files::list(
        &st.db,
        &st,
        &p.worker()?,
        &k,
        q.get("task").map(|x| x.as_str()),
        q.get("question").map(|x| x.as_str()),
    )
    .await?)
}
/// `{name, size, content_type?, task?, question?}`: an upload slot (presigned PUT + commands).
async fn files_create_upload(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let n = new_file(Some(k), &HashMap::new(), Some(&b));
    ok(files::create_upload(&st.db, &st, &p.worker()?, n).await?)
}
/// The console's upload: raw body (Content-Type, Content-Length), `?name=&task=&question=`.
async fn files_upload(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> J {
    let length: u64 = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| Error::Invalid("Content-Length is required".into()))?;
    let mut n = new_file(Some(k), &q, None);
    if n.content_type.is_none() {
        n.content_type = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
    }
    let stream = reqwest::Body::wrap_stream(body.into_data_stream());
    ok(files::upload_stream(&st.db, &st, &p.worker()?, n, length, stream).await?)
}
async fn files_get(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(files::get(&st.db, &st, &p.worker()?, &id).await?)
}
async fn files_delete(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(files::delete(&st.db, &st, &p.worker()?, &id).await?)
}

// ---- stations (registered agent sessions) ----
async fn stations_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(stations::list(
        &st.db,
        &p.worker()?,
        q.get("project").map(|x| x.as_str()),
        q.get("cook").map(|x| x.as_str()),
    )
    .await?)
}
async fn stations_add(State(st): State<AppState>, Authn(p): Auth, Json(b): Json<Value>) -> J {
    let n = stations::NewStation {
        cook: s(&b, "cook"),
        project: req_s(&b, "project")?,
        runtime: req_s(&b, "runtime")?,
        session: req_s(&b, "session")?,
        directory: s(&b, "directory"),
        label: s(&b, "label"),
        repo: s(&b, "repo"),
        confined: b["confined"].as_bool(),
    };
    ok(stations::register(&st.db, &p.worker()?, n).await?)
}
async fn stations_remove(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(stations::remove(&st.db, &p.worker()?, &id).await?)
}

/// `{main: bool}` (default true): make the station its cook's main one in its pot, or unset it.
async fn stations_main(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> J {
    let main = body.and_then(|Json(b)| b["main"].as_bool()).unwrap_or(true);
    ok(stations::set_main(&st.db, &p.worker()?, &id, main).await?)
}

// ---- the question pool ----
async fn questions_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(questions::list(
        &st.db,
        &p.worker()?,
        &k,
        q.get("status").map(|x| x.as_str()),
    )
    .await?)
}
async fn questions_ask(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let a = questions::Ask {
        project: Some(k),
        title: s(&b, "title"),
        description: s(&b, "description"),
        body: s(&b, "question"),
        kind: s(&b, "kind"),
        choices: b.get("choices").cloned(),
        options: arr(&b, "options"),
        allow_other: b["allow_other"].as_bool().unwrap_or(false),
        min: b["min"].as_i64(),
        max: b["max"].as_i64(),
        task: s(&b, "task"),
        to: s(&b, "to"),
        wait: b["wait"].as_bool().unwrap_or(false),
        station: s(&b, "station"),
    };
    ok(questions::ask(&st.db, &p.worker()?, a).await?)
}
async fn questions_mine(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(questions::mine(&st.db, &p.worker()?, q.get("status").map(|x| x.as_str())).await?)
}
async fn questions_get(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(questions::get(&st.db, &p.worker()?, &id).await?)
}
async fn questions_answer(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    let reply = questions::Reply {
        choices: b.get("choices").cloned(),
        text: s(&b, "text"),
        answer: s(&b, "answer"),
    };
    ok(questions::answer(&st.db, &p.worker()?, &id, reply, false).await?)
}
async fn questions_note(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(questions::note(&st.db, &p.worker()?, &id, &req_s(&b, "note")?).await?)
}
async fn questions_dismiss(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> J {
    let reason = body.and_then(|Json(b)| s(&b, "reason")).unwrap_or_default();
    ok(questions::answer(
        &st.db,
        &p.worker()?,
        &id,
        questions::Reply::plain(&reason),
        true,
    )
    .await?)
}
async fn questions_release(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
) -> J {
    ok(questions::release(&st.db, &p.worker()?, &id).await?)
}
async fn questions_pick(
    State(st): State<AppState>,
    Authn(p): Auth,
    body: Option<Json<Value>>,
) -> J {
    let project = body.and_then(|Json(b)| s(&b, "project"));
    ok(questions::pick(&st.db, &p.worker()?, project.as_deref()).await?)
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
        notify: b["notify"].as_bool().unwrap_or(false),
        station: s(b, "station"),
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
/// `?notify=1` tells everyone now (a broadcast on the pass).
async fn pantry_forget(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path((k, mk)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let notify = q.get("notify").is_some_and(|v| v == "1" || v == "true");
    ok(pantry::forget(
        &st.db,
        &p.worker()?,
        &k,
        &mk,
        notify,
        q.get("station").cloned(),
    )
    .await?)
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
    let repos = b.get("repos").map(|_| arr(&b, "repos"));
    ok(service::update_task(
        &st.db,
        &p.worker()?,
        &id,
        TaskEdit {
            title: s(&b, "title"),
            body: s(&b, "body"),
            labels,
            repos,
            priority: i(&b, "priority"),
            menu: s(&b, "menu"),
            owner: s(&b, "owner"),
        },
    )
    .await?)
}
/// `{"station": "st_…"}` optional: the lease records the session.
async fn task_claim(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> J {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let station = s(&b, "station");
    ok(service::claim_task(
        &st.db,
        &p.worker()?,
        &id,
        station.as_deref(),
        b["ttl_s"].as_f64(),
    )
    .await?)
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
        s(&b, "station").as_deref(),
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
        b["no_propagate"].as_bool().unwrap_or(false),
        &s(&b, "learned").unwrap_or_default(),
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
        s(&b, "learned").as_deref(),
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
        s(&b, "learned").as_deref(),
    )
    .await?)
}
async fn task_unblock(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::human_transition(&st.db, &p.worker()?, &id, "unblock", None).await?)
}
/// Toss: `{"reason": "…"}` optional.
async fn task_cancel(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> J {
    let reason = body.and_then(|Json(b)| s(&b, "reason"));
    ok(service::human_transition(&st.db, &p.worker()?, &id, "cancel", reason.as_deref()).await?)
}
async fn task_restore(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(service::human_transition(&st.db, &p.worker()?, &id, "restore", None).await?)
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
/// `?project=PAY` optional: what the caller owns, holds and is waited on for (see `my_work`).
async fn me_work(
    State(st): State<AppState>,
    Authn(p): Auth,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(crate::my_work::my_work(&st.db, &p.worker()?, q.get("project").map(|x| x.as_str())).await?)
}

/// `?station=st_...`: what the station should do now (see `what_now`).
async fn what_now(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    ok(crate::what_now::what_now(
        &st.db,
        &p.worker()?,
        &k,
        q.get("station").map(|x| x.as_str()),
    )
    .await?)
}

/// `{guide: "<markdown>"}`: set (or, empty, clear) the pot's guide. Team owners and admins.
async fn set_guide(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(crate::what_now::set_guide(
        &st.db,
        &p.worker()?,
        &k,
        &s(&b, "guide").unwrap_or_default(),
    )
    .await?)
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

// ---- reviews ----
/// `?mine=true&state=&task=`
async fn reviews_list(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(k): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> J {
    let mine = q.get("mine").is_some_and(|v| v == "true" || v == "1");
    ok(reviews::list(
        &st.db,
        &p.worker()?,
        &k,
        mine,
        q.get("state").map(|x| x.as_str()),
        q.get("task").map(|x| x.as_str()),
    )
    .await?)
}
/// `{"reviewer": "bob", "note": "..."}`
async fn reviews_request(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(reviews::request(
        &st.db,
        &p.worker()?,
        &id,
        s(&b, "reviewer").as_deref(),
        s(&b, "note").as_deref(),
    )
    .await?)
}
/// `{"verdict": "approve|changes|comment", "body": "..."}` on the review waiting on the caller.
async fn reviews_verdict_task(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(reviews::give(
        &st.db,
        &p.worker()?,
        None,
        Some(&id),
        &req_s(&b, "verdict")?,
        s(&b, "body").as_deref(),
    )
    .await?)
}
async fn reviews_verdict(
    State(st): State<AppState>,
    Authn(p): Auth,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> J {
    ok(reviews::give(
        &st.db,
        &p.worker()?,
        Some(&id),
        None,
        &req_s(&b, "verdict")?,
        s(&b, "body").as_deref(),
    )
    .await?)
}
async fn reviews_cancel(State(st): State<AppState>, Authn(p): Auth, Path(id): Path<String>) -> J {
    ok(reviews::cancel(&st.db, &p.worker()?, &id).await?)
}
