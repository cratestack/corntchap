//! Admin side: users, teams, members, projects, agents, relays, tokens, wake targets.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    auth::{generate_token, sha256},
    error::{Error, Result, invalid},
    state::Principal,
};

pub fn norm_name(s: &str) -> Result<String> {
    let n = s.trim().to_lowercase();
    if n.is_empty()
        || n.len() > 64
        || !n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return invalid("names use letters, digits, '-', '_' and '.' (max 64)");
    }
    Ok(n)
}

/// Names the pass gives a meaning to (`@all` reaches everyone) or that read as an address to a
/// crowd; no person's handle may be one of them.
pub const RESERVED_HANDLES: &[&str] = &["all", "everyone", "here", "channel", "me", "system"];

/// A person's handle: a valid name that is not reserved. Letters are lowercased.
pub fn norm_handle(s: &str) -> Result<String> {
    let n = norm_name(s)?;
    if RESERVED_HANDLES.contains(&n.as_str()) {
        return invalid(format!("{n} is reserved: pick another handle"));
    }
    Ok(n)
}

const ADJECTIVES: &[&str] = &[
    "saffron",
    "golden",
    "smoky",
    "spicy",
    "sweet",
    "salty",
    "crispy",
    "creamy",
    "tender",
    "zesty",
    "peppery",
    "toasted",
    "roasted",
    "braised",
    "grilled",
    "steamy",
    "juicy",
    "fragrant",
    "tangy",
    "mellow",
    "bold",
    "rustic",
    "humble",
    "merry",
    "ripe",
    "fresh",
    "wild",
    "amber",
    "ochre",
    "scarlet",
    "crimson",
    "emerald",
    "indigo",
    "copper",
    "velvet",
    "sunny",
    "cosy",
    "lively",
    "nimble",
    "gentle",
    "bright",
    "hearty",
    "savory",
    "buttery",
    "nutty",
    "honeyed",
    "charred",
    "simmering",
    "sizzling",
    "pickled",
    "cured",
    "fermented",
    "whipped",
    "folded",
    "stuffed",
    "glazed",
    "dusted",
    "candied",
    "minted",
    "herbal",
    "citrus",
    "cassava",
];

const DISHES: &[&str] = &[
    "okra",
    "plantain",
    "cassava",
    "yam",
    "jollof",
    "ndole",
    "fufu",
    "suya",
    "egusi",
    "kelewele",
    "injera",
    "berbere",
    "tagine",
    "couscous",
    "harissa",
    "peanut",
    "millet",
    "sorghum",
    "taro",
    "mango",
    "papaya",
    "guava",
    "baobab",
    "hibiscus",
    "ginger",
    "turmeric",
    "cumin",
    "cardamom",
    "pepper",
    "tomato",
    "onion",
    "garlic",
    "lentil",
    "chickpea",
    "bean",
    "rice",
    "maize",
    "corn",
    "stew",
    "broth",
    "soup",
    "skillet",
    "ladle",
    "whisk",
    "cleaver",
    "mortar",
    "pestle",
    "colander",
    "griddle",
    "kettle",
    "casserole",
    "wok",
    "cauldron",
    "spatula",
    "tongs",
    "apron",
    "sieve",
    "ramekin",
    "pantry",
    "hearth",
    "oven",
];

/// A random kitchen handle such as `saffron-okra-42`: two words and a 2-3 digit number (a 6 digit one
/// when `long`). `pick(n)` returns a number below `n`; it is injectable so tests can force collisions.
/// A source of randomness: `pick(n)` returns a number below `n`.
pub type Pick<'a> = &'a mut (dyn FnMut(usize) -> usize + Send);

pub fn generate_handle(pick: Pick<'_>, long: bool) -> String {
    let adjective = ADJECTIVES[pick(ADJECTIVES.len()) % ADJECTIVES.len()];
    let dish = DISHES[pick(DISHES.len()) % DISHES.len()];
    let number = if long {
        100_000 + pick(900_000) % 900_000
    } else {
        10 + pick(990) % 990
    };
    format!("{adjective}-{dish}-{number}")
}

/// Production randomness for [`generate_handle`].
pub fn random_pick(n: usize) -> usize {
    rand::random_range(0..n)
}

