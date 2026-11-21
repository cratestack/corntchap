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

pub async fn create_user(db: &PgPool, p: &Principal, login: &str, is_admin: bool) -> Result<Value> {
    if !p.is_admin {
        return Err(Error::Forbidden("only an admin can add users".into()));
    }
    let login = norm_name(login)?;
    let id = Uuid::new_v4();
    sqlx::query(
        "insert into principals (id, kind, name, oidc_login, is_admin) values ($1,'user',$2,$2,$3)",
    )
    .bind(id)
    .bind(&login)
    .bind(is_admin)
    .execute(db)
    .await?;
    Ok(json!({"id": id, "name": login, "login": login, "is_admin": is_admin}))
}

pub async fn list_users(db: &PgPool, p: &Principal) -> Result<Value> {
    if !p.is_admin {
        return Err(Error::Forbidden("only an admin can list users".into()));
    }
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', name, 'login', oidc_login, 'is_admin', is_admin,
            'linked', oidc_sub is not null, 'disabled', disabled_at is not null) from principals where kind = 'user' order by name",
    )
    .fetch_all(db)
    .await?;
    Ok(Value::Array(rows))
}

pub async fn create_team(db: &PgPool, p: &Principal, name: &str) -> Result<Value> {
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
    if role != "owner" && role != "member" {
        return invalid("role must be owner or member");
    }
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
        "select jsonb_build_object('name', pr.name, 'kind', pr.kind, 'role', m.role) from team_members m
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
    let rows: Vec<Value> = sqlx::query_scalar(
        "select jsonb_build_object('name', a.name, 'kind', a.kind, 'agent_kind', a.agent_kind, 'runtime', a.runtime,
            'max_leases', a.max_leases, 'owner', (select name from principals where id = a.owner_id),
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
    let mut c = db.acquire().await?;
    let (id, kind) = managed(&mut c, p, agent).await?;
    if kind != "agent" {
        return invalid("wake targets belong to agents");
    }
    if typ != "relay" && !p.is_admin {
        return Err(Error::Forbidden(
            "only an admin can add webhook and github-comment wake targets".into(),
        ));
    }
    match typ {
        "relay" => {}
        "webhook" => {
            if config.get("url").and_then(|u| u.as_str()).is_none() {
                return invalid("webhook targets need config.url");
            }
        }
        "github-comment" => {
            for k in ["repo", "issue", "token"] {
                if config.get(k).is_none() {
                    return invalid(format!("github-comment targets need config.{k}"));
                }
            }
        }
        _ => return invalid("type must be relay, webhook or github-comment"),
    }
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
    Ok(json!({"id": tid, "agent": agent.to_lowercase(), "type": typ}))
}

pub async fn list_wake_targets(db: &PgPool, p: &Principal, agent: &str) -> Result<Value> {
    let mut c = db.acquire().await?;
    let (id, _) = managed(&mut c, p, agent).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        // secrets in config are never echoed back
        "select jsonb_build_object('id', id, 'type', type, 'projects', projects, 'labels', labels,
            'config', config - 'token' - 'headers') from wake_targets where agent_id = $1 order by id",
    )
    .bind(id)
    .fetch_all(&mut *c)
    .await?;
    Ok(Value::Array(rows))
}

pub async fn delete_wake_target(db: &PgPool, p: &Principal, id: Uuid) -> Result<Value> {
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

/// `corntchap init --admin <login>`: the first admin plus a short-lived bootstrap token.
pub async fn init_admin(db: &PgPool, login: &str) -> Result<(String, Option<String>)> {
    let login = norm_name(login)?;
    let mut tx = db.begin().await?;
    let existing: Option<Uuid> =
        sqlx::query_scalar("select id from principals where name = $1 and kind = 'user'")
            .bind(&login)
            .fetch_optional(&mut *tx)
            .await?;
    let id = match existing {
        Some(id) => {
            sqlx::query(
                "update principals set is_admin = true, admin_source = 'init' where id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            id
        }
        None => {
            let id = Uuid::new_v4();
            sqlx::query("insert into principals (id, kind, name, oidc_login, is_admin, admin_source) values ($1,'user',$2,$2,true,'init')")
                .bind(id)
                .bind(&login)
                .execute(&mut *tx)
                .await?;
            id
        }
    };
    let token = mint_token(&mut tx, id, "bootstrap", Some(7 * 86_400)).await?;
    tx.commit().await?;
    Ok((login, Some(token)))
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
