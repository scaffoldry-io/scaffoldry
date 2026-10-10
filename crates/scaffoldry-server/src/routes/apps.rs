use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::service::organizations::{unit_in_scope, OrgCaller};
use crate::state::{lock_err, RecordDecisionInput, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{
    ActionEffect, ActionType, AutomationRule, DecisionType, ProcessInstance, ProcessStatus,
    StepKind, TriggerEvent, WorkflowExecutionResult,
};
use scaffoldry_engine::{AppManifest, AutomationEngine};
use scaffoldry_policy::{PolicyDecision, WorkspaceActionInput};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces/{id}/apps", post(create_app_in_workspace))
        .route("/apps/{slug}", get(get_app).put(update_app))
        .route("/apps/{slug}/publish", post(publish_app))
        .route("/apps/{slug}/automations", get(list_app_automations).post(create_app_automation))
        .route("/apps/{slug}/automations/simulate", post(simulate_app_automation))
        .route("/apps/{slug}/processes", get(list_app_processes))
        .route("/apps/{slug}/processes/{id}/decide", post(decide_app_process))
}

async fn create_app_in_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(ws_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<AppManifest>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    let ws = {
        let ws_guard = state.workspaces.read().map_err(|_| lock_err())?;
        ws_guard.get(&ws_id).cloned().ok_or_else(|| {
            ServiceError::not_found(format!("Workspace '{ws_id}' not found")).into_pair()
        })?
    };

    let collabs = {
        let collabs_guard = state.collaborators.read().map_err(|_| lock_err())?;
        collabs_guard.get(&ws_id).cloned().unwrap_or_default()
    };
    let (is_member, member_role) = match collabs.iter().find(|m| m.eppn == user.eppn) {
        Some(m) => (true, Some(m.role.clone())),
        None => (false, None),
    };
    let norm_role = member_role.as_ref().map(|r| r.to_lowercase());

    let decision = state
        .policy_engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: &user.eppn,
            principal_affiliation: &user.affiliation,
            principal_department: &user.department,
            action_name: "manage_app",
            workspace_id: &ws.id,
            workspace_department: &ws.department,
            workspace_visibility: &ws.visibility,
            is_member,
            member_role: norm_role.as_deref(),
        })
        .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;

    let mut allowed = decision.decision == PolicyDecision::Allow;
    if !allowed {
        if let Some(org_id) = ws.organization_id {
            let org_caller = OrgCaller {
                eppn: user.eppn.clone(),
                affiliation: user.affiliation.clone(),
            };
            let (orgs, roles) = {
                let o_guard = state.organizations.read().map_err(|_| lock_err())?;
                let r_guard = state.roles.read().map_err(|_| lock_err())?;
                let org_list: Vec<_> = o_guard.values().cloned().collect();
                (org_list, r_guard.clone())
            };
            if unit_in_scope(&org_caller, org_id, &orgs, &roles) {
                allowed = true;
            }
        }
    }

    if !allowed {
        return Err(ServiceError::forbidden("Forbidden: Cedar policy denied app creation in workspace").into_pair());
    }

    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| ServiceError::bad_request(e.to_string()).into_pair())?;

    manifest.workspace_id = Some(ws_id.clone());

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| ServiceError::bad_request(e.to_string()).into_pair())?;

    if let Some(ref repo) = state.repository {
        repo.upsert_app_manifest(&manifest)
            .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;
    }

    Ok((StatusCode::CREATED, Json(manifest)))
}

async fn get_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    use scaffoldry_engine::HostRouter;
    let manifest = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine
            .resolve_by_slug(&slug)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
    };
    Ok(Json(with_version(&state, &manifest)))
}

/// The manifest as JSON, with the version a save must name.
fn with_version(state: &SharedState, manifest: &AppManifest) -> Value {
    let version = state
        .repository
        .as_ref()
        .and_then(|r| r.app_manifest_version(&manifest.slug).ok().flatten())
        .unwrap_or(1);
    let mut v = serde_json::to_value(manifest).unwrap_or(Value::Null);
    if let Some(map) = v.as_object_mut() {
        map.insert("version".to_string(), Value::from(version));
    }
    v
}

