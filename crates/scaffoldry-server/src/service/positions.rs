//! Positions: the kinds of post a unit can have, and who holds them.
//!
//! A holding is a `roles` row with a `position_key`, so a position is held through the same
//! table as an appointment. Every change is a ledger entry written before the change.

use crate::service::admin::admin_write;
use crate::service::organizations::{unit_in_scope, OrgCaller};
use crate::repository_people::Holding;
use crate::service::people;
use crate::service::ServiceError;
use crate::state::{AuthUser, OrganizationNode, PositionType, RoleRow, SharedState};
use chrono::Utc;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

/// The unit types a position may apply to.
pub const ORG_TYPES: [&str; 5] = ["Institution", "College", "Department", "Center", "Program"];

fn orgs(state: &SharedState) -> Vec<OrganizationNode> {
    state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
}

fn position(state: &SharedState, key: &str) -> Result<PositionType, ServiceError> {
    state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
        .cloned()
        .ok_or_else(|| ServiceError::not_found(format!("Position '{key}' not found")))
}

fn row(pt: &PositionType, holder_count: usize) -> Value {
    json!({
        "key": pt.key,
        "name": pt.name,
        "description": pt.description,
        "org_types": pt.org_types,
        "max_holders": pt.max_holders,
        "retired": pt.retired_at.is_some(),
        "retired_at": pt.retired_at,
        "holder_count": holder_count,
        "created_by": pt.created_by,
        "created_at": pt.created_at,
    })
}

fn holder_count(counts: &std::collections::HashMap<String, i64>, key: &str) -> usize {
    counts.get(key).copied().unwrap_or(0) as usize
}

fn check_org_types(org_types: &[String]) -> Result<(), ServiceError> {
    if org_types.is_empty() {
        return Err(ServiceError::bad_request("org_types needs at least one unit type"));
    }
    if let Some(bad) = org_types.iter().find(|t| !ORG_TYPES.contains(&t.as_str())) {
        return Err(ServiceError::bad_request(format!(
            "'{bad}' is not a unit type. Use one of: {}",
            ORG_TYPES.join(", ")
        )));
    }
    Ok(())
}

fn check_max_holders(n: i64) -> Result<i32, ServiceError> {
    if (1..=50).contains(&n) {
        Ok(n as i32)
    } else {
        Err(ServiceError::bad_request("max_holders must be between 1 and 50"))
    }
}

fn check_key(key: &str) -> Result<(), ServiceError> {
    let ok = !key.is_empty()
        && key.len() <= 64
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(ServiceError::bad_request(
            "key must be 1 to 64 characters: lowercase letters, digits, '-' and '_'",
        ))
    }
}

// ---------------------------------------------------------------------------------------------
// Position types (Platform Admin)
// ---------------------------------------------------------------------------------------------

pub fn list_positions(state: &SharedState) -> Vec<Value> {
    let counts = state.repository.holder_counts().unwrap_or_default();
    let mut types: Vec<PositionType> =
        state.position_types.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    types.sort_by(|a, b| a.key.cmp(&b.key));
    types.iter().map(|pt| row(pt, holder_count(&counts, &pt.key))).collect()
}

pub struct NewPosition {
    pub key: String,
    pub name: String,
    pub description: String,
    pub org_types: Vec<String>,
    pub max_holders: i64,
}

pub fn create_position(state: &SharedState, admin: &AuthUser, new: NewPosition, reason: &str) -> Result<Value, ServiceError> {
    check_key(&new.key)?;
    if new.name.trim().is_empty() {
        return Err(ServiceError::bad_request("name is required"));
    }
    check_org_types(&new.org_types)?;
    let max_holders = check_max_holders(new.max_holders)?;
    if state.position_types.read().unwrap_or_else(|p| p.into_inner()).contains_key(&new.key) {
        return Err(ServiceError::conflict(format!("Position '{}' already exists", new.key)));
    }
    let pt = PositionType {
        key: new.key,
        name: new.name.trim().to_string(),
        description: new.description,
        org_types: new.org_types,
        max_holders,
        retired_at: None,
        created_by: admin.eppn.clone(),
        created_at: Utc::now().to_rfc3339(),
    };
    let payload = json!({ "action": "create_position", "key": pt.key, "org_types": pt.org_types, "max_holders": pt.max_holders });
    admin_write(state, admin, DecisionType::PositionChanged, "AC-02", reason, &payload, || {
        // The database decides who owns the key, so two servers creating it at once cannot both win.
        let created = state.repository.create_position_type(&pt).map_err(|e| ServiceError::Internal(e.to_string()))?;
        if !created {
            return Err(ServiceError::conflict(format!("Position '{}' already exists", pt.key)));
        }
        crate::service::reload_after_write(state)?;
        Ok(())
    })?;
    Ok(row(&pt, 0))
}

#[derive(Default)]
pub struct PositionPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub max_holders: Option<i64>,
    pub org_types: Option<Vec<String>>,
    pub retired: Option<bool>,
}

