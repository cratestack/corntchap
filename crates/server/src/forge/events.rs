//! What a forge event does inside corntchap. Provider neutral: it works on [`ForgeEvent`]s.
//!
//! * `forge_prs` follows each pull request of a repository that belongs to a pot (or that a person
//!   linked by hand). Events about other repositories are dropped.
//! * A PR is linked to a task by a person (explicit, wins), else by the task's ref in the head
//!   branch, the title or the body (`SHOP-12`, any case), never by creating anything.
//! * Every event updates the row and, when the PR is linked, emits a `forge.*` task event. It never
//!   changes the task's status, not on merge and not on close.
//! * The task's holder (its claimer, else the cook that held it last; never a person) is woken for
//!   a CI failure (once per head sha), a conflict (once per head sha), changes requested and
//!   comments by others, and told about a merge.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{CheckSource, ForgeEvent, ForgeKind, PrState, ReviewState, store};
use crate::{
    error::Result,
    repos::{self, Upstream, normalize_git_url},
    service,
};

/// Wake reason for something a cook should act on: starts a cloud run, like a comment on its task.
pub const REASON: &str = "forge";
/// Wake reason for a merge notification: information, not a call. Starts a cloud run only for
/// targets that take work, like `upstream`.
pub const REASON_MERGED: &str = "forge-merged";

const MAX_FAILING: usize = 10;

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Pr {
    pub id: Uuid,
    pub provider: String,
    pub repo: String,
    pub norm_url: String,
    pub number: i64,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub state: String,
    pub draft: bool,
    pub head_branch: String,
    pub head_sha: String,
    pub base_branch: Option<String>,
    pub checks: String,
    pub failing: Vec<String>,
    pub failing_sha: String,
    pub checks_url: Option<String>,
    pub review: String,
    pub mergeable: String,
    pub task_id: Option<Uuid>,
    pub explicit: bool,
    pub unlinked: bool,
    pub notified: Vec<String>,
}

/// What came of one event: `ok` or `ignored`, and why.
pub struct Applied {
    pub status: &'static str,
    pub detail: String,
}

fn ok(detail: impl Into<String>) -> Result<Applied> {
    Ok(Applied {
        status: "ok",
        detail: detail.into(),
    })
}
fn ignored(detail: impl Into<String>) -> Result<Applied> {
    Ok(Applied {
        status: "ignored",
        detail: detail.into(),
    })
}

/// Apply the events of one delivery. The status of the delivery is `ok` when any event did
/// something, `ignored` when none did.
pub async fn process(db: &PgPool, connection: Uuid, events: &[ForgeEvent]) -> Result<Applied> {
    let mut did = vec![];
    let mut skipped = vec![];
    for ev in events {
        let a = apply(db, connection, ev).await?;
        if a.status == "ok" {
            did.push(a.detail);
        } else {
            skipped.push(a.detail);
        }
    }
    if did.is_empty() {
        ignored(if skipped.is_empty() {
            "nothing corntchap uses".to_string()
        } else {
            skipped.join("; ")
        })
    } else {
        ok(did.join("; "))
    }
}

/// The pots (id, key) that have this repository, by `project_repos` and by the legacy `repo_url`.
pub async fn pots_for(c: &mut PgConnection, norm: &str) -> Result<Vec<(Uuid, String)>> {
    let mut pots: Vec<(Uuid, String)> = sqlx::query_as(
        "select pr.id, pr.key from projects pr
         where exists (select 1 from project_repos r where r.project_id = pr.id and r.norm_url = $1)",
    )
    .bind(norm)
    .fetch_all(&mut *c)
    .await?;
    let legacy: Vec<(Uuid, String, String)> = sqlx::query_as(
        "select id, key, repo_url from projects where repo_url is not null and btrim(repo_url) <> ''",
    )
    .fetch_all(&mut *c)
    .await?;
    for (id, key, url) in legacy {
        if normalize_git_url(&url) == norm && !pots.iter().any(|p| p.0 == id) {
            pots.push((id, key));
        }
    }
    pots.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(pots)
}

