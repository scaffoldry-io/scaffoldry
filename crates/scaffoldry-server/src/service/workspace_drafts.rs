//! MCP guidance phase 6: a tool that would change who may reach a workspace, who owns it, or how it is labelled drafts the
//! change. A person confirms it in the desk. These are the same drafts as `access_drafts`: a pending `access_changes` row that
//! applies nothing, names the tool that made it, and ends in seven days. A change that widens access is confirmed by someone
//! other than the person it was drafted for.
//!
//! Two kinds of change are drafted here, and each is applied by the service a person's own call would use, run as the person
//! who confirms it. So the confirmer needs the right to manage the workspace, as they would to make the change themselves.
//! - `workspace_settings`: visibility, data classification, department or unit of a workspace.
//! - `workspace_member`: adding, changing or removing a collaborator.

use crate::service::access::authorize_workspace;
use crate::service::access_drafts::{desk_link, store_draft};
use crate::service::history::Origin;
use crate::service::members::Role;
use crate::service::workspaces::{self, UpdateWorkspacePayload};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Whether the arguments of `update_workspace` name a field of access, ownership or labels.
pub fn touches_access(args: &Value) -> bool {
    ["visibility", "data_classification", "department", "organization_id"].iter().any(|k| args.get(k).is_some_and(|v| !v.is_null()))
}

fn visibility_rank(v: &str) -> u8 {
    match v {
        "restricted" => 0,
        "departmental" => 1,
        _ => 2,
    }
}

/// A higher number is more protection.
fn classification_rank(c: &str) -> u8 {
    match c {
        "Public" => 0,
        "Internal" => 1,
        "Restricted" => 2,
        _ => 3,
    }
}

fn drafted(state: &SharedState, caller: &AuthUser, origin: &Origin, tool: &str, mut change: Value, widens: bool, sentence: String) -> Result<Value, ServiceError> {
    let reason = format!("Drafted by {tool} for a person to confirm");
    change["sentence"] = json!(sentence);
    let kind = change["kind"].clone();
    let (id, expires) = store_draft(state, caller, origin, change, widens, &reason, tool)?;
    Ok(json!({
        "status": "Pending", "change_id": id, "kind": kind, "widens": widens, "reason": reason, "sentences": [sentence],
        "expires_at": expires, "desk_link": desk_link(id),
        "message": if widens { "Nothing has changed. A person other than the one you act for confirms this in the desk. Give them desk_link." } else { "Nothing has changed. A person confirms this in the desk. Give them desk_link." },
    }))
}

