//! Lifecycle phase 2: restoring a retired field, and archiving an app or a workspace.
//!
//! A field a proposal removes is retired (`proposals::retire_removed`): its values stay in the records and nothing
//! serves it. Restoring it is a proposal, made here, that puts the field back with its values.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::HostRouter;
use serde_json::Value;

/// Proposes putting a retired field back. The values it kept come back with it. Not data loss, so an owner or admin
/// applies it at once, and anyone else's waits for review as any proposal does.
pub fn restore_field(caller: &AuthUser, app_slug: &str, table_id: &str, field: &str, reason: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(ServiceError::BadRequest("Argument 'reason' must say why the field comes back".to_string()));
    }
    // The stored definition, not the engine's: the engine's has every saved view merged in, a person's own among them.
    let mut manifest = state
        .repository
        .get_app_manifest(app_slug)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let workspace = manifest.workspace_id.clone().unwrap_or_default();
    let table = manifest
        .tables
        .iter_mut()
        .find(|t| t.id == table_id || t.slug == table_id)
        .ok_or_else(|| ServiceError::NotFound(format!("Table '{table_id}' not found")))?;
    let at = table
        .retired_fields
        .iter()
        .position(|f| f.name == field)
        .ok_or_else(|| ServiceError::BadRequest(format!("Field '{field}' of table '{}' is not retired, so there is nothing to restore", table.name)))?;
    let mut restored = table.retired_fields.remove(at);
    restored.retired_at = None;
    table.fields.push(restored);
    let proposed = crate::service::proposals::propose(
        caller,
        crate::service::proposals::ProposeInput {
            workspace_id: workspace,
            app_slug: None,
            manifest: Some(serde_json::to_value(&manifest).map_err(|e| ServiceError::Internal(e.to_string()))?),
            rules: vec![],
            summary: format!("Restore the field '{field}': {reason}"),
            page_sources: vec![],
            notes: vec![],
        },
        state,
    )?;
    let approved = crate::service::proposals::approve_if_direct(caller, &proposed, state)?;
    let mut out = proposed;
    out["approved"] = Value::Bool(approved);
    Ok(out)
}

/// Archives (`archive` true) or unarchives an app. Its workspace's owner or admin, or a Platform Admin, with a reason.
pub fn set_app_archived(caller: &AuthUser, app_slug: &str, archive: bool, reason: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let workspace = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(app_slug)
        .map(|m| m.workspace_id.clone())
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let manager = workspace.as_deref().is_some_and(|ws| matches!(crate::service::proposals::role_in(caller, ws, state).as_deref(), Some("owner" | "admin")));
    if !manager && !is_platform_admin(caller) {
        return Err(ServiceError::forbidden("Archiving an app needs its workspace's owner or admin, or a Platform Admin"));
    }
    let (decision, control) = if archive { (DecisionType::AppArchived, "SI-12") } else { (DecisionType::AppUnarchived, "SI-12") };
    change(caller, "app", app_slug, Some(app_slug), archive, reason, decision, control, state)
}

/// Archives or unarchives a workspace: its owner or admin, or a Platform Admin, with a reason. Its apps follow it.
pub fn set_workspace_archived(caller: &AuthUser, workspace_id: &str, archive: bool, reason: &str, state: &SharedState) -> Result<Value, ServiceError> {
    if !state.workspaces.read().map_err(|e| ServiceError::Internal(e.to_string()))?.contains_key(workspace_id) {
        return Err(ServiceError::NotFound(format!("Workspace '{workspace_id}' not found")));
    }
    let manager = matches!(crate::service::proposals::role_in(caller, workspace_id, state).as_deref(), Some("owner" | "admin"));
    if !manager && !is_platform_admin(caller) {
        return Err(ServiceError::forbidden("Archiving a workspace needs its owner or admin, or a Platform Admin"));
    }
    let decision = if archive { DecisionType::WorkspaceArchived } else { DecisionType::WorkspaceUnarchived };
    change(caller, "workspace", workspace_id, None, archive, reason, decision, "SI-12", state)
}

fn is_platform_admin(caller: &AuthUser) -> bool {
    caller.affiliation == "central_admin"
}

/// The ledger entry is written first, so a change the ledger cannot record does not happen.
#[allow(clippy::too_many_arguments)]
fn change(
    caller: &AuthUser,
    kind: &str,
    id: &str,
    app_slug: Option<&str>,
    archive: bool,
    reason: &str,
    decision: DecisionType,
    control: &str,
    state: &SharedState,
) -> Result<Value, ServiceError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(ServiceError::BadRequest("Argument 'reason' must say why, in 1 to 500 characters".to_string()));
    }
    if state.is_archived(kind, id) == archive {
        let now = if archive { "archived" } else { "not archived" };
        return Err(ServiceError::conflict(format!("This {kind} is {now} already")));
    }
    let payload = serde_json::json!({ "kind": kind, "id": id, "archived": archive });
    state
        .append_ledger_entry(crate::state::RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: String::new(),
            app_slug: app_slug.map(str::to_string),
            decision_type: decision,
            oscal_control_id: control.to_string(),
            rationale: reason.to_string(),
            payload: &payload,
        })
        .map_err(|e| ServiceError::Internal(format!("The ledger could not record this: {e}")))?;
    let repo = &state.repository;
    let changed = if archive { repo.archive(kind, id, reason, &caller.eppn) } else { repo.unarchive(kind, id) }.map_err(|e| ServiceError::Internal(e.to_string()))?;
    if !changed {
        return Err(ServiceError::conflict(format!("Another request changed this {kind} first")));
    }
    state.refresh().map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(serde_json::json!({ "kind": kind, "id": id, "archived": archive, "reason": reason }))
}
