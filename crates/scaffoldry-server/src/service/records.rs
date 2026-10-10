//! Sovereign Tabular Record Service Layer with Cedar Policy Enforcement

use crate::service::ServiceError;
use crate::service::access::{authorize_app, AppAction};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use chrono::Utc;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::TriggerEvent;
use scaffoldry_engine::{AutomationEngine, EngineError};
use serde_json::Value;

pub fn create_record(
    caller: &AuthUser,
    app_slug: &str,
    data: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
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

    let mut record_val = record.data.clone();
    if let Some(map) = record_val.as_object_mut() {
        map.insert("id".to_string(), Value::String(record.id.clone()));
    }
    let mut applied_rule_ids = Vec::new();
    run_automations(
        state,
        app_slug,
        TriggerEvent::RecordCreated,
        &record_val,
        &identity,
        0,
        &mut applied_rule_ids,
    );

    // Reload the record in case automations updated it
    let final_record = get_record(caller, app_slug, &record.id, state).unwrap_or(record);

    Ok(final_record)
}

pub fn list_records(
    caller: &AuthUser,
    app_slug: &str,
    state: &SharedState,
) -> Result<Vec<DatasetRecord>, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let records = state
        .records
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(records.get(app_slug).cloned().unwrap_or_default())
}

pub fn get_record(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
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
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    payload: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
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

    let updated_rec = record.clone();
    drop(records); // release lock before automations

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

    let mut record_val = updated_rec.data.clone();
    if let Some(map) = record_val.as_object_mut() {
        map.insert("id".to_string(), Value::String(updated_rec.id.clone()));
    }
    let mut applied_rule_ids = Vec::new();
    run_automations(
        state,
        app_slug,
        TriggerEvent::RecordUpdated,
        &record_val,
        &identity,
        0,
        &mut applied_rule_ids,
    );

    let final_rec = get_record(caller, app_slug, id, state).unwrap_or(updated_rec);
    Ok(final_rec)
}

pub fn delete_record(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
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

pub fn run_automations(
    state: &SharedState,
    app_slug: &str,
    event: TriggerEvent,
    record: &Value,
    identity: &EduPersonIdentity,
    depth: u8,
    applied_rule_ids: &mut Vec<String>,
) {
    if depth >= 3 {
        return;
    }

    let rules = {
        let automations = match state.automations.read() {
            Ok(m) => m,
            Err(_) => return,
        };
        automations.get(app_slug).cloned().unwrap_or_default()
    };

    let auto_engine = AutomationEngine::new(state.policy_engine.clone());
    let mut modified = record.clone();
    let mut all_effects = Vec::new();

    for rule in &rules {
        if !rule.enabled || applied_rule_ids.contains(&rule.id) {
            continue;
        }

        let res = auto_engine.evaluate_rule(rule, &event, &modified, identity, depth);
        if res.trigger_matched && res.conditions_met && res.cedar_authorized {
            applied_rule_ids.push(rule.id.clone());
            if !res.effects.is_empty() {
                scaffoldry_core::workflow::apply_field_effects(&mut modified, &res.effects);
                all_effects.extend(res.effects);
            }
            if let Some(inst) = res.waiting_instance {
                let should_upsert = match state.process_instances.read() {
                    Ok(map) => match map.get(&inst.id) {
                        Some(existing) => existing.status != scaffoldry_core::ProcessStatus::Waiting,
                        None => true,
                    },
                    Err(_) => false,
                };
                if should_upsert {
                    let _ = state.persist_process_instance(inst);
                }
            }
        }
    }

    if modified != *record {
        if let Some(record_id) = record.get("id").and_then(|v| v.as_str()) {
            if let Ok(mut records) = state.records.write() {
                if let Some(app_records) = records.get_mut(app_slug) {
                    if let Some(rec) = app_records.iter_mut().find(|r| r.id == record_id) {
                        rec.data = modified.clone();
                    }
                }
            }
        }

        if let Some(retrigger_event) = scaffoldry_core::workflow::effects_retrigger(&all_effects) {
            run_automations(
                state,
                app_slug,
                retrigger_event,
                &modified,
                identity,
                depth + 1,
                applied_rule_ids,
            );
        }
    }
}
