//! The pass: the chefs of a pot talk, people (in the console) and cooks alike. A cook can also post
//! in its owner's name when they ask in a chat (`from_owner`: author = the owner, relayed_by = the
//! cook). The pass is working chatter, kept a week (`PASS_RETENTION_DAYS`): durable knowledge goes
//! to the pantry. A message with no recipient is on the pot's pass, read by every chef of
//! the pot; with a recipient it is direct, read only by its author, the relaying cook and the
//! recipient. A direct message to a cook, or an @mention of one on the pass, wakes it (on its last
//! woken station in that pot) with the message as the prompt.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    questions::{short, wake},
    service::{emit_to, project_by_key, task_by_ref},
    state::Principal,
};

const MAX_BODY: usize = 8000;

/// One message as callers see it.
const MSG_JSON: &str = r#"jsonb_build_object(
  'id', m.id, 'seq', m.seq, 'project', pr.key,
  'from', (select name from principals where id = m.author_id),
  'from_kind', (select kind from principals where id = m.author_id),
  'via', (select name from principals where id = m.relayed_by),
  'to', (select name from principals where id = m.to_principal),
  'body', m.body,
  'task', (select pr.key || '-' || t.number from tasks t where t.id = m.task_id),
  'at', m.created_at, 'woken', m.woken,
  'station', (select s.ref from stations s where s.id = m.station_id),
  'station_label', (select coalesce(s.label, s.directory, s.session_ref) from stations s where s.id = m.station_id),
  'to_station', (select s.ref from stations s where s.id = m.to_station_id),
  'to_station_label', (select coalesce(s.label, s.directory, s.session_ref) from stations s where s.id = m.to_station_id))"#;

/// Messages of pot `$1` that principal `$2` may read.
const VISIBLE: &str = "m.project_id = $1 and (m.to_principal is null or m.author_id = $2 or m.relayed_by = $2 or m.to_principal = $2)";

/// The conversation a visible message belongs to, for principal `$2`: 'pot', or the other chef.
const CONVERSATION: &str = "case when m.to_principal is null then 'pot'
     when m.author_id = $2 or m.relayed_by = $2 then m.to_principal::text
     else m.author_id::text end";

pub struct Post {
    pub project: String,
    pub body: String,
    pub to: Option<String>,
    pub task: Option<String>,
    /// Post in the cook's owner's name (the owner asked for it in a chat with their cook).
    pub from_owner: bool,
    /// Wake every session of every chef in the pot (also `@all` in the message).
    pub broadcast: bool,
    /// The sender's own station, so a broadcast skips that session but reaches its siblings.
    pub station: Option<String>,
}