/// Draws before the suffix grows from 2-3 to 6 digits, and draws overall.
const SHORT_ATTEMPTS: usize = 8;
const MAX_ATTEMPTS: usize = 16;

/// Insert a user. With `wanted` that exact handle (taken: 409); without, random kitchen handles until one is
/// free. When `sub` is set and its row appears meanwhile (a concurrent first sign-in of the same person) that
/// row is returned instead, so one `sub` is always one principal. `on conflict do nothing` keeps a collision
/// from aborting the surrounding transaction.
async fn insert_user(
    c: &mut PgConnection,
    wanted: Option<String>,
    sub: Option<&str>,
    display_name: Option<&str>,
    is_admin: bool,
    source: &str,
    pick: Pick<'_>,
) -> Result<(Uuid, String)> {
    for attempt in 0..MAX_ATTEMPTS {
        let handle = match &wanted {
            Some(w) => w.clone(),
            None => generate_handle(pick, attempt >= SHORT_ATTEMPTS),
        };
        let id: Option<Uuid> = sqlx::query_scalar(
            "insert into principals (id, kind, name, oidc_sub, display_name, is_admin, admin_source)
             values ($1,'user',$2,$3,$4,$5,$6) on conflict do nothing returning id",
        )
        .bind(Uuid::new_v4())
        .bind(&handle)
        .bind(sub)
        .bind(display_name)
        .bind(is_admin)
        .bind(source)
        .fetch_optional(&mut *c)
        .await?;
        if let Some(id) = id {
            return Ok((id, handle));
        }
        if let Some(sub) = sub
            && let Some(row) = sqlx::query_as::<_, (Uuid, String)>(
                "select id, name from principals where oidc_sub = $1",
            )
            .bind(sub)
            .fetch_optional(&mut *c)
            .await?
        {
            return Ok(row);
        }
        if wanted.is_some() {
            return Err(Error::Conflict(format!(
                "the handle {handle} is already taken"
            )));
        }
    }
    Err(Error::Internal(
        "could not find a free handle: too many collisions".into(),
    ))
}

/// First sign-in of an unknown `sub`: create the person (just-in-time) with a random kitchen handle.
pub async fn provision_user(
    db: &PgPool,
    sub: &str,
    display_name: Option<&str>,
    pick: Pick<'_>,
) -> Result<Uuid> {
    let mut c = db.acquire().await?;
    let (id, _) = insert_user(&mut c, None, Some(sub), display_name, false, "oidc", pick).await?;
    Ok(id)
}

/// Longest display name kept, in characters.
pub const MAX_DISPLAY_NAME: usize = 120;

