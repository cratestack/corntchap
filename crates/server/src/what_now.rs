//! `what_now`: a station's briefing, and the pot guide it carries.
//!
//! A session registered in one folder (a station) used to get no guidance at all about what it may
//! do beyond that folder. `what_now` answers "what should I do now?" for one cook and station in
//! one pot: what it holds, what waits on it, what is ready for its folder, what is ready in the
//! pot's other repositories, work that needs no checkout, the pot's guide, and ONE sentence
//! (`next`) chosen by a fixed order. The guide ("how we work here") is a markdown text the pot's
//! team owner writes for agents (`set_pot_guide`).

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    admin,
    error::{Result, invalid},
    service::{NO_OPEN_PR_SQL, READY_SQL, project_by_key, takeable_sql},
    state::Principal,
};

/// Characters a pot's guide may have.
pub const MAX_GUIDE: usize = 20_000;
/// Characters of the guide that come with `register_station` and `whoami`.
pub const GUIDE_EXCERPT: usize = 1_500;
/// Tasks listed per section.
const TOP: i64 = 5;
/// Tasks listed per other repository.
const TOP_ELSEWHERE: i64 = 3;

/// A task as the briefing lists it (needs aliases `t` and `pr`).
const NOW_TASK: &str =
    "jsonb_build_object('id', pr.key || '-' || t.number, 'title', t.title, 'priority', t.priority,
  'repos', to_jsonb(t.repos), 'labels', to_jsonb(t.labels))";

const FROM: &str = "from tasks t join projects pr on pr.id = t.project_id";

/// The start of a guide, cut at a character boundary, with a pointer to the rest.
pub fn excerpt(guide: &str) -> String {
    let g = guide.trim();
    if g.chars().count() <= GUIDE_EXCERPT {
        return g.to_string();
    }
    let cut: String = g.chars().take(GUIDE_EXCERPT).collect();
    format!(
        "{}…\n\n(The guide goes on: read the rest with what_now.)",
        cut.trim_end()
    )
}

/// The pot's guide, whole.
pub async fn guide_of(c: &mut PgConnection, project: Uuid) -> Result<Option<String>> {
    let g: Option<Option<String>> = sqlx::query_scalar("select guide from projects where id = $1")
        .bind(project)
        .fetch_optional(c)
        .await?;
    Ok(g.flatten().filter(|g| !g.trim().is_empty()))
}

/// The start of the pot's guide, for `register_station` and `whoami`.
pub async fn guide_excerpt(c: &mut PgConnection, project: Uuid) -> Result<Option<String>> {
    Ok(guide_of(c, project).await?.map(|g| excerpt(&g)))
}

/// One line for a wake prompt when the pot has a guide; empty otherwise.
pub async fn wake_hint(c: &mut PgConnection, project: Uuid, key: &str) -> String {
    match guide_of(c, project).await {
        Ok(Some(_)) => format!("This pot has a guide: call what_now (project {key})."),
        _ => String::new(),
    }
}

/// Set (or clear, with an empty text) a pot's guide. Its team's owner (or a cook acting for them)
/// and admins, like renaming the pot or setting its lease length.
pub async fn set_guide(db: &PgPool, p: &Principal, key: &str, guide: &str) -> Result<Value> {
    let p = &admin::acting(db, p).await?;
    let guide = guide.trim();
    let length = guide.chars().count();
    if length > MAX_GUIDE {
        return invalid(format!("a pot's guide is at most {MAX_GUIDE} characters"));
    }
    let mut tx = db.begin().await?;
    let (id, _) = admin::owned_project(&mut tx, p, key).await?;
    let value = (!guide.is_empty()).then_some(guide);
    sqlx::query("update projects set guide = $2 where id = $1")
        .bind(id)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        Some(id),
        None,
        "project.guide_changed",
        Some(p.id),
        json!({"length": length}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"key": key.trim().to_uppercase(), "guide": value, "length": length}))
}

