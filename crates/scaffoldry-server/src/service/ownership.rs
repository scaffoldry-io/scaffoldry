//! When an owner leaves (lifecycle phase 5). The orphan list, a preview that changes nothing, and a transfer.
//!
//! A transfer takes the old owner's access away first, then makes the new person an owner, then revokes the old owner's
//! agent tokens for those workspaces. The order is chosen so that a failure part way leaves the old owner without access
//! and never both people with it. A workspace left with no owner shows in the orphan list to be finished.
//! The ledger entry is written before any of it. History, usage and the ledger keep their actors: nothing is reattributed.

use crate::service::admin::{admin_write, require_platform_admin, require_platform_admin_or_compliance};
use crate::service::identity::resolve_user;
use crate::service::{people, reload_after_write, ServiceError};
use crate::state::{AuthUser, CollaboratorRecord, SharedState};
use chrono::Utc;
use scaffoldry_core::ledger::DecisionType;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct TransferBody {
    pub from: Option<String>,
    pub to: Option<String>,
    /// Only these workspaces. Omitted: every workspace `from` owns.
    pub workspaces: Option<Vec<String>>,
    pub reason: Option<String>,
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// Plain words for why a person has gone.
fn why(reason: &str) -> &'static str {
    match reason {
        "inactive" => "is no longer active",
        "on hold" => "is on hold",
        _ => "has no current appointment",
    }
}

/// The workspaces with no owner who is here, and the open proposals and outside clients of people who have gone.
pub fn orphans(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let rows = state.repository.orphans().map_err(internal)?;
    let of = |kind: &str| -> Vec<Value> {
        rows.iter()
            .filter(|o| o.kind == kind)
            .map(|o| {
                let mut v = json!({ "id": o.id, "name": o.name });
                if kind == "workspace" {
                    v["owners"] = o.detail.clone();
                } else {
                    v["proposed_by"] = o.detail["proposed_by"].clone();
                    v["why"] = o.detail["why"].clone();
                    if kind == "proposal" {
                        v["workspace_id"] = o.detail["workspace_id"].clone();
                    }
                }
                v
            })
            .collect()
    };
    Ok(json!({ "workspaces": of("workspace"), "proposals": of("proposal"), "clients": of("client") }))
}

struct Plan {
    from: String,
    to: String,
    blockers: Vec<String>,
    view: Value,
    workspace_ids: Vec<String>,
}

fn plan(state: &SharedState, body: &TransferBody) -> Result<Plan, ServiceError> {
    let named = |v: &Option<String>, what: &str| -> Result<String, ServiceError> {
        let v = v.as_deref().unwrap_or("").trim().to_string();
        if v.is_empty() {
            return Err(ServiceError::bad_request(format!("{what} is required")));
        }
        Ok(v)
    };
    let (from, to) = (named(&body.from, "from")?, named(&body.to, "to")?);
    let only = body.workspaces.clone().unwrap_or_default();
    let repo = &state.repository;
    let mut blockers: Vec<String> = Vec::new();

    if from.eq_ignore_ascii_case(&to) {
        blockers.push(format!("{from} and {to} are the same person."));
    }
    if people::find_by_user_name(state, &to).is_none() {
        blockers.push(format!("{to} is not a person this server knows."));
    } else if let Some(reason) = repo.person_gone(&to).map_err(internal)? {
        blockers.push(format!("{to} {}, so they cannot take over a workspace.", why(&reason).replace("is no longer active", "is not active")));
    }

    let owned = repo.owned_workspaces(&from, &to, &only).map_err(internal)?;
    for id in &only {
        if !owned.iter().any(|w| &w.id == id) {
            blockers.push(format!("{from} is not an owner of {id}."));
        }
    }
    if owned.is_empty() && only.is_empty() {
        blockers.push(format!("{from} owns no workspace."));
    }

    let clients = repo.clients_proposed_by(&from).map_err(internal)?;
    let workspaces: Vec<Value> = owned
        .iter()
        .map(|w| {
            let gains = match w.to_role.as_deref() {
                Some("owner") => format!("{to} is already an owner of {}. They gain nothing new.", w.name),
                Some(role) => format!("{to} goes from {role} to owner of {}: they can manage its members, apps and settings.", w.name),
                None => format!("{to} becomes an owner of {}: they can manage its members, apps and settings.", w.name),
            };
            let mut loses = vec![format!("{from} loses their owner role in {}, and with it every right the role gave.", w.name)];
            if w.from_app_roles > 0 {
                loses.push(format!("{from} also loses {} role(s) limited to one app in {}.", w.from_app_roles, w.name));
            }
            if !w.tokens.is_empty() {
                loses.push(format!("{} of {from}'s agent tokens limited to apps in {} are revoked.", w.tokens.len(), w.name));
            }
            json!({
                "id": w.id,
                "name": w.name,
                "to_role": w.to_role,
                "gains": [gains],
                "loses": loses,
                "open_proposals": w.proposals,
                "process_steps": w.process_steps,
                "tokens": w.tokens,
            })
        })
        .collect();

    let view = json!({
        "from": from,
        "to": to,
        "can_transfer": blockers.is_empty(),
        "blockers": blockers,
        "workspaces": workspaces,
        "clients_from_created": clients,
    });
    Ok(Plan { workspace_ids: owned.iter().map(|w| w.id.clone()).collect(), from, to, blockers, view })
}

