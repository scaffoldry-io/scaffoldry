//! Views (views phase 1). Rights follow the view's ownership and are checked in `service::views`.

use crate::service::views::{self, CreateBody, UpdateBody};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/views", get(list_views).post(create_view))
        .route("/apps/{slug}/views/{id}", axum::routing::put(update_view).delete(delete_view))
        .route("/apps/{slug}/views/{id}/duplicate", post(duplicate_view))
        .route("/apps/{slug}/views/{id}/data", get(view_data))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

/// A view id that is not a UUID names no view.
fn view_id(raw: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(raw).map_err(|_| ServiceError::NotFound("View not found".to_string()))
}

async fn list_views(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(views::list(&caller, &slug, &state)?))
}

async fn create_view(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(body): Json<CreateBody>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let created = tokio::task::spawn_blocking(move || views::create(&caller, &slug, body, &state))
        .await
        .map_err(|_| ServiceError::Internal("The save stopped".to_string()))??;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update_view(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let id = view_id(&id)?;
    let saved = tokio::task::spawn_blocking(move || views::update(&caller, &slug, id, body, &state))
        .await
        .map_err(|_| ServiceError::Internal("The save stopped".to_string()))??;
    Ok(Json(saved))
}

async fn delete_view(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let id = view_id(&id)?;
    let gone = tokio::task::spawn_blocking(move || views::delete(&caller, &slug, id, &state))
        .await
        .map_err(|_| ServiceError::Internal("The delete stopped".to_string()))??;
    Ok(Json(gone))
}

async fn duplicate_view(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let id = view_id(&id)?;
    let copy = tokio::task::spawn_blocking(move || views::duplicate(&caller, &slug, id, &state))
        .await
        .map_err(|_| ServiceError::Internal("The copy stopped".to_string()))??;
    Ok((StatusCode::CREATED, Json(copy)))
}

/// `GET /apps/{slug}/views/{id}/data?cursor=&limit=&group_path=`. `group_path` is a JSON array of group keys.
#[derive(serde::Deserialize)]
struct DataQuery {
    cursor: Option<String>,
    limit: Option<usize>,
    group_path: Option<String>,
}

async fn view_data(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<DataQuery>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let id = view_id(&id)?;
    let group_path: Vec<String> = match q.group_path.as_deref().filter(|p| !p.trim().is_empty()) {
        None => Vec::new(),
        Some(p) => serde_json::from_str(p).map_err(|_| ServiceError::BadRequest("group_path must be a JSON array of strings".to_string()))?,
    };
    let origin = crate::guard::read_origin(&state, &headers);
    let body = tokio::task::spawn_blocking(move || views::run(&caller, &origin, &slug, id, views::RunQuery { cursor: q.cursor, limit: q.limit, group_path }, &state))
        .await
        .map_err(|_| ServiceError::Internal("The read stopped".to_string()))??;
    Ok(Json(body))
}
