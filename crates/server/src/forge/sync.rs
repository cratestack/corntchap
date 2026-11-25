//! Reading a pull request from GitHub instead of waiting for a webhook: `link_pr` and `refresh_pr`
//! fill the row right away, and a repository that joins a pot is searched for open PRs that name its
//! tasks (the backfill).
//!
//! GitHub REST endpoints (verified against docs.github.com on 2026-11-24):
//! * `GET /repos/{owner}/{repo}/installation` with the app JWT: the installation that covers a
//!   repository (404 when the app is not installed on it).
//! * `GET /repos/{owner}/{repo}/pulls/{n}` with the installation token: `title`, `state`, `merged`,
//!   `draft`, `head.ref`, `head.sha`, `base.ref`, `user.login`, `body`; `mergeable` is true, false or
//!   null while GitHub computes it, `mergeable_state` says `dirty` for a conflict.
//! * `GET /repos/{owner}/{repo}/commits/{sha}/check-runs?per_page=100`: `check_runs[]` with `name`,
//!   `status` (`completed` or not), `conclusion`, `html_url`, `output.title`.
//! * `GET /repos/{owner}/{repo}/pulls/{n}/reviews?per_page=100`: `state` (APPROVED,
//!   CHANGES_REQUESTED, COMMENTED, DISMISSED, PENDING), `user.login`, `body`, `html_url`.
//! * `GET /repos/{owner}/{repo}/pulls?state=open&per_page=100`: the open PRs, for the backfill.
//!
//! What GitHub says is turned into the same [`ForgeEvent`]s a webhook delivery would be and handed
//! to `events::apply`, so the row is filled by the code that fills it for deliveries (and a failed
//! check or requested changes tells the holder the same way).

use std::collections::HashMap;

use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    CheckSource, Conclusion, ForgeEvent, ForgeKind, ReviewState,
    catalog::{self, Conn},
    events::{self, auto_link},
    excerpt, github,
};
use crate::{error::Result, repos::normalize_git_url, service, state::AppState};

/// A connection and the installation of it that covers one repository.
pub struct Access {
    conn: Conn,
    installation: i64,
    token: String,
}

fn host_of(web_url: &str) -> String {
    web_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// The installation that covers `repo` (`owner/name`) on the git host `host`, with a token for it.
/// None when no connected app is installed on the repository.
pub async fn access_for(st: &AppState, host: &str, repo: &str) -> Result<Option<Access>> {
    let host = host.to_lowercase();
    for conn in catalog::connections(st).await? {
        if host_of(&conn.web_url) != host {
            continue;
        }
        let jwt = catalog::app_jwt(conn.app_id, &conn.pem, chrono::Utc::now())?;
        let url = format!("{}/repos/{repo}/installation", conn.api_url);
        let Ok((status, v, _)) = catalog::get_json(st, &conn, &jwt, &url).await else {
            continue;
        };
        let Some(installation) = status.is_success().then(|| v["id"].as_i64()).flatten() else {
            continue;
        };
        let token = catalog::installation_token(st, &conn, installation).await?;
        return Ok(Some(Access {
            conn,
            installation,
            token,
        }));
    }
    Ok(None)
}

async fn get(st: &AppState, a: &Access, path: &str) -> Result<(http::StatusCode, Value)> {
    let (s, v, _) =
        catalog::get_json(st, &a.conn, &a.token, &format!("{}{path}", a.conn.api_url)).await?;
    Ok((s, v))
}

/// Every item of a paginated listing: `key` names the array in the body (empty: the body is it).
async fn get_all(st: &AppState, a: &Access, path: &str, key: &str) -> Result<Vec<Value>> {
    let mut url = Some(format!("{}{path}", a.conn.api_url));
    let mut out = vec![];
    let mut pages = 0;
    while let Some(u) = url.take() {
        pages += 1;
        if pages > 10 {
            break;
        }
        let (s, v, next) = catalog::get_json(st, &a.conn, &a.token, &u).await?;
        if !s.is_success() {
            break;
        }
        let items = if key.is_empty() { &v } else { &v[key] };
        out.extend(items.as_array().cloned().unwrap_or_default());
        url = next;
    }
    Ok(out)
}

/// The latest review state of each reviewer, reduced to one for the PR: changes requested beats
/// approved beats commented. A comment does not take back an approval or a request, a dismissal
/// does.
pub fn review_of(reviews: &[Value]) -> Option<(ReviewState, String, String, String)> {
    let mut by: HashMap<String, (ReviewState, String, String)> = HashMap::new();
    for r in reviews {
        let who = r["user"]["login"].as_str().unwrap_or("").to_string();
        let state = match r["state"].as_str().map(str::to_uppercase).as_deref() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("COMMENTED") => ReviewState::Commented,
            Some("DISMISSED") => ReviewState::Dismissed,
            _ => continue,
        };
        let keep = matches!(
            by.get(&who),
            Some((ReviewState::Approved | ReviewState::ChangesRequested, _, _))
        ) && state == ReviewState::Commented;
        if keep {
            continue;
        }
        by.insert(
            who,
            (
                state,
                excerpt(r["body"].as_str().unwrap_or(""), 300),
                r["html_url"].as_str().unwrap_or("").to_string(),
            ),
        );
    }
    for want in [
        ReviewState::ChangesRequested,
        ReviewState::Approved,
        ReviewState::Commented,
    ] {
        let mut hit: Vec<_> = by.iter().filter(|(_, v)| v.0 == want).collect();
        hit.sort_by(|a, b| a.0.cmp(b.0));
        if let Some((who, (_, body, url))) = hit.first() {
            return Some((want, who.to_string(), body.clone(), url.clone()));
        }
    }
    None
}

