//! The question pool, per pot. A cook asks (optionally parking its task: it stays reserved for
//! that cook and station, waiting); people answer in the console, or a cook in a chat with its
//! owner picks the question, asks the owner and records the answer in the owner's name ("selast,
//! via desktop-wow"). An answer hands a parked task back to the cook with a fresh lease and wakes
//! the very station that asked, with the answer in the prompt.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    error::{Error, Result, invalid},
    service::{emit, parse_task_ref, project_by_key, task_by_ref},
    state::Principal,
    stations,
};

const PICK_MINUTES: i64 = 15;

const QUESTION_JSON: &str = r#"jsonb_build_object(
  'id', pr.key || '-Q' || q.number, 'project', pr.key,
  'task', (select pr.key || '-' || t.number from tasks t where t.id = q.task_id),
  'asked_by', (select name from principals where id = q.asked_by),
  'station', (select ref from stations where id = q.station_id),
  'to', (select name from principals where id = q.to_principal),
  'body', q.body, 'title', q.title, 'description', q.description, 'kind', q.kind,
  'choices', q.choices, 'allow_other', q.allow_other, 'min', q.min_choices, 'max', q.max_choices,
  'options', to_jsonb(q.options), 'wait', q.wait, 'status', q.status,
  'picked_by', case when q.pick_expires_at > now() then (select name from principals where id = q.picked_by) end,
  'answer', q.answer, 'answer_choices', to_jsonb(q.answer_choices), 'answer_text', q.answer_text,
  'answered_by', (select name from principals where id = q.answered_by),
  'relayed_by', (select name from principals where id = q.relayed_by),
  'answered_at', q.answered_at, 'created_at', q.created_at, 'updated_at', q.updated_at,
  'notes', coalesce((select jsonb_agg(jsonb_build_object('author', (select name from principals where id = n.author_id),
      'body', n.body, 'at', n.created_at) order by n.created_at) from question_notes n where n.question_id = q.id), '[]'::jsonb),
  'files', coalesce((select jsonb_agg(jsonb_build_object('id', pr.key || '-F' || f.number, 'name', f.name,
      'content_type', f.content_type, 'size', f.size, 'status', f.status) order by f.number)
      from files f where f.question_id = q.id and f.status <> 'deleted'), '[]'::jsonb))"#;

/// `PAY-Q3` -> ("PAY", 3)
pub fn parse_ref(s: &str) -> Result<(String, i64)> {
    let bad = || Error::Invalid(format!("bad question id {s:?}, expected like PAY-Q3"));
    let (k, n) = s
        .trim()
        .rsplit_once("-Q")
        .or_else(|| s.trim().rsplit_once("-q"))
        .ok_or_else(bad)?;
    let n: i64 = n.parse().map_err(|_| bad())?;
    if k.is_empty() {
        return Err(bad());
    }
    Ok((k.to_uppercase(), n))
}

struct Q {
    id: Uuid,
    project_id: Uuid,
    key: String,
    number: i64,
    team_id: Uuid,
    task_id: Option<Uuid>,
    asked_by: Option<Uuid>,
    station_id: Option<Uuid>,
    to_principal: Option<Uuid>,
    status: String,
    wait: bool,
    picked_by: Option<Uuid>,
    pick_expires_at: Option<DateTime<Utc>>,
}

async fn load(c: &mut PgConnection, p: &Principal, r: &str, lock: bool) -> Result<Q> {
    let (key, number) = parse_ref(r)?;
    let pr = project_by_key(c, p, &key).await?;
    let sql = format!(
        "select id, task_id, asked_by, station_id, to_principal, status, wait, picked_by, pick_expires_at
         from questions where project_id = $1 and number = $2 {}",
        if lock { "for update" } else { "" }
    );
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        Option<Uuid>,
        String,
        bool,
        Option<Uuid>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(number)
        .fetch_optional(&mut *c)
        .await?;
    let Some((
        id,
        task_id,
        asked_by,
        station_id,
        to_principal,
        status,
        wait,
        picked_by,
        pick_expires_at,
    )) = row
    else {
        return Err(Error::NotFound(format!("no question {key}-Q{number}")));
    };
    Ok(Q {
        id,
        project_id: pr.id,
        key: pr.key,
        number,
        team_id: pr.team_id,
        task_id,
        asked_by,
        station_id,
        to_principal,
        status,
        wait,
        picked_by,
        pick_expires_at,
    })
}

async fn view(c: &mut PgConnection, id: Uuid) -> Result<Value> {
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id where q.id = $1"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_one(c)
        .await?)
}

/// First line, without markdown decoration, cut to 100 characters (event titles).
pub(crate) fn short(s: &str) -> String {
    let first = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let plain: String = first
        .trim()
        .trim_start_matches(['#', '>', '-', '*', ' '])
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect();
    let one = plain.trim();
    if one.chars().count() > 100 {
        format!("{}…", one.chars().take(100).collect::<String>())
    } else {
        one.to_string()
    }
}

/// The kinds of question: how it is answered.
pub const KINDS: [&str; 4] = ["single", "multiple", "free", "cards"];
const TITLE_MAX: usize = 200;
const DESCRIPTION_MAX: usize = 7000;
const CHOICES_MAX: usize = 12;

/// One answer proposal.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

impl Choice {
    fn json(&self) -> Value {
        match &self.description {
            Some(d) => json!({"id": self.id, "label": self.label, "description": d}),
            None => json!({"id": self.id, "label": self.label}),
        }
    }
}

