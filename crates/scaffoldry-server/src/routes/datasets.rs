use crate::guard::session_user;
use crate::state::SharedState;
use std::collections::HashMap;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use scaffoldry_core::{DatasetField, PublishedDataset};
use serde_json::{json, Value};
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/datasets", get(list_datasets).post(publish_dataset))
        .route("/datasets/{id}", get(get_dataset))
        // Data layer phase 1: an app table is a dataset with an id. These read through the app's own access.
        .route("/data/datasets", get(list_app_datasets))
        .route("/data/datasets/{id}/records/{record_id}", get(get_dataset_record))
}

async fn list_app_datasets(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let datasets = tokio::task::spawn_blocking(move || crate::service::datasets::list(&user, &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(Json(json!({ "total": datasets.len(), "datasets": datasets })))
}

async fn get_dataset_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((id, record_id)): Path<(String, String)>,
) -> Result<Json<crate::state::DatasetRecord>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    tokio::task::spawn_blocking(move || crate::service::datasets::get_record(&user, &id, &record_id, &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn list_datasets(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    // The catalog (data layer phase 2d). Metadata and synthetic samples, for the datasets the caller may discover. Rows are
    // read through the dataset's own app, under its access rules.
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let search = crate::service::catalog::Search { query: q.get("query").cloned(), department: q.get("department").cloned() };
    let found = tokio::task::spawn_blocking(move || crate::service::catalog::search(&user, &search, &state))
        .await
        .map_err(|_| ServiceError::internal("The search stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(Json(found))
}

async fn get_dataset(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    tokio::task::spawn_blocking(move || crate::service::catalog::get(&user, &id, &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn publish_dataset(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<PublishedDataset>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    let name = payload["name"]
        .as_str()
        .ok_or_else(|| ServiceError::bad_request("name is required").into_pair())?
        .to_string();

    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_lowercase().replace(' ', "_"));

    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let department = payload
        .get("department")
        .and_then(|v| v.as_str())
        .unwrap_or("Academic Department")
        .to_string();

    crate::service::access::require(
        &state,
        &user,
        "publish_dataset",
        &scaffoldry_policy::entities::Resource::System(scaffoldry_policy::entities::SystemCtx {
            id: id.clone(),
            department: department.clone(),
            is_ferpa_sensitive: false,
        }),
        "Forbidden: Cedar policy denied dataset publishing",
    )
    .map_err(|e| e.into_pair())?;

    let organization = payload
        .get("organization")
        .and_then(|v| v.as_str())
        .unwrap_or("University")
        .to_string();

    let sensitivity = payload
        .get("sensitivity_level")
        .and_then(|v| v.as_str())
        .unwrap_or("Directory")
        .to_string();

    let herm_id = payload
        .get("herm_capability_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let fields: Vec<DatasetField> = payload
        .get("fields")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let record_count = payload.get("record_count").and_then(|v| v.as_u64()).map(|c| c as usize).unwrap_or(0);

    let dataset = PublishedDataset {
        id: id.clone(),
        name,
        description,
        department,
        organization,
        sensitivity_level: sensitivity,
        herm_capability_id: herm_id,
        fields,
        record_count,
        published_at: Utc::now().to_rfc3339(),
    };

    state.persist_dataset(dataset.clone()).map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;
    Ok((StatusCode::CREATED, Json(dataset)))
}