/// The numbers `KEY-<n>` in `text` for a pot key, any case, never inside a longer word
/// (`xshop-12` and `shop-12a` do not count).
pub fn refs_in(text: &str, key: &str) -> Vec<i64> {
    let t = text.to_lowercase();
    let needle = format!("{}-", key.to_lowercase());
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = t[from..].find(&needle) {
        let at = from + i;
        let end = at + needle.len();
        from = end;
        if t[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric())
        {
            continue;
        }
        let digits: String = t[end..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() || digits.len() > 9 {
            continue;
        }
        if t[end + digits.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric())
        {
            continue;
        }
        if let Ok(n) = digits.parse()
            && !out.contains(&n)
        {
            out.push(n);
        }
    }
    out
}

/// The task a PR belongs to by its words: branch first, then title, then body; the first task
/// that exists in a pot that has the repository.
async fn auto_link(
    c: &mut PgConnection,
    norm: &str,
    branch: &str,
    title: &str,
    body: &str,
) -> Result<Option<(Uuid, &'static str)>> {
    let pots = pots_for(c, norm).await?;
    for (via, text) in [("branch", branch), ("title", title), ("body", body)] {
        if text.is_empty() {
            continue;
        }
        for (pid, key) in &pots {
            for n in refs_in(text, key) {
                let t: Option<Uuid> = sqlx::query_scalar(
                    "select id from tasks where project_id = $1 and number = $2",
                )
                .bind(pid)
                .bind(n)
                .fetch_optional(&mut *c)
                .await?;
                if let Some(t) = t {
                    return Ok(Some((t, via)));
                }
            }
        }
    }
    Ok(None)
}

struct Ctx {
    task: Uuid,
    project: Uuid,
    key: String,
    tref: String,
}

async fn ctx_of(c: &mut PgConnection, task: Uuid) -> Result<Option<Ctx>> {
    let r: Option<(Uuid, String, i64)> = sqlx::query_as(
        "select t.project_id, pr.key, t.number from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
    )
    .bind(task)
    .fetch_optional(c)
    .await?;
    Ok(r.map(|(project, key, number)| Ctx {
        task,
        project,
        tref: format!("{key}-{number}"),
        key,
    }))
}

/// The cook to tell: the task's claimer, else the cook that held it last. Never a person.
pub async fn holder(c: &mut PgConnection, task: Uuid) -> Result<Option<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "select a.id, a.name from principals a
         where a.kind = 'agent' and a.disabled_at is null and a.id = coalesce(
           (select claimed_by from tasks where id = $1 and status = 'claimed'),
           (select e.actor_id from events e join principals x on x.id = e.actor_id and x.kind = 'agent'
             where e.task_id = $1 and e.kind in ('task.claimed','task.completed','task.released','task.blocked')
             order by e.seq desc limit 1))",
    )
    .bind(task)
    .fetch_optional(c)
    .await?)
}

/// `Octo-Bot[bot]` and `octo-bot` are the same account for this purpose.
fn same_account(login: &str, cook: &str) -> bool {
    let l = login.trim().to_lowercase();
    let l = l.strip_suffix("[bot]").unwrap_or(&l);
    !l.is_empty() && l == cook.trim().to_lowercase()
}

/// Wake the task's holder. Nothing when there is none or the words are the cook's own.
async fn tell(
    c: &mut PgConnection,
    ctx: &Ctx,
    by: Option<&str>,
    reason: &str,
    text: &str,
) -> Result<Option<String>> {
    let Some((agent, name)) = holder(c, ctx.task).await? else {
        return Ok(None);
    };
    if by.is_some_and(|b| same_account(b, &name)) {
        return Ok(None);
    }
    crate::questions::wake(
        c,
        ctx.project,
        &ctx.key,
        agent,
        None,
        Some(ctx.task),
        text,
        reason,
    )
    .await?;
    Ok(Some(name))
}

