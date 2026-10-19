//! Import phase 1 routes: upload, analyze, map, dry-run, and run.

use crate::files;
use crate::guard::session_user;
use crate::repository_files::NewRef;
use crate::repository_imports::NewImport;
use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
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
        .route("/apps/{slug}/imports", post(upload_import).layer(DefaultBodyLimit::disable()))
        .route("/apps/{slug}/imports/{id}", get(get_import))
        .route("/apps/{slug}/imports/{id}/analyze", post(analyze_import))
        .route("/apps/{slug}/imports/{id}/mapping", post(set_mapping))
        .route("/apps/{slug}/imports/{id}/dry-run", post(dry_run_import))
        .route("/apps/{slug}/imports/{id}/run", post(run_import))
        .route("/apps/{slug}/imports/{id}/rejected", get(download_rejected))
}

fn pair(e: ServiceError) -> (StatusCode, Json<Value>) {
    e.into_pair()
}

fn setting_mb(state: &SharedState, key: &str, default: u64) -> u64 {
    state.settings.read().ok().and_then(|s| s.get(key).and_then(Value::as_u64)).unwrap_or(default)
}

async fn upload_import(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    body: Body,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    crate::guard::origin(&state, &headers).map_err(pair)?;
    let repo = &state.repository;
    let dir = files::dir().ok_or_else(|| pair(ServiceError::BadRequest("File storage is not set up on this server.".to_string())))?;

    let max_mb = setting_mb(&state, "import.max_mb", 100);
    let limit_bytes = max_mb * 1024 * 1024;

    let declared: u64 = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| pair(ServiceError::BadRequest("Content-Length is required for an upload.".to_string())))?;

    if declared > limit_bytes {
        return Err(pair(ServiceError::BadRequest(format!("File exceeds import.max_mb limit of {} MB.", max_mb))));
    }

    let name = files::clean_name(&files::percent_decode(
        &headers.get("x-file-name").map(|v| String::from_utf8_lossy(v.as_bytes()).to_string()).unwrap_or_default(),
    ));

    if name.to_lowercase().ends_with(".xlsm") {
        return Err(pair(ServiceError::BadRequest("Spreadsheets containing macros (.xlsm) cannot be imported.".to_string())));
    }

    if files::blocked_name(&name) {
        return Err(pair(ServiceError::FileTypeBlocked(format!("{name} is a program or a web page. Programs and web pages cannot be uploaded."))));
    }

    let stored = files::store(&dir, body, declared, limit_bytes).await.map_err(|e| match e {
        files::StoreError::TooLarge => pair(ServiceError::BadRequest("The file is too large for the limit.".to_string())),
        files::StoreError::Blocked => pair(ServiceError::BadRequest("The file contains executable content.".to_string())),
        files::StoreError::Bad(msg) => pair(ServiceError::BadRequest(msg)),
        files::StoreError::Io(msg) => pair(ServiceError::Internal(msg)),
    })?;

    let blob_path = files::blob_path(&dir, &stored.sha256);
    if let Ok(file_bytes) = std::fs::read(&blob_path) {
        if let Err(e) = crate::import_jobs::check_spreadsheet_safety(&name, &file_bytes) {
            let _ = std::fs::remove_file(&blob_path);
            return Err(pair(ServiceError::BadRequest(e)));
        }
    }

    let file_ref_id = Uuid::new_v4();
    let new_ref = NewRef {
        id: file_ref_id,
        sha256: &stored.sha256,
        size: stored.size as i64,
        media_type: stored.media_type,
        app_slug: &slug,
        name: &name,
        uploaded_by: &user.eppn,
        scan_state: "skipped",
    };

    repo.insert_file_ref(new_ref).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    let import_id = Uuid::new_v4();
    let import = repo.insert_import(NewImport {
        id: import_id,
        app_slug: &slug,
        file_ref_id,
        target_table_id: None,
        created_by: &user.eppn,
    }).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    // Enqueue analyze job
    let job_id = crate::jobs::enqueue(
        repo,
        "import_analyze",
        json!({ "import_id": import_id, "app_slug": slug }),
        &user.eppn,
        None,
    ).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok((StatusCode::CREATED, Json(json!({
        "import_id": import.id,
        "id": import.id,
        "job_id": job_id,
        "status": import.status,
    }))))
}

async fn get_import(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::Read, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let import = state.repository.get_import(&slug, id)
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?
        .ok_or_else(|| pair(ServiceError::NotFound(format!("Import {id} not found"))))?;

    Ok(Json(json!(import)))
}

async fn analyze_import(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let job_id = crate::jobs::enqueue(
        &state.repository,
        "import_analyze",
        json!({ "import_id": id, "app_slug": slug }),
        &user.eppn,
        None,
    ).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok(Json(json!({ "job_id": job_id, "status": "analyzing" })))
}

