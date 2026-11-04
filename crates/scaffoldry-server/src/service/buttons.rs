//! Field types phase 5: a button field starts the rule it names, with its `Manual` trigger. The button stores nothing.
//!
//! Pressing checks that the person may read the record and has `run_automation` on it, and that the record meets the
//! rule's conditions. A press while the rule's instance for the record still waits returns that instance and starts no
//! second one. The changes a press makes are an `automation` batch in history, by the person who pressed.

use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{AutomationRule, ProcessStatus, TriggerEvent};
use scaffoldry_engine::{AppManifest, FieldType, HostRouter};
use serde_json::{json, Value};

struct Button {
    field: String,
    label: String,
    rule: Option<AutomationRule>,
}

fn manifest(slug: &str, state: &SharedState) -> Result<AppManifest, ServiceError> {
    state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.resolve_by_slug(slug).cloned().ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))
}

/// The buttons of the record's table, each with the rule it starts.
fn buttons_of(m: &AppManifest, rec: &DatasetRecord, state: &SharedState) -> Vec<Button> {
    let table = rec.data.get("_table_id").and_then(Value::as_str).unwrap_or_default();
    let rules = state.automations.read().map(|a| a.get(&m.slug).cloned().unwrap_or_default()).unwrap_or_default();
    m.tables
        .iter()
        .filter(|t| t.id == table || table.is_empty())
        .flat_map(|t| t.fields.iter())
        .filter(|f| f.field_type == FieldType::Button)
        .map(|f| Button {
            field: f.name.clone(),
            label: f.label.clone(),
            rule: f.button_rule.as_deref().and_then(|id| rules.iter().find(|r| r.id == id && r.trigger == TriggerEvent::Manual).cloned()),
        })
        .collect()
}

/// Why the caller may not press a button on this record now, or `None`. Permission is asked first, then the conditions.
fn refusal(caller: &AuthUser, m: &AppManifest, rule: &AutomationRule, rec: &DatasetRecord, state: &SharedState) -> Option<ServiceError> {
    let resource = scaffoldry_policy::entities::Resource::Record(scaffoldry_policy::entities::RecordCtx {
        app_slug: m.slug.clone(),
        department: m.department.clone(),
        workspace_id: m.workspace_id.clone().unwrap_or_default(),
        is_ferpa_sensitive: rec.is_ferpa_sensitive,
        categories: Default::default(),
    });
    let decision = crate::service::access::decide(state, caller, "run_automation", &resource);
    if !decision.allowed {
        let why = decision.policy.map(|p| p.description).filter(|d| !d.is_empty()).unwrap_or_else(|| "You may not run this app's processes on this record.".to_string());
        return Some(ServiceError::forbidden(why));
    }
    let mut data = rec.data.clone();
    if let Some(map) = data.as_object_mut() {
        map.insert("id".to_string(), Value::String(rec.id.clone()));
    }
    if !rule.predicates.iter().all(|p| scaffoldry_engine::AutomationEngine::evaluate_predicate(p, &data)) {
        return Some(ServiceError::conflict(format!("This record does not meet the button's conditions. It needs: {}.", scaffoldry_core::workflow::conditions_words(&rule.predicates))));
    }
    None
}

fn record(caller: &AuthUser, slug: &str, id: &str, state: &SharedState) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, slug, AppAction::Read, state)?;
    crate::service::scope::require_readable(caller, slug, id, state)?;
    crate::service::records::get_record(caller, slug, id, state)
}

/// The record's buttons, each with whether the caller may press it now and, if not, why.
pub fn list(caller: &AuthUser, slug: &str, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let m = manifest(slug, state)?;
    let rec = record(caller, slug, id, state)?;
    let out: Vec<Value> = buttons_of(&m, &rec, state)
        .into_iter()
        .map(|b| {
            let why = match &b.rule {
                None => Some("The rule this button starts is not live.".to_string()),
                Some(rule) => refusal(caller, &m, rule, &rec, state).map(|e| e.message().to_string()),
            };
            json!({ "field": b.field, "label": b.label, "enabled": why.is_none(), "reason": why })
        })
        .collect();
    Ok(json!({ "buttons": out }))
}

/// Presses a button: starts its rule for the record and returns the instance it waits in, or its run. `instance_id` is
/// null for a rule that made its changes and holds no instance.
pub fn press(caller: &AuthUser, slug: &str, id: &str, field: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let m = manifest(slug, state)?;
    let rec = record(caller, slug, id, state)?;
    let button = buttons_of(&m, &rec, state).into_iter().find(|b| b.field == field).ok_or_else(|| ServiceError::NotFound(format!("The record has no button '{field}'")))?;
    let rule = button.rule.ok_or_else(|| ServiceError::conflict(format!("The rule the button '{field}' starts is not live")))?;
    if let Some(e) = refusal(caller, &m, &rule, &rec, state) {
        return Err(e);
    }
    // A press while the rule waits on this record returns the waiting instance.
    let waiting_id = format!("{}:{}", rule.id, rec.id);
    if let Ok(Some(inst)) = state.repository.get_process_instance(&waiting_id) {
        if inst.status == ProcessStatus::Waiting {
            return Ok(json!({ "instance_id": inst.id, "started": false }));
        }
    }
    if crate::service::flows::is_flow(&rule) {
        let inst = crate::service::flows::run_flow(state, &rule, &rec, &caller.eppn, &uuid::Uuid::new_v4().to_string());
        return Ok(json!({ "instance_id": inst.id, "started": true, "status": format!("{:?}", inst.status) }));
    }
    let affiliation = match caller.affiliation.to_ascii_lowercase().as_str() {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };
    let identity = EduPersonIdentity { eppn: caller.eppn.clone(), realm: caller.eppn.split('@').nth(1).unwrap_or("university.edu").to_string(), affiliations: vec![affiliation] };
    let mut data = rec.data.clone();
    if let Some(map) = data.as_object_mut() {
        map.insert("id".to_string(), Value::String(rec.id.clone()));
    }
    // The other rules of the app count as applied, so the press runs this rule alone.
    let mut applied: Vec<String> = state.automations.read().map(|a| a.get(slug).map(|l| l.iter().filter(|r| r.id != rule.id).map(|r| r.id.clone()).collect()).unwrap_or_default()).unwrap_or_default();
    crate::service::records::run_automations(state, slug, TriggerEvent::Manual, &data, &crate::service::records::Actor { identity: &identity, department: &caller.department }, 0, &mut applied);
    let instance = state.repository.get_process_instance(&waiting_id).ok().flatten().map(|i| i.id);
    Ok(json!({ "instance_id": instance, "started": true }))
}