pub struct Ask {
    pub project: Option<String>,
    /// The short question. When empty, `body` supplies it.
    pub title: Option<String>,
    pub description: Option<String>,
    /// The older single text: its first line becomes the title, the rest the description.
    pub body: Option<String>,
    pub kind: Option<String>,
    /// `[{label, description?, id?}]`, or strings, or one string (JSON text, or one choice per line).
    pub choices: Option<Value>,
    /// The older newline-separated options (already split).
    pub options: Vec<String>,
    pub allow_other: bool,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub task: Option<String>,
    pub to: Option<String>,
    pub wait: bool,
    pub station: Option<String>,
}

/// A long first line is cut near this many characters, at a word boundary, to make a title.
const TITLE_CUT: usize = 120;

/// The older single text split into a title (its first line, cut at a word boundary) and the rest.
pub fn split_body(body: &str) -> (String, String) {
    let body = body.trim();
    let (first, rest) = match body.split_once('\n') {
        Some((f, r)) => (f.trim(), r.trim()),
        None => (body, ""),
    };
    if first.chars().count() <= TITLE_CUT {
        return (first.to_string(), rest.to_string());
    }
    let cut: String = first.chars().take(TITLE_CUT).collect();
    let head = match cut.rfind(char::is_whitespace) {
        Some(i) if !cut[..i].trim().is_empty() => cut[..i].trim_end().to_string(),
        _ => cut,
    };
    let tail = first[head.len()..].trim();
    let desc = [tail, rest]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    (format!("{head}…"), desc)
}

/// A short lowercase slug of a label, for a choice id.
fn slug(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.chars().take(24).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "choice".into() } else { out }
}

/// The choices of an ask, from the structured value or the older options; ids made unique.
pub fn parse_choices(v: Option<&Value>, options: &[String]) -> Result<Vec<Choice>> {
    // one choice per line: "Label" or "Label: one line of description"
    fn from_lines(s: &str) -> Vec<Value> {
        s.lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches("- ")
                    .trim_start_matches("* ")
                    .trim()
            })
            .filter(|l| !l.is_empty())
            .map(|l| match l.split_once(": ") {
                Some((label, d)) if !label.trim().is_empty() && !d.trim().is_empty() => {
                    json!({"label": label.trim(), "description": d.trim()})
                }
                _ => json!(l),
            })
            .collect()
    }
    let bad =
        || Error::Invalid("choices: expected a JSON array of {label, description?, id?}".into());
    let items: Vec<Value> = match v {
        None | Some(Value::Null) => options.iter().map(|o| json!(o)).collect(),
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.starts_with('[') {
                match serde_json::from_str::<Value>(s) {
                    Ok(Value::Array(a)) => a,
                    _ => return Err(bad()),
                }
            } else {
                from_lines(s)
            }
        }
        Some(_) => return Err(bad()),
    };
    let mut out: Vec<Choice> = vec![];
    for it in items {
        let (label, description, id) = match &it {
            Value::String(s) => (s.trim().to_string(), None, None),
            Value::Object(o) => {
                let g = |k: &str| {
                    o.get(k)
                        .and_then(|x| x.as_str())
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                };
                match g("label").or_else(|| g("title")) {
                    Some(l) => (l, g("description"), g("id")),
                    None => return invalid("every choice needs a label"),
                }
            }
            _ => return Err(bad()),
        };
        if label.is_empty() {
            continue;
        }
        if label.chars().count() > 120 {
            return invalid(format!(
                "choice {label:?} is too long: keep labels short (120 characters), put the detail in its description"
            ));
        }
        if description
            .as_ref()
            .is_some_and(|d| d.chars().count() > 300)
        {
            return invalid("a choice description is at most 300 characters");
        }
        let base = id.map(|i| slug(&i)).unwrap_or_else(|| slug(&label));
        let mut cid = base.clone();
        let mut n = 2;
        while out.iter().any(|c| c.id == cid) {
            cid = format!("{base}-{n}");
            n += 1;
        }
        out.push(Choice {
            id: cid,
            label,
            description,
        });
    }
    if out.len() > CHOICES_MAX {
        return invalid(format!("at most {CHOICES_MAX} choices"));
    }
    Ok(out)
}

/// A validated question shape: kind, choices and limits.
struct Shape {
    kind: String,
    choices: Vec<Choice>,
    allow_other: bool,
    min: Option<i32>,
    max: Option<i32>,
}

fn shape(a: &Ask) -> Result<Shape> {
    let choices = parse_choices(a.choices.as_ref(), &a.options)?;
    let kind = match a.kind.as_deref().map(|k| k.trim().to_lowercase()) {
        Some(k) if !k.is_empty() => k,
        _ if choices.is_empty() => "free".into(),
        _ => "single".into(),
    };
    if !KINDS.contains(&kind.as_str()) {
        return invalid("kind: single, multiple, free or cards");
    }
    if kind == "free" {
        if !choices.is_empty() {
            return invalid("a free question takes no choices: use kind single, multiple or cards");
        }
        if a.min.is_some() || a.max.is_some() {
            return invalid("min and max are for kind multiple");
        }
        return Ok(Shape {
            kind,
            choices,
            allow_other: false,
            min: None,
            max: None,
        });
    }
    let least = if kind == "cards" { 2 } else { 1 };
    if choices.len() < least {
        return invalid(format!(
            "a {kind} question needs at least {least} choice{}",
            if least == 1 { "" } else { "s" }
        ));
    }
    if kind != "multiple" && (a.min.is_some() || a.max.is_some()) {
        return invalid("min and max are for kind multiple");
    }
    let lim = |v: Option<i64>, what: &str| -> Result<Option<i32>> {
        match v {
            None => Ok(None),
            Some(n) if (0..=CHOICES_MAX as i64 + 1).contains(&n) => Ok(Some(n as i32)),
            Some(_) => invalid(format!("{what} is out of range")),
        }
    };
    let (min, max) = (lim(a.min, "min")?, lim(a.max, "max")?);
    if max == Some(0) {
        return invalid("max must be at least 1");
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return invalid("min is more than max");
    }
    let room = choices.len() as i32 + i32::from(a.allow_other);
    if min.is_some_and(|m| m > room) {
        return invalid(format!("min is more than the {room} possible picks"));
    }
    Ok(Shape {
        kind,
        choices,
        allow_other: a.allow_other,
        min,
        max,
    })
}