/// Read one PR from GitHub and apply it like a webhook delivery would. A text warning when the
/// app cannot see it (so the row stays as it was).
pub async fn sync_pr(st: &AppState, host: &str, repo: &str, number: i64) -> Result<Option<String>> {
    let Some(a) = access_for(st, host, repo).await? else {
        return Ok(Some(format!(
            "The GitHub App is not installed on {repo} (or no GitHub App is connected): the pull request's title, branch, checks and reviews cannot be read, so they will stay empty until the app is installed there (an admin: Forges page)."
        )));
    };
    sync_with(st, &a, repo, number).await
}

async fn sync_with(st: &AppState, a: &Access, repo: &str, number: i64) -> Result<Option<String>> {
    let (status, pr) = get(st, a, &format!("/repos/{repo}/pulls/{number}")).await?;
    if !status.is_success() {
        return Ok(Some(format!(
            "GitHub answered {status} for {repo}#{number}: {}",
            pr["message"].as_str().unwrap_or("no message")
        )));
    }
    let repo_ref =
        github::repo_of(&json!({"repository": pr["base"]["repo"]})).unwrap_or_else(|| {
            let html = format!("https://{}/{repo}", host_of(&a.conn.web_url));
            super::RepoRef {
                full_name: repo.to_string(),
                clone_url: format!("{html}.git"),
                norm_url: normalize_git_url(&html),
                html_url: html,
            }
        });
    let pr_ref = github::pr_ref(&pr);
    let sha = pr_ref.head_sha.clone().unwrap_or_default();
    let open = pr_ref.state == Some(super::PrState::Open);
    let mk = |kind: ForgeKind| ForgeEvent {
        provider: "github",
        repo: Some(repo_ref.clone()),
        pr: Some(pr_ref.clone()),
        kind,
        installation_id: Some(a.installation),
        sender: None,
    };
    let mut evs = vec![mk(ForgeKind::PrUpdated {
        head_changed: false,
    })];
    if open && pr_ref.mergeable == Some(false) {
        evs.push(mk(ForgeKind::Conflict));
    }
    // checks of the head commit: the runs, then the suite's verdict when none is still going
    let mut unfinished = false;
    if !sha.is_empty() {
        let runs = get_all(
            st,
            a,
            &format!("/repos/{repo}/commits/{sha}/check-runs?per_page=100"),
            "check_runs",
        )
        .await?;
        let mut failed = false;
        let mut any = false;
        for r in &runs {
            if r["status"].as_str() != Some("completed") {
                unfinished = true;
                continue;
            }
            let Some(conclusion) = github::conclusion(&r["conclusion"]) else {
                continue;
            };
            any = true;
            failed |= conclusion.failed();
            evs.push(mk(ForgeKind::ChecksCompleted {
                source: CheckSource::Run,
                name: r["name"].as_str().unwrap_or("check").to_string(),
                conclusion,
                summary: excerpt(r["output"]["title"].as_str().unwrap_or(""), 200),
                details_url: r["html_url"].as_str().map(str::to_string),
            }));
        }
        if any && !unfinished {
            evs.push(mk(ForgeKind::ChecksCompleted {
                source: CheckSource::Suite,
                name: "checks".into(),
                conclusion: if failed {
                    Conclusion::Failure
                } else {
                    Conclusion::Success
                },
                summary: String::new(),
                details_url: None,
            }));
        }
    }
    let reviews = get_all(
        st,
        a,
        &format!("/repos/{repo}/pulls/{number}/reviews?per_page=100"),
        "",
    )
    .await?;
    if let Some((state, author, body, url)) = review_of(&reviews) {
        evs.push(mk(ForgeKind::Review {
            state,
            author,
            excerpt: body,
            url,
        }));
    }
    for ev in &evs {
        events::apply(&st.db, a.conn.id, ev).await?;
    }
    if unfinished {
        sqlx::query(
            "update forge_prs set checks = 'pending' where provider = 'github' and lower(repo) = lower($1) and number = $2 and checks = 'none'",
        )
        .bind(&repo_ref.full_name)
        .bind(number)
        .execute(&st.db)
        .await?;
    }
    Ok(None)
}