async fn set_mapping(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
    Json(mapping): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let target_table = mapping.get("target_table")
        .or_else(|| mapping.get("target_table_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| pair(ServiceError::BadRequest("target_table is required".to_string())))?;

    state.repository.update_import_mapping(id, target_table, &mapping)
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok(Json(json!({ "ok": true })))
}

async fn dry_run_import(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
    payload: Option<Json<Value>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let mapping_val = payload.map(|Json(p)| p).unwrap_or(Value::Null);

    let max_rows = state.settings.read().ok().and_then(|s| s.get("import.max_rows").and_then(Value::as_u64)).unwrap_or(crate::import_jobs::DEFAULT_MAX_ROWS);

    // Check row limit before job starts
    if let Ok(Some(imp)) = state.repository.get_import(&slug, id) {
        if let Ok((bytes, _)) = crate::import_jobs::read_file_raw(&state.repository, imp.file_ref_id) {
            let delim = imp.delimiter.chars().next().unwrap_or(',');
            let sheet = mapping_val.get("sheet").and_then(|v| v.as_str());
            if let Ok((_, rows, _)) = crate::import_jobs::parse_rows_from_bytes(&bytes, sheet, Some(delim), None) {
                if rows.len() > max_rows as usize {
                    return Err(pair(ServiceError::BadRequest(format!("File exceeds the maximum row limit of {} rows.", max_rows))));
                }
            }
        }
    }

    let rules = crate::service::scope::rules_of(&slug, &state);
    let person = crate::service::scope::person_of(&user, &state);

    let mut job_payload = json!({
        "import_id": id,
        "app_slug": slug,
        "caller_eppn": user.eppn,
        "caller_affiliation": user.affiliation,
        "row_rules": rules,
        "caller_person": person,
        "max_rows": max_rows,
    });
    if !mapping_val.is_null() {
        if let Some(target_table) = mapping_val.get("target_table").or_else(|| mapping_val.get("target_table_id")).and_then(|v| v.as_str()) {
            let _ = state.repository.update_import_mapping(id, target_table, &mapping_val);
        }
        job_payload["mapping"] = mapping_val;
    }

    let job_id = crate::jobs::enqueue(
        &state.repository,
        "import_validate",
        job_payload,
        &user.eppn,
        None,
    ).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok(Json(json!({ "job_id": job_id, "status": "validating" })))
}

#[derive(Deserialize)]
struct RunParams {
    accept_errors: Option<bool>,
}

async fn run_import(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
    Query(params): Query<RunParams>,
    payload: Option<Json<Value>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let mapping_val = payload.map(|Json(p)| p).unwrap_or(Value::Null);
    let accept_errors = params.accept_errors.unwrap_or(false);

    let max_rows = state.settings.read().ok().and_then(|s| s.get("import.max_rows").and_then(Value::as_u64)).unwrap_or(crate::import_jobs::DEFAULT_MAX_ROWS);

    // Check row limit before job starts
    if let Ok(Some(imp)) = state.repository.get_import(&slug, id) {
        if let Ok((bytes, _)) = crate::import_jobs::read_file_raw(&state.repository, imp.file_ref_id) {
            let delim = imp.delimiter.chars().next().unwrap_or(',');
            let sheet = mapping_val.get("sheet").and_then(|v| v.as_str());
            if let Ok((_, rows, _)) = crate::import_jobs::parse_rows_from_bytes(&bytes, sheet, Some(delim), None) {
                if rows.len() > max_rows as usize {
                    return Err(pair(ServiceError::BadRequest(format!("File exceeds the maximum row limit of {} rows.", max_rows))));
                }
            }
        }
    }

    let rules = crate::service::scope::rules_of(&slug, &state);
    let person = crate::service::scope::person_of(&user, &state);

    let mut job_payload = json!({
        "import_id": id,
        "app_slug": slug,
        "caller_eppn": user.eppn,
        "caller_affiliation": user.affiliation,
        "accept_errors": accept_errors,
        "row_rules": rules,
        "caller_person": person,
        "max_rows": max_rows,
    });
    if !mapping_val.is_null() {
        if let Some(target_table) = mapping_val.get("target_table").or_else(|| mapping_val.get("target_table_id")).and_then(|v| v.as_str()) {
            let _ = state.repository.update_import_mapping(id, target_table, &mapping_val);
        }
        job_payload["mapping"] = mapping_val;
    }

    let job_id = crate::jobs::enqueue(
        &state.repository,
        "import_run",
        job_payload,
        &user.eppn,
        None,
    ).map_err(|e| pair(ServiceError::Internal(e.to_string())))?;

    Ok(Json(json!({ "job_id": job_id, "status": "running" })))
}

async fn download_rejected(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id_str)): Path<(String, String)>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::Read, &state).map_err(pair)?;
    let id = Uuid::parse_str(&id_str).map_err(|_| pair(ServiceError::BadRequest("Invalid import id".to_string())))?;

    let import = state.repository.get_import(&slug, id)
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?
        .ok_or_else(|| pair(ServiceError::NotFound("Import not found".to_string())))?;

    let rej_id = import.rejected_file_ref_id
        .ok_or_else(|| pair(ServiceError::NotFound("No rejected rows for this import".to_string())))?;

    let (bytes, _sha) = crate::import_jobs::read_file_raw(&state.repository, rej_id)
        .map_err(|e| pair(ServiceError::Internal(format!("{:?}", e))))?;

    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (header::CONTENT_DISPOSITION, &format!("attachment; filename=\"rejected_rows_{}.csv\"", id)),
        ],
        bytes,
    ).into_response())
}
