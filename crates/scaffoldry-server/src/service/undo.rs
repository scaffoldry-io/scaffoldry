//! Undo a batch, and restore a record's fields to an earlier version.

use crate::repository_undo::{UndoOutcome, UndoTable};
use crate::service::access::{authorize_app, AppAction};
use crate::service::history::{write_ctx, Origin};
use crate::service::records::table_fields_lenient;
use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use serde_json::{json, Value};
use std::collections::HashMap;

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    state.repository.as_deref().ok_or_else(|| ServiceError::Internal("Undo needs PostgreSQL".to_string()))
}

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// Puts a batch back as it was, as a new batch. The person who made the batch, or an owner or admin
/// of the app, can do it, and only while every record still holds what the batch wrote.
pub fn undo_batch(caller: &AuthUser, origin: &Origin, app_slug: &str, batch_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let orig = uuid::Uuid::parse_str(batch_id).map_err(|_| ServiceError::BadRequest("batch id is not one this server issued".to_string()))?;
    let repo = repo(state)?;
    let entries = repo.batch_entries(app_slug, batch_id).map_err(internal)?;
    if entries.is_empty() {
        return Err(ServiceError::NotFound("That batch was not found".to_string()));
    }
    let own = entries.iter().all(|e| e.actor.eq_ignore_ascii_case(&caller.eppn));
    if !own && authorize_app(caller, app_slug, AppAction::Manage, state).is_err() {
        return Err(ServiceError::forbidden("Only the person who made the change, or an owner or admin of the app, can undo it."));
    }

    let new_batch = uuid::Uuid::new_v4();
    let mut tables: HashMap<String, UndoTable> = HashMap::new();
    for entry in &entries {
        if tables.contains_key(&entry.table_id) {
            continue;
        }
        let mut ctx = write_ctx(&caller.eppn, origin, app_slug, &json!({ "_table_id": entry.table_id }), state);
        ctx.batch_id = new_batch;
        ctx.reverts_batch = Some(orig);
        tables.insert(entry.table_id.clone(), UndoTable { ctx });
    }
    match repo.undo_batch(app_slug, batch_id, tables).map_err(internal)? {
        UndoOutcome::Undone { records } => Ok(json!({ "batch_id": new_batch, "reverts_batch": orig, "records": records })),
        UndoOutcome::Conflicts(found) => Err(ServiceError::UndoConflict {
            message: "Someone changed this since. Undo stopped.".to_string(),
            conflicts: found.into_iter().map(|c| json!({ "record_id": c.record_id, "fields": c.fields })).collect(),
        }),
    }
}

/// Writes the values some fields had at `to_version` back into the record, as one new version with the
/// action `restore`. The save names the version it read, like any other save.
pub fn restore_record(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    id: &str,
    payload: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let to_version = payload.get("to_version").and_then(Value::as_i64).filter(|v| *v >= 1).ok_or_else(|| ServiceError::Invalid {
        message: "to_version is required: the version to restore from".to_string(),
        fields: std::collections::BTreeMap::from([("to_version".to_string(), "required".to_string())]),
    })?;
    let expected = payload
        .get("version")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| ServiceError::Invalid {
            message: "version is required: send the version you read".to_string(),
            fields: std::collections::BTreeMap::from([("version".to_string(), "required".to_string())]),
        })?;

    let current = state
        .find_record(app_slug, id)
        .map_err(internal)?
        .filter(|r| crate::service::scope::allows(caller, app_slug, r, scaffoldry_core::access::RowOp::Update, state))
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;
    let entries = repo(state)?.record_entries(app_slug, id).map_err(internal)?;
    if !entries.iter().any(|e| i64::from(e.version) == to_version) {
        return Err(ServiceError::BadRequest(format!("This record has no version {to_version}")));
    }

    // The record as it stood at that version: each entry up to it sets the values it wrote.
    let mut then = serde_json::Map::new();
    for entry in entries.iter().filter(|e| i64::from(e.version) <= to_version && e.action != "comment") {
        for (field, change) in entry.changes.as_object().into_iter().flatten() {
            match change.get("new") {
                Some(value) => then.insert(field.clone(), value.clone()),
                None => then.remove(field),
            };
        }
    }

    let table = current.data.get("_table_id").and_then(Value::as_str).unwrap_or("").to_string();
    let (fields, _) = table_fields_lenient(app_slug, &table, state);
    let writable = |name: &str| fields.iter().any(|f| f.name == name && !f.field_type.is_computed());
    let now = current.data.as_object().cloned().unwrap_or_default();
    let chosen: Vec<String> = match payload.get("fields").and_then(Value::as_array) {
        Some(list) => {
            let names: Vec<String> = list.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            if let Some(bad) = names.iter().find(|n| !writable(n)) {
                return Err(ServiceError::BadRequest(format!("'{bad}' is not a field of this table that can be restored")));
            }
            names
        }
        None => then.keys().chain(now.keys()).filter(|k| writable(k)).cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
    };

    let mut data = now.clone();
    for field in &chosen {
        match then.get(field) {
            Some(value) => data.insert(field.clone(), value.clone()),
            None => data.remove(field),
        };
    }
    if data == now {
        return Err(ServiceError::BadRequest(format!("Those fields already hold what they held in version {to_version}")));
    }

    let data = Value::Object(data);
    if !crate::service::scope::allows_data(caller, app_slug, &table, &data, scaffoldry_core::access::RowOp::Update, state) {
        return Err(ServiceError::forbidden("You cannot save a record that is outside the rows you have access to."));
    }
    let mut ctx = write_ctx(&caller.eppn, origin, app_slug, &data, state);
    ctx.action = Some("restore");
    let (outcome, saved) = state.save_record_checked(app_slug, id, &data, expected, &ctx).map_err(internal)?;
    match (outcome, saved) {
        (crate::repository::RecordWrite::Saved(_), Some(rec)) => Ok(rec),
        (crate::repository::RecordWrite::Conflict(version), _) => Err(ServiceError::StaleVersion {
            message: "Someone else saved this record first. Reload it and try again.".to_string(),
            version,
        }),
        _ => Err(ServiceError::NotFound(format!("Record '{id}' not found"))),
    }
}
