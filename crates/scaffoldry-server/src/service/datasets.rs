//! Data layer phase 1: an app table is a dataset with an id.
//!
//! This is a second way to name what the app path already serves. It adds no access rule. A list shows the datasets of the
//! apps the caller may read, and a record read goes through `records::get_record`, so the row rules and the field masks
//! are the ones the app path applies.

use crate::service::{my_apps, records, ServiceError};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use std::collections::HashMap;

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The datasets of the apps the caller may read.
pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let readable: HashMap<String, String> = my_apps::my_apps(caller, state)
        .into_iter()
        .filter_map(|a| Some((a["slug"].as_str()?.to_string(), a["title"].as_str().unwrap_or("").to_string())))
        .collect();
    let manifests: HashMap<String, scaffoldry_engine::AppManifest> =
        state.engine.read().map(|e| e.list_manifests()).unwrap_or_default().into_iter().filter(|m| readable.contains_key(&m.slug)).map(|m| (m.slug.clone(), m)).collect();
    let mut out = Vec::new();
    for (id, slug, table_id) in state.repository.list_datasets().map_err(internal)? {
        let Some(table) = manifests.get(&slug).and_then(|m| m.tables.iter().find(|t| t.id == table_id)) else { continue };
        out.push(json!({ "id": id, "app_slug": slug, "app_title": readable[&slug], "table_id": table_id, "name": table.name }));
    }
    Ok(out)
}

/// One record of a dataset. It is the row the app path returns for the same record, or not found.
pub fn get_record(caller: &AuthUser, dataset_id: &str, record_id: &str, state: &SharedState) -> Result<DatasetRecord, ServiceError> {
    let not_found = || ServiceError::NotFound("Not found".to_string());
    let (slug, table_id) = state.repository.dataset_by_id(dataset_id).map_err(internal)?.ok_or_else(not_found)?;
    let record = records::get_record(caller, &slug, record_id, state).map_err(|e| match e {
        ServiceError::Forbidden { .. } => not_found(),
        other => other,
    })?;
    // A record with no table belongs to the app's first table.
    let first = state.engine.read().ok().and_then(|e| e.resolve_by_slug(&slug).and_then(|m| m.tables.first().map(|t| t.id.clone()))).unwrap_or_default();
    let of = record.data.get("_table_id").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or(first);
    if of != table_id {
        return Err(not_found());
    }
    Ok(record)
}
