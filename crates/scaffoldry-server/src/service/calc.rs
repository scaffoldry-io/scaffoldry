//! Changing what a field holds: the dry run of a type conversion.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::recalc::conversion_supported;
use scaffoldry_engine::{FieldType, HostRouter};
use serde_json::{json, Value};

/// Queues a dry run of changing a field's type: how many values would convert, how many would become
/// blank, and the first 50 of each. For an owner or admin of the app. Nothing is written.
pub fn convert_preview(caller: &AuthUser, app_slug: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let table_id = body.get("table_id").and_then(Value::as_str).ok_or_else(|| ServiceError::BadRequest("table_id is required".to_string()))?;
    let field = body.get("field").and_then(Value::as_str).ok_or_else(|| ServiceError::BadRequest("field is required".to_string()))?;
    let to: FieldType = serde_json::from_value(body.get("to_type").cloned().unwrap_or(Value::Null)).map_err(|_| ServiceError::BadRequest("to_type is not a field type".to_string()))?;
    let from = {
        let engine = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        let manifest = engine.resolve_by_slug(app_slug).ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
        let table = manifest.tables.iter().find(|t| t.id == table_id).ok_or_else(|| ServiceError::NotFound(format!("Table '{table_id}' not found")))?;
        table.fields.iter().find(|f| f.name == field).ok_or_else(|| ServiceError::NotFound(format!("Field '{field}' not found")))?.field_type
    };
    if !conversion_supported(from, to) {
        return Err(ServiceError::BadRequest(format!("A {from:?} field cannot be converted to {to:?}")));
    }
    let repo = &*state.repository;
    let payload = json!({ "app_slug": app_slug, "table_id": table_id, "field": field, "from": from, "to": to });
    let job = crate::jobs::enqueue(repo, "convert_preview", payload, &caller.eppn, None).map_err(|e| match e {
        crate::jobs::JobsError::Quota(message) => ServiceError::QuotaExceeded(message),
        other => ServiceError::Internal(other.to_string()),
    })?;
    Ok(json!({ "job_id": job }))
}