pub async fn ask(db: &PgPool, p: &Principal, a: Ask) -> Result<Value> {
    let (title, description) = match a.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => (
            t.to_string(),
            a.description.as_deref().unwrap_or("").trim().to_string(),
        ),
        None => match a.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => {
                let (t, mut d) = split_body(b);
                if let Some(extra) = a
                    .description
                    .as_deref()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                {
                    d = if d.is_empty() {
                        extra.to_string()
                    } else {
                        format!("{d}\n\n{extra}")
                    };
                }
                (t, d)
            }
            None => return invalid("the question needs a title"),
        },
    };
    if title.contains('\n') {
        return invalid("the title is one line: put the context in the description");
    }
    if title.chars().count() > TITLE_MAX {
        return invalid(format!(
            "the title is too long ({TITLE_MAX} characters at most): keep it short, put the context in the description"
        ));
    }
    if description.chars().count() > DESCRIPTION_MAX {
        return invalid(format!(
            "the description is too long ({DESCRIPTION_MAX} characters at most)"
        ));
    }
    let body = if description.is_empty() {
        title.clone()
    } else {
        format!("{title}\n\n{description}")
    };
    let sh = shape(&a)?;
    let key = match (&a.project, &a.task) {
        (Some(k), _) => k.trim().to_uppercase(),
        (None, Some(t)) => parse_task_ref(t)?.0,
        (None, None) => return invalid("project (or task) is required"),
    };
    let mut tx = db.begin().await?;
    let pr = project_by_key(&mut tx, p, &key).await?;
    let task = match a.task.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => {
            let t = task_by_ref(&mut tx, p, t).await?;
            if t.project_id != pr.id {
                return invalid(format!("{} is not in {}", t.key, pr.key));
            }
            Some(t)
        }
        None => None,
    };
    let to = match a
        .to
        .as_deref()
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
    {
        None => None,
        Some(name) => Some(
            sqlx::query_scalar::<_, Uuid>(
                "select pr.id from principals pr join team_members tm on tm.principal_id = pr.id
                 where pr.name = $1 and tm.team_id = $2 and pr.kind <> 'relay'",
            )
            .bind(&name)
            .bind(pr.team_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| Error::NotFound(format!("nobody named {name} in {}'s team", pr.key)))?,
        ),
    };
    if let Some(cook) = to
        && crate::cloud::is_fire_and_forget(&mut tx, cook).await?
    {
        let name: String = sqlx::query_scalar("select name from principals where id = $1")
            .bind(cook)
            .fetch_one(&mut *tx)
            .await?;
        return Err(crate::cloud::refusal(&name));
    }
    let mut station = stations::for_claim(&mut tx, p, a.station.as_deref(), pr.id).await?;
    if station.is_none()
        && let Some(t) = &task
    {
        // Asked without `station` about a task this cook holds: the session that holds it asked.
        station = sqlx::query_scalar(
            "select station_id from tasks where id = $1 and claimed_by = $2 and status = 'claimed'",
        )
        .bind(t.id)
        .bind(p.id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
    }
    let mut wait = a.wait;
    if wait {
        let Some(t) = &task else {
            return invalid("wait needs the task you are working on");
        };
        if !p.is_agent() {
            wait = false;
        } else if crate::cloud::is_fire_and_forget(&mut tx, p.id).await? {
            return invalid(format!(
                "{} is fire-and-forget: nobody can wake it when the answer comes. Block the task instead (block, with what a person must decide), or ask without wait",
                p.name
            ));
        } else if t.status != "claimed" || t.claimed_by != Some(p.id) {
            return invalid(format!(
                "wait parks a task you hold: {} is {} {}",
                t.key,
                t.status,
                if t.claimed_by == Some(p.id) {
                    ""
                } else {
                    "and not yours"
                }
            ));
        }
    }
    let options: Vec<String> = sh.choices.iter().map(|c| c.label.clone()).collect();
    let choices_json = Value::Array(sh.choices.iter().map(Choice::json).collect());
    let number: i64 = sqlx::query_scalar(
        "update projects set next_question = next_question + 1 where id = $1 returning next_question - 1",
    )
    .bind(pr.id)
    .fetch_one(&mut *tx)
    .await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into questions (project_id, number, task_id, asked_by, station_id, to_principal, body, title,
           description, kind, choices, allow_other, min_choices, max_choices, options, wait)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) returning id",
    )
    .bind(pr.id)
    .bind(number)
    .bind(task.as_ref().map(|t| t.id))
    .bind(p.id)
    .bind(station)
    .bind(to)
    .bind(&body)
    .bind(&title)
    .bind(&description)
    .bind(&sh.kind)
    .bind(&choices_json)
    .bind(sh.allow_other)
    .bind(sh.min)
    .bind(sh.max)
    .bind(&options)
    .bind(wait)
    .fetch_one(&mut *tx)
    .await?;
    let qref = format!("{}-Q{number}", pr.key);
    if wait && let Some(t) = &task {
        // Park: the task waits for the answer and stays with this cook (and station).
        sqlx::query(
            "update tasks set status = 'blocked', result = $2, reserved_for = $3,
               station_id = coalesce($4, station_id), lease_expires_at = null, updated_at = now()
             where id = $1",
        )
        .bind(t.id)
        .bind(format!("waiting for an answer to {qref}"))
        .bind(p.id)
        .bind(station)
        .execute(&mut *tx)
        .await?;
        emit(
            &mut tx,
            Some(pr.id),
            Some(t.id),
            "task.blocked",
            Some(p.id),
            json!({"reason": format!("waiting for an answer to {qref}"), "question": qref}),
        )
        .await?;
    }
    emit(
        &mut tx,
        Some(pr.id),
        task.as_ref().map(|t| t.id),
        "question.asked",
        Some(p.id),
        json!({"question": qref, "title": short(&title), "kind": sh.kind, "to": a.to}),
    )
    .await?;
    // Asked of a cook: it is woken (its main station, or the session holding the task). A
    // fire-and-forget cook was refused above.
    if let Some(cook) = to
        && cook != p.id
        && sqlx::query_scalar::<_, bool>(
            "select kind = 'agent' and disabled_at is null from principals where id = $1",
        )
        .bind(cook)
        .fetch_one(&mut *tx)
        .await?
    {
        let prompt = ask_prompt(
            &p.name,
            &qref,
            &title,
            &description,
            &sh,
            task.as_ref().map(|t| format!("{}-{}", t.key, t.number)),
        );
        wake(
            &mut tx,
            pr.id,
            &pr.key,
            cook,
            None,
            task.as_ref().map(|t| t.id),
            &prompt,
            "question",
        )
        .await?;
    }
    let out = view(&mut tx, id).await?;
    tx.commit().await?;
    Ok(out)
}

