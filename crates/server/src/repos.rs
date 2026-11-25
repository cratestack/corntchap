//! A pot's repositories, what a task is about and what a cook handles.
//!
//! Everything here is optional: a pot with no repositories, a task with no `repos` and a cook with
//! no scope behave as they always did.
//!
//! * `project_repos`: a pot's repositories, each with a short alias (`name`) unique in the pot.
//! * `tasks.repos`: aliases of the pot's repositories a task is about; empty means any.
//! * `cook_repos`: the aliases a cook handles in a pot; no row means all.
//!
//! Routing rule (one predicate, [`scope_sql`], used by `claim_next`, `wait_for_work` and the
//! dispatcher): a task is for a cook when the task names no repository, or the cook has no scope in
//! the pot, or the two lists overlap.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    admin,
    error::{Error, Result, invalid},
    service::{self, project_by_key, task_by_ref},
    state::Principal,
};

/// `host/path` in lower case, whatever the spelling: https, ssh://, scp-style `git@host:path`,
/// with or without `.git`, port, credentials or a trailing slash. The CLI (`station.rs`) and the
/// SQL of migration 0024 normalize the same way.
pub fn normalize_git_url(url: &str) -> String {
    let u = url.trim();
    let rest = match u.split_once("://") {
        Some((_, r)) => {
            let (auth, path) = r.split_once('/').unwrap_or((r, ""));
            let host = auth.rsplit('@').next().unwrap_or(auth);
            let host = host.split(':').next().unwrap_or(host);
            format!("{host}/{path}")
        }
        None => match u.split_once(':') {
            // scp-style: [user@]host:path
            Some((h, p)) if !h.contains('/') => {
                format!("{}/{}", h.rsplit('@').next().unwrap_or(h), p)
            }
            _ => u.to_string(),
        },
    };
    let mut r = rest.trim_end_matches('/').to_lowercase();
    if let Some(x) = r.strip_suffix(".git") {
        r = x.trim_end_matches('/').to_string();
    }
    r.trim_start_matches('/').to_string()
}

const MAX_ALIAS: usize = 40;

/// An alias as people write it: lower case, `a-z 0-9 . _ -`, starting with a letter or digit.
pub fn clean_alias(raw: &str) -> Result<String> {
    let a = raw.trim().to_lowercase();
    let ok = !a.is_empty()
        && a.len() <= MAX_ALIAS
        && a.starts_with(|c: char| c.is_ascii_alphanumeric())
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(a)
    } else {
        invalid(format!(
            "repository name {raw:?}: 1 to {MAX_ALIAS} characters, a-z 0-9 . _ - only, starting with a letter or digit"
        ))
    }
}

/// The alias a repository gets when none is given: the last path segment (`backend` for
/// `github.com/acme/backend`), made safe.
pub fn default_alias(norm_url: &str) -> String {
    let last = norm_url.rsplit('/').next().unwrap_or("");
    let mut s: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    s = s
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string();
    s.truncate(MAX_ALIAS);
    if s.is_empty() { "repo".into() } else { s }
}

/// SQL predicate: is task `t` (alias `t`, with `t.project_id` and `t.repos`) for the cook whose id
/// is the SQL expression `agent`? True when the task names no repository, the cook has no scope in
/// the task's pot, or the task's repositories and the cook's overlap.
pub fn scope_sql(agent: &str) -> String {
    format!(
        "(cardinality(t.repos) = 0 or not exists (select 1 from cook_repos cr where cr.agent_id = {agent} and cr.project_id = t.project_id)
          or exists (select 1 from cook_repos cr where cr.agent_id = {agent} and cr.project_id = t.project_id and cr.repos && t.repos))"
    )
}

/// SQL: the JSON array of a pot's repositories (with what each depends on), first added first;
/// `project` is the SQL expression of the pot's id.
pub fn repos_json_sql(project: &str) -> String {
    format!(
        "coalesce((select jsonb_agg(jsonb_build_object('name', r.name, 'url', r.url, 'default_branch', r.default_branch,
              'depends_on', coalesce((select jsonb_agg(jsonb_build_object('name', u.name, 'mode', l.mode) order by u.name)
                   from repo_links l join project_repos u on u.id = l.depends_on_id where l.repo_id = r.id), '[]'::jsonb))
              order by r.seq) from project_repos r where r.project_id = {project}), '[]'::jsonb)"
    )
}

