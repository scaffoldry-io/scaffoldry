use crate::guard::session_user;
use crate::state::SharedState;
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
    "jobs.schedules_disabled",
    "sensitivity.categories",
    "sensitivity.detectors",
    "platform.timezone",
    "files.max_mb",
    "files.app_quota_mb",
    "files.scanner",
    "files.require_scan",
    "import.max_rows",
    "import.max_mb",
];

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/settings", get(get_settings))
        .route("/settings/{key}", put(put_setting))
        .route("/settings/sensitivity/presets/{id}", axum::routing::post(enable_preset))
}

pub fn validate_setting(key: &str, value: &Value) -> bool {
    match key {
        "oidc.issuer" | "oidc.audience" => value.is_string(),
        "platform.timezone" => value.as_str().is_some_and(|s| !s.trim().is_empty()),
        "oidc.jwks" => value.is_object(),
        "cors.allowed_origins" => {
            value.as_array().is_some_and(|arr| arr.iter().all(|v| v.is_string()))
        }
        "tokens.max_days" | "process.stale_days" => {
            value.as_i64().is_some_and(|n| (1..=365).contains(&n))
        }
        "tokens.agent_enabled" | "files.require_scan" => value.is_boolean(),
        // No scanner is null. Otherwise `{ "kind": "clamd", "socket": "/run/clamav/clamd.ctl" }`.
        "files.scanner" => value.is_null() || crate::scan::Scanner::from_setting(Some(value)).is_some(),
        "files.max_mb" => value.as_i64().is_some_and(|n| (1..=crate::files::MAX_MAX_MB as i64).contains(&n)),
        "files.app_quota_mb" => value.as_i64().is_some_and(|n| (1..=10_000_000).contains(&n)),
        "import.max_rows" => value.as_i64().is_some_and(|n| (1..=10_000_000).contains(&n)),
        "import.max_mb" => value.as_i64().is_some_and(|n| (1..=10_000).contains(&n)),
        "mcp.disabled_tools" | "pages.disabled" | "jobs.schedules_disabled" => {
            value.as_array().is_some_and(|arr| arr.iter().all(|v| v.is_string()))
        }
        // The shape of these two is checked in full, with a path, where the state is known.
        "sensitivity.detectors" => scaffoldry_engine::sensitivity::validate_detectors(value).is_ok(),
        "sensitivity.categories" => value.is_array(),
        _ => false,
    }
}

/// A setting error as a 400 that names the path, such as `sensitivity.detectors[0].shape`.
fn invalid_at(key: &str, e: scaffoldry_engine::sensitivity::SettingsError) -> ServiceError {
    let path = format!("{key}{}", e.path);
    ServiceError::Invalid {
        message: format!("{path}: {}", e.message),
        fields: std::collections::BTreeMap::from([(path, e.message)]),
    }
}

fn stored(state: &SharedState, key: &str) -> Value {
    state.settings.read().unwrap().get(key).cloned().unwrap_or_else(|| json!([]))
}