pub async fn post(db: &PgPool, p: &Principal, a: Post) -> Result<Value> {
    let p = p.clone().worker()?;
    let body = a.body.trim();
    if body.is_empty() {
        return invalid("the message is empty");
    }
    if body.chars().count() > MAX_BODY {
        return invalid(format!("a message holds {MAX_BODY} characters at most"));
    }
    if a.from_owner && !p.is_agent() {
        return invalid("fromOwner is for cooks speaking for their owner: you post as yourself");
    }
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, &p, &a.project.trim().to_uppercase()).await?;
    // Who it is from: this cook, or its owner with this cook relaying.
    let (author, author_name, relayed_by) = if a.from_owner {
        let owner = p
            .owner_id
            .ok_or_else(|| Error::Invalid("you have no owner to speak for".into()))?;
        let name: String = sqlx::query_scalar("select name from principals where id = $1")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        if chef(&mut tx, pr.team_id, &name).await?.is_none() {
            return invalid(format!(
                "your owner {name} is not a chef of {}: they cannot speak on its pass",
                pr.key
            ));
        }
        (owner, name.clone(), Some(p.id))
    } else {
        (p.id, p.name.clone(), None)
    };
    let signature = match relayed_by {
        Some(_) => format!("{author_name} (via {})", p.name),
        None => author_name.clone(),
    };
    // the session sending it, if it said: shown with the message, a broadcast skips it, and a
    // direct answer can come back to it
    let own_station = crate::stations::for_claim(&mut tx, &p, a.station.as_deref(), pr.id).await?;
    let own_ref: Option<(String, String)> =
        match own_station {
            Some(s) => sqlx::query_as(
                "select ref, coalesce(label, directory, session_ref) from stations where id = $1",
            )
            .bind(s)
            .fetch_optional(&mut *tx)
            .await?,
            None => None,
        };
    // `to` names a chef, or one session (st_...): then exactly that session is woken
    let mut to_station: Option<(Uuid, String, String)> = None;
    let to = match a.to.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(r) if r.starts_with("st_") => {
            let row: Option<(Uuid, Uuid, String, String)> = sqlx::query_as(
                "select st.id, st.agent_id, a.name, coalesce(st.label, st.directory, st.session_ref)
                 from stations st join principals a on a.id = st.agent_id
                 join team_members tm on tm.principal_id = a.id and tm.team_id = $3
                 where st.ref = $1 and st.project_id = $2 and a.disabled_at is null",
            )
            .bind(r)
            .bind(pr.id)
            .bind(pr.team_id)
            .fetch_optional(&mut *tx)
            .await?;
            let (sid, agent, name, label) =
                row.ok_or_else(|| Error::NotFound(format!("no station {r} in {}", pr.key)))?;
            if own_station == Some(sid) {
                return invalid(
                    "that is your own session: name another station, or leave `to` out",
                );
            }
            to_station = Some((sid, r.to_string(), label));
            Some((agent, name))
        }
        Some(name) => {
            let name = name.trim_start_matches('@').to_lowercase();
            let id = chef(&mut tx, pr.team_id, &name)
                .await?
                .ok_or_else(|| Error::NotFound(format!("no chef named {name} in {}", pr.key)))?;
            if id == p.id || id == author {
                return invalid(
                    "that is you: leave `to` out to post on the pot's pass, or name one of your other sessions (to: st_...)",
                );
            }
            Some((id, name))
        }
    };
    let task = match a.task.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(r) => {
            let t = task_by_ref(&mut tx, &p, r).await?;
            if t.project_id != pr.id {
                return invalid(format!("{r} is not in {}", pr.key));
            }
            Some(t.id)
        }
    };
    let (id, seq): (Uuid, i64) = sqlx::query_as(
        "insert into pass_messages (project_id, author_id, to_principal, body, task_id, relayed_by, station_id, to_station_id)
         values ($1,$2,$3,$4,$5,$6,$7,$8) returning id, seq",
    )
    .bind(pr.id)
    .bind(author)
    .bind(to.as_ref().map(|t| t.0))
    .bind(body)
    .bind(task)
    .bind(relayed_by)
    .bind(own_station)
    .bind(to_station.as_ref().map(|t| t.0))
    .fetch_one(&mut *tx)
    .await?;
    // Your own message is read (by its author and by the cook that relayed it).
    let conv = conversation(to.as_ref().map(|t| t.0));
    mark_read(&mut tx, p.id, pr.id, &conv, seq).await?;
    if author != p.id {
        mark_read(&mut tx, author, pr.id, &conv, seq).await?;
    }
    // A direct message's event is for its author, the relaying cook and the recipient only.
    let audience = to.as_ref().map(|t| {
        let mut a = vec![author, t.0];
        a.extend(relayed_by);
        a
    });
    emit_to(
        &mut tx,
        Some(pr.id),
        task,
        "pass.posted",
        Some(author),
        json!({"message": id, "to": to.as_ref().map(|t| &t.1),
               "via": relayed_by.map(|_| p.name.clone()), "title": short(body)}),
        audience.as_deref(),
    )
    .await?;

    let mentioned = mentions(body);
    let broadcast = a.broadcast
        || (mentioned.contains("all") && chef(&mut tx, pr.team_id, "all").await?.is_none());
    if broadcast && to.is_some() {
        return invalid("a broadcast goes to everyone: leave `to` out");
    }
    let mut reached: Vec<Value> = vec![];
    if broadcast {
        let rows: Vec<(Uuid, String, Option<Uuid>, Option<String>)> = sqlx::query_as(
            "select a.id, a.name, st.id, st.ref from principals a
             join team_members tm on tm.principal_id = a.id and tm.team_id = $1
             left join stations st on st.agent_id = a.id and st.project_id = $2
             where a.kind = 'agent' and a.disabled_at is null
             order by a.name, st.created_at",
        )
        .bind(pr.team_id)
        .bind(pr.id)
        .fetch_all(&mut *tx)
        .await?;
        let prompt = format!(
            "{signature} wrote to everyone on {}'s pass (every chef's sessions got it):\n\n{body}\n\n\
             Read it, and answer on the pass (the `pass` tool, project {}) only if you have something to add; \
             the other sessions are reading it too.",
            pr.key, pr.key
        );
        for (agent, name, station, sref) in rows {
            let own = match (own_station, station) {
                (Some(mine), Some(st)) => st == mine,
                // without its station the sender's cook is skipped whole: it cannot tell its sessions apart
                _ => agent == p.id,
            };
            if own {
                continue;
            }
            reached.push(
                wake(
                    &mut tx,
                    pr.id,
                    &pr.key,
                    agent,
                    station,
                    task,
                    &prompt,
                    "broadcast",
                )
                .await?,
            );
            let _ = (name, sref);
        }
        let out = record_woken(&mut tx, id, &reached).await?;
        tx.commit().await?;
        return Ok(out);
    }

    // Wake the cooks it is for: the recipient of a direct message, or the cooks @mentioned.
    let mut woken: BTreeSet<Uuid> = BTreeSet::new();
    let targets: Vec<(Uuid, bool)> = match &to {
        Some((id, _)) => vec![(*id, true)],
        None => {
            let mut v = vec![];
            for name in mentioned {
                if let Some(id) = chef(&mut tx, pr.team_id, &name).await?
                    && id != p.id
                    && id != author
                {
                    v.push((id, false));
                }
            }
            v
        }
    };
    for (id, direct) in targets {
        let is_agent: bool =
            sqlx::query_scalar("select kind = 'agent' from principals where id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if !is_agent || !woken.insert(id) {
            continue;
        }
        // answers go back to the sending session when it said which one it is
        let (from, reply_to) = match &own_ref {
            Some((r, label)) => (format!("{signature} ({label})"), r.clone()),
            None => (signature.clone(), author_name.clone()),
        };
        let prompt = if direct {
            let whom = match &to_station {
                Some((_, _, label)) => format!("this session ({label})"),
                None => "you".to_string(),
            };
            format!(
                "{from} wrote to {whom} on {}'s pass (only you can read it):\n\n{body}\n\nAnswer with the `pass` tool (project {}, to {reply_to}); read_pass shows the conversation.",
                pr.key, pr.key
            )
        } else {
            format!(
                "{signature} mentioned you on {}'s pass:\n\n{body}\n\nAnswer with the `pass` tool (project {}); read_pass shows what else was said.",
                pr.key, pr.key
            )
        };
        let station = if direct {
            to_station.as_ref().map(|t| t.0)
        } else {
            None
        };
        reached.push(wake(&mut tx, pr.id, &pr.key, id, station, task, &prompt, "pass").await?);
    }
    let out = record_woken(&mut tx, id, &reached).await?;
    tx.commit().await?;
    Ok(out)
}

