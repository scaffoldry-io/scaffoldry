//! Changing what a field holds: the dry run of a type conversion, and turning a value join into a link.

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

/// The value join an app has under this id, with the table and field it starts from.
fn relationship_of(app_slug: &str, relation_id: &str, state: &SharedState) -> Result<(scaffoldry_engine::AppManifest, scaffoldry_engine::TableRelationship), ServiceError> {
    // The stored definition, not the engine's: the engine's has every saved view merged in, a person's own among them.
    let manifest = state
        .repository
        .get_app_manifest(app_slug)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let relation = manifest
        .relationships
        .iter()
        .find(|r| r.id == relation_id)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("The app has no value join '{relation_id}'")))?;
    Ok((manifest, relation))
}

/// Queues a dry run of turning a value join into a link: how many records match one record of the other table, none, or
/// several, and the first 50 of each that do not match one. For an owner or admin of the app. Nothing is written.
pub fn relation_preview(caller: &AuthUser, app_slug: &str, relation_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let (_, relation) = relationship_of(app_slug, relation_id, state)?;
    let payload = json!({
        "app_slug": app_slug,
        "relation_id": relation.id,
        "table_id": relation.source_table_id,
        "field": relation.source_field,
        "target_table_id": relation.target_table_id,
        "target_field": relation.target_field,
    });
    let job = crate::jobs::enqueue(&state.repository, "relation_preview", payload, &caller.eppn, None).map_err(|e| match e {
        crate::jobs::JobsError::Quota(message) => ServiceError::QuotaExceeded(message),
        other => ServiceError::Internal(other.to_string()),
    })?;
    Ok(json!({ "job_id": job }))
}

/// Proposes turning a value join into a link: the joined text field becomes a `Link` field to the other table, and the
/// join is removed. The proposal is flagged `data_loss` when the preview found a record that matches none or several, or
/// when no preview was run. Approval queues the job that links the records. For an owner or admin of the app.
pub fn relation_convert(caller: &AuthUser, app_slug: &str, relation_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let (mut manifest, relation) = relationship_of(app_slug, relation_id, state)?;
    let workspace = manifest.workspace_id.clone().unwrap_or_default();
    let field = manifest
        .tables
        .iter_mut()
        .find(|t| t.id == relation.source_table_id)
        .and_then(|t| t.fields.iter_mut().find(|f| f.name == relation.source_field))
        .ok_or_else(|| ServiceError::BadRequest(format!("The join starts from '{}', which the app does not have", relation.source_field)))?;
    if field.field_type != FieldType::Text {
        return Err(ServiceError::BadRequest(format!("'{}' is a {:?} field. Only a text field can be turned into a link", relation.source_field, field.field_type)));
    }
    field.field_type = FieldType::Link;
    field.link_target_table = Some(relation.target_table_id.clone());
    field.link_cardinality = scaffoldry_engine::LinkCardinality::One;
    manifest.relationships.retain(|r| r.id != relation.id);
    let summary = format!("Turn the value join '{}' into a link", relation.name);
    crate::service::proposals::propose(
        caller,
        crate::service::proposals::ProposeInput {
            workspace_id: workspace,
            app_slug: None,
            manifest: Some(serde_json::to_value(&manifest).map_err(|e| ServiceError::Internal(e.to_string()))?),
            rules: vec![],
            summary,
            page_sources: vec![],
            notes: vec![],
        },
        state,
    )
}