async fn note(c: &mut PgConnection, ctx: &Ctx, row: &Pr, kind: &str, extra: Value) -> Result<()> {
    let mut p = json!({
        "provider": row.provider, "repo": row.repo, "number": row.number, "url": row.url,
        "title": row.title, "state": row.state, "checks": row.checks, "review": row.review,
        "task": ctx.tref,
    });
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    service::emit(c, Some(ctx.project), Some(ctx.task), kind, None, p).await?;
    Ok(())
}

fn pr_word(r: &Pr) -> String {
    format!("{}#{}", r.repo, r.number)
}

/// A new head: its checks start over.
fn new_head(row: &mut Pr, sha: &str) {
    row.head_sha = sha.to_string();
    row.checks = "pending".into();
    row.failing = vec![];
    row.failing_sha = sha.to_string();
    row.checks_url = None;
    row.mergeable = "unknown".into();
}

pub async fn apply(db: &PgPool, connection: Uuid, ev: &ForgeEvent) -> Result<Applied> {
    if let Some(i) = ev.installation_id {
        match &ev.kind {
            ForgeKind::InstallationRemoved => store::remove_installation(db, connection, i).await?,
            ForgeKind::InstallationAdded {
                account,
                account_type,
                repo_selection,
            } => {
                store::upsert_installation(
                    db,
                    connection,
                    i,
                    account.as_deref(),
                    account_type.as_deref(),
                    repo_selection.as_deref(),
                )
                .await?
            }
            ForgeKind::InstallationSuspended(s) => {
                store::upsert_installation(db, connection, i, None, None, None).await?;
                store::suspend_installation(db, connection, i, *s).await?;
            }
            _ => store::upsert_installation(db, connection, i, None, None, None).await?,
        }
    }
    match &ev.kind {
        ForgeKind::InstallationAdded { .. }
        | ForgeKind::InstallationRemoved
        | ForgeKind::InstallationSuspended(_) => return ok("installation updated"),
        ForgeKind::Push {
            branch,
            sha,
            deleted,
        } => return push(db, ev, branch, sha, *deleted).await,
        _ => {}
    }
    let (Some(repo), Some(pr)) = (&ev.repo, &ev.pr) else {
        return ignored("no pull request in the event");
    };
    let mut tx = db.begin().await?;
    let c = &mut *tx;

    // find the row, or start following the PR when its repository belongs to a pot
    let found: Option<Uuid> = if pr.number == 0 {
        match pr.head_sha.as_deref().filter(|s| !s.is_empty()) {
            Some(sha) => {
                sqlx::query_scalar(
                    "select id from forge_prs where provider = $1 and lower(repo) = lower($2) and head_sha = $3 and state = 'open'
                     order by updated_at desc limit 1",
                )
                .bind(ev.provider)
                .bind(&repo.full_name)
                .bind(sha)
                .fetch_optional(&mut *c)
                .await?
            }
            None => None,
        }
    } else {
        sqlx::query_scalar(
            "select id from forge_prs where provider = $1 and lower(repo) = lower($2) and number = $3",
        )
        .bind(ev.provider)
        .bind(&repo.full_name)
        .bind(pr.number)
        .fetch_optional(&mut *c)
        .await?
    };
    let id = match found {
        Some(id) => id,
        None if pr.number == 0 => return ignored("a check for a commit that is no followed PR"),
        None => {
            if pots_for(c, &repo.norm_url).await?.is_empty() {
                return ignored(format!("{} is in no pot", repo.full_name));
            }
            sqlx::query(
                "insert into forge_prs (provider, repo, norm_url, number, url) values ($1,$2,$3,$4,$5)
                 on conflict (provider, lower(repo), number) do nothing",
            )
            .bind(ev.provider)
            .bind(&repo.full_name)
            .bind(&repo.norm_url)
            .bind(pr.number)
            .bind(&pr.url)
            .execute(&mut *c)
            .await?;
            sqlx::query_scalar(
                "select id from forge_prs where provider = $1 and lower(repo) = lower($2) and number = $3",
            )
            .bind(ev.provider)
            .bind(&repo.full_name)
            .bind(pr.number)
            .fetch_one(&mut *c)
            .await?
        }
    };
    // one event at a time per PR: a burst of deliveries must tell a cook once
    let mut row: Pr = sqlx::query_as("select id, provider, repo, norm_url, number, url, title, author, state, draft, head_branch, head_sha, base_branch, checks, failing, failing_sha, checks_url, review, mergeable, task_id, explicit, unlinked, notified from forge_prs where id = $1 for update")
        .bind(id)
        .fetch_one(&mut *c)
        .await?;

    // a check for a commit that is no longer the head says nothing about the PR
    let sha = pr.head_sha.clone().filter(|s| !s.is_empty());
    if matches!(ev.kind, ForgeKind::ChecksCompleted { .. })
        && let Some(s) = &sha
        && !row.head_sha.is_empty()
        && &row.head_sha != s
    {
        return ignored(format!("{} checks of an older commit", pr_word(&row)));
    }

    // what the event says about the PR
    if !pr.url.is_empty() {
        row.url = pr.url.clone();
    }
    if let Some(t) = &pr.title {
        row.title = t.clone();
    }
    if pr.author.is_some() {
        row.author = pr.author.clone();
    }
    if let Some(d) = pr.draft {
        row.draft = d;
    }
    if pr.base_branch.is_some() {
        row.base_branch = pr.base_branch.clone();
    }
    if let Some(b) = pr.head_branch.as_deref().filter(|b| !b.is_empty()) {
        row.head_branch = b.to_string();
    }
    if let Some(s) = pr.state {
        row.state = s.as_str().to_string();
    }
    let full = matches!(
        ev.kind,
        ForgeKind::PrOpened | ForgeKind::PrUpdated { .. } | ForgeKind::PrReopened
    );
    let mut head_changed = false;
    if let Some(s) = &sha {
        if row.head_sha.is_empty() {
            row.head_sha = s.clone();
            row.failing_sha = s.clone();
        } else if full && &row.head_sha != s {
            new_head(&mut row, s);
            head_changed = true;
        }
    }
    if let Some(m) = pr.mergeable {
        row.mergeable = m.to_string();
    }

    let mut first_merge = false;
    match &ev.kind {
        ForgeKind::PrMerged => {
            row.state = PrState::Merged.as_str().into();
            first_merge = !row.notified.iter().any(|k| k == "merged");
            if first_merge {
                row.notified.push("merged".into());
            }
        }
        ForgeKind::PrClosed => row.state = PrState::Closed.as_str().into(),
        ForgeKind::PrOpened | ForgeKind::PrReopened => {
            if row.state != "merged" {
                row.state = PrState::Open.as_str().into();
            }
        }
        ForgeKind::Conflict => row.mergeable = "false".into(),
        ForgeKind::ChecksCompleted {
            source,
            name,
            conclusion,
            details_url,
            ..
        } => {
            if row.failing_sha != row.head_sha {
                row.failing = vec![];
                row.failing_sha = row.head_sha.clone();
            }
            let names_it = *source != CheckSource::Suite;
            if conclusion.failed() {
                if (names_it || row.failing.is_empty()) && !row.failing.contains(name) {
                    row.failing.push(name.clone());
                    row.failing.truncate(MAX_FAILING);
                }
                if *source != CheckSource::Run {
                    row.checks = "failure".into();
                    row.checks_url = details_url.clone();
                }
            } else if *conclusion == super::Conclusion::Success {
                row.failing.retain(|n| n != name);
                if *source != CheckSource::Run {
                    row.checks = if row.failing.is_empty() {
                        "success"
                    } else {
                        "failure"
                    }
                    .into();
                    row.checks_url = details_url.clone();
                }
            }
        }
        ForgeKind::Review { state, .. } => match state {
            ReviewState::Approved => row.review = "approved".into(),
            ReviewState::ChangesRequested => row.review = "changes_requested".into(),
            ReviewState::Commented => {
                if row.review == "none" {
                    row.review = "commented".into()
                }
            }
            ReviewState::Dismissed => row.review = "none".into(),
        },
        _ => {}
    }

    // the task: a person's link wins; else the words in the branch, title and body
    let mut linked_now: Option<&'static str> = None;
    if row.task_id.is_none()
        && !row.explicit
        && !row.unlinked
        && let Some((t, via)) = auto_link(
            c,
            &row.norm_url,
            &row.head_branch,
            &row.title,
            pr.body.as_deref().unwrap_or(""),
        )
        .await?
    {
        row.task_id = Some(t);
        linked_now = Some(via);
    }
    let ctx = match row.task_id {
        Some(t) => ctx_of(c, t).await?,
        None => None,
    };
    let mut told: Vec<String> = vec![];
    if let Some(ctx) = &ctx {
        if let Some(via) = linked_now {
            note(c, ctx, &row, "forge.linked", json!({"via": via})).await?;
        }
        told = react(c, ctx, &mut row, ev, head_changed, first_merge).await?;
    }
    if first_merge {
        pass_on_merge(c, &row, ctx.as_ref()).await?;
    }

    sqlx::query(
        "update forge_prs set url=$2, title=$3, author=$4, state=$5, draft=$6, head_branch=$7, head_sha=$8, base_branch=$9,
           checks=$10, failing=$11, failing_sha=$12, checks_url=$13, review=$14, mergeable=$15, task_id=$16, notified=$17,
           updated_at = now() where id = $1",
    )
    .bind(row.id)
    .bind(&row.url)
    .bind(&row.title)
    .bind(&row.author)
    .bind(&row.state)
    .bind(row.draft)
    .bind(&row.head_branch)
    .bind(&row.head_sha)
    .bind(&row.base_branch)
    .bind(&row.checks)
    .bind(&row.failing)
    .bind(&row.failing_sha)
    .bind(&row.checks_url)
    .bind(&row.review)
    .bind(&row.mergeable)
    .bind(row.task_id)
    .bind(&row.notified)
    .execute(&mut *c)
    .await?;
    tx.commit().await?;
    let mut d = format!("{} {}", pr_word(&row), kind_name(&ev.kind));
    if let Some(c) = &ctx {
        d.push_str(&format!(" -> {}", c.tref));
    }
    if !told.is_empty() {
        d.push_str(&format!(", told {}", told.join(", ")));
    }
    ok(d)
}