/// Keep who a message woke with the message itself, so every reader sees it. The sender is told
/// when a cloud cook was among them: that is fire and forget.
async fn record_woken(c: &mut PgConnection, id: Uuid, woken: &[Value]) -> Result<Value> {
    sqlx::query("update pass_messages set woken = $2 where id = $1")
        .bind(id)
        .bind(json!(woken))
        .execute(&mut *c)
        .await?;
    let mut out = one(c, id).await?;
    if let Some(n) = cloud_note(woken) {
        out["note"] = json!(n);
    }
    Ok(out)
}

/// What to know about the cloud cooks a message woke, if any.
pub fn cloud_note(woken: &[Value]) -> Option<String> {
    let mut names: Vec<&str> = woken
        .iter()
        .filter(|w| w["cloud"] == true)
        .filter_map(|w| w["cook"].as_str())
        .collect();
    names.dedup();
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "{} {} a cloud cook: fire and forget. Each message starts a fresh run that reads the recent conversation, acts, answers once and stops; it does not stay to chat or keep working between messages. Put everything it needs in one message, or in a task.",
        names.join(", "),
        if names.len() == 1 { "is" } else { "are" }
    ))
}

/// Read a pot's pass. `chef`: none for everything you can read (the pass and your direct
/// messages), `pot` for the pass only, or a chef's name for your conversation with them. Newest
/// `limit` messages, or those after `after` (a seq), oldest first. What you read counts as read.
pub async fn read(
    db: &PgPool,
    p: &Principal,
    project: &str,
    with: Option<&str>,
    after: Option<i64>,
    limit: Option<i64>,
) -> Result<Value> {
    let p = p.clone().worker()?;
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, &p, &project.trim().to_uppercase()).await?;
    let limit = limit.unwrap_or(30).clamp(1, 200);
    let (mode, peer): (&str, Option<Uuid>) = match with.map(str::trim).filter(|s| !s.is_empty()) {
        None => ("all", None),
        Some(w) if w.eq_ignore_ascii_case("pot") || w == "#" => ("pot", None),
        Some(name) => {
            let name = name.trim_start_matches('@').to_lowercase();
            let id = chef(&mut tx, pr.team_id, &name)
                .await?
                .ok_or_else(|| Error::NotFound(format!("no chef named {name} in {}", pr.key)))?;
            ("peer", Some(id))
        }
    };
    let sql = format!(
        "select m.seq, {MSG_JSON} from pass_messages m join projects pr on pr.id = m.project_id
         where {VISIBLE} and {} and ($3::bigint is null or m.seq > $3)
         order by case when $3::bigint is null then -m.seq else m.seq end limit $4",
        scope("$5", "$6")
    );
    let mut rows: Vec<(i64, Value)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(p.id)
        .bind(after)
        .bind(limit)
        .bind(peer)
        .bind(mode)
        .fetch_all(&mut *tx)
        .await?;
    if after.is_none() {
        rows.reverse();
    }
    // Everything up to the newest message returned is read, per conversation.
    if let Some(max) = rows.iter().map(|r| r.0).max() {
        let sql = format!(
            "insert into pass_reads (principal_id, project_id, conversation, last_seq)
             select $2, $1, {CONVERSATION}, max(m.seq)
             from pass_messages m where {VISIBLE} and {} and m.seq <= $3
             group by 3
             on conflict (principal_id, project_id, conversation)
             do update set last_seq = greatest(pass_reads.last_seq, excluded.last_seq)",
            scope("$4", "$5")
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(pr.id)
            .bind(p.id)
            .bind(max)
            .bind(peer)
            .bind(mode)
            .execute(&mut *tx)
            .await?;
    }
    let unread = unread(&mut tx, p.id, pr.id).await?;
    tx.commit().await?;
    let cursor = rows.iter().map(|r| r.0).max().or(after).unwrap_or(0);
    Ok(json!({
        "messages": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "cursor": cursor,
        "unread": unread,
    }))
}