/// What a cook is told when a question is addressed to it: the question, its choices and the call
/// that answers it.
fn ask_prompt(
    asker: &str,
    qref: &str,
    title: &str,
    description: &str,
    sh: &Shape,
    task: Option<String>,
) -> String {
    let about = task.map(|t| format!(" (about {t})")).unwrap_or_default();
    let mut s = format!("{asker} asks you {qref}{about}: {title}\n");
    if !description.is_empty() {
        s.push_str(&format!("\n{description}\n"));
    }
    if !sh.choices.is_empty() {
        s.push_str("\nChoices (answer with their ids):\n");
        for c in &sh.choices {
            s.push_str(&format!("- {} = {}", c.id, c.label));
            if let Some(d) = &c.description {
                s.push_str(&format!(": {d}"));
            }
            s.push('\n');
        }
        s.push_str(if sh.kind == "multiple" {
            "Pick several"
        } else {
            "Pick one"
        });
        s.push_str(if sh.allow_other {
            " or type your own (text).\n"
        } else {
            ".\n"
        });
    }
    s.push_str(&format!(
        "\nAnswer with the `answer_question` tool: id {qref}, {}. Use comment_question {qref} to ask them something back first.",
        if sh.kind == "free" {
            "text (your answer)"
        } else {
            "choices (ids), or text when it allows another answer"
        }
    ));
    s
}

/// The pot's pool: open questions by default (oldest first), or `answered` / `dismissed` / `all`
/// (newest first).
pub async fn list(
    db: &PgPool,
    p: &Principal,
    project: &str,
    status: Option<&str>,
) -> Result<Value> {
    let status = status
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "open".into());
    if !["open", "answered", "dismissed", "all"].contains(&status.as_str()) {
        return invalid("status: open, answered, dismissed or all");
    }
    let mut c = db.acquire().await?;
    let pr = project_by_key(&mut c, p, &project.to_uppercase()).await?;
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id
         where q.project_id = $1 and ($2 = 'all' or q.status = $2)
         order by case when $2 = 'open' then q.created_at end asc, q.created_at desc limit 200"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(pr.id)
        .bind(&status)
        .fetch_all(&mut *c)
        .await?;
    Ok(Value::Array(rows))
}

/// Questions the caller asked (a cook: its own), open ones first.
pub async fn mine(db: &PgPool, p: &Principal, status: Option<&str>) -> Result<Value> {
    let status = status
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| "all".into());
    let sql = format!(
        "select {QUESTION_JSON} from questions q join projects pr on pr.id = q.project_id
         where q.asked_by = $1 and ($2 = 'all' or q.status = $2)
         order by q.status <> 'open', q.created_at desc limit 100"
    );
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(p.id)
        .bind(&status)
        .fetch_all(db)
        .await?;
    Ok(Value::Array(rows))
}

pub async fn get(db: &PgPool, p: &Principal, r: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let q = load(&mut c, p, r, false).await?;
    view(&mut c, q.id).await
}

/// The person a cook relays for: its owner, who must be in the pot's team.
async fn owner_in_team(c: &mut PgConnection, p: &Principal, team: Uuid) -> Result<Uuid> {
    let owner = p
        .owner_id
        .ok_or_else(|| Error::Forbidden(format!("{} has no owner to ask", p.name)))?;
    let member: bool = sqlx::query_scalar(
        "select exists (select 1 from team_members where team_id = $1 and principal_id = $2)",
    )
    .bind(team)
    .bind(owner)
    .fetch_one(c)
    .await?;
    if !member {
        return Err(Error::Forbidden(format!(
            "{}'s owner is not in this pot's team, so {} cannot relay its questions",
            p.name, p.name
        )));
    }
    Ok(owner)
}

