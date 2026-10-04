//! Tabular Record Data Ingestion, Retrieval, and Modification

use crate::state::{DatasetRecord, SharedState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/records", get(list_records).post(create_record))
        .route(
            "/apps/{slug}/records/{id}",
            get(get_record).patch(update_record).delete(delete_record),
        )
}

async fn list_records(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let records = state.records.read().unwrap();
    let app_records = records.get(&slug).cloned().unwrap_or_default();
    let total = app_records.len();

    Json(json!({
        "app_slug": slug,
        "total": total,
        "records": app_records
    }))
}

async fn create_record(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<DatasetRecord>), (StatusCode, Json<Value>)> {
    let caller_eppn = payload
        .get("caller_eppn")
        .and_then(|v| v.as_str())
        .unwrap_or("user@university.edu");

    let caller_affiliation_str = payload
        .get("caller_affiliation")
        .and_then(|v| v.as_str())
        .unwrap_or("faculty");

    let affiliation = match caller_affiliation_str {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };

    let realm = caller_eppn.split('@').nth(1).unwrap_or("university.edu").to_string();

    let caller = EduPersonIdentity {
        eppn: caller_eppn.to_string(),
        realm,
        affiliations: vec![affiliation],
    };

    let data_payload = payload.get("data").unwrap_or(&payload);

    let submitted = {
        let engine = state.engine.read().unwrap();
        engine
            .submit_record(&caller, &slug, data_payload)
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))))?
    };

    let record = DatasetRecord {
        id: submitted.id.to_string(),
        app_slug: slug.clone(),
        data: submitted.data,
        ceds_mapping: submitted.ceds_mapping,
        is_ferpa_sensitive: submitted.is_ferpa_sensitive,
        created_at: Utc::now().to_rfc3339(),
    };

    let mut records = state.records.write().unwrap();
    records.entry(slug).or_default().push(record.clone());

    Ok((StatusCode::CREATED, Json(record)))
}

async fn get_record(
    State(state): State<SharedState>,
    Path((slug, id)): Path<(String, String)>,
) -> Result<Json<DatasetRecord>, StatusCode> {
    let records = state.records.read().unwrap();
    let app_records = records.get(&slug).ok_or(StatusCode::NOT_FOUND)?;
    let record = app_records.iter().find(|r| r.id == id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(record.clone()))
}

async fn update_record(
    State(state): State<SharedState>,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<DatasetRecord>, StatusCode> {
    let mut records = state.records.write().unwrap();
    let app_records = records.get_mut(&slug).ok_or(StatusCode::NOT_FOUND)?;
    let record = app_records.iter_mut().find(|r| r.id == id).ok_or(StatusCode::NOT_FOUND)?;

    if let Some(new_data) = payload.get("data") {
        record.data = new_data.clone();
    }

    Ok(Json(record.clone()))
}

async fn delete_record(
    State(state): State<SharedState>,
    Path((slug, id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let mut records = state.records.write().unwrap();
    let app_records = records.get_mut(&slug).ok_or(StatusCode::NOT_FOUND)?;
    let pos = app_records.iter().position(|r| r.id == id).ok_or(StatusCode::NOT_FOUND)?;
    app_records.remove(pos);
    Ok(StatusCode::NO_CONTENT)
}