/// What a principal has not read in a pot: on the pass, and per direct conversation. Every direct
/// conversation is listed, the most recent first, with 0 when it is all read.
async fn unread(c: &mut PgConnection, me: Uuid, project: Uuid) -> Result<Value> {
    let sql = format!(
        "with conv as (
           select {CONVERSATION} as conversation, m.seq, m.author_id, m.relayed_by
           from pass_messages m
           where m.project_id = $1 and (m.to_principal is null or m.author_id = $2 or m.relayed_by = $2 or m.to_principal = $2))
         select c.conversation,
                (select name from principals where id::text = c.conversation),
                count(*) filter (where c.seq > coalesce(r.last_seq, 0)
                                   and c.author_id is distinct from $2 and c.relayed_by is distinct from $2)
         from conv c left join pass_reads r
           on r.principal_id = $2 and r.project_id = $1 and r.conversation = c.conversation
         group by c.conversation order by max(c.seq) desc"
    );
    let rows: Vec<(String, Option<String>, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(project)
        .bind(me)
        .fetch_all(&mut *c)
        .await?;
    let pot = rows.iter().find(|r| r.0 == "pot").map(|r| r.2).unwrap_or(0);
    let direct: Vec<Value> = rows
        .iter()
        .filter(|r| r.0 != "pot")
        .map(|r| json!({"chef": r.1, "count": r.2}))
        .collect();
    Ok(json!({"pot": pot, "direct": direct}))
}

