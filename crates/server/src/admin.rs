//! Admin side: users, teams, members, projects, agents, relays, tokens, wake targets.

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::{
    auth::{generate_token, sha256},
    error::{Error, Result, invalid},
    state::{Principal, Role},
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

/// The syntax of a handle (a person's, a cook's or a fire's): it appears in `@mentions` and in URL path segments
/// (`/teams/{team}/members/{name}`), so it starts and ends with a letter or digit (never `.`, `..`, `-h`, `all.`),
/// uses lowercase letters, digits, `.`, `_`, `-` inside, and is at most 64 long. `st_` is the prefix of station refs
/// in the pass's `to:`, so no handle starts with it. Letters are lowercased.
pub fn norm_handle_syntax(s: &str) -> Result<String> {
    let n = s.trim().to_lowercase();
    let b = n.as_bytes();
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    let ok = !b.is_empty()
        && b.len() <= 64
        && alnum(b[0])
        && alnum(b[b.len() - 1])
        && b.iter()
            .all(|&c| alnum(c) || matches!(c, b'.' | b'_' | b'-'))
        && !n.starts_with("st_");
    if !ok {
        return invalid(
            "handles start and end with a letter or digit and use letters, digits, '.', '_' and '-' inside (max 64); \"st_\" is taken",
        );
    }
    Ok(n)
}

/// Names the pass gives a meaning to (`@all` reaches everyone) or that read as an address to a
/// crowd; no person's handle may be one of them.
pub const RESERVED_HANDLES: &[&str] = &["all", "everyone", "here", "channel", "me", "system"];

/// A person's handle: valid syntax and not reserved. Letters are lowercased.
pub fn norm_handle(s: &str) -> Result<String> {
    let n = norm_handle_syntax(s)?;
    if RESERVED_HANDLES.contains(&n.as_str()) {
        return invalid(format!("{n} is reserved: pick another handle"));
    }
    Ok(n)
}

pub const ADJECTIVES: &[&str] = &[
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
];

pub const DISHES: &[&str] = &[
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

/// The handle belonged to another account at some point (a person who renamed away from it, or a person's
/// current handle): handles are never handed to a different account.
fn handle_taken(handle: &str) -> Error {
    Error::Conflict(format!(
        "the handle {handle} is not available: it is, or once was, another account's"
    ))
}

/// Claim `handle` for `owner` in the ledger of every handle a person ever had. True when it is the owner's
/// (new, or already theirs); false when another account holds the claim.
async fn claim_handle(c: &mut PgConnection, handle: &str, owner: Uuid) -> Result<bool> {
    let holder: Uuid = sqlx::query_scalar(
        "insert into handle_claims (name, principal_id) values ($1, $2)
         on conflict (name) do update set principal_id = handle_claims.principal_id
         returning principal_id",
    )
    .bind(handle)
    .bind(owner)
    .fetch_one(c)
    .await?;
    Ok(holder == owner)
}

/// Refuse a cook or fire name that a person has used (it would shadow a retired handle).
pub async fn ensure_not_a_handle(c: &mut PgConnection, name: &str) -> Result<()> {
    let used: Option<i32> = sqlx::query_scalar("select 1 from handle_claims where name = $1")
        .bind(name)
        .fetch_optional(c)
        .await?;
    match used {
        Some(_) => Err(handle_taken(name)),
        None => Ok(()),
    }
}

/// The identity at the identity provider: the issuer and the subject, together.
pub struct OidcIdentity<'a> {
    pub issuer: &'a str,
    pub sub: &'a str,
}

/// Insert a user. With `wanted` that exact handle (taken: 409); without, random kitchen handles until one is
/// free. When `identity` is set and its row appears meanwhile (a concurrent first sign-in of the same person)
/// that row is returned instead, so one (issuer, sub) is always one principal. `on conflict do nothing` keeps a
/// collision from aborting the surrounding transaction; the handle ledger is claimed right after, and a
/// handle somebody else ever had undoes the insert and draws again. Run it inside a transaction.
async fn insert_user(
    c: &mut PgConnection,
    wanted: Option<String>,
    identity: Option<&OidcIdentity<'_>>,
    display_name: Option<&str>,
    role: Role,
    source: &str,
    pick: Pick<'_>,
) -> Result<(Uuid, String)> {
    for attempt in 0..MAX_ATTEMPTS {
        let handle = match &wanted {
            Some(w) => w.clone(),
            None => generate_handle(pick, attempt >= SHORT_ATTEMPTS),
        };
        let (iss, sub) = (identity.map(|i| i.issuer), identity.map(|i| i.sub));
        let id: Option<Uuid> = sqlx::query_scalar(
            "insert into principals (id, kind, name, oidc_iss, oidc_sub, display_name, role, role_source)
             values ($1,'user',$2,$3,$4,$5,$6,$7) on conflict do nothing returning id",
        )
        .bind(Uuid::new_v4())
        .bind(&handle)
        .bind(iss)
        .bind(sub)
        .bind(display_name)
        .bind(role)
        .bind(source)
        .fetch_optional(&mut *c)
        .await?;
        if let Some(id) = id {
            if claim_handle(&mut *c, &handle, id).await? {
                return Ok((id, handle));
            }
            sqlx::query("delete from principals where id = $1")
                .bind(id)
                .execute(&mut *c)
                .await?;
            if wanted.is_some() {
                return Err(handle_taken(&handle));
            }
            continue;
        }
        if let Some(i) = identity
            && let Some(row) = sqlx::query_as::<_, (Uuid, String)>(
                "select id, name from principals where kind = 'user' and oidc_iss = $1 and oidc_sub = $2",
            )
            .bind(i.issuer)
            .bind(i.sub)
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

/// First sign-in of an unknown (issuer, sub): create the person (just-in-time) with a random kitchen handle.
pub async fn provision_user(
    db: &PgPool,
    identity: &OidcIdentity<'_>,
    display_name: Option<&str>,
    pick: Pick<'_>,
) -> Result<Uuid> {
    let mut tx = db.begin().await?;
    let (id, _) = insert_user(
        &mut tx,
        None,
        Some(identity),
        display_name,
        Role::Member,
        "oidc",
        pick,
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// One-time adoption at startup: people who signed in before issuers were recorded belong to the issuer
/// configured now. Returns how many rows were adopted.
pub async fn adopt_issuer(db: &PgPool, issuer: &str) -> Result<u64> {
    let n = sqlx::query(
        "update principals set oidc_iss = $1 where oidc_sub is not null and oidc_iss is null",
    )
    .bind(issuer)
    .execute(db)
    .await?
    .rows_affected();
    Ok(n)
}

/// Longest display name kept, in characters.
pub const MAX_DISPLAY_NAME: usize = 120;

/// Characters that look like the "name · @handle" separator and so could fake a handle suffix.
const SEPARATOR_LOOKALIKES: &[char] = &['\u{00B7}', '\u{2022}', '\u{2027}', '\u{30FB}', '\u{FF65}'];
/// Invisible fillers that are letters or marks by category: Hangul fillers and the combining grapheme joiner.
const INVISIBLE_FILLERS: &[char] = &['\u{115F}', '\u{1160}', '\u{3164}', '\u{FFA0}', '\u{034F}'];

/// A claim from the ID token made safe to store and show: characters of the categories Cc, Cf, Co, Cs, Zl and
/// Zp removed (controls, bidi and other format characters, private use, line and paragraph separators), plus
/// invisible fillers and separator look-alikes; trimmed; cut to [`MAX_DISPLAY_NAME`] characters at a
/// grapheme-cluster boundary. Empty after cleaning becomes `None`.
pub fn clean_display_name(raw: &str) -> Option<String> {
    use unicode_properties::{GeneralCategory as G, UnicodeGeneralCategory};
    use unicode_segmentation::UnicodeSegmentation;
    let kept: String = raw
        .chars()
        .filter(|c| {
            !matches!(
                c.general_category(),
                G::Control
                    | G::Format
                    | G::PrivateUse
                    | G::Surrogate
                    | G::LineSeparator
                    | G::ParagraphSeparator
            ) && !INVISIBLE_FILLERS.contains(c)
                && !SEPARATOR_LOOKALIKES.contains(c)
        })
        .collect();
    let mut out = String::new();
    let mut chars = 0;
    for g in kept.trim().graphemes(true) {
        let n = g.chars().count();
        if chars + n > MAX_DISPLAY_NAME {
            break;
        }
        chars += n;
        out.push_str(g);
    }
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_string())
}

/// Change your own handle. Only a person can (agents and relays are named by their owner), and a
/// disabled person cannot. Same rules as any handle; a taken one is a 409, and so is one another account
/// ever had (you may take back your own old handle).
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
    if !claim_handle(&mut tx, &new, p.id).await? {
        return Err(handle_taken(&new));
    }
    let done = sqlx::query("update principals set name = $2 where id = $1")
        .bind(p.id)
        .bind(&new)
        .execute(&mut *tx)
        .await;
    match done {
        Ok(_) => {}
        Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => {
            return Err(Error::Conflict(format!(
                "the handle {new} is already taken"
            )));
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

/// People who can actually act as super-admin: enabled, and either linked to a sign-in or made by `init` (the
/// CLI's bootstrap identity). A super-admin row nobody can sign in as does not count. Locks the rows so two
/// concurrent demotions or disables cannot both pass.
pub(crate) async fn usable_super_admins(c: &mut PgConnection) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "select id from principals
         where kind = 'user' and role = 'super_admin' and disabled_at is null
           and (oidc_sub is not null or role_source = 'init')
         for update",
    )
    .fetch_all(c)
    .await?)
}

pub(crate) fn last_super_admin() -> Error {
    Error::Conflict(
        "that is the last super-admin who can sign in: make someone else a super-admin first"
            .into(),
    )
}

pub(crate) async fn person_id(c: &mut PgConnection, handle: &str) -> Result<Uuid> {
    sqlx::query_scalar("select id from principals where name = $1 and kind = 'user'")
        .bind(handle)
        .fetch_optional(c)
        .await?
        .ok_or_else(|| Error::NotFound(format!("no user with the handle {handle}")))
}

pub(crate) fn clean_handle_arg(handle: &str) -> String {
    handle.trim().trim_start_matches('@').to_lowercase()
}

/// Disable or enable a person (super-admins only). A disabled person's tokens and browser sessions stop working at
/// once (and are revoked, so enabling later does not revive them); nobody can disable the last usable super-admin.
pub async fn set_user_disabled(
    db: &PgPool,
    p: &Principal,
    handle: &str,
    disable: bool,
) -> Result<Value> {
    let v = crate::roles::update_person(db, p, handle, None, Some(disable)).await?;
    Ok(json!({"name": v["name"], "disabled": disable}))
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
/// as its owner (with the owner's role only if it is also granted `admin`; else at most an orchestrator's). Anything
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
    let row: Option<(Uuid, String, Role, i64)> = sqlx::query_as(
        "select id, name, role, max_leases from principals where id = $1 and kind = 'user' and disabled_at is null",
    )
    .bind(owner)
    .fetch_optional(db)
    .await?;
    let (id, name, role, max_leases) =
        row.ok_or_else(|| Error::Forbidden(format!("{}'s owner is disabled", p.name)))?;
    Ok(Principal {
        id,
        kind: "user".into(),
        name,
        owner_id: None,
        // `manage` lets a cook work as its owner's orchestrator side (their teams, pots, cooks); only `admin`
        // passes a super-admin's role on, so an orchestrator's cook never becomes a super-admin.
        role: if p.granted("admin") {
            role
        } else {
            role.min(Role::Orchestrator)
        },
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

/// Super-admins see every person with their sign-in state; an orchestrator sees the people directory (handle,
/// display name and role: no sign-in or contact details) to put people in their teams.
pub async fn list_users(db: &PgPool, p: &Principal) -> Result<Value> {
    let p = &acting_or_self(db, p).await?;
    if !p.is_user() || !p.can_orchestrate() {
        return Err(Error::Forbidden(
            "only an orchestrator or a super-admin can list people".into(),
        ));
    }
    let sql = if p.is_super_admin() {
        "select jsonb_build_object('name', name, 'display_name', display_name, 'role', role,
            'linked', oidc_sub is not null, 'disabled', disabled_at is not null) from principals where kind = 'user' order by name"
    } else {
        "select jsonb_build_object('name', name, 'display_name', display_name, 'role', role)
         from principals where kind = 'user' and disabled_at is null order by name"
    };
    let rows: Vec<Value> = sqlx::query_scalar(sql).fetch_all(db).await?;
    Ok(Value::Array(rows))
}

pub async fn create_team(db: &PgPool, p: &Principal, name: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    crate::roles::require_orchestrator(p, "create teams")?;
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
    .bind(p.is_super_admin())
    .bind(p.id)
    .fetch_all(db)
    .await?;
    Ok(Value::Array(rows))
}

/// Resolve a team the principal owns (or any team for super-admins). Owning needs the orchestrator role: a demoted
/// owner's row stays but counts as a plain member until they are an orchestrator again.
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
            if p.is_super_admin() || (owner && p.is_user() && p.can_orchestrate()) {
                Ok(id)
            } else if member {
                Err(Error::Forbidden("only a team owner can do that".into()))
            } else {
                Err(Error::NotFound(format!("team {team} not found")))
            }
        }
    }
}

/// Resolve a team the principal belongs to in any role (or any team for admins), with whether
/// they may manage it (owner or admin). Non-members get "not found".
async fn member_team(c: &mut PgConnection, p: &Principal, team: &str) -> Result<(Uuid, bool)> {
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
            if p.is_super_admin() || (owner && p.is_user() && p.can_orchestrate()) {
                Ok((id, true))
            } else if member {
                Ok((id, false))
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
    let (tid, manager) = member_team(&mut c, p, team).await?;
    let target: Option<(Uuid, String, Option<Uuid>, Role)> =
        sqlx::query_as("select id, kind, owner_id, role from principals where name = $1 and kind <> 'relay' and disabled_at is null")
            .bind(principal.to_lowercase())
            .fetch_optional(&mut *c)
            .await?;
    let Some((pid, kind, owner, target_role)) = target else {
        return Err(Error::NotFound(format!(
            "no user or agent named {principal}"
        )));
    };
    // An agent joins a team only with its owner's say-so (otherwise a foreign team could put work on their machine).
    if kind == "agent" && !(p.is_super_admin() || owner == Some(p.id)) {
        return Err(Error::Forbidden(
            "only the agent's owner (or a super-admin) can add it to a team".into(),
        ));
    }
    if role == "owner" && kind != "user" {
        return invalid("only users can own a team");
    }
    if role == "owner" && target_role < Role::Orchestrator {
        return invalid(format!(
            "{principal} is a {target_role}: only an orchestrator or a super-admin can own a team (a super-admin sets roles)"
        ));
    }
    // A plain member (chef) may add only their own cooks, as members.
    if !manager && (kind != "agent" || owner != Some(p.id) || role != "member") {
        return Err(Error::Forbidden(
            "a team member can only add their own cooks; only a team owner can add anyone else or change roles".into(),
        ));
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
    let (tid, manager) = member_team(&mut c, p, team).await?;
    if !manager {
        let own: Option<Uuid> = sqlx::query_scalar(
            "select id from principals where name = $1 and kind = 'agent' and owner_id = $2",
        )
        .bind(principal.to_lowercase())
        .bind(p.id)
        .fetch_optional(&mut *c)
        .await?;
        if own.is_none() {
            return Err(Error::Forbidden(
                "a team member can only remove their own cooks".into(),
            ));
        }
    }
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
    .bind(p.is_super_admin())
    .bind(p.id)
    .fetch_optional(&mut *c)
    .await?;
    let tid = row.ok_or_else(|| Error::NotFound(format!("team {team} not found")))?;
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select jsonb_build_object('name', pr.name, 'display_name', pr.display_name, 'kind', pr.kind, 'role', m.role,
                'fireAndForget', {}) from team_members m
         join principals pr on pr.id = m.principal_id where m.team_id = $1 order by pr.name",
        crate::cloud::fire_and_forget_sql("pr.id")
    )))
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
    let mut c = db.begin().await?;
    let tid = owned_team(&mut c, p, team).await?;
    let key = key.trim().to_uppercase();
    let ttl = lease_ttl_s.unwrap_or(600).clamp(10, 86_400);
    let id = Uuid::new_v4();
    // `repo_url` is the pot's first repository (projects.repo_url follows the repositories table)
    sqlx::query("insert into projects (id, team_id, key, name, lease_ttl) values ($1,$2,$3,$4,make_interval(secs => $5::float8))")
        .bind(id)
        .bind(tid)
        .bind(&key)
        .bind(name)
        .bind(ttl as f64)
        .execute(&mut *c)
        .await?;
    if let Some(url) = repo_url.map(str::trim).filter(|u| !u.is_empty()) {
        crate::repos::insert_repo(&mut c, id, url, None, None).await?;
    }
    c.commit().await?;
    Ok(json!({"key": key, "name": name, "team": team, "lease_ttl_s": ttl}))
}

/// A pot's default lease length, for claims that ask for none (10 s to a day). Its team's owner
/// (or a cook acting for them) and admins may change it; live leases keep their expiry.
pub async fn set_lease_ttl(db: &PgPool, p: &Principal, key: &str, ttl_s: i64) -> Result<Value> {
    let p = &acting(db, p).await?;
    let mut c = db.acquire().await?;
    let key = key.trim().to_uppercase();
    let team: Option<String> = sqlx::query_scalar(
        "select t.name from projects pr join teams t on t.id = pr.team_id where pr.key = $1",
    )
    .bind(&key)
    .fetch_optional(&mut *c)
    .await?;
    let team = team.ok_or_else(|| Error::NotFound(format!("pot {key} not found")))?;
    owned_team(&mut c, p, &team).await?;
    let ttl = ttl_s.clamp(10, 86_400);
    sqlx::query("update projects set lease_ttl = make_interval(secs => $2::float8) where key = $1")
        .bind(&key)
        .bind(ttl as f64)
        .execute(&mut *c)
        .await?;
    Ok(json!({"key": key, "lease_ttl_s": ttl}))
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
    let name = norm_handle_syntax(name)?;
    let mut tx = db.begin().await?;
    ensure_not_a_handle(&mut tx, &name).await?;
    let owner_id = match owner {
        Some(o) if o.to_lowercase() != p.name => {
            if !p.is_super_admin() {
                return Err(Error::Forbidden(
                    "only a super-admin can create agents for someone else".into(),
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
        if ![
            "claude-code",
            "codex",
            "opencode",
            "copilot",
            "antigravity",
            "other",
        ]
        .contains(&ak)
        {
            return invalid(
                "kind must be claude-code, codex, opencode, copilot, antigravity or other",
            );
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
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select jsonb_build_object('id', a.id, 'name', a.name, 'kind', a.kind, 'agent_kind', a.agent_kind, 'runtime', a.runtime,
            'fireAndForget', {},
            'max_leases', a.max_leases, 'owner', (select name from principals where id = a.owner_id),
            'grants', to_jsonb(a.grants),
            'repo_scopes', coalesce((select jsonb_agg(jsonb_build_object('project', rp.key, 'repos', to_jsonb(cr.repos)) order by rp.key)
                from cook_repos cr join projects rp on rp.id = cr.project_id where cr.agent_id = a.id), '[]'::jsonb),
            'disabled', a.disabled_at is not null, 'last_seen_at', a.last_seen_at,
            'busy', (select count(*) from tasks t where t.claimed_by = a.id and t.status = 'claimed' and t.lease_expires_at > now()),
            'teams', coalesce((select jsonb_agg(t.name order by t.name) from team_members m join teams t on t.id = m.team_id
                               where m.principal_id = a.id), '[]'::jsonb))
         from principals a where a.kind = $1 and ($2 or a.owner_id = $3) order by a.name",
        crate::cloud::fire_and_forget_sql("a.id")
    )))
    .bind(kind)
    .bind(p.is_super_admin())
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
        Some((id, kind, owner)) if p.is_super_admin() || owner == Some(p.id) => Ok((id, kind)),
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
        _ => crate::connect::cloud_config(typ, &config, p.is_super_admin())?,
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
            'config', (config - 'token' - 'headers' - 'secret') || coalesce((
                select jsonb_build_object('session_url', r.session_url) from coder_runs r
                where r.target_id = wake_targets.id and r.session_url is not null
                order by r.updated_at desc limit 1), '{}'::jsonb))
            from wake_targets where agent_id = $1 order by id",
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
    .bind(p.is_super_admin())
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
/// re-run; each run mints another 7-day token and older ones stay valid until they expire), else a random
/// handle is drawn. A name that already belongs to a local user makes that user an `init` admin (never
/// demoted by the IdP); one that belongs to a person who signs in through the IdP is refused.
pub async fn init_admin(db: &PgPool, name: Option<&str>) -> Result<(String, Option<String>)> {
    let mut tx = db.begin().await?;
    let existing: Option<(Uuid, String, bool)> = match name {
        Some(n) => {
            let n = norm_handle(n)?;
            sqlx::query_as(
                "select id, name, oidc_sub is not null from principals where name = $1 and kind = 'user'",
            )
            .bind(n)
            .fetch_optional(&mut *tx)
            .await?
        }
        None => {
            sqlx::query_as(
                "select id, name, oidc_sub is not null from principals
                 where kind = 'user' and role_source = 'init' and role = 'super_admin' and disabled_at is null
                 order by created_at limit 1",
            )
            .fetch_optional(&mut *tx)
            .await?
        }
    };
    let (id, handle) = match existing {
        Some((_, handle, true)) => {
            return Err(Error::Conflict(format!(
                "{handle} is a person who signs in through the identity provider: init makes local admins only (use `corntchap-cli user role {handle} super-admin`)"
            )));
        }
        Some((id, handle, false)) => {
            sqlx::query(
                "update principals set role = 'super_admin', role_source = 'init' where id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            (id, handle)
        }
        None => {
            let wanted = name.map(norm_handle).transpose()?;
            insert_user(
                &mut tx,
                wanted,
                None,
                None,
                Role::SuperAdmin,
                "init",
                &mut random_pick,
            )
            .await?
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

// ---- rename and delete: cooks, fires, pots, teams ----

/// The cook (`kind = "agent"`) or fire (`"relay"`) `name`, if `p` may manage it: its id.
async fn manage_target(
    c: &mut PgConnection,
    p: &Principal,
    name: &str,
    kind: &str,
) -> Result<Uuid> {
    let (id, k) = managed(c, p, name).await?;
    if k != kind {
        return Err(Error::NotFound(format!(
            "no {} named {name}",
            if kind == "agent" { "cook" } else { "fire" }
        )));
    }
    Ok(id)
}

/// Rename a cook (`kind = "agent"`) or a fire (`"relay"`): its owner or an admin. The id never changes, so
/// tokens, stations, leases, wake targets and team memberships keep working. What stores the old *name* as
/// text does not follow: `@old` in comments and pass messages, the old name in earlier events' payloads,
/// and the `agent = "old"` of a fire's `[[target]]` in config.toml.
pub async fn rename_principal(
    db: &PgPool,
    p: &Principal,
    kind: &str,
    name: &str,
    new_name: &str,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    let new = norm_handle_syntax(new_name)?;
    let mut tx = db.begin().await?;
    let id = manage_target(&mut tx, p, name, kind).await?;
    let old: String = sqlx::query_scalar("select name from principals where id = $1 for update")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if old == new {
        return Ok(json!({"name": new, "previous": old}));
    }
    ensure_not_a_handle(&mut tx, &new).await?;
    let done = sqlx::query("update principals set name = $2 where id = $1")
        .bind(id)
        .bind(&new)
        .execute(&mut *tx)
        .await;
    match done {
        Ok(_) => {}
        Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => {
            return Err(Error::Conflict(format!("the name {new} is already taken")));
        }
        Err(e) => return Err(e.into()),
    }
    crate::service::emit(
        &mut tx,
        None,
        None,
        "principal.renamed",
        Some(id),
        json!({"from": old, "to": new, "kind": kind}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"name": new, "previous": old}))
}

/// Delete a cook or a fire: its owner or an admin, never the caller itself. A cook's live leases go first
/// (tasks back to `open`, as `release` does, with a `task.released` event); then the principal row goes and
/// tokens, stations, wake targets, team memberships, cloud setups, messages sent *to* it, usage and OAuth
/// grants cascade. Rows that only record who did something (comment, task and checklist authors, question
/// and pass-message authors, uploader) become null (migration 0022), and the events it caused keep its name
/// in their payload (`actor_name`), so history survives.
pub async fn delete_principal(db: &PgPool, p: &Principal, kind: &str, name: &str) -> Result<Value> {
    let caller = p.id;
    let p = &acting(db, p).await?;
    let mut tx = db.begin().await?;
    let id = manage_target(&mut tx, p, name, kind).await?;
    if id == caller {
        return Err(Error::Forbidden(
            "a cook cannot delete itself: ask its owner".into(),
        ));
    }
    let shown: String = sqlx::query_scalar("select name from principals where id = $1 for update")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    // every lease it holds, expired or not
    let held: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "select id, project_id from tasks where claimed_by = $1 and status = 'claimed' for update",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    for (task, project) in &held {
        sqlx::query(
            "update tasks set status = 'open', claimed_by = null, lease_expires_at = null,
                lease_epoch = lease_epoch + 1, updated_at = now()
             where id = $1",
        )
        .bind(task)
        .execute(&mut *tx)
        .await?;
        crate::service::emit(
            &mut tx,
            Some(*project),
            Some(*task),
            "task.released",
            Some(id),
            json!({"reason": format!("{shown} was deleted")}),
        )
        .await?;
    }
    sqlx::query(
        "update events set payload = payload || jsonb_build_object('actor_name', $2::text) where actor_id = $1",
    )
    .bind(id)
    .bind(&shown)
    .execute(&mut *tx)
    .await?;
    sqlx::query("delete from principals where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        None,
        None,
        "principal.deleted",
        Some(p.id),
        json!({"name": shown, "kind": kind, "released": held.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"name": shown, "released": held.len()}))
}

/// A pot of a team `p` owns (admins: any): (project id, display name).
pub(crate) async fn owned_project(
    c: &mut PgConnection,
    p: &Principal,
    key: &str,
) -> Result<(Uuid, String)> {
    let key = key.trim().to_uppercase();
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("select id, team_id, name from projects where key = $1")
            .bind(&key)
            .fetch_optional(&mut *c)
            .await?;
    let Some((id, team, name)) = row else {
        return Err(Error::NotFound(format!("pot {key} not found")));
    };
    let (owner, member): (bool, bool) = sqlx::query_as(
        "select coalesce(bool_or(role = 'owner'), false), count(*) > 0 from team_members
         where team_id = $1 and principal_id = $2",
    )
    .bind(team)
    .bind(p.id)
    .fetch_one(c)
    .await?;
    if p.is_super_admin() || (owner && p.is_user() && p.can_orchestrate()) {
        Ok((id, name))
    } else if member {
        Err(Error::Forbidden(
            "only an owner of the pot's team (or an admin) can do that".into(),
        ))
    } else {
        Err(Error::NotFound(format!("pot {key} not found")))
    }
}

/// Change a pot's display name. The key never changes: it is part of every task id (`PAY-12`), of file
/// references, of saved links and of the `/mcp/<KEY>` URLs agents are configured with, and it is written
/// into comments, messages and events, so changing it would break all of those at once.
pub async fn rename_project(db: &PgPool, p: &Principal, key: &str, name: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return invalid("a pot's name is 1 to 120 characters");
    }
    let mut tx = db.begin().await?;
    let (id, old) = owned_project(&mut tx, p, key).await?;
    sqlx::query("update projects set name = $2 where id = $1")
        .bind(id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        Some(id),
        None,
        "project.renamed",
        Some(p.id),
        json!({"from": old, "to": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"key": key.trim().to_uppercase(), "name": name, "previous": old}))
}

/// Delete a pot and everything in it (owners of its team, admins; `confirm` must be the key). The database
/// cascades (tasks, checklists, comments, pass, pantry, views, menus, questions, stations, files' metadata,
/// tokens scoped to the pot, usage: migration 0022 adds the task cascade); `events` and its daily counts
/// carry no foreign key and are deleted here. Stored file objects are removed best effort after the commit.
pub async fn delete_project(
    db: &PgPool,
    storage: Option<&crate::files::Storage>,
    p: &Principal,
    key: &str,
    confirm: &str,
) -> Result<Value> {
    let p = &acting(db, p).await?;
    let key = key.trim().to_uppercase();
    if confirm.trim().to_uppercase() != key {
        return invalid(format!(
            "type the pot's key ({key}) as `confirm` to delete it and everything in it"
        ));
    }
    let mut tx = db.begin().await?;
    let (id, name) = owned_project(&mut tx, p, &key).await?;
    let keys: Vec<String> =
        sqlx::query_scalar("select storage_key from files where project_id = $1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    let tasks: i64 = sqlx::query_scalar("select count(*) from tasks where project_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("delete from events where project_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from event_daily_counts where project_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("delete from projects where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::service::emit(
        &mut tx,
        None,
        None,
        "project.deleted",
        Some(p.id),
        json!({"key": key, "name": name, "tasks": tasks}),
    )
    .await?;
    tx.commit().await?;
    let (mut gone, mut left) = (0, 0);
    for k in &keys {
        match storage {
            Some(s) => match s.delete(k).await {
                Ok(()) => gone += 1,
                Err(e) => {
                    tracing::warn!("deleting {k} from storage after deleting pot {key}: {e}");
                    left += 1;
                }
            },
            None => left += 1,
        }
    }
    Ok(json!({"key": key, "name": name, "tasks": tasks, "files": gone, "files_left": left}))
}

/// Rename a team: its owners or an admin.
pub async fn rename_team(db: &PgPool, p: &Principal, name: &str, new_name: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    let new = norm_name(new_name)?;
    let mut tx = db.begin().await?;
    let id = owned_team(&mut tx, p, name).await?;
    let old: String = sqlx::query_scalar("select name from teams where id = $1 for update")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if old != new {
        sqlx::query("update teams set name = $2 where id = $1")
            .bind(id)
            .bind(&new)
            .execute(&mut *tx)
            .await
            .map_err(
                |e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
                    Some("23505") => Error::Conflict(format!("a team named {new} already exists")),
                    _ => e.into(),
                },
            )?;
        crate::service::emit(
            &mut tx,
            None,
            None,
            "team.renamed",
            Some(p.id),
            json!({"from": old, "to": new}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"name": new, "previous": old}))
}

/// Delete a team with no pots (owners, admins; `confirm` must be the team's name). Memberships go with it.
pub async fn delete_team(db: &PgPool, p: &Principal, name: &str, confirm: &str) -> Result<Value> {
    let p = &acting(db, p).await?;
    let name = name.trim().to_lowercase();
    if confirm.trim().to_lowercase() != name {
        return invalid(format!(
            "type the team's name ({name}) as `confirm` to delete it"
        ));
    }
    let mut tx = db.begin().await?;
    let id = owned_team(&mut tx, p, &name).await?;
    let pots: Vec<String> =
        sqlx::query_scalar("select key from projects where team_id = $1 order by key")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    if !pots.is_empty() {
        return Err(Error::Conflict(format!(
            "team {name} still has pots ({}): delete them first",
            pots.join(", ")
        )));
    }
    sqlx::query("delete from teams where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(
            |e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
                // a pot was created between the check and the delete
                Some("23503") => Error::Conflict(format!("team {name} still has pots")),
                _ => e.into(),
            },
        )?;
    crate::service::emit(
        &mut tx,
        None,
        None,
        "team.deleted",
        Some(p.id),
        json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(json!({"name": name, "released": 0}))
}
