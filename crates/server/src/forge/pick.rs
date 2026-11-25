//! Picking a pot's repositories from what a connected GitHub App can see: `list_forge_repos` and
//! `add_repos` (the bulk form of `add_repo`). Same rule as `add_repo`: owners of the pot's team,
//! admins, and cooks acting for them with `manage`. Without a pot, only admins list.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::catalog::{self, Visible};
use crate::{
    admin,
    error::{Error, Result, invalid},
    repos::{self, clean_alias, default_alias, normalize_git_url},
    service,
    state::{AppState, Principal},
};

/// The alias a picked repository gets in a pot whose aliases in use are `taken`: the repository's
/// name, then `owner-name`, then `owner-name-2`, ...
pub fn pick_alias(full_name: &str, taken: &HashSet<String>) -> String {
    let (owner, name) = full_name.split_once('/').unwrap_or(("", full_name));
    let base = default_alias(&normalize_git_url(&format!("github.com/{owner}/{name}")));
    if !taken.contains(&base) {
        return base;
    }
    let both = clean_alias(&format!("{owner}-{name}")).unwrap_or_else(|_| format!("{base}-x"));
    let mut n = both.clone();
    let mut i = 2;
    while taken.contains(&n) {
        n = format!("{both}-{i}");
        i += 1;
    }
    n
}

fn view(
    r: &Visible,
    in_pot: Option<&str>,
    taken: &HashSet<String>,
    unfollowed: Option<i32>,
) -> Value {
    json!({
        "full_name": r.full_name, "html_url": r.html_url, "clone_url": r.clone_url,
        "default_branch": r.default_branch, "private": r.private, "archived": r.archived,
        "description": r.description, "pushed_at": r.pushed_at,
        "account": r.account, "installation": r.installation,
        "in_pot": in_pot.is_some(), "pot_name": in_pot,
        "suggested_name": pick_alias(&r.full_name, taken),
        "unfollowed_events": unfollowed,
    })
}

/// Who may see the list: with a pot, whoever may add a repository to it; without, admins.
async fn check(
    st: &AppState,
    p: &Principal,
    project: Option<&str>,
) -> Result<Option<(uuid::Uuid, String)>> {
    let p = &admin::acting(&st.db, p).await?;
    match project.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => {
            let mut c = st.db.acquire().await?;
            let (id, _) = admin::owned_project(&mut c, p, k).await?;
            Ok(Some((id, k.to_uppercase())))
        }
        None if p.is_super_admin() => Ok(None),
        None => Err(Error::Forbidden(
            "name a pot (project): only a super-admin lists GitHub repositories without one".into(),
        )),
    }
}

/// The pot's repositories as (norm_url, alias).
async fn in_pot(st: &AppState, project: uuid::Uuid) -> Result<Vec<(String, String)>> {
    Ok(
        sqlx::query_as("select norm_url, name from project_repos where project_id = $1")
            .bind(project)
            .fetch_all(&st.db)
            .await?,
    )
}

/// The repositories the caller may add to `project`, marking those already in it.
pub async fn list(
    st: &AppState,
    p: &Principal,
    project: Option<&str>,
    query: Option<&str>,
    installation: Option<i64>,
    limit: Option<i64>,
) -> Result<Value> {
    let pot = check(st, p, project).await?;
    let have = match &pot {
        Some((id, _)) => in_pot(st, *id).await?,
        None => vec![],
    };
    let taken: HashSet<String> = have.iter().map(|(_, n)| n.clone()).collect();
    let cat = catalog::visible(st, installation).await?;
    let unf = super::unfollowed::counts(&st.db).await?;
    let q = query.unwrap_or("").trim().to_lowercase();
    let matching = cat
        .repos
        .iter()
        .filter(|r| q.is_empty() || r.full_name.to_lowercase().contains(&q));
    // `total` counts every match; `limit` (at most 500) only cuts what is shown
    let total = matching.clone().count();
    let repos: Vec<Value> = matching
        .take(limit.map_or(usize::MAX, |l| l.clamp(1, 500) as usize))
        .map(|r| {
            let norm = r.norm_url();
            let name = have
                .iter()
                .find(|(n, _)| *n == norm)
                .map(|(_, a)| a.as_str());
            view(r, name, &taken, unf.get(&norm).copied())
        })
        .collect();
    Ok(json!({
        "project": pot.map(|(_, k)| k),
        "connected": cat.connected,
        "total": total,
        "repos": repos,
        "errors": cat.errors,
    }))
}

