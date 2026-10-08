use crate::guard::session_user;
use crate::state::{lock_err, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use std::str::FromStr;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{
    ActionEffect, ActionType, AutomationRule, DecisionType, ProcessInstance, ProcessStatus,
    StepKind, TriggerEvent, WorkflowExecutionResult,
};
use crate::state::RecordDecisionInput;
use serde::Deserialize;
use scaffoldry_engine::{AppManifest, AutomationEngine};
use scaffoldry_policy::PolicyDecision;
use serde_json::{json, Value};

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
    Path(_ws_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<AppManifest>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;

    let manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    let auth = state
        .policy_engine
        .authorize_departmental_action(
            &user.eppn,
            &user.affiliation,
            &user.department,
            "create_app",
            &manifest.slug,
            &manifest.department,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if auth.decision == PolicyDecision::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Forbidden: Cedar policy denied app creation"})),
        ));
    }

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_app_manifest(&manifest);
    }

    Ok((StatusCode::CREATED, Json(manifest)))
}

async fn get_app(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Json<AppManifest>, StatusCode> {
    use scaffoldry_engine::HostRouter;
    let engine = state.engine.read().unwrap();
    let manifest = engine.resolve_by_slug(&slug).ok_or(StatusCode::NOT_FOUND)?;
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

    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    manifest.slug = slug.clone();

    let auth = state
        .policy_engine
        .authorize_departmental_action(
            &user.eppn,
            &user.affiliation,
            &user.department,
            "update_app",
            &slug,
            &manifest.department,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if auth.decision == PolicyDecision::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Forbidden: Cedar policy denied app update"})),
        ));
    }

    let mut engine = state.engine.write().map_err(|_| lock_err())?;
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_app_manifest(&manifest);
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

    let auth = state
        .policy_engine
        .authorize_departmental_action(
            &user.eppn,
            &user.affiliation,
            &user.department,
            "publish_app",
            &slug,
            &manifest.department,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if auth.decision == PolicyDecision::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Forbidden: Cedar policy denied app publishing"})),
        ));
    }

    manifest.custom_domain = Some(domain);
    manifest.custom_domain_verified = true;

    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_app_manifest(&manifest);
    }

    Ok(Json(manifest))
}

async fn list_app_automations(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<AutomationRule>>, (StatusCode, Json<Value>)> {
    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();
    Ok(Json(rules))
}

async fn create_app_automation(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<AutomationRule>), (StatusCode, Json<Value>)> {
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

    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_workflow_automation(&rule);
    }

    Ok((StatusCode::CREATED, Json(rule)))
}

async fn simulate_app_automation(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Vec<WorkflowExecutionResult>>, (StatusCode, Json<Value>)> {
    let event: TriggerEvent = serde_json::from_value(payload["event"].clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": format!("Invalid trigger event: {e}")}))))?;
    let record = payload.get("record").cloned().unwrap_or(json!({}));
    let principal_eppn = payload["principal"].as_str().unwrap_or("dr.smith@university.edu");
    let aff_str = payload["affiliation"].as_str().unwrap_or("faculty");
    let affiliation = EduPersonAffiliation::from_str(aff_str).unwrap_or(EduPersonAffiliation::Faculty);
    let realm = principal_eppn.split('@').nth(1).unwrap_or("university.edu");
    let identity = EduPersonIdentity {
        eppn: principal_eppn.to_string(),
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
    let _user = session_user(&state, &headers)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Unauthorized"}))))?;
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

    let decision = payload.decision.to_lowercase();
    if decision != "approve" && decision != "reject" {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "Decision must be 'approve' or 'reject'"}))));
    }

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

    // Authorize via Cedar
    let affiliation = EduPersonAffiliation::from_str(&user.affiliation).unwrap_or(EduPersonAffiliation::Faculty);
    let realm = user.eppn.split('@').nth(1).unwrap_or("university.edu");
    let identity = EduPersonIdentity {
        eppn: user.eppn.clone(),
        realm: realm.to_string(),
        affiliations: vec![affiliation],
    };

    let auth_res = state.policy_engine.authorize_record_action(
        &identity,
        "approve",
        &instance.app_slug,
        "institutional",
        false,
    );
    let allowed = match auth_res {
        Ok(res) => res.decision == PolicyDecision::Allow,
        Err(_) => false,
    };
    if !allowed {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "Forbidden"}))));
    }

    // Load rule and waiting step
    let waiting_step_id = instance.waiting_step_id.clone().unwrap_or_default();
    let mut field_effects = Vec::new();

    {
        let automations = state.automations.read().map_err(|_| lock_err())?;
        if let Some(rules) = automations.get(&slug) {
            if let Some(rule) = rules.iter().find(|r| r.id == instance.rule_id) {
                if let Some(step) = rule.steps.iter().find(|s| s.id == waiting_step_id) {
                    if let StepKind::UserTask { approve, reject, .. } = &step.kind {
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

    // Apply field effects and write record
    if !field_effects.is_empty() {
        if let Ok(mut records) = state.records.write() {
            if let Some(app_records) = records.get_mut(&slug) {
                if let Some(rec) = app_records.iter_mut().find(|r| r.id == instance.record_id) {
                    scaffoldry_core::workflow::apply_field_effects(&mut rec.data, &field_effects);
                }
            }
        }
    }

    // Append ledger entry
    let decision_payload = json!({
        "instance_id": instance.id,
        "decision": decision,
    });
    let _ = state.append_ledger_entry(RecordDecisionInput {
        principal: user.eppn.clone(),
        organization_code: "institutional".to_string(),
        app_slug: Some(instance.app_slug.clone()),
        decision_type: DecisionType::WorkflowRuleApproved,
        oscal_control_id: "AC-03".to_string(),
        rationale: format!("{decision} by {} on {}", user.eppn, instance.id),
        payload: &decision_payload,
    });

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
