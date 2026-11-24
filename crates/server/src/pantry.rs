//! The pantry: shared memory per project. Agents save what they learn about a project
//! (conventions, gotchas, decisions, how-tos, facts) under a stable key; the next agent
//! recalls it instead of rediscovering it. Every MCP tool and REST handler calls these.
//!
//! Visibility is the project's: whoever can see the project can read and save memories.
//! Overwrites keep the previous version (`memory_revisions`). Forgetting is for people and
//! admins, and for the agent that created the memory.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    service::{emit, project_by_key, task_by_ref},
    state::Principal,
};

pub const KINDS: &[&str] = &["note", "convention", "gotcha", "decision", "howto", "fact"];
pub const MAX_BODY: usize = 20_000;
const MAX_TITLE: usize = 200;
/// Bodies longer than this are cut in search results (`truncated: true`); `get_memory` reads all.
const EXCERPT: usize = 600;
/// The digest stops adding bodies past this many characters and lists the rest by title.
const DIGEST_BUDGET: usize = 24_000;

/// SQL expression for one memory's JSON. Needs aliases `m` (memories) and `pr` (projects).
const MEMORY_JSON: &str = r#"jsonb_build_object(
  'key', m.key, 'project', pr.key, 'title', m.title, 'body', m.body, 'kind', m.kind,
  'tags', to_jsonb(m.tags),
  'task', (select tp.key || '-' || t.number from tasks t join projects tp on tp.id = t.project_id where t.id = m.task_id),
  'created_by', (select name from principals where id = m.created_by),
  'updated_by', (select name from principals where id = m.updated_by),
  'revision', m.revision, 'created_at', m.created_at, 'updated_at', m.updated_at,
  'uses', m.uses, 'last_used_at', m.last_used_at)"#;

/// A memory key: lowercase letters, digits, `.`, `_` and `-`, up to 80 characters, starting with
/// a letter or digit. Anything else is folded into `-`, so a title makes a usable key.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_') {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let mut out: String = out.chars().take(80).collect();
    while out.ends_with(['-', '.', '_']) {
        out.pop();
    }
    out
}

fn key_of(raw: &str) -> Result<String> {
    let k = slug(raw);
    if k.is_empty() {
        return invalid("memory key is empty: use letters, digits, '.', '_' or '-'");
    }
    Ok(k)
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tags
        .iter()
        .map(|t| slug(t))
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out.truncate(20);
    out
}

fn kind_of(k: Option<&str>) -> Result<Option<String>> {
    match k.map(str::trim).filter(|k| !k.is_empty()) {
        None => Ok(None),
        Some(k) => {
            let k = k.to_lowercase();
            if KINDS.contains(&k.as_str()) {
                Ok(Some(k))
            } else {
                invalid(format!("unknown kind {k:?}: one of {}", KINDS.join(", ")))
            }
        }
    }
}