/// The pot's repositories, first added first.
pub async fn repos_of(c: &mut PgConnection, project: Uuid) -> Result<Vec<Value>> {
    let v: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select {}",
        repos_json_sql("$1")
    )))
    .bind(project)
    .fetch_one(c)
    .await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Keep `projects.repo_url` equal to the first repository's url (old clients read it).
async fn sync_repo_url(c: &mut PgConnection, project: Uuid) -> Result<()> {
    sqlx::query(
        "update projects set repo_url = (select url from project_repos where project_id = $1 order by seq limit 1) where id = $1",
    )
    .bind(project)
    .execute(c)
    .await?;
    Ok(())
}

/// Insert one repository into a pot (the alias defaults to the url's last segment, made unique).
/// Used by `add_repo` and by `create_project` for its `repo_url`.
pub(crate) async fn insert_repo(
    c: &mut PgConnection,
    project: Uuid,
    url: &str,
    name: Option<&str>,
    default_branch: Option<&str>,
) -> Result<Value> {
    let url = url.trim();
    if url.is_empty() || url.len() > 500 {
        return invalid("a repository url is 1 to 500 characters");
    }
    let norm = normalize_git_url(url);
    if norm.is_empty() {
        return invalid(format!("repository url {url:?} has no host or path"));
    }
    let dup: Option<String> = sqlx::query_scalar(
        "select name from project_repos where project_id = $1 and norm_url = $2",
    )
    .bind(project)
    .bind(&norm)
    .fetch_optional(&mut *c)
    .await?;
    if let Some(d) = dup {
        return Err(Error::Conflict(format!(
            "the pot already has that repository, as {d}"
        )));
    }
    let taken: Vec<String> =
        sqlx::query_scalar("select name from project_repos where project_id = $1")
            .bind(project)
            .fetch_all(&mut *c)
            .await?;
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            let n = clean_alias(n)?;
            if taken.contains(&n) {
                return Err(Error::Conflict(format!(
                    "the pot already has a repository named {n}"
                )));
            }
            n
        }
        None => {
            let base = default_alias(&norm);
            let mut n = base.clone();
            let mut i = 2;
            while taken.contains(&n) {
                n = format!("{base}-{i}");
                i += 1;
            }
            n
        }
    };
    let branch = default_branch.map(str::trim).filter(|b| !b.is_empty());
    if branch.is_some_and(|b| b.len() > 200 || b.contains(char::is_whitespace)) {
        return invalid("a default branch is a branch name, at most 200 characters");
    }
    sqlx::query(
        "insert into project_repos (project_id, url, norm_url, name, default_branch) values ($1,$2,$3,$4,$5)",
    )
    .bind(project)
    .bind(url)
    .bind(&norm)
    .bind(&name)
    .bind(branch)
    .execute(&mut *c)
    .await?;
    sync_repo_url(c, project).await?;
    Ok(json!({"name": name, "url": url, "default_branch": branch, "depends_on": []}))
}

/// Add a repository to a pot (owners of its team, admins, cooks acting for them with `manage`).
pub async fn add_repo(
    db: &PgPool,
    p: &Principal,
    project: &str,
    url: &str,
    name: Option<&str>,
    default_branch: Option<&str>,
) -> Result<Value> {
    let p = &admin::acting(db, p).await?;
    let mut tx = db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, project).await?;
    let repo = insert_repo(&mut tx, id, url, name, default_branch).await?;
    service::emit(
        &mut tx,
        Some(id),
        None,
        "project.repo_added",
        Some(p.id),
        json!({"name": repo["name"]}),
    )
    .await?;
    let repos = repos_of(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"project": project.trim().to_uppercase(), "added": repo, "repos": repos}))
}

/// Remove a repository from a pot. Open, claimed and blocked tasks and cook scopes that named it
/// stop naming it (a task left with no repository is for any cook; a cook left with no scope takes
/// everything); finished tasks keep their history.
pub async fn remove_repo(db: &PgPool, p: &Principal, project: &str, name: &str) -> Result<Value> {
    let p = &admin::acting(db, p).await?;
    let name = name.trim().to_lowercase();
    let mut tx = db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, project).await?;
    let gone = sqlx::query("delete from project_repos where project_id = $1 and name = $2")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(Error::NotFound(format!(
            "the pot has no repository named {name}"
        )));
    }
    sqlx::query(
        "update tasks set repos = array_remove(repos, $2) where project_id = $1 and $2 = any(repos)
           and status in ('open','claimed','blocked')",
    )
    .bind(id)
    .bind(&name)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update cook_repos set repos = array_remove(repos, $2), updated_at = now() where project_id = $1 and $2 = any(repos)")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from cook_repos where project_id = $1 and cardinality(repos) = 0")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("update stations set repo = null where project_id = $1 and repo = $2")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    sync_repo_url(&mut tx, id).await?;
    service::emit(
        &mut tx,
        Some(id),
        None,
        "project.repo_removed",
        Some(p.id),
        json!({"name": name}),
    )
    .await?;
    let repos = repos_of(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"project": project.trim().to_uppercase(), "removed": name, "repos": repos}))
}