/// Refresh the PRs linked to a task (one of them when `url` names it) from GitHub.
pub async fn refresh_task(
    st: &AppState,
    task: Uuid,
    only: Option<(String, i64)>,
) -> Result<Vec<String>> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "select repo, number, norm_url from forge_prs where task_id = $1 order by updated_at desc",
    )
    .bind(task)
    .fetch_all(&st.db)
    .await?;
    let mut warnings = vec![];
    for (repo, number, norm) in rows {
        if let Some((r, n)) = &only
            && (!r.eq_ignore_ascii_case(&repo) || *n != number)
        {
            continue;
        }
        let host = norm.split('/').next().unwrap_or("github.com").to_string();
        if let Some(w) = sync_pr(st, &host, &repo, number).await? {
            warnings.push(w);
        }
    }
    Ok(warnings)
}

/// After repositories joined pot `project`: in the background, find their open PRs that name the
/// pot's tasks and link them. The outcome is an event on the pot.
pub fn spawn_backfill(st: &AppState, project: &str, urls: Vec<String>) {
    let (st, project) = (st.clone(), project.to_string());
    tokio::spawn(async move {
        for url in urls {
            if let Err(e) = backfill(&st, &project, &url).await {
                tracing::warn!("forge backfill of {url}: {e}");
            }
        }
    });
}

async fn record(st: &AppState, project: &str, mut payload: Value) -> Result<()> {
    payload["reason"] = json!("backfill");
    payload["why"] = json!(
        "a repository joined the pot: its open pull requests were searched for ones that name the pot's tasks"
    );
    let mut tx = st.db.begin().await?;
    let pid: Option<Uuid> = sqlx::query_scalar("select id from projects where key = $1")
        .bind(project.to_uppercase())
        .fetch_optional(&mut *tx)
        .await?;
    service::emit(
        &mut tx,
        pid,
        None,
        "forge.backfill",
        Some(crate::state::RESTAURANT),
        payload,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Backfill one repository (by its url in the pot).
pub async fn backfill(st: &AppState, project: &str, url: &str) -> Result<()> {
    let norm = normalize_git_url(url);
    let (host, path) = norm.split_once('/').unwrap_or((&norm, ""));
    let repo: String = path.split('/').take(2).collect::<Vec<_>>().join("/");
    if repo.is_empty() || catalog::connections(st).await?.is_empty() {
        return Ok(());
    }
    let Some(a) = access_for(st, host, &repo).await? else {
        return record(
            st,
            project,
            json!({"repo": repo, "listed": 0, "linked": 0,
                   "note": "the GitHub App is not installed on this repository: its pull requests are not followed"}),
        )
        .await;
    };
    let prs = get_all(
        st,
        &a,
        &format!("/repos/{repo}/pulls?state=open&per_page=100"),
        "",
    )
    .await?;
    let mut linked = 0;
    for p in &prs {
        let Some(number) = p["number"].as_i64() else {
            continue;
        };
        let named = {
            let mut c = st.db.acquire().await?;
            auto_link(
                &mut c,
                &norm,
                p["head"]["ref"].as_str().unwrap_or(""),
                p["title"].as_str().unwrap_or(""),
                p["body"].as_str().unwrap_or(""),
            )
            .await?
        };
        if named.is_some() && sync_with(st, &a, &repo, number).await?.is_none() {
            linked += 1;
        }
    }
    record(
        st,
        project,
        json!({"repo": repo, "listed": prs.len(), "linked": linked}),
    )
    .await
}