fn kind_name(k: &ForgeKind) -> &'static str {
    match k {
        ForgeKind::PrOpened => "opened",
        ForgeKind::PrUpdated { .. } => "updated",
        ForgeKind::PrClosed => "closed",
        ForgeKind::PrMerged => "merged",
        ForgeKind::PrReopened => "reopened",
        ForgeKind::ChecksCompleted { .. } => "checks",
        ForgeKind::Review { .. } => "review",
        ForgeKind::Comment { .. } => "comment",
        ForgeKind::Conflict => "conflict",
        ForgeKind::Push { .. } => "push",
        _ => "installation",
    }
}

/// Emit the task event and wake the holder for one event on a linked PR. Returns who was woken.
async fn react(
    c: &mut PgConnection,
    ctx: &Ctx,
    row: &mut Pr,
    ev: &ForgeEvent,
    head_changed: bool,
    first_merge: bool,
) -> Result<Vec<String>> {
    let mut told = vec![];
    let word = pr_word(row);
    let tref = &ctx.tref;
    match &ev.kind {
        ForgeKind::PrOpened => note(c, ctx, row, "forge.pr", json!({"action": "opened"})).await?,
        ForgeKind::PrReopened => {
            note(c, ctx, row, "forge.pr", json!({"action": "reopened"})).await?
        }
        ForgeKind::PrClosed => note(c, ctx, row, "forge.pr", json!({"action": "closed"})).await?,
        ForgeKind::PrUpdated { .. } if head_changed => {
            note(
                c,
                ctx,
                row,
                "forge.pr",
                json!({"action": "updated", "sha": row.head_sha}),
            )
            .await?
        }
        ForgeKind::PrMerged if first_merge => {
            note(c, ctx, row, "forge.merged", json!({})).await?;
            let text = format!(
                "{word} merged ({}); {tref}'s status is unchanged, serve it if the work is done.",
                row.url
            );
            told.extend(tell(c, ctx, None, REASON_MERGED, &text).await?);
        }
        ForgeKind::ChecksCompleted {
            source,
            conclusion,
            details_url,
            ..
        } if *source != CheckSource::Run => {
            let sha = row.head_sha.clone();
            let ckey = format!("checks:{sha}:{}", conclusion.as_str());
            if !row.notified.contains(&ckey) {
                row.notified.push(ckey);
                note(
                    c,
                    ctx,
                    row,
                    "forge.checks",
                    json!({"conclusion": conclusion.as_str(), "failing": row.failing.join(", "),
                           "details_url": details_url, "sha": sha}),
                )
                .await?;
            }
            let wkey = format!("ci:{sha}");
            if conclusion.failed() && !row.notified.contains(&wkey) {
                row.notified.push(wkey);
                let names = if row.failing.is_empty() {
                    "checks".to_string()
                } else {
                    row.failing.join(", ")
                };
                let url = details_url.clone().unwrap_or_else(|| row.url.clone());
                let text = format!(
                    "CI failed on {word} ({names}): {url}\n\nIt is the pull request of {tref} (commit {}).",
                    sha.chars().take(8).collect::<String>()
                );
                told.extend(tell(c, ctx, None, REASON, &text).await?);
            }
        }
        ForgeKind::Review {
            state,
            author,
            excerpt,
            url,
        } => {
            note(
                c,
                ctx,
                row,
                "forge.review",
                json!({"review": state.as_str(), "author": author, "excerpt": excerpt, "link": url}),
            )
            .await?;
            let calls = *state == ReviewState::ChangesRequested
                || (*state == ReviewState::Commented && !excerpt.is_empty());
            if calls {
                let what = if *state == ReviewState::ChangesRequested {
                    "asked for changes on"
                } else {
                    "reviewed"
                };
                let text = format!(
                    "{author} {what} {word} ({tref}): {excerpt}\n{url}",
                    excerpt = if excerpt.is_empty() {
                        "(no text)"
                    } else {
                        excerpt
                    }
                );
                told.extend(tell(c, ctx, Some(author), REASON, &text).await?);
            }
        }
        ForgeKind::Comment {
            inline,
            author,
            excerpt,
            path,
            line,
            url,
        } => {
            note(
                c,
                ctx,
                row,
                "forge.comment",
                json!({"author": author, "excerpt": excerpt, "path": path, "line": line, "link": url,
                       "inline": inline}),
            )
            .await?;
            let at = match (path, line) {
                (Some(p), Some(l)) => format!(" on {p}:{l}"),
                (Some(p), None) => format!(" on {p}"),
                _ => String::new(),
            };
            let text = format!("{author} commented{at} on {word} ({tref}): {excerpt}\n{url}");
            told.extend(tell(c, ctx, Some(author), REASON, &text).await?);
        }
        ForgeKind::Conflict => {
            let ckey = format!("conflict:{}", row.head_sha);
            if !row.notified.contains(&ckey) {
                row.notified.push(ckey);
                note(c, ctx, row, "forge.conflict", json!({})).await?;
                let text = format!(
                    "{word} ({tref}) has merge conflicts with {}: update the branch and resolve them. {}",
                    row.base_branch.as_deref().unwrap_or("its base"),
                    row.url
                );
                told.extend(tell(c, ctx, None, REASON, &text).await?);
            }
        }
        _ => {}
    }
    Ok(told)
}