/// For a cook in a chat with its owner: take the next open question it may put to its owner
/// (unaddressed or addressed to the owner), or one addressed to the cook itself, and hold it for
/// a few minutes so no other chat asks the same thing.
pub async fn pick(db: &PgPool, p: &Principal, project: Option<&str>) -> Result<Value> {
    if !p.is_agent() {
        return invalid(
            "pick_question is for cooks relaying to their owner; people answer in the console",
        );
    }
    let mut tx = db.begin().await?;
    let pid = match project {
        Some(k) if !k.trim().is_empty() => Some(
            project_by_key(&mut tx, p, &k.trim().to_uppercase())
                .await?
                .id,
        ),
        _ => None,
    };
    let row: Option<Uuid> = sqlx::query_scalar(
        "select q.id from questions q join projects pr on pr.id = q.project_id
         where q.status = 'open' and ($1::uuid is null or q.project_id = $1)
           and (q.picked_by is null or q.picked_by = $2 or q.pick_expires_at < now())
           and (
             q.to_principal = $2
             or ((q.to_principal is null or q.to_principal = $3)
                 and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $3)
                 and exists (select 1 from team_members tm where tm.team_id = pr.team_id and tm.principal_id = $2)))
         order by (q.to_principal = $2) desc nulls last, q.created_at
         for update of q skip locked limit 1",
    )
    .bind(pid)
    .bind(p.id)
    .bind(p.owner_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = row else {
        tx.commit().await?;
        return Ok(json!({"none": true}));
    };
    sqlx::query(
        "update questions set picked_by = $2, pick_expires_at = now() + make_interval(mins => $3::int) where id = $1",
    )
    .bind(id)
    .bind(p.id)
    .bind(PICK_MINUTES as i32)
    .execute(&mut *tx)
    .await?;
    let out = view(&mut tx, id).await?;
    tx.commit().await?;
    Ok(json!({"question": out}))
}

pub async fn release(db: &PgPool, p: &Principal, r: &str) -> Result<Value> {
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, true).await?;
    if q.picked_by != Some(p.id) {
        return Err(Error::Conflict(format!(
            "{} is not picked by {}",
            r.trim(),
            p.name
        )));
    }
    sqlx::query("update questions set picked_by = null, pick_expires_at = null where id = $1")
        .bind(q.id)
        .execute(&mut *tx)
        .await?;
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A question's kind, choices and limits, as stored.
async fn shape_of(
    c: &mut PgConnection,
    id: Uuid,
) -> Result<(String, Vec<Choice>, bool, Option<i32>, Option<i32>)> {
    let (kind, choices, allow_other, min, max): (String, Value, bool, Option<i32>, Option<i32>) =
        sqlx::query_as(
            "select kind, choices, allow_other, min_choices, max_choices from questions where id = $1",
        )
        .bind(id)
        .fetch_one(c)
        .await?;
    let choices = choices
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Choice {
            id: c["id"].as_str().unwrap_or_default().to_string(),
            label: c["label"].as_str().unwrap_or_default().to_string(),
            description: c["description"].as_str().map(String::from),
        })
        .collect();
    Ok((kind, choices, allow_other, min, max))
}

/// What an answerer sends: chosen choices (ids or labels), typed text, or the older plain string.
#[derive(Default)]
pub struct Reply {
    pub choices: Option<Value>,
    pub text: Option<String>,
    pub answer: Option<String>,
}

impl Reply {
    pub fn plain(s: &str) -> Reply {
        Reply {
            answer: Some(s.to_string()),
            ..Reply::default()
        }
    }
}

/// A validated answer: the chosen ids (in the question's order), the typed text, and the readable
/// rendering stored in `answer`.
#[derive(Debug, PartialEq)]
pub struct Given {
    pub choices: Vec<String>,
    pub text: Option<String>,
    pub rendered: String,
}