/// Clean and validate aliases against the pot's repositories. Empty in, empty out.
pub async fn check_aliases(
    c: &mut PgConnection,
    project: Uuid,
    key: &str,
    names: &[String],
) -> Result<Vec<String>> {
    let mut out: Vec<String> = vec![];
    for n in names {
        let n = n.trim().to_lowercase();
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    if out.is_empty() {
        return Ok(out);
    }
    let known: Vec<String> =
        sqlx::query_scalar("select name from project_repos where project_id = $1 order by seq")
            .bind(project)
            .fetch_all(c)
            .await?;
    if known.is_empty() {
        return invalid(format!(
            "pot {key} has no repositories: add one first (add_repo)"
        ));
    }
    if let Some(bad) = out.iter().find(|n| !known.contains(n)) {
        return invalid(format!(
            "pot {key} has no repository named {bad}: its repositories are {}",
            known.join(", ")
        ));
    }
    Ok(out)
}

/// Set the repositories a cook handles in a pot (empty: all of them). The cook itself, its owner,
/// admins and a cook acting for its owner with `manage` may.
pub async fn set_cook_repos(
    db: &PgPool,
    p: &Principal,
    cook: Option<&str>,
    project: &str,
    repos: &[String],
) -> Result<Value> {
    let cook = crate::stations::cook_for(db, p, cook).await?;
    let key = project.trim().to_uppercase();
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, &cook, &key).await.map_err(|_| {
        Error::NotFound(format!(
            "{} cannot see pot {key}: add the cook to the pot's team first",
            cook.name
        ))
    })?;
    let names = check_aliases(&mut tx, pr.id, &key, repos).await?;
    if names.is_empty() {
        sqlx::query("delete from cook_repos where agent_id = $1 and project_id = $2")
            .bind(cook.id)
            .bind(pr.id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query(
            "insert into cook_repos (agent_id, project_id, repos) values ($1,$2,$3)
             on conflict (agent_id, project_id) do update set repos = excluded.repos, updated_at = now()",
        )
        .bind(cook.id)
        .bind(pr.id)
        .bind(&names)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"cook": cook.name, "project": key, "repos": names}))
}

/// The cook's scope in a pot: the aliases it handles, empty for all.
pub async fn scope_of(c: &mut PgConnection, agent: Uuid, project: Uuid) -> Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("select repos from cook_repos where agent_id = $1 and project_id = $2")
            .bind(agent)
            .bind(project)
            .fetch_optional(c)
            .await?
            .unwrap_or_default(),
    )
}

/// `claim_task` on a named task is never refused for scope: a warning says when it is outside it.
pub async fn scope_warning(
    c: &mut PgConnection,
    agent: Uuid,
    project: Uuid,
    task_repos: &[String],
) -> Result<Option<String>> {
    if task_repos.is_empty() {
        return Ok(None);
    }
    let scope = scope_of(c, agent, project).await?;
    if scope.is_empty() || scope.iter().any(|s| task_repos.contains(s)) {
        return Ok(None);
    }
    Ok(Some(format!(
        "this task is about {} and you handle {}: it is outside your repository scope",
        task_repos.join(", "),
        scope.join(", ")
    )))
}

