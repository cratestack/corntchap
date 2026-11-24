//! Roles: super_admin > orchestrator > member (see [`Role`]). Who may change them, how the IdP maps onto them,
//! and the two checks the rest of the server uses.

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    error::{Error, Result},
    state::{OidcConfig, Principal, Role},
};

/// A person who runs the instance.
pub fn require_super_admin(p: &Principal, what: &str) -> Result<()> {
    if p.is_user() && p.is_super_admin() {
        Ok(())
    } else {
        Err(Error::Forbidden(format!("only a super-admin can {what}")))
    }
}

/// A person who may own teams and pots: an orchestrator or a super-admin.
pub fn require_orchestrator(p: &Principal, what: &str) -> Result<()> {
    if p.is_user() && p.can_orchestrate() {
        Ok(())
    } else {
        Err(Error::Forbidden(format!(
            "only an orchestrator or a super-admin can {what}"
        )))
    }
}

/// True if the ID token carries `role` in Keycloak's `realm_access.roles` or a top-level
/// `roles` / `groups` array (group paths like `/corntchap-admin` match too).
pub fn has_idp_role(id_token: &str, role: &str) -> bool {
    let claim = |k: &str| crate::web::id_token_claim(id_token, k);
    let matches = |v: &Value| {
        v.as_array().is_some_and(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .any(|x| x == role || x.trim_start_matches('/') == role)
        })
    };
    claim("realm_access")
        .and_then(|ra| ra.get("roles").cloned())
        .is_some_and(|r| matches(&r))
        || ["roles", "groups"]
            .iter()
            .any(|k| claim(k).is_some_and(|v| matches(&v)))
}

/// The role an ID token asks for: the admin role makes a super-admin, else the orchestrator role an
/// orchestrator, else a member.
pub fn oidc_role(id_token: &str, cfg: &OidcConfig) -> Role {
    if has_idp_role(id_token, &cfg.admin_role) {
        Role::SuperAdmin
    } else if has_idp_role(id_token, &cfg.orchestrator_role) {
        Role::Orchestrator
    } else {
        Role::Member
    }
}

/// Make a person a super-admin, an orchestrator or a member (super-admins only). Any change makes the role
/// manual (`role_source = 'manual'`): the IdP sync only ever touches roles it made itself, so a manual
/// demotion sticks and a manual promotion is explicit. The last usable super-admin cannot be demoted.
/// A demoted team owner keeps the owner row but loses its powers until they are an orchestrator again.
pub async fn set_role(db: &PgPool, p: &Principal, handle: &str, role: Role) -> Result<Value> {
    require_super_admin(p, "change people's roles")?;
    let handle = crate::admin::clean_handle_arg(handle);
    let mut tx = db.begin().await?;
    let admins = crate::admin::usable_super_admins(&mut tx).await?;
    let target = crate::admin::person_id(&mut tx, &handle).await?;
    if role != Role::SuperAdmin && admins.contains(&target) && admins.iter().all(|a| *a == target) {
        return Err(crate::admin::last_super_admin());
    }
    sqlx::query("update principals set role = $2, role_source = 'manual' where id = $1")
        .bind(target)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(json!({
        "name": handle, "role": role, "role_source": "manual",
        "note": "manual role: the IdP role no longer changes it",
    }))
}