pub fn patch_position(
    state: &SharedState,
    admin: &AuthUser,
    key: &str,
    patch: PositionPatch,
    reason: &str,
) -> Result<Value, ServiceError> {
    let mut pt = position(state, key)?;
    let held = holder_count(&state.repository.holder_counts().unwrap_or_default(), key);

    if let Some(name) = patch.name {
        if name.trim().is_empty() {
            return Err(ServiceError::bad_request("name cannot be empty"));
        }
        pt.name = name.trim().to_string();
    }
    if let Some(description) = patch.description {
        pt.description = description;
    }
    if let Some(n) = patch.max_holders {
        pt.max_holders = check_max_holders(n)?;
    }
    if let Some(org_types) = patch.org_types {
        check_org_types(&org_types)?;
        if org_types != pt.org_types && held > 0 {
            return Err(ServiceError::conflict("The unit types cannot change while the position has holders"));
        }
        pt.org_types = org_types;
    }
    match patch.retired {
        Some(true) if pt.retired_at.is_none() => pt.retired_at = Some(Utc::now().to_rfc3339()),
        Some(false) => pt.retired_at = None,
        _ => {}
    }

    let payload = json!({ "action": "patch_position", "key": key, "retired": pt.retired_at.is_some(), "max_holders": pt.max_holders });
    admin_write(state, admin, DecisionType::PositionChanged, "AC-02", reason, &payload, || {
        // The row is locked while the holders are counted, so a holder cannot arrive in between.
        state.repository.change_position_type(&pt).map_err(|e| match e {
            crate::repository::RepositoryError::Conflict(m) => ServiceError::conflict(m),
            other => ServiceError::Internal(other.to_string()),
        })?;
        crate::service::reload_after_write(state)?;
        Ok(())
    })?;
    Ok(row(&pt, held))
}

/// One row for each unit a position applies to that has no holder.
pub fn vacancies(state: &SharedState) -> Vec<Value> {
    let filled = state.repository.filled_positions().unwrap_or_default();
    let units = orgs(state);
    let mut types: Vec<PositionType> = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.retired_at.is_none())
        .cloned()
        .collect();
    types.sort_by(|a, b| a.key.cmp(&b.key));
    let mut rows = Vec::new();
    for pt in &types {
        let mut open: Vec<&OrganizationNode> = units
            .iter()
            .filter(|u| pt.org_types.contains(&u.org_type) && !filled.contains(&(pt.key.clone(), u.id)))
            .collect();
        open.sort_by(|a, b| a.name.cmp(&b.name));
        for u in open {
            rows.push(json!({
                "unit": { "id": u.id, "name": u.name },
                "org_type": u.org_type,
                "position": { "key": pt.key, "name": pt.name },
            }));
        }
    }
    rows
}

// ---------------------------------------------------------------------------------------------
// Holders (Platform Admin, or an Org Unit Admin in scope)
// ---------------------------------------------------------------------------------------------

fn org_caller(c: &AuthUser) -> OrgCaller {
    OrgCaller { eppn: c.eppn.clone(), affiliation: c.affiliation.clone() }
}

fn unit_of(state: &SharedState, unit: Uuid) -> Result<OrganizationNode, ServiceError> {
    orgs(state)
        .into_iter()
        .find(|o| o.id == unit)
        .ok_or_else(|| ServiceError::not_found(format!("Organization '{unit}' not found")))
}

fn require_scope(state: &SharedState, caller: &AuthUser, unit: Uuid) -> Result<(), ServiceError> {
    if unit_in_scope(&org_caller(caller), unit, &orgs(state), &state.roles_of(&caller.eppn)) {
        Ok(())
    } else {
        Err(ServiceError::forbidden("Caller is not in scope for this organization unit"))
    }
}

/// The position types that apply to a unit, with their holders. Anyone in scope may read it.
pub fn unit_positions(state: &SharedState, caller: &AuthUser, unit: Uuid) -> Result<Value, ServiceError> {
    let node = unit_of(state, unit)?;
    require_scope(state, caller, unit)?;
    let roles = state.repository.roles_in_units(&[unit]).map_err(|e| ServiceError::Internal(e.to_string()))?;
    let mut types: Vec<PositionType> = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.retired_at.is_none() && p.org_types.contains(&node.org_type))
        .cloned()
        .collect();
    types.sort_by(|a, b| a.key.cmp(&b.key));
    let positions: Vec<Value> = types
        .iter()
        .map(|pt| {
            let holders: Vec<Value> = roles
                .iter()
                .filter(|r| r.position_key.as_deref() == Some(pt.key.as_str()))
                .map(|r| {
                    let display = people::find_by_user_name(state, &r.eppn)
                        .map(|u| people::display_name(&u))
                        .unwrap_or_else(|| r.eppn.clone());
                    json!({ "eppn": r.eppn, "display_name": display, "source": r.source })
                })
                .collect();
            json!({
                "key": pt.key,
                "name": pt.name,
                "description": pt.description,
                "max_holders": pt.max_holders,
                "holders": holders,
            })
        })
        .collect();
    Ok(json!({ "unit": { "id": node.id, "name": node.name, "org_type": node.org_type }, "positions": positions }))
}

