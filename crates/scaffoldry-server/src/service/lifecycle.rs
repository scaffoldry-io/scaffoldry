//! Lifecycle phase 2: restoring a retired field, and archiving an app or a workspace.
//!
//! A field a proposal removes is retired (`proposals::retire_removed`): its values stay in the records and nothing
//! serves it. Restoring it is a proposal, made here, that puts the field back with its values.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
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