async fn memory_json(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {MEMORY_JSON} from memories m join projects pr on pr.id = m.project_id where m.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

pub struct Remember {
    pub project: String,
    pub key: Option<String>,
    pub title: String,
    pub body: String,
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub task: Option<String>,
    /// Add `body` to the end of the existing memory instead of replacing it.
    pub append: bool,
    /// Optimistic concurrency: fail with a conflict unless the memory is at this revision.
    pub revision: Option<i64>,
    /// Tell everyone now: a broadcast on the pass wakes every session of every cook in the pot.
    pub notify: bool,
    /// The saver's own station, so the broadcast reaches its other sessions too.
    pub station: Option<String>,
}

/// Note that a chef has just read the pot's pantry: what changes after this is its news.
async fn mark_read(c: &mut PgConnection, principal: Uuid, project: Uuid) -> Result<()> {
    sqlx::query(
        "insert into pantry_reads (principal_id, project_id) values ($1, $2)
         on conflict (principal_id, project_id) do update set last_read_at = now()",
    )
    .bind(principal)
    .bind(project)
    .execute(c)
    .await?;
    Ok(())
}

/// What was saved or changed in a pot's pantry, by someone else, since this chef last read it:
/// `{count, items: [{key, title, kind, by, updated_at}]}` (the 10 most recent).
pub async fn news(c: &mut PgConnection, principal: Uuid, project: Uuid) -> Result<Value> {
    // saved or changed memories, and memories removed since (unless saved again under that key)
    let rows: Vec<Value> = sqlx::query_scalar(
        "with since as (select coalesce((select last_read_at from pantry_reads
                                           where principal_id = $1 and project_id = $2), '-infinity') as t)
         select item from (
           select jsonb_build_object('key', m.key, 'title', m.title, 'kind', m.kind,
                    'by', (select name from principals where id = coalesce(m.updated_by, m.created_by)),
                    'updated_at', m.updated_at, 'deleted', false) as item, m.updated_at as at
           from memories m, since
           where m.project_id = $2
             and coalesce(m.updated_by, m.created_by) is distinct from $1
             and m.updated_at > since.t
           union all
           select jsonb_build_object('key', e.payload->>'key', 'title', e.payload->>'title', 'kind', 'removed',
                    'by', (select name from principals where id = e.actor_id),
                    'updated_at', e.at, 'deleted', true), e.at
           from events e, since
           where e.project_id = $2 and e.kind = 'memory.forgotten'
             and e.actor_id is distinct from $1 and e.at > since.t
             and not exists (select 1 from memories m where m.project_id = $2 and m.key = e.payload->>'key')
         ) x order by at desc",
    )
    .bind(principal)
    .bind(project)
    .fetch_all(&mut *c)
    .await?;
    let count = rows.len();
    Ok(json!({"count": count, "items": rows.into_iter().take(10).collect::<Vec<_>>()}))
}

/// Save a memory under `key` (derived from the title when omitted): creates it, or updates it,
/// keeping the previous version.
pub async fn remember(db: &PgPool, p: &Principal, r: Remember) -> Result<Value> {
    let mut tx = db.begin().await?;
    let (out, created, pr_key) = remember_tx(&mut tx, p, &r).await?;
    tx.commit().await?;
    if r.notify {
        let what = if created { "new" } else { "updated" };
        let first = crate::questions::short(&body_for_notice(&out));
        let key = out["key"].as_str().unwrap_or_default().to_string();
        crate::pass::post(
            db,
            p,
            crate::pass::Post {
                project: pr_key.clone(),
                body: format!(
                    "Pantry, {what} {}: **{}** (`{key}`)\n\n{first}\n\nRead it with `get_memory` (project {pr_key}, key {key}).",
                    out["kind"].as_str().unwrap_or("note"),
                    out["title"].as_str().unwrap_or(""),
                ),
                to: None,
                task: None,
                from_owner: false,
                broadcast: true,
                station: r.station.clone(),
            },
        )
        .await?;
    }
    Ok(out)
}

/// [`remember`] inside the caller's transaction (no broadcast): what `complete`, `release` and
/// `block` use to save what the holder learned together with ending the lease.
pub async fn remember_in(c: &mut PgConnection, p: &Principal, r: &Remember) -> Result<Value> {
    Ok(remember_tx(c, p, r).await?.0)
}