/// Split a task that names several repositories: one subtask per repository, each with that single
/// repository and the parent's title prefixed `[alias]`. The parent waits for them (a task with
/// open subtasks is not ready). Running it again adds only the missing parts.
pub async fn split_by_repo(db: &PgPool, p: &Principal, id: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let t = task_by_ref(&mut tx, p, id).await?;
    if t.status != "open" {
        return Err(Error::Conflict(format!(
            "{id} is {}: only an open task can be split",
            t.status
        )));
    }
    let (title, body, priority, labels, menu, repos, key): (
        String,
        String,
        i64,
        Vec<String>,
        Option<Uuid>,
        Vec<String>,
        String,
    ) = sqlx::query_as(
        "select t.title, t.body, t.priority, t.labels, t.menu_id, t.repos, pr.key
         from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
    )
    .bind(t.id)
    .fetch_one(&mut *tx)
    .await?;
    if repos.is_empty() {
        return invalid(format!(
            "{id} names no repository: set `repos` on it first (update_task)"
        ));
    }
    let parent_ref = format!("{key}-{}", t.number);
    let mut made = vec![];
    for alias in &repos {
        let exists: bool = sqlx::query_scalar(
            "select exists (select 1 from tasks where parent_id = $1 and repos = array[$2]::text[] and status <> 'cancelled')",
        )
        .bind(t.id)
        .bind(alias)
        .fetch_one(&mut *tx)
        .await?;
        if exists {
            continue;
        }
        let number: i64 = sqlx::query_scalar(
            "update projects set next_number = next_number + 1 where id = $1 returning next_number - 1",
        )
        .bind(t.project_id)
        .fetch_one(&mut *tx)
        .await?;
        let sub = Uuid::new_v4();
        let sub_body = format!("Part of {parent_ref}, for the repository `{alias}`.\n\n{body}");
        sqlx::query(
            "insert into tasks (id, project_id, number, title, body, priority, labels, parent_id, status, created_by, menu_id, repos)
             values ($1,$2,$3,$4,$5,$6,$7,$8,'open',$9,$10,array[$11]::text[])",
        )
        .bind(sub)
        .bind(t.project_id)
        .bind(number)
        .bind(format!("[{alias}] {title}"))
        .bind(sub_body)
        .bind(priority)
        .bind(&labels)
        .bind(t.id)
        .bind(p.id)
        .bind(menu)
        .bind(alias)
        .execute(&mut *tx)
        .await?;
        service::emit(
            &mut tx,
            Some(t.project_id),
            Some(sub),
            "task.created",
            Some(p.id),
            json!({"title": format!("[{alias}] {title}"), "labels": labels, "parent": parent_ref, "repos": [alias]}),
        )
        .await?;
        made.push(sub);
    }
    service::emit(
        &mut tx,
        Some(t.project_id),
        Some(t.id),
        "task.updated",
        Some(p.id),
        json!({"split_by_repo": made.len()}),
    )
    .await?;
    let parent = service::task_full(&mut tx, t.id).await?;
    let mut subs = vec![];
    for s in made {
        subs.push(service::task_json(&mut tx, s).await?);
    }
    tx.commit().await?;
    Ok(json!({"parent": parent, "subtasks": subs}))
}

/// Say that repository `repo` depends on `on` (app on backend). Same managers as `add_repo`.
/// Setting it again changes the mode. Cycles are allowed (propagation is one hop, never followed).
pub async fn link_repo(
    db: &PgPool,
    p: &Principal,
    project: &str,
    repo: &str,
    on: &str,
    mode: Option<&str>,
) -> Result<Value> {
    let p = &admin::acting(db, p).await?;
    let mode = mode
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("notify")
        .to_lowercase();
    if mode != "notify" && mode != "task" {
        return invalid(
            "mode is notify (tell the dependent's cooks) or task (open a follow-up task)",
        );
    }
    let (repo, on) = (repo.trim().to_lowercase(), on.trim().to_lowercase());
    if repo == on {
        return invalid("a repository cannot depend on itself");
    }
    let mut tx = db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, project).await?;
    let ids = repo_ids(&mut tx, id, &[&repo, &on]).await?;
    sqlx::query(
        "insert into repo_links (repo_id, depends_on_id, mode) values ($1,$2,$3)
         on conflict (repo_id, depends_on_id) do update set mode = excluded.mode",
    )
    .bind(ids[0])
    .bind(ids[1])
    .bind(&mode)
    .execute(&mut *tx)
    .await?;
    service::emit(
        &mut tx,
        Some(id),
        None,
        "project.repo_linked",
        Some(p.id),
        json!({"repo": repo, "depends_on": on, "mode": mode}),
    )
    .await?;
    let repos = repos_of(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"project": project.trim().to_uppercase(), "repos": repos}))
}