async fn update_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    // A save names the version it read. Without one there is nothing to check it against.
    let expected_version = payload
        .get("version")
        .and_then(|v| v.as_i64())
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| {
            ServiceError::Invalid {
                message: "version is required: send the version you read".to_string(),
                fields: std::collections::BTreeMap::from([("version".to_string(), "required".to_string())]),
            }
            .into_pair()
        })?;

    use scaffoldry_engine::HostRouter;
    let existing = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine
            .resolve_by_slug(&slug)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
    };

    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| ServiceError::bad_request(e.to_string()).into_pair())?;

    manifest.slug = slug.clone();
    manifest.workspace_id = existing.workspace_id;
    manifest.department = existing.department;
    manifest.custom_domain = existing.custom_domain;
    manifest.custom_domain_verified = existing.custom_domain_verified;

    // The conditional update comes first. Only a save that wins reaches the running engine.
    if let Some(ref repo) = state.repository {
        match repo
            .update_app_manifest_checked(&manifest, expected_version)
            .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?
        {
            crate::repository::RecordWrite::Saved(_) => {}
            crate::repository::RecordWrite::Conflict(version) => {
                return Err(ServiceError::StaleVersion {
                    message: "Someone else saved this app first. Reload it and try again.".to_string(),
                    version,
                }
                .into_pair());
            }
            crate::repository::RecordWrite::Missing => {
                return Err(ServiceError::not_found("App not found").into_pair());
            }
        }
    }

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| ServiceError::bad_request(e.to_string()).into_pair())?;
    drop(engine);

    Ok(Json(with_version(&state, &manifest)))
}

async fn publish_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    let domain = payload["custom_domain"]
        .as_str()
        .ok_or_else(|| ServiceError::bad_request("custom_domain is required").into_pair())?
        .to_string();

    use scaffoldry_engine::HostRouter;
    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    let mut manifest = engine
        .resolve_by_slug(&slug)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?;

    manifest.custom_domain = Some(domain);
    manifest.custom_domain_verified = false;

    engine
        .register_manifest(manifest.clone())
        .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;

    if let Some(ref repo) = state.repository {
        repo.upsert_app_manifest(&manifest)
            .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;
    }

    Ok(Json(manifest))
}

async fn list_app_automations(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Vec<AutomationRule>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();
    Ok(Json(rules))
}

async fn create_app_automation(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<AutomationRule>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    let mut rule: AutomationRule = serde_json::from_value(payload)
        .map_err(|e| ServiceError::bad_request(e.to_string()).into_pair())?;
    rule.app_slug = slug.clone();
    crate::service::approvers::validate_rule(&state, &rule).map_err(ServiceError::into_pair)?;

    let mut automations = state.automations.write().map_err(|_| lock_err())?;
    let rules = automations.entry(slug).or_default();
    if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
        rules[pos] = rule.clone();
    } else {
        rules.push(rule.clone());
    }

    if let Some(repo) = state.repository.as_ref() {
        repo.upsert_workflow_automation(&rule).map_err(|e| {
            ServiceError::internal(e.to_string()).into_pair()
        })?;
    }

    Ok((StatusCode::CREATED, Json(rule)))
}

async fn simulate_app_automation(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Vec<WorkflowExecutionResult>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    let event: TriggerEvent = serde_json::from_value(payload["event"].clone())
        .map_err(|e| ServiceError::bad_request(format!("Invalid trigger event: {e}")).into_pair())?;
    let record = payload.get("record").cloned().unwrap_or(json!({}));

    let affiliation = EduPersonAffiliation::from_str(&user.affiliation)
        .unwrap_or(EduPersonAffiliation::Staff);
    let realm = user.eppn.split('@').nth(1).unwrap_or("university.edu");
    let identity = EduPersonIdentity {
        eppn: user.eppn.clone(),
        realm: realm.to_string(),
        affiliations: vec![affiliation],
    };

    let auto_engine = AutomationEngine;
    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();

    let results: Vec<WorkflowExecutionResult> = rules
        .iter()
        .map(|r| auto_engine.evaluate_rule(r, &event, &record, &identity, &user.department, 0))
        .collect();

    Ok(Json(results))
}

#[derive(Debug, Deserialize, Default)]
pub struct ProcessListQuery {
    pub status: Option<String>,
    /// Only the instances the caller can decide now.
    pub mine: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessDecisionPayload {
    pub decision: String,
}

async fn list_app_processes(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<ProcessListQuery>,
) -> Result<Json<Vec<Value>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let mut all: Vec<ProcessInstance> = state
        .process_instances
        .read()
        .map_err(|_| lock_err())?
        .values()
        .filter(|inst| inst.app_slug == slug)
        .cloned()
        .collect();
    // Who may decide changes as people are appointed, so the flag is refreshed when listed.
    for inst in all.iter_mut() {
        if crate::service::approvers::refresh_no_approver(&state, inst) {
            let _ = state.persist_process_instance(inst.clone());
        }
    }
    let filtered: Vec<Value> = all
        .iter()
        .filter(|inst| {
            if inst.app_slug != slug {
                return false;
            }
            if let Some(ref st) = query.status {
                let status_str = match inst.status {
                    ProcessStatus::Waiting => "Waiting",
                    ProcessStatus::Completed => "Completed",
                    ProcessStatus::Rejected => "Rejected",
                    ProcessStatus::Failed => "Failed",
                };
                if !status_str.eq_ignore_ascii_case(st) {
                    return false;
                }
            }
            if query.mine.unwrap_or(false)
                && !crate::service::approvers::caller_can_decide(&state, &user, inst)
            {
                return false;
            }
            true
        })
        .map(|inst| crate::service::approvers::instance_view(&state, inst))
        .collect();
    Ok(Json(filtered))
}

