//! Attachments phase 1: upload a file, and download one through the record that holds it.

use crate::files::{self, StoreError};
use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::service::disclosure::{log_disclosure, Release};
use crate::repository_files::NewRef;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        // The body is read as a stream and stopped at `files.max_mb`, so the general body limit does not apply.
        .route("/apps/{slug}/files", post(upload).layer(DefaultBodyLimit::disable()))
        .route("/apps/{slug}/records/{id}/files/{ref_id}", get(download))
}

fn pair(e: ServiceError) -> (StatusCode, Json<Value>) {
    e.into_pair()
}

async fn upload(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    body: Body,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| pair(ServiceError::unauthorized("Unauthorized")))?;
    authorize_app(&user, &slug, AppAction::WriteRecords, &state).map_err(pair)?;
    crate::guard::origin(&state, &headers).map_err(pair)?;
    let repo = state.repository.as_deref().ok_or_else(|| pair(ServiceError::Internal("Files need PostgreSQL".to_string())))?;
    let dir = files::dir().ok_or_else(|| pair(ServiceError::BadRequest("File storage is not set up on this server.".to_string())))?;

    let declared: u64 = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| pair(ServiceError::BadRequest("Content-Length is required for an upload.".to_string())))?;
    let name = files::clean_name(&headers.get("x-file-name").map(|v| String::from_utf8_lossy(v.as_bytes()).to_string()).unwrap_or_default());
    if files::blocked_name(&name) {
        return Err(pair(ServiceError::FileTypeBlocked(format!("'{name}' is a program or a web page. Programs and web pages cannot be uploaded."))));
    }
    let limit = files::max_bytes(&state);
    let too_large = || ServiceError::FileTooLarge(format!("That file is over the limit of {} MB.", limit / 1024 / 1024));
    if declared > limit {
        return Err(pair(too_large()));
    }
    let used = repo.app_file_bytes(&slug).map_err(|e| pair(ServiceError::Internal(e.to_string())))?.max(0) as u64;
    if used + declared > files::quota_bytes(&state) {
        return Err(pair(ServiceError::QuotaExceeded("This app has used its file storage. Remove files or ask an admin to raise the limit.".to_string())));
    }

    let stored = files::store(&dir, body, declared, limit).await.map_err(|e| {
        pair(match e {
            StoreError::TooLarge => too_large(),
            StoreError::Blocked => ServiceError::FileTypeBlocked("This file is a program or a web page. Programs and web pages cannot be uploaded.".to_string()),
            StoreError::Bad(m) => ServiceError::BadRequest(m),
            StoreError::Io(m) => ServiceError::Internal(format!("The file could not be stored: {m}")),
        })
    })?;
    let ref_id = Uuid::new_v4();
    repo.insert_file_ref(NewRef {
        id: ref_id,
        sha256: &stored.sha256,
        size: stored.size as i64,
        media_type: stored.media_type,
        app_slug: &slug,
        name: &name,
        uploaded_by: &user.eppn,
    })
        .map_err(|e| pair(ServiceError::Internal(e.to_string())))?;
    Ok((StatusCode::CREATED, Json(json!({ "ref_id": ref_id, "name": name, "size": stored.size, "media_type": stored.media_type }))))
}

fn not_found() -> Response {
    ServiceError::NotFound("File not found".to_string()).into_response()
}

async fn download(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id, ref_id)): Path<(String, String, String)>) -> Response {
    let Some(user) = session_user(&state, &headers) else {
        return ServiceError::unauthorized("Unauthorized").into_response();
    };
    // Every refusal is the same 404, so a response never says whether a file exists.
    let Ok(ref_id) = ref_id.parse::<Uuid>() else { return not_found() };
    let Ok(record) = crate::service::records::get_record(&user, &slug, &id, &state) else { return not_found() };
    let Some(repo) = state.repository.as_deref() else { return not_found() };
    let Ok(Some(file)) = repo.record_file_ref(&slug, &id, ref_id) else { return not_found() };
    let Some(field) = file.field.clone() else { return not_found() };
    // The record came back with the fields the caller may not read removed. The file counts only if its field is left.
    let listed = record
        .data
        .get(&field)
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|i| i.get("ref_id").and_then(Value::as_str) == Some(&ref_id.to_string())));
    if !listed {
        return not_found();
    }
    let Some(dir) = files::dir() else { return not_found() };
    let Ok(bytes) = tokio::fs::read(files::blob_path(&dir, &file.sha256)).await else { return not_found() };

    let table = file.table_id.clone().unwrap_or_default();
    if let Err(e) = log_disclosure(
        &state,
        Release {
            kind: "file_download",
            actor: &user.eppn,
            app_slug: &slug,
            table_id: Some(&table),
            record_ids: vec![id.clone()],
            fields: vec![field],
            recipient: None,
            note: &format!("downloaded '{}'", file.name),
        },
    ) {
        return e.into_response();
    }

    let disposition = if files::inline_type(&file.media_type) { "inline" } else { "attachment" };
    let mut resp = Response::new(Body::from(bytes));
    let h = resp.headers_mut();
    let set = |h: &mut HeaderMap, k: header::HeaderName, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    };
    set(h, header::CONTENT_TYPE, &file.media_type);
    set(h, header::CONTENT_DISPOSITION, &format!("{disposition}; filename=\"{}\"", file.name));
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox");
    set(h, header::CACHE_CONTROL, "private, no-store");
    resp
}
