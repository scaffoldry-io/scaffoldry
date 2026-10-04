//! Application and Dataset Manifest Management Endpoints

use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces/{id}/apps", post(create_app_in_workspace))
        .route("/apps/{slug}", get(get_app).put(update_app))
        .route("/apps/{slug}/publish", post(publish_app))
}

async fn create_app_in_workspace(
    State(state): State<SharedState>,
    Path(_ws_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<AppManifest>), (StatusCode, Json<Value>)> {
    let manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    let mut engine = state.engine.write().unwrap();
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

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
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    let mut manifest: AppManifest = serde_json::from_value(payload)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    manifest.slug = slug;
    let mut engine = state.engine.write().unwrap();
    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(manifest))
}

async fn publish_app(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<AppManifest>, (StatusCode, Json<Value>)> {
    use scaffoldry_engine::HostRouter;
    let domain = payload["custom_domain"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "custom_domain is required"}))))?
        .to_string();

    let mut engine = state.engine.write().unwrap();
    let mut manifest = engine
        .resolve_by_slug(&slug)
        .cloned()
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "App not found"}))))?;

    manifest.custom_domain = Some(domain);
    manifest.custom_domain_verified = true;

    engine
        .register_manifest(manifest.clone())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(manifest))
}
