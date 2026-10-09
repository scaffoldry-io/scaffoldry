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
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    let ws = {
        let ws_guard = state.workspaces.read().map_err(|_| lock_err())?;
        ws_guard.get(&ws_id).cloned().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Workspace '{ws_id}' not found")})),
            )
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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

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
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Forbidden: Cedar policy denied app creation in workspace"})),
        ));
    }

    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    manifest.workspace_id = Some(ws_id.clone());

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_app_manifest(&manifest)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    }

    Ok((StatusCode::CREATED, Json(manifest)))
}

async fn get_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    use scaffoldry_engine::HostRouter;
    let engine = state.engine.read().map_err(|_| lock_err())?;
    let manifest = engine
        .resolve_by_slug(&slug)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "App not found"}))))?;
    Ok(Json(manifest.clone()))
}

async fn update_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    use scaffoldry_engine::HostRouter;
    let existing = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine
            .resolve_by_slug(&slug)
            .cloned()
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "App not found"}))))?
    };

    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    manifest.slug = slug.clone();
    manifest.workspace_id = existing.workspace_id;
    manifest.department = existing.department;
    manifest.custom_domain = existing.custom_domain;
    manifest.custom_domain_verified = existing.custom_domain_verified;

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_app_manifest(&manifest)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    }

    Ok(Json(manifest))
}

async fn publish_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    let domain = payload["custom_domain"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "custom_domain is required"}))))?
        .to_string();

    use scaffoldry_engine::HostRouter;
    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    let mut manifest = engine
        .resolve_by_slug(&slug)
        .cloned()
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "App not found"}))))?;

    manifest.custom_domain = Some(domain);
    manifest.custom_domain_verified = false;

    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        repo.upsert_app_manifest(&manifest)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    }

    Ok(Json(manifest))
}

async fn list_app_automations(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Vec<AutomationRule>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

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
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    let mut rule: AutomationRule = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;
    rule.app_slug = slug.clone();

    let mut automations = state.automations.write().map_err(|_| lock_err())?;
    let rules = automations.entry(slug).or_default();
    if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
        rules[pos] = rule.clone();
    } else {
        rules.push(rule.clone());
    }

    if let Some(repo) = state.repository.as_ref() {
        repo.upsert_workflow_automation(&rule).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
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
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    let event: TriggerEvent = serde_json::from_value(payload["event"].clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid trigger event: {e}")}))))?;
    let record = payload.get("record").cloned().unwrap_or(json!({}));

    let affiliation = EduPersonAffiliation::from_str(&user.affiliation)
        .unwrap_or(EduPersonAffiliation::Staff);
    let realm = user.eppn.split('@').nth(1).unwrap_or("university.edu");
    let identity = EduPersonIdentity {
        eppn: user.eppn.clone(),
        realm: realm.to_string(),
        affiliations: vec![affiliation],
    };

    let auto_engine = AutomationEngine::new(state.policy_engine.clone());
    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();

    let results: Vec<WorkflowExecutionResult> = rules
        .iter()
        .map(|r| auto_engine.evaluate_rule(r, &event, &record, &identity, 0))
        .collect();

    Ok(Json(results))
}

#[derive(Debug, Deserialize, Default)]
pub struct ProcessListQuery {
    pub status: Option<String>,
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
) -> Result<Json<Vec<ProcessInstance>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    let instances = state.process_instances.read().map_err(|_| lock_err())?;
    let filtered: Vec<ProcessInstance> = instances
        .values()
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
            true
        })
        .cloned()
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
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(|err| (err.status_code(), Json(json!({"error": err.message()}))))?;

    let decision = payload.decision.to_lowercase();
    if decision != "approve" && decision != "reject" {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "Decision must be 'approve' or 'reject'"}))));
    }

    // Unknown affiliation is 403. Delete unwrap_or(EduPersonAffiliation::Faculty).
    let _ = EduPersonAffiliation::from_str(&user.affiliation)
        .map_err(|_| (StatusCode::FORBIDDEN, Json(json!({"error": "Unknown affiliation"}))))?;

    let mut instance = {
        let instances = state.process_instances.read().map_err(|_| lock_err())?;
        instances.get(&id).cloned().ok_or_else(|| {
            (StatusCode::NOT_FOUND, Json(json!({"error": "Process instance not found"})))
        })?
    };

    if instance.app_slug != slug {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "Process instance not found in this app"}))));
    }

    if instance.status != ProcessStatus::Waiting {
        return Err((StatusCode::CONFLICT, Json(json!({"error": "Process instance is not in Waiting status"}))));
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

    let req_role = required_role.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Json(json!({"error": "Waiting step not found in automations"})))
    })?;

    // Check caller's collaborator role or affiliation equals the waiting step's role.
    // A Platform Admin does not bypass the role match.
    use scaffoldry_engine::HostRouter;
    let app_manifest = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine.resolve_by_slug(&slug).cloned().ok_or_else(|| {
            (StatusCode::NOT_FOUND, Json(json!({"error": "App not found"})))
        })?
    };
    let ws_id = app_manifest.workspace_id.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Json(json!({"error": "App has no workspace"})))
    })?;
    let collabs = {
        let collabs_guard = state.collaborators.read().map_err(|_| lock_err())?;
        collabs_guard.get(&ws_id).cloned().unwrap_or_default()
    };
    let caller_member_role = collabs.iter().find(|m| m.eppn == user.eppn).map(|m| m.role.clone());
    let role_matches = caller_member_role.as_ref().map(|r| r.eq_ignore_ascii_case(&req_role)).unwrap_or(false)
        || user.affiliation.eq_ignore_ascii_case(&req_role);

    if !role_matches {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden: Caller does not match waiting step role"}))));
    }

    // Apply field effects and write record
    let mut updated_record: Option<crate::state::DatasetRecord> = None;
    if !field_effects.is_empty() {
        if let Ok(mut records) = state.records.write() {
            if let Some(app_records) = records.get_mut(&slug) {
                if let Some(rec) = app_records.iter_mut().find(|r| r.id == instance.record_id) {
                    scaffoldry_core::workflow::apply_field_effects(&mut rec.data, &field_effects);
                    updated_record = Some(rec.clone());
                }
            }
        }
    }

    // decide_app_process writes the changed record with repo.upsert_record and returns the error.
    if let Some(ref rec) = updated_record {
        if let Some(ref repo) = state.repository {
            repo.upsert_record(rec).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
            })?;
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
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    // Update instance status and clear waiting_step_id
    if decision == "approve" {
        instance.status = ProcessStatus::Completed;
    } else {
        instance.status = ProcessStatus::Rejected;
    }
    instance.waiting_step_id = None;

    state.persist_process_instance(instance.clone()).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    Ok(Json(instance))
}