/// The saved memory, whether it was new, and its project's key.
async fn remember_tx(
    tx: &mut PgConnection,
    p: &Principal,
    r: &Remember,
) -> Result<(Value, bool, String)> {
    let title = r.title.trim().to_string();
    let body = r.body.trim().to_string();
    if title.is_empty() && r.key.is_none() {
        return invalid("title is required");
    }
    if title.chars().count() > MAX_TITLE {
        return invalid(format!("title is longer than {MAX_TITLE} characters"));
    }
    if body.is_empty() {
        return invalid("body is empty: say what you learned");
    }
    let key = key_of(r.key.as_deref().unwrap_or(&title))?;
    let kind = kind_of(r.kind.as_deref())?;
    let tags = r.tags.as_deref().map(clean_tags);

    let pr = project_by_key(tx, p, &r.project.to_uppercase()).await?;
    let task_id = match r.task.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Some(task_by_ref(tx, p, t).await?.id),
        None => None,
    };
    let existing: Option<(
        Uuid,
        i64,
        String,
        String,
        String,
        Vec<String>,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "select id, revision, title, body, kind, tags, updated_by, updated_at from memories
             where project_id = $1 and key = $2 for update",
    )
    .bind(pr.id)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;

    let (id, created) = match existing {
        None => {
            if let Some(rev) = r.revision
                && rev != 0
            {
                return Err(Error::Conflict(format!(
                    "memory {key} does not exist (expected revision {rev})"
                )));
            }
            if title.is_empty() {
                return invalid("title is required for a new memory");
            }
            if body.chars().count() > MAX_BODY {
                return invalid(format!("body is longer than {MAX_BODY} characters"));
            }
            let id = Uuid::new_v4();
            sqlx::query(
                "insert into memories (id, project_id, key, title, body, kind, tags, task_id, created_by, updated_by)
                 values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)",
            )
            .bind(id)
            .bind(pr.id)
            .bind(&key)
            .bind(&title)
            .bind(&body)
            .bind(kind.as_deref().unwrap_or("note"))
            .bind(tags.unwrap_or_default())
            .bind(task_id)
            .bind(p.id)
            .execute(&mut *tx)
            .await?;
            (id, true)
        }
        Some((id, rev, old_title, old_body, old_kind, old_tags, old_by, old_at)) => {
            if let Some(want) = r.revision
                && want != rev
            {
                return Err(Error::Conflict(format!(
                    "memory {key} is at revision {rev}, not {want}: someone saved it meanwhile; \
                     read it again (get_memory) and merge"
                )));
            }
            let new_body = if r.append {
                format!("{old_body}\n\n{body}")
            } else {
                body.clone()
            };
            if new_body.chars().count() > MAX_BODY {
                return invalid(format!(
                    "body would be longer than {MAX_BODY} characters: condense it, or split it into another memory"
                ));
            }
            let new_title = if title.is_empty() {
                old_title.clone()
            } else {
                title.clone()
            };
            let new_kind = kind.unwrap_or_else(|| old_kind.clone());
            let new_tags = tags.unwrap_or_else(|| old_tags.clone());
            if new_title == old_title
                && new_body == old_body
                && new_kind == old_kind
                && new_tags == old_tags
            {
                let out = memory_json(tx, id).await?;
                return Ok((out, false, pr.key));
            }
            sqlx::query(
                "insert into memory_revisions (memory_id, revision, title, body, kind, tags, updated_by, updated_at)
                 values ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(id)
            .bind(rev)
            .bind(&old_title)
            .bind(&old_body)
            .bind(&old_kind)
            .bind(&old_tags)
            .bind(old_by)
            .bind(old_at)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "update memories set title = $2, body = $3, kind = $4, tags = $5,
                   task_id = coalesce($6, task_id), updated_by = $7, revision = revision + 1
                 where id = $1",
            )
            .bind(id)
            .bind(&new_title)
            .bind(&new_body)
            .bind(&new_kind)
            .bind(&new_tags)
            .bind(task_id)
            .bind(p.id)
            .execute(&mut *tx)
            .await?;
            (id, false)
        }
    };
    let out = memory_json(tx, id).await?;
    emit(
        tx,
        Some(pr.id),
        task_id,
        "memory.saved",
        Some(p.id),
        json!({"key": key, "title": out["title"], "created": created}),
    )
    .await?;
    Ok((out, created, pr.key))
}

fn body_for_notice(m: &Value) -> String {
    m["body"].as_str().unwrap_or("").to_string()
}

/// Memories offered with a claim.
const RELEVANT: i64 = 5;
/// Characters of a body in a relevant-memory excerpt.
const RELEVANT_EXCERPT: usize = 240;

/// Words of a title that can stand as search terms: lowercase letters and digits only (so the
/// text goes safely into a tsquery), no short or filler words.
fn title_words(title: &str) -> Vec<String> {
    const FILLER: &[&str] = &[
        "the", "and", "for", "with", "from", "into", "that", "this", "when", "then", "than", "are",
        "was", "not", "all", "can", "has", "have", "its", "our", "out", "per", "via", "add",
        "make",
    ];
    let mut out: Vec<String> = vec![];
    for w in title
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
    {
        if w.chars().count() >= 3 && !FILLER.contains(&w.as_str()) && !out.contains(&w) {
            out.push(w);
        }
        if out.len() >= 12 {
            break;
        }
    }
    out
}