/// Remove the dependency of `repo` on `on`.
pub async fn unlink_repo(
    db: &PgPool,
    p: &Principal,
    project: &str,
    repo: &str,
    on: &str,
) -> Result<Value> {
    let p = &admin::acting(db, p).await?;
    let (repo, on) = (repo.trim().to_lowercase(), on.trim().to_lowercase());
    let mut tx = db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, project).await?;
    let ids = repo_ids(&mut tx, id, &[&repo, &on]).await?;
    let gone = sqlx::query("delete from repo_links where repo_id = $1 and depends_on_id = $2")
        .bind(ids[0])
        .bind(ids[1])
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if gone == 0 {
        return Err(Error::NotFound(format!("{repo} does not depend on {on}")));
    }
    let repos = repos_of(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"project": project.trim().to_uppercase(), "repos": repos}))
}

async fn repo_ids(c: &mut PgConnection, project: Uuid, names: &[&str]) -> Result<Vec<Uuid>> {
    let mut out = vec![];
    for n in names {
        let id: Option<Uuid> =
            sqlx::query_scalar("select id from project_repos where project_id = $1 and name = $2")
                .bind(project)
                .bind(n)
                .fetch_optional(&mut *c)
                .await?;
        out.push(
            id.ok_or_else(|| Error::NotFound(format!("the pot has no repository named {n}")))?,
        );
    }
    Ok(out)
}

/// A task naming repositories was served (`complete`): for each repository that depends on one of
/// them (and is not named by the task itself), tell its scoped cooks (mode notify: an "upstream"
/// wake; unscoped cooks are not woken) or open a follow-up task for it (mode task). One hop: the
/// follow-up is marked `from_upstream` and does not propagate when it is served, so a cycle cannot
/// ping-pong. Nothing happens for a task with no repositories or one that is itself a follow-up.
pub async fn propagate(
    c: &mut PgConnection,
    actor: Option<Uuid>,
    task: Uuid,
    result: &str,
) -> Result<Value> {
    let (project, key, number, title, repos, menu, priority, from_upstream): (
        Uuid,
        String,
        i64,
        String,
        Vec<String>,
        Option<Uuid>,
        i64,
        bool,
    ) = sqlx::query_as(
        "select t.project_id, pr.key, t.number, t.title, t.repos, t.menu_id, t.priority, t.from_upstream
         from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
    )
    .bind(task)
    .fetch_one(&mut *c)
    .await?;
    if repos.is_empty() || from_upstream {
        return Ok(json!([]));
    }
    let tref = format!("{key}-{number}");
    let up = Upstream {
        look: format!(" (get_task {tref} has the details)"),
        label: tref,
        title,
        did: "was served",
        detail_name: "Result",
        detail: result.to_string(),
        task: Some(task),
        menu,
        priority,
    };
    pass_on(c, actor, project, &key, &repos, &up).await
}

/// What happened upstream, for [`pass_on`]: a served task, or a merged pull request (forge events).
pub struct Upstream {
    /// `PAY-3`, or `acme/api#12`.
    pub label: String,
    pub title: String,
    /// "was served" or "was merged".
    pub did: &'static str,
    /// "Result" or "Pull request".
    pub detail_name: &'static str,
    pub detail: String,
    /// What a cook can read for details, e.g. ` (get_task PAY-3 has the details)`; may be empty.
    pub look: String,
    /// The task it concerns (it gets the note and the events), if any.
    pub task: Option<Uuid>,
    pub menu: Option<Uuid>,
    pub priority: i64,
}

