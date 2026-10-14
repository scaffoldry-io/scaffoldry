//! Export routes: initiate export and download generated export files.

use crate::files;
use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/exports", post(create_export))
        .route("/apps/{slug}/exports/{id}/download", get(download_export))
}

fn pair(e: ServiceError) -> (StatusCode, Json<Value>) {
    e.into_pair()
}

#[derive(Deserialize)]
pub struct ExportRequest {
    pub table_id: Option<String>,
    pub view_id: Option<String>,
    pub fields: Option<Vec<String>>,
    pub format: Option<String>,
    pub mock_row_count: Option<usize>,
    pub filter: Option<Value>,
}

async fn create_export(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<ExportRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::Read, &state).map_err(pair)?;
    crate::guard::origin(&state, &headers).map_err(pair)?;

    if let Some(count) = payload.mock_row_count {
        if count > 100_000 {
            return Err(pair(ServiceError::BadRequest(
                "Export exceeds limit of 100,000 records. Please narrow your query using a filter.".to_string(),
            )));
        }
    }

    let manifest = state.repository.get_app_manifest(&slug)
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?
        .ok_or_else(|| pair(ServiceError::NotFound(format!("App '{slug}' not found"))))?;

    let target_table_id = payload.table_id.clone().unwrap_or_else(|| {
        manifest.tables.first().map(|t| t.id.clone()).unwrap_or_else(|| "items".to_string())
    });

    if state.repository.table_has_more_than(&slug, &target_table_id, 100_000).unwrap_or(false) {
        return Err(pair(ServiceError::BadRequest(
            "Export exceeds limit of 100,000 records. Please narrow your query using a filter.".to_string(),
        )));
    }

    let field_vis = crate::service::scope::field_visibility_of(&user, &slug, &target_table_id, &state);
    let disallowed_fields = field_vis.hidden_names();

    let rules = crate::service::scope::rules_of(&slug, &state);
    let person = crate::service::scope::person_of(&user, &state);

    let format_str = payload.format.unwrap_or_else(|| "csv".to_string()).to_lowercase();
    let export_id = Uuid::new_v4();

    let job_id = crate::jobs::enqueue(
        &state.repository,
        "export_run",
        json!({
            "export_id": export_id,
            "app_slug": slug,
            "table_id": target_table_id,
            "fields": payload.fields,
            "format": format_str,
            "caller_eppn": user.eppn,
            "caller_affiliation": user.affiliation,
            "caller_person": person,
            "row_rules": rules,
            "disallowed_fields": disallowed_fields,
            "mock_row_count": payload.mock_row_count,
        }),
        &user.eppn,
        None,
    ).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok((StatusCode::OK, Json(json!({
        "export_id": export_id.to_string(),
        "job_id": job_id.to_string(),
        "status": "enqueued",
    }))))
}

async fn download_export(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::Read, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid export id".to_string())))?;

    let file_ref = state.repository.file_ref_any(id)
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?
        .ok_or_else(|| pair(ServiceError::NotFound("Export not found".to_string())))?;

    if file_ref.app_slug != slug {
        return Err(pair(ServiceError::NotFound("Export not found for this app".to_string())));
    }

    if file_ref.uploaded_by != user.eppn {
        return Err(pair(ServiceError::forbidden("Only the requester can download this export.")));
    }

    let dir = files::dir().ok_or_else(|| pair(ServiceError::Internal("File storage not configured".to_string())))?;
    let path = files::blob_path(&dir, &file_ref.sha256);
    let bytes = std::fs::read(&path).map_err(|e| pair(ServiceError::Internal(format!("Failed to read export file: {e}"))))?;

    let content_type = match file_ref.media_type.as_str() {
        "text/csv" => "text/csv; charset=utf-8",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/json" => "application/json; charset=utf-8",
        "application/x-ndjson" => "application/x-ndjson; charset=utf-8",
        _ => "application/octet-stream",
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"{}\"", file_ref.name)),
        ],
        bytes,
    ).into_response())
}