/// What a transfer would do. Changes nothing, writes nothing, not even a ledger entry.
pub fn preview(caller: &AuthUser, body: &TransferBody, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    Ok(plan(state, body)?.view)
}

/// Makes the transfer. Refused with the blockers when the preview has any.
pub fn transfer(caller: &AuthUser, body: &TransferBody, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let plan = plan(state, body)?;
    if !plan.blockers.is_empty() {
        return Err(ServiceError::Conflict(format!("This transfer cannot be made: {}", plan.blockers.join(" "))));
    }
    let target = resolve_user(&plan.to, state).ok_or_else(|| ServiceError::bad_request(format!("{} cannot be resolved or is on hold", plan.to)))?;
    let reason = body.reason.clone().unwrap_or_default();
    let payload = json!({ "from": plan.from, "to": plan.to, "workspaces": plan.workspace_ids });

    let (revoked_tokens, moved) = admin_write(state, caller, DecisionType::OwnershipTransferred, "AC-02", &reason, &payload, || {
        let repo = &state.repository;
        // 1. The old owner loses access first.
        repo.revoke_memberships_for_transfer(&plan.workspace_ids, &plan.from).map_err(internal)?;
        // 2. The new owner is added. A failure here leaves the workspaces without an owner, which the orphan list shows.
        let added = (|| -> Result<usize, ServiceError> {
            for id in &plan.workspace_ids {
                repo.upsert_collaborator(&CollaboratorRecord {
                    id: Uuid::new_v4().to_string(),
                    workspace_id: id.clone(),
                    eppn: target.eppn.clone(),
                    name: target.name.clone(),
                    role: "owner".to_string(),
                    scoped_affiliation: String::new(),
                    department: target.department.clone(),
                    added_at: Utc::now().to_rfc3339(),
                    app_slug: String::new(),
                })
                .map_err(crate::service::workspaces::membership_error)?;
                let ws = state.workspaces.read().map_err(internal)?.get(id).cloned();
                if let Some(mut ws) = ws {
                    ws.lead = target.name.clone();
                    repo.upsert_workspace(&ws).map_err(internal)?;
                }
            }
            Ok(plan.workspace_ids.len())
        })();
        // The caches follow the database whether or not the add worked.
        let reloaded = reload_after_write(state);
        let moved = added?;
        reloaded?;
        // 3. The old owner's agent tokens for those workspaces.
        let revoked = repo.revoke_agent_tokens_in(&plan.workspace_ids, &plan.from).map_err(internal)?;
        Ok((revoked, moved))
    })?;

    Ok(json!({ "status": "transferred", "from": plan.from, "to": plan.to, "workspaces": moved, "revoked_tokens": revoked_tokens }))
}