/// Tell what depends on `repos` (aliases in `project`) about `up`: see [`propagate`]. `actor` is
/// left out of the cooks told and is the creator of follow-up tasks (none for a forge event).
pub async fn pass_on(
    c: &mut PgConnection,
    actor: Option<Uuid>,
    project: Uuid,
    key: &str,
    repos: &[String],
    up_: &Upstream,
) -> Result<Value> {
    let (title, menu, priority) = (&up_.title, up_.menu, up_.priority);
    let task = up_.task;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "select r.name, u.name, l.mode from repo_links l
           join project_repos r on r.id = l.repo_id join project_repos u on u.id = l.depends_on_id
         where r.project_id = $1 and u.name = any($2) and not (r.name = any($2))
         order by r.name, u.name",
    )
    .bind(project)
    .bind(repos)
    .fetch_all(&mut *c)
    .await?;
    // dependent -> (upstream names, mode: task wins over notify)
    let mut by_dep: Vec<(String, Vec<String>, String)> = vec![];
    for (dep, up, mode) in rows {
        match by_dep.iter_mut().find(|d| d.0 == dep) {
            Some(d) => {
                d.1.push(up);
                if mode == "task" {
                    d.2 = mode;
                }
            }
            None => by_dep.push((dep, vec![up], mode)),
        }
    }
    let (tref, did, look, detail_name) = (&up_.label, up_.did, &up_.look, up_.detail_name);
    let short: String = up_.detail.chars().take(1500).collect();
    let mut told = vec![];
    for (dep, ups, mode) in by_dep {
        let up = ups.join(", ");
        if mode == "task" {
            let n: i64 = sqlx::query_scalar(
                "update projects set next_number = next_number + 1 where id = $1 returning next_number - 1",
            )
            .bind(project)
            .fetch_one(&mut *c)
            .await?;
            let sub = Uuid::new_v4();
            let ftitle = format!("[{dep}] Follow up on {tref}: {title}");
            let quoted = short.replace('\n', "\n> ");
            let body = format!(
                "{tref} (\"{title}\") {did} in `{up}`, which `{dep}` depends on. Check whether `{dep}` needs a change; if it does not, say so and complete this task.\n\nWhat was done:\n\n> {quoted}"
            );
            sqlx::query(
                "insert into tasks (id, project_id, number, title, body, priority, status, created_by, menu_id, repos, from_upstream)
                 values ($1,$2,$3,$4,$5,$6,'open',$7,$8,array[$9]::text[],true)",
            )
            .bind(sub)
            .bind(project)
            .bind(n)
            .bind(&ftitle)
            .bind(&body)
            .bind(priority)
            .bind(actor)
            .bind(menu)
            .bind(&dep)
            .execute(&mut *c)
            .await?;
            service::emit(
                c,
                Some(project),
                Some(sub),
                "task.created",
                actor.or(Some(crate::state::RESTAURANT)),
                json!({"title": ftitle, "labels": [], "repos": [dep], "upstream": tref, "reason": "upstream",
                       "why": format!("{tref} {did} in {up}, which {dep} depends on: a follow-up task was opened for it")}),
            )
            .await?;
            told.push(json!({"repo": dep, "mode": "task", "task": format!("{key}-{n}")}));
        } else {
            let cooks: Vec<(Uuid, String)> = sqlx::query_as(
                "select a.id, a.name from cook_repos cr join principals a on a.id = cr.agent_id and a.disabled_at is null
                 join projects pr on pr.id = cr.project_id
                 where cr.project_id = $1 and $2 = any(cr.repos) and ($3::uuid is null or a.id <> $3)
                   and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = a.id)
                 order by a.name",
            )
            .bind(project)
            .bind(&dep)
            .bind(actor)
            .fetch_all(&mut *c)
            .await?;
            let text = format!(
                "Upstream change: {tref} \"{title}\" {did} in `{up}`, which `{dep}` depends on. {detail_name}: {short}\n\nCheck whether `{dep}` needs a change{look}; if it does, open a task with create_task (repos: [\"{dep}\"]). If nothing is affected, do nothing."
            );
            let mut names = vec![];
            for (id, name) in cooks {
                let w = crate::questions::wake(c, project, key, id, None, task, &text, "upstream")
                    .await?;
                // a fire-and-forget cook is not woken: do not list it as told
                if w["woken"] != false {
                    names.push(name);
                }
            }
            told.push(json!({"repo": dep, "mode": "notify", "cooks": names}));
        }
    }
    if told.is_empty() {
        return Ok(json!([]));
    }
    let line: Vec<String> = told
        .iter()
        .map(|t| {
            let repo = t["repo"].as_str().unwrap_or("");
            match t["mode"].as_str() {
                Some("task") => format!("{repo}: follow-up {}", t["task"].as_str().unwrap_or("")),
                _ => format!(
                    "{repo}: told {}",
                    match t["cooks"].as_array().filter(|a| !a.is_empty()) {
                        Some(a) => a
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", "),
                        None => "no scoped cook".into(),
                    }
                ),
            }
        })
        .collect();
    if let Some(task) = task {
        // the server's own note: its author is the restaurant
        service::system_comment(
            c,
            task,
            &format!("Upstream change passed on ({}).", line.join("; ")),
        )
        .await?;
    }
    service::emit(
        c,
        Some(project),
        task,
        "task.propagated",
        actor.or(Some(crate::state::RESTAURANT)),
        json!({"upstream": repos.join(", "), "told": told, "reason": "propagate",
               "why": format!("{tref} {did} in {}: what depends on it was told", repos.join(", "))}),
    )
    .await?;
    Ok(Value::Array(told))
}