/// Check an answer against the question's kind, choices and limits.
#[allow(clippy::too_many_arguments)]
pub fn check_answer(
    kind: &str,
    choices: &[Choice],
    allow_other: bool,
    min: Option<i32>,
    max: Option<i32>,
    reply: &Reply,
) -> Result<Given> {
    let mut text = reply
        .text
        .as_deref()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from);
    let mut wanted: Vec<String> = match &reply.choices {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.starts_with('[') {
                match serde_json::from_str::<Value>(s) {
                    Ok(Value::Array(a)) => a
                        .iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .collect(),
                    _ => return invalid("choices: expected a JSON array of choice ids or labels"),
                }
            } else {
                s.lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect()
            }
        }
        Some(_) => return invalid("choices: expected a JSON array of choice ids or labels"),
    };
    // the older plain answer: a choice label when it equals one, else the typed text (taken as
    // "other" whatever the question allows: older cooks and chats answer in free words)
    let mut allow_other = allow_other;
    if let Some(plain) = reply
        .answer
        .as_deref()
        .map(str::trim)
        .filter(|x| !x.is_empty())
        && wanted.is_empty()
        && text.is_none()
    {
        let lower = plain.to_lowercase();
        match choices.iter().find(|c| c.label.to_lowercase() == lower) {
            Some(c) if kind != "free" => wanted.push(c.id.clone()),
            _ => {
                text = Some(plain.to_string());
                allow_other = true;
            }
        }
    }
    if kind == "free" && !wanted.is_empty() {
        return invalid("this question takes text, not choices");
    }
    let mut ids: Vec<String> = vec![];
    for w in &wanted {
        let lower = w.to_lowercase();
        let c = choices
            .iter()
            .find(|c| c.id == *w)
            .or_else(|| choices.iter().find(|c| c.label.to_lowercase() == lower));
        match c {
            Some(c) => {
                if !ids.contains(&c.id) {
                    ids.push(c.id.clone());
                }
            }
            None => {
                return invalid(format!(
                    "{w:?} is not one of the choices ({})",
                    choices
                        .iter()
                        .map(|c| format!("{} = {}", c.id, c.label))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
        }
    }
    // keep the question's own order
    ids.sort_by_key(|i| choices.iter().position(|c| c.id == *i));
    match kind {
        "free" => {
            if !ids.is_empty() {
                return invalid("this question takes text, not choices");
            }
            if text.is_none() {
                return invalid("the answer is empty: this question takes text");
            }
        }
        "single" | "cards" => {
            if ids.len() > 1 {
                return invalid("choose one");
            }
            if !ids.is_empty() && text.is_some() {
                return invalid("choose one of the choices or type your own answer, not both");
            }
            if ids.is_empty() && text.is_none() {
                return invalid("choose one of the choices");
            }
            if ids.is_empty() && !allow_other {
                return invalid("this question takes one of the choices, not text");
            }
        }
        _ => {
            if text.is_some() && !allow_other {
                return invalid("this question takes choices, not text");
            }
            let n = ids.len() as i32 + i32::from(text.is_some());
            if n == 0 {
                return invalid("choose at least one");
            }
            if let Some(lo) = min
                && n < lo
            {
                return invalid(format!("choose at least {lo}"));
            }
            if let Some(hi) = max
                && n > hi
            {
                return invalid(format!("choose at most {hi}"));
            }
        }
    }
    let labels: Vec<&str> = ids
        .iter()
        .filter_map(|i| {
            choices
                .iter()
                .find(|c| c.id == *i)
                .map(|c| c.label.as_str())
        })
        .collect();
    let rendered = match (&text, labels.is_empty()) {
        (Some(t), true) => t.clone(),
        (Some(t), false) => format!("{} — plus: {t}", labels.join(", ")),
        (None, _) => labels.join(", "),
    };
    Ok(Given {
        choices: ids,
        text,
        rendered,
    })
}

/// Answer (or dismiss) a question. People answer as themselves. A cook answers a question
/// addressed to it as itself; any other question it can only relay from its owner (recorded as
/// answered by the owner, relayed by the cook). Dismissing takes the reason as the plain answer.
pub async fn answer(
    db: &PgPool,
    p: &Principal,
    r: &str,
    reply: Reply,
    dismiss: bool,
) -> Result<Value> {
    if !dismiss {
        let none = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
        if none(&reply.answer)
            && none(&reply.text)
            && matches!(&reply.choices, None | Some(Value::Null))
        {
            return invalid("the answer is empty");
        }
    }
    let dismiss_reason = reply.answer.as_deref().map(str::trim).unwrap_or("");
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, true).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    if q.status != "open" {
        return Err(Error::Conflict(format!("{qref} is already {}", q.status)));
    }
    // People answer as themselves, and so do the asker (dismissing) and a cook a question is
    // addressed to; any other cook can only relay its owner's answer.
    let as_self =
        p.is_user() || (dismiss && q.asked_by == Some(p.id)) || q.to_principal == Some(p.id);
    let (answered_by, relayed_by) = if as_self {
        (p.id, None)
    } else if p.is_agent() {
        let owner = owner_in_team(&mut tx, p, q.team_id).await?;
        if q.to_principal.is_some_and(|to| to != owner) {
            return Err(Error::Forbidden(format!(
                "{qref} is addressed to someone else"
            )));
        }
        if q.picked_by.is_some_and(|by| by != p.id)
            && q.pick_expires_at.is_some_and(|t| t > Utc::now())
        {
            return Err(Error::Conflict(format!(
                "{qref} is being asked in another chat right now"
            )));
        }
        (owner, Some(p.id))
    } else {
        return Err(Error::Forbidden("relays cannot answer questions".into()));
    };
    let status = if dismiss { "dismissed" } else { "answered" };
    let mut free = true;
    let given = if dismiss {
        Given {
            choices: vec![],
            text: None,
            rendered: dismiss_reason.to_string(),
        }
    } else {
        let (kind, choices, allow_other, min, max) = shape_of(&mut tx, q.id).await?;
        free = kind == "free";
        check_answer(&kind, &choices, allow_other, min, max, &reply)?
    };
    let stored: Option<&str> = Some(given.rendered.as_str()).filter(|s| !s.is_empty());
    let text = given.rendered.as_str();
    sqlx::query(
        "update questions set status = $2, answer = $3, answer_choices = $6, answer_text = $7,
           answered_by = $4, relayed_by = $5, answered_at = now(),
           picked_by = null, pick_expires_at = null where id = $1",
    )
    .bind(q.id)
    .bind(status)
    .bind(stored)
    .bind(answered_by)
    .bind(relayed_by)
    .bind(&given.choices)
    .bind(&given.text)
    .execute(&mut *tx)
    .await?;
    let names: (String, Option<String>) = sqlx::query_as(
        "select (select name from principals where id = $1), (select name from principals where id = $2)",
    )
    .bind(answered_by)
    .bind(relayed_by)
    .fetch_one(&mut *tx)
    .await?;
    let who = match &names.1 {
        Some(via) => format!("{}, via {via}", names.0),
        None => names.0.clone(),
    };
    emit(
        &mut tx,
        Some(q.project_id),
        q.task_id,
        if dismiss {
            "question.dismissed"
        } else {
            "question.answered"
        },
        Some(p.id),
        json!({"question": qref, "answer": stored.map(short), "relayed_by": names.1}),
    )
    .await?;

    // Hand a parked task back to the cook that asked, with a fresh lease.
    let mut resumed: Option<(String, i64)> = None;
    if q.wait
        && let (Some(tid), Some(asker)) = (q.task_id, q.asked_by)
    {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "update tasks t set status = 'claimed', claimed_by = $2, reserved_for = null, result = null,
               lease_epoch = t.lease_epoch + 1, updated_at = now(),
               lease_expires_at = now() + (select lease_ttl from projects where id = t.project_id)
             where t.id = $1 and t.status = 'blocked' and t.reserved_for = $2
             returning t.lease_epoch, t.number",
        )
        .bind(tid)
        .bind(asker)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((epoch, number)) = row {
            let asker_name: String =
                sqlx::query_scalar("select name from principals where id = $1")
                    .bind(asker)
                    .fetch_one(&mut *tx)
                    .await?;
            emit(
                &mut tx,
                Some(q.project_id),
                Some(tid),
                "task.claimed",
                Some(asker),
                json!({"agent": asker_name, "epoch": epoch, "reason": format!("{qref} {status}")}),
            )
            .await?;
            resumed = Some((format!("{}-{number}", q.key), epoch));
        }
    }

    // Wake the asking cook (its very station when it asked from one) with the answer.
    if let Some(asker) = q.asked_by
        && asker != p.id
    {
        let is_agent: bool =
            sqlx::query_scalar("select kind = 'agent' from principals where id = $1")
                .bind(asker)
                .fetch_one(&mut *tx)
                .await?;
        if is_agent {
            let mut prompt = if dismiss {
                format!(
                    "Your question {qref} was dismissed by {who}{}.",
                    stored.map(|s| format!(": {s}")).unwrap_or_default()
                )
            } else {
                let mut s = format!("Your question {qref} was answered by {who}:\n\n{text}\n");
                if !given.choices.is_empty() {
                    s.push_str(&format!("\nChosen (ids): {}\n", given.choices.join(", ")));
                }
                if let Some(other) = given.text.as_deref().filter(|_| !free) {
                    s.push_str(&format!("Other (typed): {other}\n"));
                }
                s
            };
            if let Some((task, epoch)) = &resumed {
                prompt.push_str(&format!(
                    "\nYou hold {task} again with lease epoch {epoch}: continue it (pass that epoch to check_item, complete, ...)."
                ));
            }
            wake(
                &mut tx,
                q.project_id,
                &q.key,
                asker,
                q.station_id,
                q.task_id,
                &prompt,
                "answer",
            )
            .await?;
        }
    }
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A follow-up in a question's thread. A person's note to an open question wakes the cook that
/// asked, so it can clarify.
pub async fn note(db: &PgPool, p: &Principal, r: &str, body: &str) -> Result<Value> {
    let body = body.trim();
    if body.is_empty() {
        return invalid("the note is empty");
    }
    let mut tx = db.begin().await?;
    let q = load(&mut tx, p, r, false).await?;
    let qref = format!("{}-Q{}", q.key, q.number);
    sqlx::query("insert into question_notes (question_id, author_id, body) values ($1,$2,$3)")
        .bind(q.id)
        .bind(p.id)
        .bind(body)
        .execute(&mut *tx)
        .await?;
    emit(
        &mut tx,
        Some(q.project_id),
        q.task_id,
        "question.noted",
        Some(p.id),
        json!({"question": qref, "title": short(body)}),
    )
    .await?;
    if q.status == "open"
        && let Some(asker) = q.asked_by
        && asker != p.id
    {
        let is_agent: bool =
            sqlx::query_scalar("select kind = 'agent' from principals where id = $1")
                .bind(asker)
                .fetch_one(&mut *tx)
                .await?;
        if is_agent {
            let prompt = format!(
                "{} replied on your question {qref}:\n\n{body}\n\nAnswer with comment_question {qref} if they need more from you.",
                p.name
            );
            wake(
                &mut tx,
                q.project_id,
                &q.key,
                asker,
                q.station_id,
                q.task_id,
                &prompt,
                "note",
            )
            .await?;
        }
    }
    let out = view(&mut tx, q.id).await?;
    tx.commit().await?;
    Ok(out)
}

/// A direct wake for one cook (and station), carrying its own prompt text. A fire-and-forget cook
/// (see [`crate::cloud::fire_and_forget_sql`]) is never woken this way: see [`wake_from`].
pub async fn wake(
    c: &mut PgConnection,
    project: Uuid,
    key: &str,
    agent: Uuid,
    station: Option<Uuid>,
    task: Option<Uuid>,
    text: &str,
    reason: &str,
) -> Result<Value> {
    wake_from(c, project, key, agent, station, task, text, reason, None).await
}

/// [`wake`] for a message sent by `from` (a principal id). The choke point of every wake: a
/// fire-and-forget cook is woken only by a direct message on the pass (reason `pass`) whose sender
/// is its own owner, and for anything else nothing is emitted (no wake, no usage record) and the
/// reply says `woken: false`.
pub async fn wake_from(
    c: &mut PgConnection,
    project: Uuid,
    key: &str,
    agent: Uuid,
    station: Option<Uuid>,
    task: Option<Uuid>,
    text: &str,
    reason: &str,
    from: Option<Uuid>,
) -> Result<Value> {
    let (name, owner): (String, Option<Uuid>) =
        sqlx::query_as("select name, owner_id from principals where id = $1")
            .bind(agent)
            .fetch_one(&mut *c)
            .await?;
    let cloud = crate::cloud::is_fire_and_forget(&mut *c, agent).await?;
    if cloud && !(reason == "pass" && owner.is_some() && from == owner) {
        return Ok(json!({"cook": name, "woken": false, "cloud": true}));
    }
    // its owner's message starts a run only where a cloud target serves the pot
    if cloud {
        let covered: bool = sqlx::query_scalar(
            "select exists (select 1 from wake_targets where agent_id = $1
               and (projects is null or cardinality(projects) = 0 or $2 = any(projects)))",
        )
        .bind(agent)
        .bind(key)
        .fetch_one(&mut *c)
        .await?;
        if !covered {
            return Ok(json!({"cook": name, "woken": false, "cloud": true, "uncovered": true}));
        }
    }
    let task_ref: Option<String> = match task {
        Some(t) => sqlx::query_scalar(
            "select pr.key || '-' || t.number from tasks t join projects pr on pr.id = t.project_id where t.id = $1",
        )
        .bind(t)
        .fetch_optional(&mut *c)
        .await?,
        None => None,
    };
    let station = match station {
        Some(s) => Some(s),
        None => infer_station(&mut *c, project, agent, task).await?,
    };
    let st = match station {
        Some(s) => stations::payload_of(&mut *c, s).await?,
        None => None,
    };
    let news = crate::pantry::news(&mut *c, agent, project).await?;
    let text = match news["count"].as_u64().unwrap_or(0) {
        0 => text.to_string(),
        n => format!(
            "{text}\n\n(Pantry: {n} change{} since you last read it: {}. Read with get_memory or read_pantry; removed ones no longer apply.)",
            if n == 1 { "" } else { "s" },
            news["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    let k = i["key"].as_str()?;
                    Some(if i["deleted"] == true {
                        format!("{k} (removed)")
                    } else {
                        k.to_string()
                    })
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let hint = crate::what_now::wake_hint(&mut *c, project, key).await;
    let text = if hint.is_empty() {
        text
    } else {
        format!("{text}\n\n{hint}")
    };
    let text = text.as_str();
    // A cook can run several sessions: tell this one who it is, so its answer says so.
    let text = match st.as_ref().and_then(|s| s["id"].as_str()) {
        Some(id) => format!(
            "{text}\n\n(You are station {id}: pass `station: \"{id}\"` to pass, comment, ask and claim_next, so people see which session answers.)"
        ),
        None => text.to_string(),
    };
    crate::usage::record_in(
        c,
        agent,
        Some(project),
        crate::usage::WAKE,
        reason,
        text.chars().count(),
    )
    .await;
    emit(
        c,
        Some(project),
        None,
        "wake",
        None,
        json!({"agent": name, "agent_id": agent, "project": key, "station": st, "text": text, "reason": reason,
               "task": task_ref}),
    )
    .await?;
    // who was woken, for the record: the cook, and the session when there is one
    let mut out = json!({
        "cook": name,
        "station": st.as_ref().and_then(|s| s["id"].as_str()),
        "label": st.as_ref().and_then(|s| s["label"].as_str().or(s["directory"].as_str())),
    });
    if cloud {
        out["cloud"] = json!(true);
    }
    Ok(out)
}

/// The station to wake when none is named (answers, notes, pass messages and mentions): the
/// station that holds or is parked on the task in question (it has the context), else the cook's
/// main station in this pot, else its station woken last, else none.
async fn infer_station(
    c: &mut PgConnection,
    project: Uuid,
    agent: Uuid,
    task: Option<Uuid>,
) -> Result<Option<Uuid>> {
    if let Some(t) = task {
        let on_task: Option<Uuid> = sqlx::query_scalar(
            "select t.station_id from tasks t join stations s on s.id = t.station_id
             where t.id = $1 and s.agent_id = $2",
        )
        .bind(t)
        .bind(agent)
        .fetch_optional(&mut *c)
        .await?
        .flatten();
        if on_task.is_some() {
            return Ok(on_task);
        }
    }
    // the cook's main station in the pot, else the one woken last
    Ok(sqlx::query_scalar(
        "select id from stations where agent_id = $1 and project_id = $2
         order by main desc, last_woken_at desc nulls last, created_at desc limit 1",
    )
    .bind(agent)
    .bind(project)
    .fetch_optional(&mut *c)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::{parse_ref, short};

    #[test]
    fn titles_are_plain() {
        assert_eq!(
            short("**Round** `down` or _up_?\nmore"),
            "Round down or up?"
        );
        assert_eq!(short("\n## Which currency?"), "Which currency?");
    }

    #[test]
    fn question_refs() {
        assert_eq!(parse_ref("PAY-Q3").unwrap(), ("PAY".into(), 3));
        assert_eq!(parse_ref("ai-helm-q12").unwrap(), ("AI-HELM".into(), 12));
        assert!(parse_ref("PAY-3").is_err());
        assert!(parse_ref("-Q3").is_err());
    }
}