pub fn assign_holder(
    state: &SharedState,
    caller: &AuthUser,
    unit: Uuid,
    key: &str,
    eppn: &str,
    replace: bool,
    reason: &str,
) -> Result<Value, ServiceError> {
    let node = unit_of(state, unit)?;
    require_scope(state, caller, unit)?;
    let pt = position(state, key)?;
    if pt.retired_at.is_some() {
        return Err(ServiceError::bad_request(format!("Position '{key}' is retired")));
    }
    if !pt.org_types.contains(&node.org_type) {
        return Err(ServiceError::bad_request(format!(
            "'{}' does not apply to a {}. It applies to: {}",
            pt.name,
            node.org_type,
            pt.org_types.join(", ")
        )));
    }
    let user = people::find_by_user_name(state, eppn)
        .ok_or_else(|| ServiceError::not_found(format!("No user '{eppn}'")))?;
    if !user.active || user.admin_hold {
        return Err(ServiceError::bad_request("The holder must be an active user who is not on hold"));
    }

    // A first look, so the usual refusals cost no ledger entry. The write below decides again under a row lock.
    let current = state.repository.roles_at(unit, key).map_err(|e| ServiceError::Internal(e.to_string()))?;
    if current.iter().any(|r| r.eppn.eq_ignore_ascii_case(&user.user_name)) {
        return Err(ServiceError::conflict("This person already holds the position here"));
    }
    let full = current.len() as i32 >= pt.max_holders;
    if full && !replace {
        return Err(ServiceError::PositionFull(format!(
            "{} at {} already has {} holder(s)",
            pt.name, node.name, current.len()
        )));
    }
    let vacate: Vec<RoleRow> = if full { current } else { Vec::new() };
    if vacate.iter().any(|r| r.source == "scim") {
        return Err(ServiceError::conflict(
            "The registry owns a current holding. Change it in the registry",
        ));
    }

    let holding = RoleRow {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        eppn: user.user_name.clone(),
        organization_id: unit,
        role_title: pt.name.clone(),
        scoped_affiliation: "position".to_string(),
        is_primary: false,
        source: "api".to_string(),
        position_key: Some(pt.key.clone()),
    };
    let grant = json!({ "action": "assign_holder", "organization_id": unit, "position": key, "eppn": holding.eppn });
    let write_grant = || {
        admin_write(state, caller, DecisionType::AccessRoleGranted, "AC-02", reason, &grant, || {
            let outcome = state
                .repository
                .assign_holding(&holding, pt.max_holders, replace)
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
            match outcome {
                Holding::Given => Ok(()),
                Holding::AlreadyHeld => Err(ServiceError::conflict("This person already holds the position here")),
                Holding::Full(n) => Err(ServiceError::PositionFull(format!(
                    "{} at {} already has {n} holder(s)",
                    pt.name, node.name
                ))),
                Holding::RegistryOwned => {
                    Err(ServiceError::conflict("The registry owns a current holding. Change it in the registry"))
                }
            }
        })
    };

    if vacate.is_empty() {
        write_grant()?;
    } else {
        let revoke = json!({
            "action": "vacate_for_replace", "organization_id": unit, "position": key,
            "vacated": vacate.iter().map(|r| r.eppn.clone()).collect::<Vec<_>>(),
        });
        // The vacating happens inside the grant, in the same transaction.
        admin_write(state, caller, DecisionType::AccessRoleRevoked, "AC-02", reason, &revoke, || Ok(()))?;
        write_grant()?;
    }
    Ok(json!({ "holder": { "eppn": holding.eppn, "position": key, "organization_id": unit } }))
}

pub fn vacate_holder(
    state: &SharedState,
    caller: &AuthUser,
    unit: Uuid,
    key: &str,
    eppn: &str,
    reason: &str,
) -> Result<(), ServiceError> {
    unit_of(state, unit)?;
    require_scope(state, caller, unit)?;
    let holding = state
        .repository
        .roles_at(unit, key)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .into_iter()
        .find(|r| r.eppn.eq_ignore_ascii_case(eppn))
        .ok_or_else(|| ServiceError::not_found("No such holder"))?;
    if holding.source == "scim" {
        return Err(ServiceError::conflict("The registry owns this holding. Change it in the registry"));
    }
    let payload = json!({ "action": "vacate_holder", "organization_id": unit, "position": key, "eppn": holding.eppn });
    admin_write(state, caller, DecisionType::AccessRoleRevoked, "AC-02", reason, &payload, || {
        state.delete_role(holding.id).map_err(|e| ServiceError::Internal(e.to_string()))
    })
}