/// Run a query with the pot ($1) and the cook ($2) bound.
async fn rows(c: &mut PgConnection, sql: String, project: Uuid, me: Uuid) -> Result<Vec<Value>> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(project)
        .bind(me)
        .fetch_all(c)
        .await?)
}

/// The same, with the station's repository ($3) bound too.
async fn rows_repo(
    c: &mut PgConnection,
    sql: String,
    project: Uuid,
    me: Uuid,
    repo: Option<&str>,
) -> Result<Vec<Value>> {
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(project)
        .bind(me)
        .bind(repo)
        .fetch_all(c)
        .await?)
}

/// What a station should do now (see the module text).
pub async fn what_now(
    db: &PgPool,
    p: &Principal,
    project: &str,
    station: Option<&str>,
) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.trim().to_uppercase()).await?;
    let st = crate::stations::for_claim(&mut c, p, station, pr.id).await?;
    let st_row: Option<(String, Option<String>, Option<String>, bool)> = match st {
        Some(id) => {
            sqlx::query_as("select ref, repo, directory, confined from stations where id = $1")
                .bind(id)
                .fetch_optional(&mut *c)
                .await?
        }
        None => None,
    };
    let (st_ref, st_repo, st_dir, confined) = match st_row {
        Some((r, repo, dir, conf)) => (Some(r), repo, dir, conf),
        None => (None, None, None, false),
    };
    let repo = st_repo.as_deref();
    let (project_id, me) = (pr.id, p.id);
    let scope = crate::repos::scope_sql("$2");
    let takeable = takeable_sql("$2");
    let here = "($3::text is null or cardinality(t.repos) = 0 or $3 = any(t.repos))";
    let from = FROM;

    // what it holds, and what is parked for it
    let holding = rows(
        &mut c,
        format!(
            "select {NOW_TASK} || jsonb_build_object('lease_expires_at', t.lease_expires_at,
                 'station', (select ref from stations where id = t.station_id))
             {from} where t.project_id = $1 and t.claimed_by = $2 and t.status = 'claimed' and t.lease_expires_at > now()
             order by t.lease_expires_at"
        ),
        project_id,
        me,
    )
    .await?;
    let parked = rows(
        &mut c,
        format!(
            "select {NOW_TASK} || jsonb_build_object('station', (select ref from stations where id = t.station_id))
             {from} where t.project_id = $1 and t.status = 'blocked' and t.reserved_for = $2
             order by t.updated_at"
        ),
        project_id,
        me,
    )
    .await?;

    // ready for this folder
    let ready_here = rows_repo(
        &mut c,
        format!(
            "select {NOW_TASK} {from} where t.project_id = $1 and {READY_SQL} and {scope} and {takeable} and {here}
             order by t.priority desc, t.created_at limit {TOP}"
        ),
        project_id,
        me,
        repo,
    )
    .await?;
    let ready_here_total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select count(*) {from} where t.project_id = $1 and {READY_SQL} and {scope} and {takeable} and {here}"
    )))
    .bind(project_id)
    .bind(me)
    .bind(repo)
    .fetch_one(&mut *c)
    .await?;

    // ready in the pot's other repositories (only when the station knows its own)
    let ready_elsewhere = if repo.is_some() {
        rows_repo(
            &mut c,
            format!(
                "with r as (
                   select u.repo, row_number() over (partition by u.repo order by t.priority desc, t.created_at) rn,
                          count(*) over (partition by u.repo) n, {NOW_TASK} as obj
                   {from} cross join lateral unnest(t.repos) as u(repo)
                   where t.project_id = $1 and {READY_SQL} and {scope} and {takeable}
                     and cardinality(t.repos) > 0 and not (t.repos && array[$3::text]))
                 select jsonb_build_object('repo', r.repo, 'url', pr_.url, 'default_branch', pr_.default_branch,
                          'count', max(r.n), 'top', jsonb_agg(r.obj order by r.rn) filter (where r.rn <= {TOP_ELSEWHERE}))
                 from r left join project_repos pr_ on pr_.project_id = $1 and pr_.name = r.repo
                 group by r.repo, pr_.url, pr_.default_branch, pr_.seq
                 order by max(r.n) desc, pr_.seq nulls last, r.repo"
            ),
            project_id,
            me,
            repo,
        )
        .await?
    } else {
        vec![]
    };

    // what waits on it
    let questions_to_me = rows(
        &mut c,
        "select jsonb_build_object('id', pr.key || '-Q' || q.number, 'title', q.title,
             'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
             'from', (select name from principals where id = q.asked_by))
         from questions q join projects pr on pr.id = q.project_id
         where q.project_id = $1 and q.status = 'open' and q.to_principal = $2
         order by q.created_at limit 10"
            .into(),
        project_id,
        me,
    )
    .await?;
    let comments = rows(
        &mut c,
        "select jsonb_build_object('task', pr.key || '-' || t.number,
             'author', (select name from principals where id = cm.author_id),
             'excerpt', case when char_length(cm.body) > 160 then left(cm.body, 160) || '…' else cm.body end,
             'at', cm.created_at)
         from comment_replies cr join comments cm on cm.id = cr.comment_id
           join tasks t on t.id = cr.task_id join projects pr on pr.id = t.project_id
         where cr.project_id = $1 and cr.agent_id = $2 and cr.answered_at is null
         order by cm.created_at limit 10"
            .into(),
        project_id,
        me,
    )
    .await?;
    let reviews = rows(
        &mut c,
        "select jsonb_build_object('id', pr.key || '-R' || r.number,
             'task', (select pr.key || '-' || t.number from tasks t where t.id = r.task_id),
             'from', (select name from principals where id = r.requested_by),
             'note', r.note, 'round', r.round)
         from reviews r join projects pr on pr.id = r.project_id
         where r.project_id = $1 and r.state = 'requested'
           and (r.reviewer = $2 or (r.reviewer is null and r.requested_by is distinct from $2))
         order by r.reviewer is null, r.created_at limit 10"
            .into(),
        project_id,
        me,
    )
    .await?;

    // work that needs no checkout
    let open_questions = rows(
        &mut c,
        "select jsonb_build_object('id', pr.key || '-Q' || q.number, 'title', q.title,
             'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
             'from', (select name from principals where id = q.asked_by))
         from questions q join projects pr on pr.id = q.project_id
         where q.project_id = $1 and q.status = 'open' and q.to_principal is null
           and q.asked_by is distinct from $2 and (q.picked_by is null or q.pick_expires_at < now())
         order by q.created_at limit 5"
            .into(),
        project_id,
        me,
    )
    .await?;
    let open_questions_total: i64 = sqlx::query_scalar(
        "select count(*) from questions q where q.project_id = $1 and q.status = 'open' and q.to_principal is null
           and q.asked_by is distinct from $2 and (q.picked_by is null or q.pick_expires_at < now())",
    )
    .bind(project_id)
    .bind(me)
    .fetch_one(&mut *c)
    .await?;
    let labelled = |labels: &str, extra: &str| {
        format!(
            "select {NOW_TASK} {from} where t.project_id = $1 and t.status = 'open' and t.labels && array[{labels}]::text[]
               and {scope} and {takeable} {extra}
             order by t.priority desc, t.created_at limit {TOP}"
        )
    };
    let needs_split = rows(
        &mut c,
        labelled(
            "'epic','roadmap'",
            "and not exists (select 1 from tasks sc where sc.parent_id = t.id)",
        ),
        project_id,
        me,
    )
    .await?;
    let triage = rows(&mut c, labelled("'triage','docs'", ""), project_id, me).await?;
    let stale = rows(
        &mut c,
        format!(
            "select obj from (
               select {NOW_TASK} || jsonb_build_object('holder', l.holder, 'lapsed_at', l.at) as obj, l.at
               {from} cross join lateral (
                 select e.kind, e.at, e.payload->>'agent' as holder from events e
                 where e.task_id = t.id and e.kind in ('task.claimed','task.released','task.blocked','task.completed',
                       'task.lease_expired','task.cancelled','task.unblocked','task.restored')
                 order by e.seq desc limit 1) l
               where t.project_id = $1 and t.status = 'open' and l.kind = 'task.lease_expired'
                 and l.at > now() - interval '14 days' and {NO_OPEN_PR_SQL}
               union all
               select {NOW_TASK} || jsonb_build_object('holder', (select name from principals where id = t.claimed_by),
                        'lapsed_at', t.lease_expires_at), t.lease_expires_at
               {from} where t.project_id = $1 and t.status = 'claimed' and t.lease_expires_at < now() and {NO_OPEN_PR_SQL}
             ) x order by at desc limit {TOP}"
        ),
        project_id,
        me,
    )
    .await?;
    let shown: Vec<String> = ready_here
        .iter()
        .filter_map(|t| t["id"].as_str().map(String::from))
        .collect();
    let no_repo: Vec<Value> = rows(
        &mut c,
        format!(
            "select {NOW_TASK} {from} where t.project_id = $1 and {READY_SQL} and cardinality(t.repos) = 0 and {takeable}
             order by t.priority desc, t.created_at limit {}",
            TOP * 2
        ),
        project_id,
        me,
    )
    .await?
    .into_iter()
    .filter(|t| !shown.iter().any(|s| Some(s.as_str()) == t["id"].as_str()))
    .take(TOP as usize)
    .collect();
    let pantry_news = crate::pantry::news(&mut c, me, project_id).await?["count"]
        .as_u64()
        .unwrap_or(0);
    let guide = guide_of(&mut c, project_id).await?;

    let next = next_step(&Parts {
        pot: &pr.key,
        station: st_ref.as_deref(),
        repo,
        confined,
        holding: &holding,
        questions: &questions_to_me,
        comments: &comments,
        reviews: &reviews,
        ready_here: &ready_here,
        ready_here_total,
        ready_elsewhere: &ready_elsewhere,
        open_questions: &open_questions,
        open_questions_total,
        needs_split: &needs_split,
        triage: &triage,
        stale: &stale,
        no_repo: &no_repo,
        pantry_news,
    });

    Ok(json!({
        "project": pr.key, "cook": p.name,
        "station": st_ref, "station_repo": st_repo, "station_directory": st_dir, "confined": confined,
        "holding": holding, "parked": parked,
        "ready_here": ready_here, "ready_here_total": ready_here_total,
        "ready_elsewhere": ready_elsewhere,
        "waiting_on_you": {"questions": questions_to_me, "comments": comments, "reviews": reviews},
        "folder_free": {
            "open_questions": open_questions, "open_questions_total": open_questions_total,
            "needs_split": needs_split, "triage": triage, "stale": stale, "no_repo": no_repo,
            "pantry_news": pantry_news,
        },
        "guide": guide,
        "next": next,
    }))
}