/// A merge reaches the repositories that depend on this one (what a served task does, see
/// `repos::pass_on`): their scoped cooks are told, or a follow-up task is opened (mode task).
async fn pass_on_merge(c: &mut PgConnection, row: &Pr, ctx: Option<&Ctx>) -> Result<()> {
    for (project, key) in pots_for(c, &row.norm_url).await? {
        let aliases: Vec<String> = sqlx::query_scalar(
            "select name from project_repos where project_id = $1 and norm_url = $2",
        )
        .bind(project)
        .bind(&row.norm_url)
        .fetch_all(&mut *c)
        .await?;
        if aliases.is_empty() {
            // a pot that has it only as its legacy repo_url: nothing can depend on it
            continue;
        }
        let task = ctx.filter(|x| x.project == project).map(|x| x.task);
        let up = Upstream {
            label: pr_word(row),
            title: row.title.clone(),
            did: "was merged",
            detail_name: "Pull request",
            detail: row.url.clone(),
            look: String::new(),
            task,
            menu: None,
            priority: 0,
        };
        repos::pass_on(c, None, project, &key, &aliases, &up).await?;
    }
    Ok(())
}

/// A push to a branch: a followed PR whose head it is has new commits.
async fn push(
    db: &PgPool,
    ev: &ForgeEvent,
    branch: &str,
    sha: &str,
    deleted: bool,
) -> Result<Applied> {
    let (Some(repo), false) = (&ev.repo, deleted || sha.is_empty()) else {
        return ignored("push without a commit");
    };
    let mut tx = db.begin().await?;
    let c = &mut *tx;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "select id from forge_prs where provider = $1 and lower(repo) = lower($2) and head_branch = $3 and state = 'open' and head_sha <> $4
         order by id for update",
    )
    .bind(ev.provider)
    .bind(&repo.full_name)
    .bind(branch)
    .bind(sha)
    .fetch_all(&mut *c)
    .await?;
    let mut n = 0;
    for id in ids {
        let mut row: Pr = sqlx::query_as("select id, provider, repo, norm_url, number, url, title, author, state, draft, head_branch, head_sha, base_branch, checks, failing, failing_sha, checks_url, review, mergeable, task_id, explicit, unlinked, notified from forge_prs where id = $1")
            .bind(id)
            .fetch_one(&mut *c)
            .await?;
        new_head(&mut row, sha);
        sqlx::query(
            "update forge_prs set head_sha=$2, checks=$3, failing='{}', failing_sha=$2, checks_url=null, mergeable='unknown', updated_at=now() where id=$1",
        )
        .bind(id)
        .bind(sha)
        .bind(&row.checks)
        .execute(&mut *c)
        .await?;
        if let Some(t) = row.task_id
            && let Some(ctx) = ctx_of(c, t).await?
        {
            note(
                c,
                &ctx,
                &row,
                "forge.pr",
                json!({"action": "updated", "sha": sha}),
            )
            .await?;
        }
        n += 1;
    }
    tx.commit().await?;
    if n == 0 {
        ignored(format!("push to {branch}: no followed PR"))
    } else {
        ok(format!("push to {branch}: {n} PR(s) have new commits"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_refs_in_words() {
        assert_eq!(refs_in("shop-12-fix", "SHOP"), vec![12]);
        assert_eq!(refs_in("feat/SHOP-12", "shop"), vec![12]);
        assert_eq!(refs_in("Fixes SHOP-3 and shop-40.", "SHOP"), vec![3, 40]);
        assert!(refs_in("xshop-12", "SHOP").is_empty());
        assert!(refs_in("shop-12a", "SHOP").is_empty());
        assert!(refs_in("shop-", "SHOP").is_empty());
        assert_eq!(refs_in("ai-helm-7-x", "AI-HELM"), vec![7]);
    }

    #[test]
    fn own_account() {
        assert!(same_account("octo-bot[bot]", "Octo-Bot"));
        assert!(!same_account("bob", "octo"));
        assert!(!same_account("", ""));
    }
}