async fn decide_app_process(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<ProcessDecisionPayload>,
) -> Result<Json<ProcessInstance>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let decision = payload.decision.to_lowercase();
    if decision != "approve" && decision != "reject" {
        return Err(ServiceError::bad_request("Decision must be 'approve' or 'reject'").into_pair());
    }

    // Unknown affiliation is 403. Delete unwrap_or(EduPersonAffiliation::Faculty).
    let _ = EduPersonAffiliation::from_str(&user.affiliation)
        .map_err(|_| ServiceError::forbidden("Unknown affiliation").into_pair())?;

    let mut instance = {
        let instances = state.process_instances.read().map_err(|_| lock_err())?;
        instances.get(&id).cloned().ok_or_else(|| {
            ServiceError::not_found("Process instance not found").into_pair()
        })?
    };

    if instance.app_slug != slug {
        return Err(ServiceError::not_found("Process instance not found in this app").into_pair());
    }

    if instance.status != ProcessStatus::Waiting {
        return Err(ServiceError::conflict("Process instance is not in Waiting status").into_pair());
    }

    let waiting_step_id = instance.waiting_step_id.clone().unwrap_or_default();
    let mut required_role: Option<String> = None;
    let mut field_effects = Vec::new();

    {
        let automations = state.automations.read().map_err(|_| lock_err())?;
        if let Some(rules) = automations.get(&slug) {
            if let Some(rule) = rules.iter().find(|r| r.id == instance.rule_id) {
                if let Some(step) = rule.steps.iter().find(|s| s.id == waiting_step_id) {
                    if let StepKind::UserTask { ref role, ref approve, ref reject, .. } = step.kind {
                        required_role = Some(role.clone());
                        let actions = if decision == "approve" { approve } else { reject };
                        for act in actions {
                            match act {
                                ActionType::UpdateRecordStatus { new_status } => {
                                    field_effects.push(ActionEffect::SetFields {
                                        fields: vec![("status".into(), new_status.clone())],
                                    });
                                }
                                ActionType::NotifyCollaborator { role, message_template } => {
                                    instance.log.push(format!("Notified role '{role}': {message_template}"));
                                }
                                ActionType::CreateLedgerAuditEntry { summary, oscal_control } => {
                                    instance.log.push(format!("Appended audit log: {summary} (Control: {oscal_control})"));
                                }
                                ActionType::WebhookDispatch { target_url } => {
                                    instance.log.push(format!("Webhook not sent: {target_url}"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    required_role.ok_or_else(|| {
        ServiceError::bad_request("Waiting step not found in automations").into_pair()
    })?;

    // The resolver decides who may act. The submitter and the starter never may, and a Platform
    // Admin does not bypass it.
    if !crate::service::approvers::caller_can_decide(&state, &user, &instance) {
        return Err(ServiceError::forbidden("Forbidden: Caller may not decide this step").into_pair());
    }

    // Apply field effects and write the record. This is a system write, so it needs no version.
    if !field_effects.is_empty() {
        let current = state
            .find_record(&slug, &instance.record_id)
            .map_err(|e| ServiceError::internal(e).into_pair())?;
        if let Some(rec) = current {
            let mut data = rec.data.clone();
            scaffoldry_core::workflow::apply_field_effects(&mut data, &field_effects);
            state
                .overwrite_record_data(&slug, &instance.record_id, &data)
                .map_err(|e| ServiceError::internal(e).into_pair())?;
        }
    }

    // Append ledger entry
    let decision_payload = json!({
        "instance_id": instance.id,
        "decision": decision,
    });
    state.append_ledger_entry(RecordDecisionInput {
        principal: user.eppn.clone(),
        organization_code: "institutional".to_string(),
        app_slug: Some(instance.app_slug.clone()),
        decision_type: DecisionType::WorkflowRuleApproved,
        oscal_control_id: "AC-03".to_string(),
        rationale: format!("{decision} by {} on {}", user.eppn, instance.id),
        payload: &decision_payload,
    }).map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;

    // Update instance status and clear waiting_step_id
    if decision == "approve" {
        instance.status = ProcessStatus::Completed;
    } else {
        instance.status = ProcessStatus::Rejected;
    }
    instance.waiting_step_id = None;

    state.persist_process_instance(instance.clone()).map_err(|e| {
        ServiceError::internal(e.to_string()).into_pair()
    })?;

    Ok(Json(instance))
}