/// Checks a save of either sensitivity setting against the other, so the two never disagree.
fn validate_sensitivity(state: &SharedState, key: &str, value: &Value) -> Result<(), ServiceError> {
    use scaffoldry_engine::sensitivity::{check_references, validate_categories, validate_detectors};
    if key == "sensitivity.detectors" {
        let detectors = validate_detectors(value).map_err(|e| invalid_at(key, e))?;
        // Every detector a stored category names must still exist after this save.
        let current: Vec<scaffoldry_engine::sensitivity::Category> =
            serde_json::from_value(stored(state, "sensitivity.categories")).unwrap_or_default();
        check_references(&current, &detectors).map_err(|e| invalid_at("sensitivity.categories", e))?;
        Ok(())
    } else {
        let detectors = validate_detectors(&stored(state, "sensitivity.detectors")).unwrap_or_default();
        validate_categories(value, &detectors).map_err(|e| invalid_at(key, e))?;
        Ok(())
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

    // The two sensitivity settings are checked in full, and an error names its path.
    if key == "sensitivity.categories" || key == "sensitivity.detectors" {
        if let Err(e) = validate_sensitivity(&state, &key, &payload) {
            return e.into_pair().into_response();
        }
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

    // The row, the counter, and the ledger entry commit together. Then this instance reloads, as every other will.
    let entry = match state.repository.put_platform_setting(
        &key,
        &payload,
        &user.eppn,
        crate::repository::AppendDecisionParams {
            principal: user.eppn.clone(),
            organization_code: "INST".to_string(),
            app_slug: None,
            decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
            oscal_control_id: "AC-03".to_string(),
            rationale: format!("Platform setting '{}' updated by {}", key, user.eppn),
            payload: json!({ "key": key, "value_sha256": val_hash }),
        },
    ) {
        Ok(entry) => entry,
        Err(e) => return ServiceError::internal(format!("Database write failed: {e}")).into_pair().into_response(),
    };
    state.ledger.write().unwrap_or_else(|p| p.into_inner()).push(entry);
    if let Err(e) = state.refresh() {
        return ServiceError::StorageUnavailable(format!("The setting was saved, but this server could not reload: {e}")).into_response();
    }

    (StatusCode::OK, Json(json!({ "key": key, "value": payload }))).into_response()
}

fn sha256_hex(value: &Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(value).unwrap_or_default());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Switches a preset on: copies its category and detectors into the two settings and writes one
/// ledger entry. A preset whose category or detector id is already there is a 409.
async fn enable_preset(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user) = session_user(&state, &headers) else {
        return ServiceError::unauthorized("401 Unauthorized").into_pair().into_response();
    };
    if user.affiliation != "central_admin" {
        return ServiceError::forbidden("Forbidden: Platform Admin only").into_pair().into_response();
    }
    let preset = match scaffoldry_engine::sensitivity::preset(&id) {
        None => return ServiceError::not_found(format!("No preset named '{id}'")).into_pair().into_response(),
        Some(Err(e)) => {
            return ServiceError::internal(format!("Preset '{id}' is invalid: {}", e.message)).into_pair().into_response()
        }
        Some(Ok(p)) => p,
    };

    // Both settings are locked in PostgreSQL while the preset is merged, so two requests take turns on any number of
    // instances. The merge, the writes, the counter, and the ledger entry commit together.
    let (preset_id, eppn) = (id.clone(), user.eppn.clone());
    let outcome = state.repository.update_settings_locked(&user.eppn, ["sensitivity.categories", "sensitivity.detectors"], move |[current_categories, current_detectors]| {
        let mut categories = current_categories.as_array().cloned().unwrap_or_default();
        let mut detectors = current_detectors.as_array().cloned().unwrap_or_default();

        if categories.iter().any(|c| c["id"] == preset.category.id.as_str()) {
            return Err(ServiceError::conflict(format!("The category '{}' is already in settings", preset.category.id)));
        }
        if let Some(d) = preset.detectors.iter().find(|d| detectors.iter().any(|x| x["id"] == d.id.as_str())) {
            return Err(ServiceError::conflict(format!("A detector with the id '{}' is already in settings", d.id)));
        }
        categories.push(serde_json::to_value(&preset.category).unwrap_or(Value::Null));
        for d in &preset.detectors {
            detectors.push(serde_json::to_value(d).unwrap_or(Value::Null));
        }
        let (categories, detectors) = (Value::Array(categories), Value::Array(detectors));

        // The merged settings must still be valid together.
        if let Err(e) = scaffoldry_engine::sensitivity::validate_detectors(&detectors)
            .and_then(|d| scaffoldry_engine::sensitivity::validate_categories(&categories, &d).map(|_| ()))
        {
            return Err(ServiceError::conflict(format!("The preset does not fit the current settings: {}", e.message)));
        }

        // The ledger entry holds hashes, never a value.
        let ledger = crate::repository::AppendDecisionParams {
            principal: eppn.clone(),
            organization_code: "INST".to_string(),
            app_slug: None,
            decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
            oscal_control_id: "RA-02".to_string(),
            rationale: format!("Sensitivity preset '{preset_id}' switched on by {eppn}"),
            payload: json!({
                "preset": preset_id,
                "categories_sha256": sha256_hex(&categories),
                "detectors_sha256": sha256_hex(&detectors),
            }),
        };
        Ok((
            vec![("sensitivity.categories".to_string(), categories), ("sensitivity.detectors".to_string(), detectors)],
            ledger,
        ))
    });
    let (updates, entry) = match outcome {
        Err(e) => return ServiceError::internal(format!("Database write failed: {e}")).into_pair().into_response(),
        Ok(Err(refusal)) => return refusal.into_pair().into_response(),
        Ok(Ok(done)) => done,
    };
    state.ledger.write().unwrap_or_else(|p| p.into_inner()).push(entry);
    if let Err(e) = state.refresh() {
        return ServiceError::StorageUnavailable(format!("The preset was saved, but this server could not reload: {e}")).into_response();
    }
    let value_of = |k: &str| updates.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
    (StatusCode::OK, Json(json!({ "preset": id, "categories": value_of("sensitivity.categories"), "detectors": value_of("sensitivity.detectors") }))).into_response()
}