/// The memories of a pot most relevant to a task, at most five, each `{key, title, kind,
/// excerpt}`: those learned on the task, its parent or its dependencies; those tagged with one
/// of its labels or repositories; the pot's conventions and gotchas; and those whose key or
/// title shares a word with the task's title. Deterministic (no model): ranked by lineage, then
/// tag, then text match, then kind, then recency. Without a task: conventions and gotchas, the
/// newest first.
pub async fn relevant(
    c: &mut PgConnection,
    project: Uuid,
    task: Option<Uuid>,
) -> Result<Vec<Value>> {
    let (title, labels): (String, Vec<String>) = match task {
        Some(t) => {
            let row: Option<(String, Vec<String>, Vec<String>)> =
                sqlx::query_as("select title, labels, repos from tasks where id = $1")
                    .bind(t)
                    .fetch_optional(&mut *c)
                    .await?;
            row.map(|(t, mut l, r)| {
                l.extend(r);
                (t, l)
            })
            .unwrap_or_default()
        }
        None => (String::new(), vec![]),
    };
    let tags = clean_tags(&labels);
    let tsq = title_words(&title).join(" | ");
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "with lineage as (
           select $2::uuid as id
           union select parent_id from tasks where id = $2 and parent_id is not null
           union select depends_on from task_deps where task_id = $2),
         q as (select case when $4 = '' then ''::tsquery else to_tsquery('simple', $4) end as tq)
         select m.key, m.title, m.kind, left(m.body, $5 + 1)
         from memories m, q
         where m.project_id = $1
           and (m.task_id in (select id from lineage)
                or (cardinality($3::text[]) > 0 and m.tags && $3::text[])
                or m.kind in ('convention', 'gotcha')
                or m.search @@ q.tq)
         order by 4 * coalesce(m.task_id in (select id from lineage), false)::int
                + 2 * (cardinality($3::text[]) > 0 and m.tags && $3::text[])::int
                + ts_rank(m.search, q.tq)
                + (case m.kind when 'convention' then 1 when 'gotcha' then 1 else 0 end) desc,
                  m.updated_at desc
         limit $6",
    )
    .bind(project)
    .bind(task)
    .bind(&tags)
    .bind(&tsq)
    .bind(RELEVANT_EXCERPT as i32)
    .bind(RELEVANT)
    .fetch_all(&mut *c)
    .await?;
    let keys: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    bump_uses(c, project, &keys).await?;
    Ok(rows
        .into_iter()
        .map(|(key, title, kind, body)| {
            let excerpt = if body.chars().count() > RELEVANT_EXCERPT {
                let cut: String = body.chars().take(RELEVANT_EXCERPT).collect();
                format!("{}…", cut.trim_end())
            } else {
                body
            };
            json!({"key": key, "title": title, "kind": kind, "excerpt": excerpt})
        })
        .collect())
}

/// Memories that were handed to a reader (a claim, recall, the digest, get_memory) were used: they
/// earn their place, and are not forget candidates.
async fn bump_uses(c: &mut PgConnection, project: Uuid, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "update memories set uses = uses + 1, last_used_at = now() where project_id = $1 and key = any($2)",
    )
    .bind(project)
    .bind(keys)
    .execute(c)
    .await?;
    Ok(())
}

/// One memory, in full.
pub async fn get(db: &PgPool, p: &Principal, project: &str, key: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.to_uppercase()).await?;
    let key = key_of(key)?;
    mark_read(&mut c, p.id, pr.id).await?;
    // read for use: which memories earn their place, and which nobody opens (forget candidates)
    sqlx::query(
        "update memories set uses = uses + 1, last_used_at = now() where project_id = $1 and key = $2",
    )
    .bind(pr.id)
    .bind(&key)
    .execute(&mut *c)
    .await?;
    let sql = format!(
        "select {MEMORY_JSON} from memories m join projects pr on pr.id = m.project_id
         where m.project_id = $1 and m.key = $2"
    );
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&key)
        .fetch_optional(&mut *c)
        .await?
        .ok_or_else(|| Error::NotFound(format!("no memory {key} in {}", pr.key)))
}

