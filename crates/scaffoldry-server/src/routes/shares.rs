//! Views phase 6: shared links. The routes under `/apps/{slug}/views/{id}/shares` need a session and the right to manage the
//! app. The routes under `/shared/{token}` need only the token, and a session when the link is for the institution.

use crate::service::shares::{self, CreateBody};
use crate::service::views::RunQuery;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::Value;
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/views/{id}/shares", get(list).post(create))
        .route("/apps/{slug}/views/{id}/shares/{share}", axum::routing::delete(revoke))
        .route("/shared/{token}", get(describe))
        .route("/shared/{token}/data", get(data))
        .route("/shared/{token}/download", get(download))
        .route("/admin/shares", get(admin_list))
        .route("/admin/shares/revoke", post(admin_revoke))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn id_of(raw: &str, what: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(raw).map_err(|_| ServiceError::NotFound(format!("{what} not found")))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped".to_string()))?
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let view = id_of(&id, "View")?;
    Ok(Json(blocking(move || shares::list(&caller, &slug, view, &state)).await?))
}

async fn create(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>, Json(body): Json<CreateBody>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let view = id_of(&id, "View")?;
    let made = blocking(move || shares::create(&caller, &slug, view, body, &state)).await?;
    Ok((StatusCode::CREATED, Json(made)))
}

async fn revoke(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((slug, id, share)): Path<(String, String, String)>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let (view, share) = (id_of(&id, "View")?, id_of(&share, "Link")?);
    Ok(Json(blocking(move || shares::revoke(&caller, &slug, view, share, &state)).await?))
}

async fn describe(State(state): State<SharedState>, headers: HeaderMap, Path(token): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    Ok(Json(blocking(move || shares::describe(&state, &shares::open(&state, &headers, &token)?)).await?))
}

#[derive(serde::Deserialize)]
struct DataQuery {
    cursor: Option<String>,
    limit: Option<usize>,
    group_path: Option<String>,
}

async fn data(State(state): State<SharedState>, headers: HeaderMap, Path(token): Path<String>, axum::extract::Query(q): axum::extract::Query<DataQuery>) -> Result<impl IntoResponse, ServiceError> {
    let group_path: Vec<String> = match q.group_path.as_deref().filter(|p| !p.trim().is_empty()) {
        None => Vec::new(),
        Some(p) => serde_json::from_str(p).map_err(|_| ServiceError::BadRequest("group_path must be a JSON array of strings".to_string()))?,
    };
    let origin = crate::guard::read_origin(&state, &headers);
    let body = blocking(move || {
        let opened = shares::open(&state, &headers, &token)?;
        shares::data(&state, &opened, &origin, RunQuery { cursor: q.cursor, limit: q.limit, group_path, ..Default::default() })
    })
    .await?;
    Ok(Json(body))
}

async fn download(State(state): State<SharedState>, headers: HeaderMap, Path(token): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let origin = crate::guard::read_origin(&state, &headers);
    let csv = blocking(move || {
        let opened = shares::open(&state, &headers, &token)?;
        shares::download(&state, &opened, &origin)
    })
    .await?;
    Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"view.csv\"")], csv))
}

async fn admin_list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(blocking(move || shares::admin_list(&caller, &state)).await?))
}

async fn admin_revoke(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    Ok(Json(blocking(move || shares::admin_revoke_app(&caller, &body, &state)).await?))
}