/// A claim from the ID token made safe to store and show: control and invisible direction characters
/// removed, surrounding space trimmed, cut to [`MAX_DISPLAY_NAME`] characters. Empty becomes `None`.
pub fn clean_display_name(raw: &str) -> Option<String> {
    let kept: String = raw
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    let cut: String = kept.trim().chars().take(MAX_DISPLAY_NAME).collect();
    let cut = cut.trim();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// Change your own handle. Only a person can (agents and relays are named by their owner), and a
/// disabled person cannot. Same rules as any handle; a taken one is a 409.
pub async fn rename_self(db: &PgPool, p: &Principal, new_name: &str) -> Result<Value> {
    if !p.is_user() {
        return Err(Error::Forbidden(
            "only a person can rename themselves; a cook or a fire keeps the name its owner gave it".into(),
        ));
    }
    let new = norm_handle(new_name)?;
    let mut tx = db.begin().await?;
    let old: Option<String> = sqlx::query_scalar(
        "select name from principals where id = $1 and kind = 'user' and disabled_at is null for update",
    )
    .bind(p.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(old) = old else {
        return Err(Error::Forbidden("this account is disabled".into()));
    };
    if old == new {
        return Ok(json!({"name": new, "previous": old}));
    }
    let taken = || Error::Conflict(format!("the handle {new} is already taken"));
    let done = sqlx::query("update principals set name = $2 where id = $1")
        .bind(p.id)
        .bind(&new)
        .execute(&mut *tx)
        .await;
    match done {
        Ok(_) => {}
        Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => {
            return Err(taken());
        }
        Err(e) => return Err(e.into()),
    }
    crate::service::emit(
        &mut tx,
        None,
        None,
        "principal.renamed",
        Some(p.id),
        json!({"from": old, "to": new}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"name": new, "previous": old}))
}

/// Make a person an admin, or not (admins only). A hand-made admin is never demoted by the IdP's role
/// sync (`manual`); the last admin cannot be demoted.
pub async fn set_admin(db: &PgPool, p: &Principal, handle: &str, on: bool) -> Result<Value> {
    if !p.is_user() || !p.is_admin {
        return Err(Error::Forbidden(
            "only an admin can change who is an admin".into(),
        ));
    }
    let handle = handle.trim().trim_start_matches('@').to_lowercase();
    let mut tx = db.begin().await?;
    // Lock every current admin row so two concurrent demotions cannot leave none.
    let admins: Vec<Uuid> = sqlx::query_scalar(
        "select id from principals where kind = 'user' and is_admin and disabled_at is null for update",
    )
    .fetch_all(&mut *tx)
    .await?;
    let target: Option<Uuid> = sqlx::query_scalar(
        "select id from principals where name = $1 and kind = 'user' and disabled_at is null",
    )
    .bind(&handle)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(target) = target else {
        return Err(Error::NotFound(format!("no user with the handle {handle}")));
    };
    if !on && admins.iter().all(|a| *a == target) {
        return Err(Error::Conflict(
            "that is the last admin: make someone else an admin first".into(),
        ));
    }
    sqlx::query(
        "update principals set is_admin = $2, admin_source = case when $2 then 'manual' else admin_source end where id = $1",
    )
    .bind(target)
    .bind(on)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(json!({"name": handle, "is_admin": on}))
}

pub async fn mint_token(
    c: &mut PgConnection,
    principal: Uuid,
    label: &str,
    ttl_secs: Option<i64>,
) -> Result<String> {
    let token = generate_token();
    sqlx::query(
        "insert into tokens (principal_id, token_hash, label, expires_at)
         values ($1, $2, $3, case when $4::bigint is null then null else now() + make_interval(secs => $4::float8) end)",
    )
    .bind(principal)
    .bind(sha256(token.as_bytes()))
    .bind(label)
    .bind(ttl_secs)
    .execute(c)
    .await?;
    Ok(token)
}

/// Who an administrative call acts as: a person acts as themselves; a cook granted `manage` acts
/// as its owner (with the owner's admin powers only if it is also granted `admin`). Anything
/// else is refused with how to grant it.
pub async fn acting(db: &PgPool, p: &Principal) -> Result<Principal> {
    if p.is_user() {
        return Ok(p.clone());
    }
    if p.is_agent() && p.granted("manage") {
        return owner_of(db, p).await;
    }
    Err(Error::Forbidden(format!(
        "{} may not do that: a person does it, or its owner grants it `manage` (corntchap-cli cook grant {} manage)",
        p.name, p.name
    )))
}

/// For listings: the owner's view for a cook granted `manage`, otherwise the caller's own.
pub async fn acting_or_self(db: &PgPool, p: &Principal) -> Result<Principal> {
    if p.is_agent() && p.granted("manage") {
        return owner_of(db, p).await;
    }
    Ok(p.clone())
}

async fn owner_of(db: &PgPool, p: &Principal) -> Result<Principal> {
    let owner = p
        .owner_id
        .ok_or_else(|| Error::Forbidden(format!("{} has no owner to act for", p.name)))?;
    let row: Option<(Uuid, String, bool, i64)> = sqlx::query_as(
        "select id, name, is_admin, max_leases from principals where id = $1 and kind = 'user' and disabled_at is null",
    )
    .bind(owner)
    .fetch_optional(db)
    .await?;
    let (id, name, is_admin, max_leases) =
        row.ok_or_else(|| Error::Forbidden(format!("{}'s owner is disabled", p.name)))?;
    Ok(Principal {
        id,
        kind: "user".into(),
        name,
        owner_id: None,
        is_admin: is_admin && p.granted("admin"),
        max_leases,
        runtime: None,
        agent_kind: None,
        token_id: None,
        grants: vec![],
    })
}

pub const GRANTS: &[&str] = &["plan", "manage", "admin"];

/// Set what a cook may do beyond working its tasks. Only a person (the cook's owner or an admin)
/// can, never a cook, even one granted `manage`: a cook cannot raise its own rights.
pub async fn set_grants(
    db: &PgPool,
    p: &Principal,
    name: &str,
    grants: &[String],
) -> Result<Value> {
    if !p.is_user() {
        return Err(Error::Forbidden(
            "only a person can change what a cook may do".into(),
        ));
    }
    let mut g: Vec<String> = grants
        .iter()
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty() && x != "none")
        .collect();
    g.sort();
    g.dedup();
    if let Some(bad) = g.iter().find(|x| !GRANTS.contains(&x.as_str())) {
        return invalid(format!("unknown grant {bad:?}: plan, manage or admin"));
    }
    let mut c = db.acquire().await?;
    let (id, kind) = managed(&mut c, p, name).await?;
    if kind != "agent" {
        return invalid("grants are for cooks (agents)");
    }
    sqlx::query("update principals set grants = $2 where id = $1")
        .bind(id)
        .bind(&g)
        .execute(&mut *c)
        .await?;
    Ok(json!({"name": name.to_lowercase(), "grants": g}))
}