pub struct Recall {
    pub project: String,
    pub query: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub limit: Option<i64>,
}

/// Search the pantry: full-text over key, title and body (ranked; words OR'ed when no memory has
/// all of them), filtered by kind and tags (all must match). Without a query: most recent first.
/// Bodies are cut to an excerpt; `get_memory` reads one in full.
pub async fn recall(db: &PgPool, p: &Principal, r: Recall) -> Result<Value> {
    let kind = kind_of(r.kind.as_deref())?;
    let tags = clean_tags(&r.tags);
    let limit = r.limit.unwrap_or(20).clamp(1, 100);
    let query = r.query.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &r.project.to_uppercase()).await?;
    mark_read(&mut c, p.id, pr.id).await?;

    let run = |tsq: &'static str| {
        format!(
            "with q as (select {tsq} as tq)
             select {MEMORY_JSON} || jsonb_build_object('rank', round(ts_rank_cd(m.search, q.tq)::numeric, 4))
             from memories m join projects pr on pr.id = m.project_id, q
             where m.project_id = $1
               and ($2::text is null or m.kind = $2)
               and m.tags @> $3
               and ($4::text is null or m.search @@ q.tq or m.key ilike '%' || $4 || '%' or m.title ilike '%' || $4 || '%')
             order by case when $4::text is null then 0 else ts_rank_cd(m.search, q.tq) end desc,
                      m.updated_at desc
             limit $5"
        )
    };
    // 1. every word (websearch syntax: "quoted phrases", -exclusions, or); 2. any word.
    let strict = run(
        "(websearch_to_tsquery('english', coalesce($4, '')) || websearch_to_tsquery('simple', coalesce($4, '')))",
    );
    let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(strict))
        .bind(pr.id)
        .bind(kind.as_deref())
        .bind(&tags)
        .bind(query)
        .bind(limit)
        .fetch_all(&mut *c)
        .await?;
    if rows.is_empty()
        && let Some(q) = query
        && q.split_whitespace().count() > 1
    {
        let loose = run(
            "(select coalesce(to_tsquery('english', string_agg(quote_literal(w) || ':*', ' | ')), ''::tsquery)
              from unnest(tsvector_to_array(to_tsvector('simple', coalesce($4, '')))) w)",
        );
        rows = sqlx::query_scalar(sqlx::AssertSqlSafe(loose))
            .bind(pr.id)
            .bind(kind.as_deref())
            .bind(&tags)
            .bind(Some(q))
            .bind(limit)
            .fetch_all(&mut *c)
            .await?;
    }
    let keys: Vec<String> = rows
        .iter()
        .filter_map(|m| m["key"].as_str().map(String::from))
        .collect();
    bump_uses(&mut c, pr.id, &keys).await?;
    for m in rows.iter_mut() {
        if query.is_none() {
            m.as_object_mut().map(|o| o.remove("rank"));
        }
        let body = m["body"].as_str().unwrap_or("");
        if body.chars().count() > EXCERPT {
            let cut: String = body.chars().take(EXCERPT).collect();
            m["body"] = json!(format!("{}…", cut.trim_end()));
            m["truncated"] = json!(true);
        }
    }
    Ok(Value::Array(rows))
}

/// Forget a memory. People and admins can forget any memory of a project they can see; an agent
/// only the ones it created (ask a human for the rest, or overwrite with `remember`).
pub async fn forget(
    db: &PgPool,
    p: &Principal,
    project: &str,
    key: &str,
    notify: bool,
    station: Option<String>,
) -> Result<Value> {
    let key = key_of(key)?;
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &project.to_uppercase()).await?;
    let row: Option<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "select id, title, created_by from memories where project_id = $1 and key = $2 for update",
    )
    .bind(pr.id)
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((id, title, created_by)) = row else {
        return Err(Error::NotFound(format!("no memory {key} in {}", pr.key)));
    };
    if !p.can_plan() && !p.is_super_admin() && created_by != Some(p.id) {
        return Err(Error::Forbidden(format!(
            "memory {key} was saved by someone else: agents only forget their own memories (overwrite it with remember, or ask a human)"
        )));
    }
    sqlx::query("delete from memories where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(pr.id),
        None,
        "memory.forgotten",
        Some(p.id),
        json!({"key": key, "title": title}),
    )
    .await?;
    tx.commit().await?;
    if notify {
        crate::pass::post(
            db,
            p,
            crate::pass::Post {
                project: pr.key.clone(),
                body: format!(
                    "Pantry, removed: **{title}** (`{key}`). It no longer applies: do not rely on it."
                ),
                to: None,
                task: None,
                from_owner: false,
                broadcast: true,
                station,
            },
        )
        .await?;
    }
    Ok(json!({"ok": true}))
}

