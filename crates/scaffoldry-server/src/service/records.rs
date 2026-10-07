//! Sovereign Tabular Record Service Layer with Cedar Policy Enforcement

use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use chrono::Utc;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_engine::EngineError;
use serde_json::Value;

pub fn create_record(
    caller: &AuthUser,
    app_slug: &str,
    data: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    let affiliation = match caller.affiliation.to_lowercase().as_str() {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };

    let realm = caller
        .eppn
        .split('@')
        .nth(1)
        .unwrap_or("university.edu")
        .to_string();

    let identity = EduPersonIdentity {
        eppn: caller.eppn.clone(),
        realm,
        affiliations: vec![affiliation],
    };

    let data_payload = data.get("data").unwrap_or(data);

    let submitted = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        engine
            .submit_record(&identity, app_slug, data_payload)
            .map_err(|e| match e {
                EngineError::AccessDenied(msg) => ServiceError::Forbidden {
                    message: format!("403 Forbidden: Cedar Policy denies record mutation: {msg}"),
                    reasons: vec![msg],
                    diagnostics: Vec::new(),
                },
                other => ServiceError::BadRequest(other.to_string()),
            })?
    };

    let record = DatasetRecord {
        id: submitted.id.to_string(),
        app_slug: app_slug.to_string(),
        data: submitted.data,
        ceds_mapping: submitted.ceds_mapping,
        is_ferpa_sensitive: submitted.is_ferpa_sensitive,
        created_at: Utc::now().to_rfc3339(),
    };

    {
        let mut records = state
            .records
            .write()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        records
            .entry(app_slug.to_string())
            .or_default()
            .push(record.clone());
    }

    Ok(record)
}

pub fn list_records(
    _caller: &AuthUser,
    app_slug: &str,
    state: &SharedState,
) -> Result<Vec<DatasetRecord>, ServiceError> {
    let records = state
        .records
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(records.get(app_slug).cloned().unwrap_or_default())
}

pub fn get_record(
    _caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    let records = state
        .records
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let app_records = records
        .get(app_slug)
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' records not found")))?;
    let record = app_records
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;
    Ok(record.clone())
}

pub fn update_record(
    _caller: &AuthUser,
    app_slug: &str,
    id: &str,
    payload: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    let mut records = state
        .records
        .write()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let app_records = records
        .get_mut(app_slug)
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let record = app_records
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;

    let current_table_id = record.data.get("_table_id").cloned();

    if let Some(new_data) = payload.get("data") {
        record.data = new_data.clone();
    } else {
        record.data = payload.clone();
    }

    if let (Some(tid), Some(map)) = (current_table_id, record.data.as_object_mut()) {
        if !map.contains_key("_table_id") {
            map.insert("_table_id".to_string(), tid);
        }
    }

    Ok(record.clone())
}

pub fn delete_record(
    _caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    let mut records = state
        .records
        .write()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let app_records = records
        .get_mut(app_slug)
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let initial_len = app_records.len();
    app_records.retain(|r| r.id != id);
    if app_records.len() == initial_len {
        return Err(ServiceError::NotFound(format!("Record '{id}' not found")));
    }
    Ok(())
}
