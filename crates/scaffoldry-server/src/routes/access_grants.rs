//! Access model phase 4: the grant routes under `/admin/access`.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::guard::session_user;
use crate::service::access_grants as grants;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/access/grants", get(list_grants).post(create_grant))
        .route("/admin/access/grants/{id}/revoke", post(revoke_grant))
        .route("/admin/access/grants/{id}/review", post(review_grant))
        .route("/admin/access/reviews", get(list_reviews))
        .route("/admin/access/grants/preview", post(preview_grant))
        .route("/admin/access/people", get(search_people))
        .route("/admin/hierarchies", post(create_hierarchy))
        .route("/admin/hierarchies/{key}/retire", post(retire_hierarchy))
        .route("/admin/access/changes/{id}/decide", post(decide_change))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

fn parse(body: &Bytes) -> Result<Value, ServiceError> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|e| ServiceError::BadRequest(format!("The body is not valid JSON: {e}")))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    holder: Option<String>,
    anchor: Option<String>,
    hierarchy: Option<String>,
    source: Option<String>,
    state: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_grants(State(state): State<SharedState>, headers: HeaderMap, Query(p): Query<ListParams>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let q = grants::ListQuery { holder: p.holder, anchor: p.anchor, hierarchy: p.hierarchy, source: p.source, state: p.state, cursor: p.cursor, limit: p.limit };
    tokio::task::spawn_blocking(move || grants::list(&state, &c, &q))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn create_grant(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    let out = tokio::task::spawn_blocking(move || grants::propose_grant(&state, &c, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))??;
    Ok((StatusCode::ACCEPTED, Json(out)))
}

async fn revoke_grant(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    let id: i64 = id.parse().map_err(|_| grants::missing(&state, &c, "Grant"))?;
    tokio::task::spawn_blocking(move || grants::revoke_grant(&state, &c, id, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn review_grant(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    let id: i64 = id.parse().map_err(|_| grants::missing(&state, &c, "Grant"))?;
    tokio::task::spawn_blocking(move || grants::review_grant(&state, &c, id, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

#[derive(Debug, Deserialize)]
struct PageParams {
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_reviews(State(state): State<SharedState>, headers: HeaderMap, Query(p): Query<PageParams>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    tokio::task::spawn_blocking(move || grants::reviews(&state, &c, p.cursor.as_deref(), p.limit))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn preview_grant(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    tokio::task::spawn_blocking(move || grants::preview_grant(&state, &c, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn search_people(State(state): State<SharedState>, headers: HeaderMap, Query(p): Query<PageParams>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    tokio::task::spawn_blocking(move || grants::search_people(&state, &c, p.q.as_deref().unwrap_or(""), p.cursor.as_deref(), p.limit))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn create_hierarchy(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    let out = tokio::task::spawn_blocking(move || crate::service::hierarchies::propose(&state, &c, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))??;
    Ok((StatusCode::ACCEPTED, Json(out)))
}

async fn retire_hierarchy(State(state): State<SharedState>, headers: HeaderMap, Path(key): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    tokio::task::spawn_blocking(move || crate::service::hierarchies::retire(&state, &c, &key, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn decide_change(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body)?;
    let id = Uuid::parse_str(&id).map_err(|_| grants::missing(&state, &c, "Change"))?;
    tokio::task::spawn_blocking(move || grants::decide_change(&state, &c, id, &body))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