/// The whole pantry as one markdown document, grouped by kind (conventions and gotchas first):
/// what an agent reads when it starts on a project. Past a size budget the rest is listed by
/// title only, to be read with `get_memory`.
pub async fn digest(db: &PgPool, p: &Principal, project: &str) -> Result<String> {
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.to_uppercase()).await?;
    mark_read(&mut c, p.id, pr.id).await?;
    let rows: Vec<(String, String, String, String, Vec<String>)> = sqlx::query_as(
        "select key, title, body, kind, tags from memories where project_id = $1
         order by array_position(array['convention','gotcha','decision','howto','fact','note'], kind),
                  updated_at desc",
    )
    .bind(pr.id)
    .fetch_all(&mut *c)
    .await?;
    let mut md = format!("# {} pantry\n\n", pr.name);
    if rows.is_empty() {
        md.push_str(
            "Nothing saved yet. When you learn something the next agent should know, save it with `remember`.\n",
        );
        return Ok(md);
    }
    let mut used = 0usize;
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut shown: Vec<String> = Vec::new();
    let mut current = "";
    for (key, title, body, kind, tags) in &rows {
        if used > DIGEST_BUDGET {
            skipped.push((key.clone(), title.clone()));
            continue;
        }
        shown.push(key.clone());
        if kind != current {
            current = kind;
            md.push_str(&format!("## {}\n\n", heading(kind)));
        }
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!(" · {}", tags.join(", "))
        };
        let entry = format!("### {title}\n`{key}`{tags}\n\n{}\n\n", body.trim());
        used += entry.len();
        md.push_str(&entry);
    }
    if !skipped.is_empty() {
        md.push_str("## More (read with get_memory)\n\n");
        for (key, title) in skipped {
            md.push_str(&format!("- `{key}`: {title}\n"));
        }
    }
    let unused: Vec<(String, String)> = sqlx::query_as(
        "select key, title from memories
         where project_id = $1 and coalesce(last_used_at, created_at) < now() - interval '90 days'
         order by coalesce(last_used_at, created_at) limit 20",
    )
    .bind(pr.id)
    .fetch_all(&mut *c)
    .await?;
    // staleness is judged before this read: what the digest showed counts as used from now on
    bump_uses(&mut c, pr.id, &shown).await?;
    if !unused.is_empty() {
        md.push_str(
            "\n## Possibly stale (not read for 90 days)\n\nConsider `forget` only if one is obsolete or wrong: not being read does not make a memory wrong. Reading it (`get_memory`) keeps it.\n\n",
        );
        for (key, title) in unused {
            md.push_str(&format!("- `{key}`: {title}\n"));
        }
    }
    Ok(md)
}

fn heading(kind: &str) -> &'static str {
    match kind {
        "convention" => "Conventions",
        "gotcha" => "Gotchas",
        "decision" => "Decisions",
        "howto" => "How-tos",
        "fact" => "Facts",
        _ => "Notes",
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slugs() {
        assert_eq!(
            slug("How we run the e2e tests!"),
            "how-we-run-the-e2e-tests"
        );
        assert_eq!(
            slug("  --DB/Migrations: append-only  "),
            "db-migrations-append-only"
        );
        assert_eq!(slug("api.v2_routes"), "api.v2_routes");
        assert_eq!(slug("!!!"), "");
        assert_eq!(slug(&"a".repeat(100)).len(), 80);
        assert_eq!(slug("Élan vital"), "lan-vital");
    }
}
