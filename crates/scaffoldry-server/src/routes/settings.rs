use crate::guard::session_user;
use crate::state::{RecordDecisionInput, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use crate::service::ServiceError;

pub const ALLOWED_KEYS: &[&str] = &[
    "oidc.issuer",
    "oidc.audience",
    "oidc.jwks",
    "cors.allowed_origins",
    "tokens.max_days",
    "tokens.agent_enabled",
    "process.stale_days",
    "mcp.disabled_tools",
    "pages.disabled",
];

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/settings", get(get_settings))
        .route("/settings/{key}", put(put_setting))
}

pub fn validate_setting(key: &str, value: &Value) -> bool {
    match key {
        "oidc.issuer" | "oidc.audience" => value.is_string(),
        "oidc.jwks" => value.is_object(),
        "cors.allowed_origins" => {
            value.as_array().is_some_and(|arr| arr.iter().all(|v| v.is_string()))
        }
        "tokens.max_days" | "process.stale_days" => {
            value.as_i64().is_some_and(|n| (1..=365).contains(&n))
        }
        "tokens.agent_enabled" => value.is_boolean(),
        "mcp.disabled_tools" | "pages.disabled" => {
            value.as_array().is_some_and(|arr| arr.iter().all(|v| v.is_string()))
        }
        _ => false,
    }
}

async fn get_settings(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user = match session_user(&state, &headers) {
        Some(u) => u,
        None => {
            return ServiceError::unauthorized("401 Unauthorized").into_pair()
                .into_response();
        }
    };

    if user.affiliation != "central_admin" {
        return ServiceError::forbidden("Forbidden: Platform Admin only").into_pair()
            .into_response();
    }

    let settings = state.settings.read().unwrap().clone();
    (StatusCode::OK, Json(settings)).into_response()
}

async fn put_setting(
    State(state): State<SharedState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let user = match session_user(&state, &headers) {
        Some(u) => u,
        None => {
            return ServiceError::unauthorized("401 Unauthorized").into_pair()
                .into_response();
        }
    };

    if user.affiliation != "central_admin" {
        return ServiceError::forbidden("Forbidden: Platform Admin only").into_pair()
            .into_response();
    }

    // Closed list. Reject any other key with 400. Validate value type.
    if !ALLOWED_KEYS.contains(&key.as_str()) || !validate_setting(&key, &payload) {
        return ServiceError::bad_request(format!("Invalid setting key or value: '{}'", key)).into_pair()
            .into_response();
    }

    // Compute SHA-256 of new value
    let val_bytes = serde_json::to_vec(&payload).unwrap_or_default();
    let val_digest = Sha256::digest(&val_bytes);
    let mut val_hash = String::with_capacity(64);
    for b in val_digest {
        use std::fmt::Write;
        let _ = write!(val_hash, "{:02x}", b);
    }

    let ledger_payload = json!({
        "key": key,
        "value_sha256": val_hash,
    });

    // Append ledger entry, DecisionType::PolicyRevision
    if let Err(e) = state.append_ledger_entry(RecordDecisionInput {
        principal: user.eppn.clone(),
        organization_code: "INST".to_string(),
        app_slug: None,
        decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
        oscal_control_id: "AC-03".to_string(),
        rationale: format!("Platform setting '{}' updated by {}", key, user.eppn),
        payload: &ledger_payload,
    }) {
        return ServiceError::internal(format!("Ledger append failed: {e}")).into_pair()
            .into_response();
    }

    // Write row to platform_settings
    if let Some(ref repo) = state.repository {
        if let Err(e) = repo.put_platform_setting(&key, &payload, &user.eppn) {
            return ServiceError::internal(format!("Database write failed: {e}")).into_pair()
                .into_response();
        }
    }

    // Update in-memory state
    state.settings.write().unwrap().insert(key.clone(), payload.clone());

    (StatusCode::OK, Json(json!({ "key": key, "value": payload }))).into_response()
}