/// What `next_step` reads: the sections of the briefing.
pub struct Parts<'a> {
    pub pot: &'a str,
    pub station: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub confined: bool,
    pub holding: &'a [Value],
    pub questions: &'a [Value],
    pub comments: &'a [Value],
    pub reviews: &'a [Value],
    pub ready_here: &'a [Value],
    pub ready_here_total: i64,
    pub ready_elsewhere: &'a [Value],
    pub open_questions: &'a [Value],
    pub open_questions_total: i64,
    pub needs_split: &'a [Value],
    pub triage: &'a [Value],
    pub stale: &'a [Value],
    pub no_repo: &'a [Value],
    pub pantry_news: u64,
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

fn task(v: &Value) -> String {
    format!("{} ({})", s(v, "id"), s(v, "title"))
}

/// The ONE sentence: finish what you hold, then answer what waits on you, then claim what is ready
/// for your folder, then work that needs no folder, then what is ready elsewhere, then wait.
pub fn next_step(x: &Parts) -> String {
    let st = x
        .station
        .map(|s| format!(" with station {s}"))
        .unwrap_or_default();
    // what this very session holds (or what no session claims to hold); another station of the
    // same cook holds the rest: that session finishes it
    let mine = x.holding.iter().find(|t| match t["station"].as_str() {
        None => true,
        Some(h) => Some(h) == x.station,
    });
    if let Some(t) = mine {
        return format!(
            "Finish {}, which you hold (lease until {}): work it, tick its items, then complete it.",
            task(t),
            s(t, "lease_expires_at")
        );
    }
    if let Some(q) = x.questions.first() {
        return format!(
            "Answer {} ({}): it is addressed to you (answer_question).",
            s(q, "id"),
            s(q, "title")
        );
    }
    if let Some(r) = x.reviews.first() {
        return format!(
            "Review {} (round {}) for {}: read the task and what the note points at ({}), then give a verdict with review (approve, changes or comment).",
            s(r, "task"),
            r["round"].as_i64().unwrap_or(1),
            r["from"].as_str().unwrap_or("its requester"),
            s(r, "note"),
        );
    }
    if let Some(c) = x.comments.first() {
        return format!(
            "Answer {}'s comment on {} with comment: {}",
            s(c, "author"),
            s(c, "task"),
            s(c, "excerpt")
        );
    }
    if let Some(t) = x.ready_here.first() {
        return format!(
            "Claim {} with claim_task{st} ({} ready for your folder{}).",
            task(t),
            x.ready_here_total,
            if x.repo.is_some() {
                "; claim_next may hand you another repository's task first"
            } else {
                ""
            }
        );
    }
    if let Some(q) = x.open_questions.first() {
        return format!(
            "Answer or relay the open question {} ({}); {} wait in the pool, no checkout needed.",
            s(q, "id"),
            s(q, "title"),
            x.open_questions_total
        );
    }
    if let Some(t) = x.needs_split.first() {
        return format!(
            "Split {} into subtasks (create_task with its id as parent); it is an epic with none yet. No checkout needed.",
            task(t)
        );
    }
    if let Some(t) = x.triage.first() {
        return format!(
            "Triage {}: read it, sharpen it or split it, no checkout needed.",
            task(t)
        );
    }
    if let Some(t) = x.stale.first() {
        return format!(
            "Check {}: the lease of {} lapsed and no pull request is open; look for its work in that cook's branch or on the pass, then claim_task it or ask.",
            task(t),
            t["holder"].as_str().unwrap_or("its holder")
        );
    }
    if let Some(t) = x.no_repo.first() {
        return format!(
            "Take {} with claim_task: it names no repository, so it needs no checkout.",
            task(t)
        );
    }
    if x.pantry_news > 0 {
        return format!(
            "Read the pantry news ({} change{}, read_pantry), then keep the pantry current.",
            x.pantry_news,
            if x.pantry_news == 1 { "" } else { "s" }
        );
    }
    if let Some(g) = x.ready_elsewhere.first()
        && !x.confined
    {
        let repo = s(g, "repo");
        let top = g["top"]
            .as_array()
            .and_then(|t| t.first())
            .map(task)
            .unwrap_or_default();
        return format!(
            "Nothing is ready for {}: {} task(s) are ready in {repo} ({}), first {top}. Open a worktree or clone of {repo} next to your folder if your harness lets you (never inside another session's worktree), then claim_next from there{st}; if you cannot leave your folder, register_station with confined=true and say so.",
            x.repo.unwrap_or("your folder"),
            g["count"],
            g["url"].as_str().unwrap_or("no url"),
        );
    }
    if x.confined && !x.ready_elsewhere.is_empty() {
        return format!(
            "Nothing for your folder in {}: the ready tasks are about other repositories and wait for sessions that can leave theirs. Call wait_for_work.",
            x.pot
        );
    }
    format!(
        "Nothing is ready in {} for you now: call wait_for_work.",
        x.pot
    )
}