pub async fn list_users(db: &PgPool, p: &Principal) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    if !p.is_admin {
        return Err(Error::Forbidden("only an admin can list users".into()));
    }
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', name, 'display_name', display_name, 'is_admin', is_admin,
            'linked', oidc_sub is not null, 'disabled', disabled_at is not null) from principals where kind = 'user' order by name",
    )
    .fetch_all(db)
    .await?;
    Ok(Value::Array(rows))
}

pub async fn create_team(db: &PgPool, p: &Principal, name: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    if !p.is_admin {
        return Err(Error::Forbidden("only an admin can create teams".into()));
    }
    let name = norm_name(name)?;
    let mut tx = db.begin().await?;
    let id = Uuid::new_v4();
    sqlx::query("insert into teams (id, name) values ($1,$2)")
        .bind(id)
        .bind(&name)
        .execute(&mut *tx)
        .await?;
    if p.is_user() {
        sqlx::query(
            "insert into team_members (team_id, principal_id, role) values ($1,$2,'owner')",
        )
        .bind(id)
        .bind(p.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"id": id, "name": name}))
}

pub async fn list_teams(db: &PgPool, p: &Principal) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', t.name, 'role', (select role from team_members where team_id = t.id and principal_id = $2),
            'members', (select count(*) from team_members where team_id = t.id),
            'projects', coalesce((select jsonb_agg(key order by key) from projects where team_id = t.id), '[]'::jsonb))
         from teams t where $1 or exists (select 1 from team_members m where m.team_id = t.id and m.principal_id = $2) order by t.name",
    )
    .bind(p.is_admin)
    .bind(p.id)
    .fetch_all(db)
    .await?;
    Ok(Value::Array(rows))
}

/// Resolve a team the principal owns (or any team for admins).
async fn owned_team(c: &mut PgConnection, p: &Principal, team: &str) -> Result<Uuid> {
    let row: Option<(Uuid, bool, bool)> = sqlx::query_as(
        "select t.id, exists (select 1 from team_members where team_id = t.id and principal_id = $2 and role = 'owner'),
                exists (select 1 from team_members where team_id = t.id and principal_id = $2)
         from teams t where t.name = $1",
    )
    .bind(team)
    .bind(p.id)
    .fetch_optional(c)
    .await?;
    match row {
        None => Err(Error::NotFound(format!("team {team} not found"))),
        Some((id, owner, member)) => {
            if p.is_admin || (owner && p.is_user()) {
                Ok(id)
            } else if member {
                Err(Error::Forbidden("only a team owner can do that".into()))
            } else {
                Err(Error::NotFound(format!("team {team} not found")))
            }
        }
    }
}

