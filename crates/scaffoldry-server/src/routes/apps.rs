use crate::guard::session_user;
use crate::state::{lock_err, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use scaffoldry_core::{AutomationRule, TriggerEvent, WorkflowExecutionResult};
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
    automations.entry(slug).or_default().push(rule.clone());

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
    let principal = payload["principal"].as_str().unwrap_or("dr.smith@university.edu");

    let auto_engine = AutomationEngine::new(state.policy_engine.clone());
    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();

    let results: Vec<WorkflowExecutionResult> = rules
        .iter()
        .map(|r| auto_engine.evaluate_rule(r, &event, &record, principal))
        .collect();

    Ok(Json(results))
}