async fn mark_read(
    c: &mut PgConnection,
    me: Uuid,
    project: Uuid,
    conversation: &str,
    seq: i64,
) -> Result<()> {
    sqlx::query(
        "insert into pass_reads (principal_id, project_id, conversation, last_seq) values ($1,$2,$3,$4)
         on conflict (principal_id, project_id, conversation)
         do update set last_seq = greatest(pass_reads.last_seq, excluded.last_seq)",
    )
    .bind(me)
    .bind(project)
    .bind(conversation)
    .bind(seq)
    .execute(c)
    .await?;
    Ok(())
}

/// Which messages a read covers: `all` you may read, the `pot`'s pass, or your conversation with
/// the `peer`. `peer` and `mode` are the placeholders bound to the peer id and the mode.
fn scope(peer: &str, mode: &str) -> String {
    format!(
        "({mode}::text = 'all' or ({mode} = 'pot' and m.to_principal is null)
          or ({mode} = 'peer' and (((m.author_id = $2 or m.relayed_by = $2) and m.to_principal = {peer}::uuid)
                                or (m.author_id = {peer}::uuid and m.to_principal = $2))))"
    )
}

fn conversation(peer: Option<Uuid>) -> String {
    peer.map(|p| p.to_string()).unwrap_or_else(|| "pot".into())
}

async fn one(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {MSG_JSON} from pass_messages m join projects pr on pr.id = m.project_id where m.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// A chef (person or cook, never a fire) of the team, by name.
pub(crate) async fn chef(c: &mut PgConnection, team: Uuid, name: &str) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "select pr.id from principals pr join team_members tm on tm.principal_id = pr.id
         where tm.team_id = $1 and pr.name = $2 and pr.kind <> 'relay' and pr.disabled_at is null",
    )
    .bind(team)
    .bind(name)
    .fetch_optional(c)
    .await?)
}

/// `@name` mentions in a message (lowercased, deduplicated).
pub(crate) fn mentions(body: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let chars: Vec<char> = body.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        let starts = *ch == '@' && (i == 0 || !chars[i - 1].is_alphanumeric());
        if !starts {
            continue;
        }
        let name: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let name = name.trim_end_matches('.').to_lowercase();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::mentions;

    #[test]
    fn finds_mentions() {
        let m = mentions("@doug can you look? cc @Alice. not mail@example.com");
        assert_eq!(
            m.into_iter().collect::<Vec<_>>(),
            vec!["alice".to_string(), "doug".to_string()]
        );
    }
}