pub async fn add_member(
    db: &PgPool,
    p: &Principal,
    team: &str,
    principal: &str,
    role: &str,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    // A team's chefs: "chef" is stored as `member`.
    let role = match role {
        "chef" | "member" => "member",
        "owner" => "owner",
        _ => return invalid("role must be chef or owner"),
    };
    let mut c = db.acquire().await?;
    let tid = owned_team(&mut c, p, team).await?;
    let target: Option<(Uuid, String, Option<Uuid>)> =
        sqlx::query_as("select id, kind, owner_id from principals where name = $1 and kind <> 'relay' and disabled_at is null")
            .bind(principal.to_lowercase())
            .fetch_optional(&mut *c)
            .await?;
    let Some((pid, kind, owner)) = target else {
        return Err(Error::NotFound(format!(
            "no user or agent named {principal}"
        )));
    };
    // An agent joins a team only with its owner's say-so (otherwise a foreign team could put work on their machine).
    if kind == "agent" && !(p.is_admin || owner == Some(p.id)) {
        return Err(Error::Forbidden(
            "only the agent's owner (or an admin) can add it to a team".into(),
        ));
    }
    if role == "owner" && kind != "user" {
        return invalid("only users can own a team");
    }
    sqlx::query(
        "insert into team_members (team_id, principal_id, role) values ($1,$2,$3)
         on conflict (team_id, principal_id) do update set role = excluded.role",
    )
    .bind(tid)
    .bind(pid)
    .bind(role)
    .execute(&mut *c)
    .await?;
    Ok(json!({"team": team, "principal": principal.to_lowercase(), "role": role}))
}

pub async fn remove_member(
    db: &PgPool,
    p: &Principal,
    team: &str,
    principal: &str,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut c = db.acquire().await?;
    let tid = owned_team(&mut c, p, team).await?;
    sqlx::query("delete from team_members where team_id = $1 and principal_id = (select id from principals where name = $2)")
        .bind(tid)
        .bind(principal.to_lowercase())
        .execute(&mut *c)
        .await?;
    Ok(json!({"ok": true}))
}

pub async fn list_members(db: &PgPool, p: &Principal, team: &str) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    let mut c = db.acquire().await?;
    let row: Option<Uuid> = sqlx::query_scalar(
        "select t.id from teams t where t.name = $1 and ($2 or exists (select 1 from team_members m where m.team_id = t.id and m.principal_id = $3))",
    )
    .bind(team)
    .bind(p.is_admin)
    .bind(p.id)
    .fetch_optional(&mut *c)
    .await?;
    let tid = row.ok_or_else(|| Error::NotFound(format!("team {team} not found")))?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', pr.name, 'display_name', pr.display_name, 'kind', pr.kind, 'role', m.role) from team_members m
         join principals pr on pr.id = m.principal_id where m.team_id = $1 order by pr.name",
    )
    .bind(tid)
    .fetch_all(&mut *c)
    .await?;
    Ok(Value::Array(rows))
}

pub async fn create_project(
    db: &PgPool,
    p: &Principal,
    team: &str,
    key: &str,
    name: &str,
    repo_url: Option<&str>,
    lease_ttl_s: Option<i64>,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut c = db.acquire().await?;
    let tid = owned_team(&mut c, p, team).await?;
    let key = key.trim().to_uppercase();
    let ttl = lease_ttl_s.unwrap_or(600).clamp(10, 86_400);
    sqlx::query("insert into projects (id, team_id, key, name, repo_url, lease_ttl) values ($1,$2,$3,$4,$5,make_interval(secs => $6::float8))")
        .bind(Uuid::new_v4())
        .bind(tid)
        .bind(&key)
        .bind(name)
        .bind(repo_url)
        .bind(ttl as f64)
        .execute(&mut *c)
        .await?;
    Ok(json!({"key": key, "name": name, "team": team, "lease_ttl_s": ttl}))
}