/// Drafts the change `update_workspace` was asked for, or `None` when it would change no field of access, ownership or labels
/// that the workspace does not already have. Nothing is applied.
pub fn draft_settings(state: &SharedState, caller: &AuthUser, origin: &Origin, args: &Value) -> Result<Option<Value>, ServiceError> {
    let id = text(args, "workspace_id").ok_or_else(|| ServiceError::BadRequest("Argument 'workspace_id' is required".to_string()))?;
    let ws = state.workspaces.read().map_err(internal)?.get(&id).cloned().ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?;
    let (visibility, class, department) = (text(args, "visibility"), text(args, "data_classification"), text(args, "department"));
    let unit = text(args, "organization_id").map(|u| Uuid::parse_str(&u).map_err(|_| ServiceError::BadRequest("organization_id must be an org unit id".to_string()))).transpose()?;
    if visibility.as_deref().is_some_and(|v| !matches!(v, "restricted" | "departmental" | "institutional")) {
        return Err(ServiceError::BadRequest("Invalid visibility: must be one of restricted, departmental, institutional".to_string()));
    }
    if class.as_deref().is_some_and(|c| !matches!(c, "Public" | "Internal" | "Restricted" | "FERPA Sensitive")) {
        return Err(ServiceError::BadRequest("Invalid data_classification: must be one of Public, Internal, Restricted, FERPA Sensitive".to_string()));
    }
    if unit.is_some_and(|u| !state.organizations.read().map(|o| o.contains_key(&u)).unwrap_or(false)) {
        return Err(ServiceError::BadRequest("Organization unit does not exist".to_string()));
    }
    let mut parts = Vec::new();
    let mut widens = false;
    if let Some(v) = visibility.as_deref().filter(|v| **v != ws.visibility) {
        widens |= visibility_rank(v) > visibility_rank(&ws.visibility);
        parts.push(format!("Change visibility from {} to {v}.", ws.visibility));
    }
    if let Some(c) = class.as_deref().filter(|c| **c != ws.data_classification) {
        widens |= classification_rank(c) < classification_rank(&ws.data_classification);
        parts.push(format!("Change data classification from {} to {c}.", ws.data_classification));
    }
    if department.as_deref().is_some_and(|d| d != ws.department) {
        // Who owns a workspace decides who may be asked about it, so a move is treated as a widening.
        widens = true;
        parts.push("Change its department.".to_string());
    }
    if let Some(u) = unit.filter(|u| ws.organization_id != Some(*u)) {
        widens = true;
        parts.push(format!("Move it to the unit {u}."));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    let settings: serde_json::Map<String, Value> = ["name", "description", "department", "visibility", "data_classification", "icon", "organization_id"].iter().filter_map(|k| args.get(*k).filter(|v| v.is_string()).map(|v| ((*k).to_string(), v.clone()))).collect();
    let change = json!({ "kind": "workspace_settings", "workspace_id": id, "settings": settings });
    drafted(state, caller, origin, "update_workspace", change, widens, format!("Workspace {id}: {}", parts.join(" "))).map(Some)
}

/// Drafts the collaborator change `manage_workspace_member` was asked for. Nothing is applied.
pub fn draft_member(state: &SharedState, caller: &AuthUser, origin: &Origin, args: &Value) -> Result<Value, ServiceError> {
    let id = text(args, "workspace_id").ok_or_else(|| ServiceError::BadRequest("Argument 'workspace_id' is required".to_string()))?;
    let eppn = text(args, "eppn").filter(|e| !e.is_empty()).ok_or_else(|| ServiceError::BadRequest("Argument 'eppn' is required".to_string()))?;
    let action = text(args, "action").unwrap_or_default().to_lowercase();
    let role = match text(args, "role") {
        Some(r) => Some(Role::parse(&r).ok_or_else(|| ServiceError::BadRequest("role must be owner, admin, editor, commenter, or viewer".to_string()))?),
        None => None,
    };
    let current = state
        .collaborators
        .read()
        .map_err(internal)?
        .get(&id)
        .and_then(|c| c.iter().find(|c| c.eppn.eq_ignore_ascii_case(&eppn) && c.app_slug.is_empty()).and_then(|c| Role::parse(&c.role)));
    if !state.workspaces.read().map_err(internal)?.contains_key(&id) {
        return Err(ServiceError::NotFound(format!("Workspace '{id}' not found")));
    }
    let (widens, sentence) = match action.as_str() {
        "add" => (true, format!("Add {eppn} to workspace {id} as {}.", role.map_or("a viewer", Role::as_str))),
        "update" => {
            let new = role.ok_or_else(|| ServiceError::BadRequest("Role is required for update".to_string()))?;
            let old = current.ok_or_else(|| ServiceError::NotFound(format!("{eppn} is not a collaborator of workspace {id}")))?;
            (new > old, format!("Change the role of {eppn} in workspace {id} from {} to {}.", old.as_str(), new.as_str()))
        }
        "remove" | "delete" => (false, format!("Remove {eppn} from workspace {id}.")),
        other => return Err(ServiceError::BadRequest(format!("Unsupported member action: {other}. Expected 'add', 'update', or 'remove'."))),
    };
    let change = json!({ "kind": "workspace_member", "workspace_id": id, "action": action, "eppn": eppn, "role": role.map(Role::as_str), "name": text(args, "name") });
    drafted(state, caller, origin, "manage_workspace_member", change, widens, sentence)
}

/// What a confirmed change does, run as the person who confirms it.
fn apply(state: &SharedState, caller: &AuthUser, change: &Value) -> Result<(), ServiceError> {
    let id = text(change, "workspace_id").ok_or_else(|| internal("The stored change has no workspace"))?;
    match change["kind"].as_str() {
        Some("workspace_settings") => {
            let s = &change["settings"];
            let payload = UpdateWorkspacePayload {
                name: text(s, "name"),
                description: text(s, "description"),
                department: text(s, "department"),
                visibility: text(s, "visibility"),
                data_classification: text(s, "data_classification"),
                icon: text(s, "icon"),
                organization_id: text(s, "organization_id").and_then(|u| Uuid::parse_str(&u).ok()),
                reason: None,
            };
            workspaces::update_workspace(caller, &id, payload, state).map(|_| ())
        }
        Some("workspace_member") => {
            let (action, eppn) = (text(change, "action").unwrap_or_default(), text(change, "eppn").unwrap_or_default());
            workspaces::manage_workspace_member(caller, &id, &action, &eppn, change["role"].as_str(), change["name"].as_str(), state).map(|_| ())
        }
        _ => Err(internal("The stored change is not a workspace change")),
    }
}

/// Confirms or rejects a workspace draft. The person it was drafted for cannot confirm a change that widens access. The
/// ledger entry names the proposer (the person, the client, the tool and the task) and the decider.
pub fn decide(state: &SharedState, caller: &AuthUser, id: Uuid, change: &Value, proposed_by: &str, confirm: bool, reason: &str) -> Result<Value, ServiceError> {
    let ws_id = text(change, "workspace_id").ok_or_else(|| internal("The stored change has no workspace"))?;
    authorize_workspace(caller, &ws_id, "manage_workspace", state).map_err(|_| ServiceError::forbidden("You do not hold the right to decide this change"))?;
    let (via, widens, expired) = state.repository.access_change_flags(id).map_err(internal)?.unwrap_or_else(|| ("person".to_string(), true, false));
    if confirm && expired {
        return Err(ServiceError::conflict("That draft expired. A draft made by an AI tool cannot be confirmed after seven days. Ask for it again"));
    }
    let narrowing_draft = via == "agent" && !widens;
    if confirm && proposed_by.eq_ignore_ascii_case(&caller.eppn) && !narrowing_draft {
        return Err(ServiceError::forbidden("The person who proposed a change cannot confirm it"));
    }
    let decide = || -> Result<crate::repository_grants::ChangeDecision, ServiceError> {
        if confirm {
            apply(state, caller, change)?;
        }
        state.repository.decide_access_change(id, &caller.eppn, confirm).map_err(internal)
    };
    let outcome = if confirm {
        let payload = json!({ "action": "workspace_change_confirmed", "change_id": id, "change": { "kind": change["kind"], "workspace_id": ws_id, "sentence": change["sentence"] }, "proposed_by": proposed_by, "proposer": crate::service::access_drafts::proposer(state, id) });
        crate::service::admin::admin_write(state, caller, DecisionType::AccessGrantChanged, "AC-02", reason, &payload, decide)?
    } else {
        decide()?
    };
    use crate::repository_grants::ChangeDecision;
    match outcome {
        ChangeDecision::Done { .. } => Ok(json!({ "change_id": id, "status": if confirm { "Confirmed" } else { "Rejected" }, "grant_id": Value::Null })),
        ChangeDecision::Missing => Err(ServiceError::NotFound("Change not found".to_string())),
        ChangeDecision::AlreadyDecided => Err(ServiceError::conflict("That change has already been decided")),
        ChangeDecision::OwnChange => Err(ServiceError::forbidden("The person who proposed a change cannot confirm it")),
        ChangeDecision::Expired => Err(ServiceError::conflict("That draft expired. A draft made by an AI tool cannot be confirmed after seven days. Ask for it again")),
    }
}