/// `owner/name` of an input line, if it is one (not a url).
fn as_full_name(s: &str) -> Option<&str> {
    let s = s.trim().trim_matches('/');
    let ok = !s.contains("://")
        && !s.contains(':')
        && !s.contains('@')
        && s.matches('/').count() == 1
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'));
    ok.then_some(s)
}

/// Add several repositories to a pot at once. `repos`: GitHub full names (`acme/api`) or urls,
/// separated by newlines, commas or spaces. A repository already in the pot is skipped; names that
/// clash get `owner-name`; the default branch comes from GitHub when the app sees the repository.
pub async fn add_many(st: &AppState, p: &Principal, project: &str, repos: &str) -> Result<Value> {
    let items: Vec<&str> = repos
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return invalid("name at least one repository (owner/name or a url)");
    }
    if items.len() > 200 {
        return invalid("add at most 200 repositories at once");
    }
    let p = &admin::acting(&st.db, p).await?;
    // a refusal comes before any call to GitHub
    {
        let mut c = st.db.acquire().await?;
        admin::owned_project(&mut c, p, project).await?;
    }
    let cat = catalog::visible(st, None).await?;
    let mut tx = st.db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, project).await?;
    let mut taken: HashSet<String> =
        sqlx::query_scalar("select name from project_repos where project_id = $1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    let mut norms: HashSet<String> =
        sqlx::query_scalar("select norm_url from project_repos where project_id = $1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();
    let (mut added, mut skipped) = (vec![], vec![]);
    for item in items {
        let found: Option<&Visible> = match as_full_name(item) {
            Some(n) => cat
                .repos
                .iter()
                .find(|r| r.full_name.eq_ignore_ascii_case(n)),
            None => {
                let norm = normalize_git_url(item);
                cat.repos.iter().find(|r| r.norm_url() == norm)
            }
        };
        let (url, norm, full, branch) = match (found, as_full_name(item)) {
            (Some(r), _) => (
                r.clone_url.clone(),
                r.norm_url(),
                r.full_name.clone(),
                r.default_branch.clone(),
            ),
            (None, Some(n)) => {
                skipped.push(json!({"input": item, "reason": format!(
                    "{n} is not visible to a connected GitHub App (install the app on it, or give its url)")}));
                continue;
            }
            (None, None) => {
                let norm = normalize_git_url(item);
                let full = norm
                    .split_once('/')
                    .map(|x| x.1)
                    .unwrap_or(&norm)
                    .to_string();
                (item.to_string(), norm, full, None)
            }
        };
        if norms.contains(&norm) {
            skipped.push(json!({"input": item, "reason": "already in the pot"}));
            continue;
        }
        let name = pick_alias(&full, &taken);
        match repos::insert_repo(&mut tx, id, &url, Some(&name), branch.as_deref()).await {
            Ok(r) => {
                taken.insert(name);
                norms.insert(norm);
                service::emit(
                    &mut tx,
                    Some(id),
                    None,
                    "project.repo_added",
                    Some(p.id),
                    json!({"name": r["name"]}),
                )
                .await?;
                added.push(r);
            }
            Err(Error::Invalid(m) | Error::Conflict(m)) => {
                skipped.push(json!({"input": item, "reason": m}));
            }
            Err(e) => return Err(e),
        }
    }
    let all = repos::repos_of(&mut tx, id).await?;
    tx.commit().await?;
    let urls: Vec<String> = added
        .iter()
        .filter_map(|r| r["url"].as_str().map(str::to_string))
        .collect();
    super::sync::spawn_backfill(st, project, urls);
    Ok(
        json!({"project": project.trim().to_uppercase(), "added": added, "skipped": skipped, "repos": all}),
    )
}

/// Super-admins only: forget a repository that sends events no pot follows.
pub async fn dismiss(st: &AppState, p: &Principal, repo: &str) -> Result<Value> {
    let p = &admin::acting(&st.db, p).await?;
    if !p.is_super_admin() {
        return Err(Error::Forbidden(
            "only a super-admin manages the connected git hosts".into(),
        ));
    }
    if !super::unfollowed::dismiss(&st.db, repo).await? {
        return Err(Error::NotFound(format!("{repo} is not on the list")));
    }
    Ok(json!({"ok": true}))
}