/// Create an agent (or relay) owned by `owner`; returns its first token, shown once.
pub async fn create_principal(
    db: &PgPool,
    p: &Principal,
    kind: &str,
    name: &str,
    agent_kind: Option<&str>,
    runtime: Option<&str>,
    max_leases: Option<i64>,
    owner: Option<&str>,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    if !p.is_user() {
        return Err(Error::Forbidden(
            "only users can create agents and relays".into(),
        ));
    }
    let name = norm_name(name)?;
    let mut tx = db.begin().await?;
    let owner_id = match owner {
        Some(o) if o.to_lowercase() != p.name => {
            if !p.is_admin {
                return Err(Error::Forbidden(
                    "only an admin can create agents for someone else".into(),
                ));
            }
            sqlx::query_scalar("select id from principals where name = $1 and kind = 'user'")
                .bind(o.to_lowercase())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| Error::NotFound(format!("no user {o}")))?
        }
        _ => p.id,
    };
    let (agent_kind, runtime) = if kind == "agent" {
        let ak = agent_kind.unwrap_or("other");
        let rt = runtime.unwrap_or("local");
        if !["claude-code", "codex", "opencode", "other"].contains(&ak) {
            return invalid("kind must be claude-code, codex, opencode or other");
        }
        if !["cloud", "local"].contains(&rt) {
            return invalid("runtime must be cloud or local");
        }
        (Some(ak), Some(rt))
    } else {
        (None, None)
    };
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into principals (id, kind, name, owner_id, agent_kind, runtime, max_leases) values ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id)
    .bind(kind)
    .bind(&name)
    .bind(owner_id)
    .bind(agent_kind)
    .bind(runtime)
    .bind(max_leases.unwrap_or(1).max(1))
    .execute(&mut *tx)
    .await?;
    let token = mint_token(&mut tx, id, "initial", None).await?;
    tx.commit().await?;
    Ok(
        json!({"name": name, "kind": kind, "agent_kind": agent_kind, "runtime": runtime, "token": token}),
    )
}

pub async fn list_principals(db: &PgPool, p: &Principal, kind: &str) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('id', a.id, 'name', a.name, 'kind', a.kind, 'agent_kind', a.agent_kind, 'runtime', a.runtime,
            'max_leases', a.max_leases, 'owner', (select name from principals where id = a.owner_id),
            'grants', to_jsonb(a.grants),
            'disabled', a.disabled_at is not null, 'last_seen_at', a.last_seen_at,
            'busy', (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()))
         from principals a where a.kind = $1 and ($2 or a.owner_id = $3) order by a.name",
    )
    .bind(kind)
    .bind(p.is_admin)
    .bind(p.id)
    .fetch_all(db)
    .await?;
    Ok(Value::Array(rows))
}

async fn managed(c: &mut PgConnection, p: &Principal, name: &str) -> Result<(Uuid, String)> {
    let row: Option<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "select id, kind, owner_id from principals where name = $1 and kind <> 'user'",
    )
    .bind(name.to_lowercase())
    .fetch_optional(c)
    .await?;
    match row {
        Some((id, kind, owner)) if p.is_admin || owner == Some(p.id) => Ok((id, kind)),
        _ => Err(Error::NotFound(format!("no agent or relay named {name}"))),
    }
}

/// Revoke every token and issue a fresh one; the old secret stops working at once.
pub async fn rotate(db: &PgPool, p: &Principal, name: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut tx = db.begin().await?;
    let (id, _) = managed(&mut tx, p, name).await?;
    sqlx::query(
        "update tokens set revoked_at = now() where principal_id = $1 and revoked_at is null",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let token = mint_token(&mut tx, id, "rotated", None).await?;
    tx.commit().await?;
    Ok(json!({"name": name.to_lowercase(), "token": token}))
}

pub async fn set_disabled(db: &PgPool, p: &Principal, name: &str, disabled: bool) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut c = db.acquire().await?;
    let (id, _) = managed(&mut c, p, name).await?;
    sqlx::query(
        "update principals set disabled_at = case when $2 then now() else null end where id = $1",
    )
    .bind(id)
    .bind(disabled)
    .execute(&mut *c)
    .await?;
    Ok(json!({"name": name.to_lowercase(), "disabled": disabled}))
}

