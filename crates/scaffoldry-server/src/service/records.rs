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
                    policy: None,
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
        created_by: Some(caller.eppn.clone()),
        version: 1,
    };

    state
        .insert_record(&record)
        .map_err(|e| ServiceError::Internal(format!("Failed to store the record: {e}")))?;

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
        &Actor { identity: &identity, department: &caller.department },
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
    state.list_app_records(app_slug).map_err(ServiceError::Internal)
}

pub fn get_record(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))
}

pub fn update_record(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    payload: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;

    // A save names the version it read. Without one there is nothing to check it against.
    let expected_version = match payload.get("version") {
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| version_required("version must be a whole number"))?,
        None => return Err(version_required("version is required: send the version you read")),
    };

    let current = state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;

    let mut new_data = match payload.get("data") {
        Some(d) => d.clone(),
        None => {
            let mut rest = payload.clone();
            if let Some(map) = rest.as_object_mut() {
                map.remove("version");
            }
            rest
        }
    };
    if let (Some(tid), Some(map)) = (current.data.get("_table_id").cloned(), new_data.as_object_mut()) {
        map.entry("_table_id".to_string()).or_insert(tid);
    }

    let (outcome, saved) = state
        .save_record_checked(app_slug, id, &new_data, expected_version)
        .map_err(ServiceError::Internal)?;
    let updated_rec = match (outcome, saved) {
        (crate::repository::RecordWrite::Saved(_), Some(rec)) => rec,
        (crate::repository::RecordWrite::Conflict(version), _) => {
            return Err(ServiceError::StaleVersion {
                message: "Someone else saved this record first. Reload it and try again.".to_string(),
                version,
            })
        }
        _ => return Err(ServiceError::NotFound(format!("Record '{id}' not found"))),
    };

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
        &Actor { identity: &identity, department: &caller.department },
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
    if state.remove_record(app_slug, id).map_err(ServiceError::Internal)? {
        Ok(())
    } else {
        Err(ServiceError::NotFound(format!("Record '{id}' not found")))
    }
}

/// Who triggered an automation: their identity and their stored department.
pub struct Actor<'a> {
    pub identity: &'a EduPersonIdentity,
    pub department: &'a str,
}

pub fn run_automations(
    state: &SharedState,
    app_slug: &str,
    event: TriggerEvent,
    record: &Value,
    actor: &Actor<'_>,
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

        let res = auto_engine.evaluate_rule(rule, &event, &modified, actor.identity, actor.department, depth);
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
                    let mut inst = inst;
                    inst.started_by = Some(actor.identity.eppn.clone());
                    inst.started_at = Utc::now().to_rfc3339();
                    crate::service::approvers::refresh_no_approver(state, &mut inst);
                    let _ = state.persist_process_instance(inst);
                }
            }
        }
    }

    if modified != *record {
        if let Some(record_id) = record.get("id").and_then(|v| v.as_str()) {
            let _ = state.overwrite_record_data(app_slug, record_id, &modified);
        }

        if let Some(retrigger_event) = scaffoldry_core::workflow::effects_retrigger(&all_effects) {
            run_automations(
                state,
                app_slug,
                retrigger_event,
                &modified,
                actor,
                depth + 1,
                applied_rule_ids,
            );
        }
    }
}

fn version_required(message: &str) -> ServiceError {
    ServiceError::Invalid {
        message: message.to_string(),
        fields: std::collections::BTreeMap::from([("version".to_string(), "required".to_string())]),
    }
}