pub async fn add_wake_target(
    db: &PgPool,
    p: &Principal,
    agent: &str,
    typ: &str,
    projects: Vec<String>,
    labels: Vec<String>,
    config: Value,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut c = db.acquire().await?;
    let (id, kind) = managed(&mut c, p, agent).await?;
    if kind != "agent" {
        return invalid("wake targets belong to agents");
    }
    // relays need nothing; cloud targets: the same rules as a cloud setup (src/connect.rs)
    let config = match typ {
        "relay" => config,
        _ => crate::connect::cloud_config(typ, &config, p.is_admin)?,
    };
    let tid = Uuid::new_v4();
    let projects: Vec<String> = projects.into_iter().map(|s| s.to_uppercase()).collect();
    sqlx::query("insert into wake_targets (id, agent_id, type, projects, labels, config) values ($1,$2,$3,$4,$5,$6)")
        .bind(tid)
        .bind(id)
        .bind(typ)
        .bind(&projects)
        .bind(&labels)
        .bind(&config)
        .execute(&mut *c)
        .await?;
    let mut out = json!({"id": tid, "agent": agent.to_lowercase(), "type": typ});
    if typ == "claude-routine" {
        // the routine's own prompt must tell it to act on what corntchap sends
        out["routine_prompt"] = json!(crate::cloud::ROUTINE_PROMPT);
    }
    Ok(out)
}

pub async fn list_wake_targets(db: &PgPool, p: &Principal, agent: &str) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    let mut c = db.acquire().await?;
    let (id, _) = managed(&mut c, p, agent).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        // secrets in config are never echoed back
        "select jsonb_build_object('id', id, 'type', type, 'projects', projects, 'labels', labels,
            'config', config - 'token' - 'headers' - 'secret') from wake_targets where agent_id = $1 order by id",
    )
    .bind(id)
    .fetch_all(&mut *c)
    .await?;
    Ok(Value::Array(rows))
}

pub async fn delete_wake_target(db: &PgPool, p: &Principal, id: Uuid) -> Result<Value> {
    let p = &acting(db, p).await?;
    let n = sqlx::query(
        "delete from wake_targets w using principals a where w.id = $1 and a.id = w.agent_id and ($2 or a.owner_id = $3)",
    )
    .bind(id)
    .bind(p.is_admin)
    .bind(p.id)
    .execute(db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(Error::NotFound("wake target not found".into()));
    }
    Ok(json!({"ok": true}))
}

/// `corntchap init [--name <handle>]`: a local admin (no OIDC link: it is the CLI's bootstrap identity) plus a
/// short-lived bootstrap token. Without a name an existing `init` admin is reused (so `just init` can be
/// re-run), else a random handle is drawn. A name that already belongs to a user makes that user an `init`
/// admin (never demoted by the IdP).
pub async fn init_admin(db: &PgPool, name: Option<&str>) -> Result<(String, Option<String>)> {
    let mut tx = db.begin().await?;
    let existing: Option<(Uuid, String)> = match name {
        Some(n) => {
            let n = norm_handle(n)?;
            sqlx::query_as("select id, name from principals where name = $1 and kind = 'user'")
                .bind(n)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => {
            sqlx::query_as(
                "select id, name from principals
                 where kind = 'user' and admin_source = 'init' and is_admin and disabled_at is null
                 order by created_at limit 1",
            )
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    let (id, handle) = match existing {
        Some((id, handle)) => {
            sqlx::query(
                "update principals set is_admin = true, admin_source = 'init' where id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            (id, handle)
        }
        None => {
            let wanted = name.map(norm_handle).transpose()?;
            let (id, handle) =
                insert_user(&mut tx, wanted, None, None, true, "init", &mut random_pick).await?;
            (id, handle)
        }
    };
    let token = mint_token(&mut tx, id, "bootstrap", Some(7 * 86_400)).await?;
    tx.commit().await?;
    Ok((handle, Some(token)))
}

/// `corntchap token --name <principal>`: operator escape hatch (needs database access).
pub async fn operator_token(db: &PgPool, name: &str, ttl_secs: Option<i64>) -> Result<String> {
    let mut tx = db.begin().await?;
    let id: Uuid = sqlx::query_scalar("select id from principals where name = $1")
        .bind(name.to_lowercase())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| Error::NotFound(format!("no principal {name}")))?;
    let t = mint_token(&mut tx, id, "operator", ttl_secs).await?;
    tx.commit().await?;
    Ok(t)
}
