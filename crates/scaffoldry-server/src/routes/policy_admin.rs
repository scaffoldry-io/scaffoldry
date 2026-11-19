//! Admin console phase 5: policy versions, test cases, the simulator and the OSCAL requirements. The service checks who may call.

use crate::service::{oscal_admin, policy_admin, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/policy/active", get(active))
        .route("/admin/policy/versions", get(versions).post(create_draft))
        .route("/admin/policy/versions/{id}", get(version))
        .route("/admin/policy/versions/{id}/clone", post(clone_version))
        .route("/admin/policy/versions/{id}/run-cases", post(run_cases))
        .route("/admin/policy/versions/{id}/activate", post(activate))
        .route("/admin/policy/cases", get(cases).post(add_case))
        .route("/admin/policy/cases/{id}", get(case).delete(remove_case))
        .route("/admin/policy/simulate", post(simulate))
        .route("/admin/policy/validate", post(validate))
        .route("/admin/oscal/requirements", get(requirements))
        .route("/admin/oscal/requirements/{control_id}", axum::routing::put(put_requirement))
        .route("/admin/oscal/component-definition", get(component_definition))
        .route("/admin/oscal/catalog", get(oscal_catalog))
        .route("/admin/oscal/coverage", get(oscal_coverage))
        .route("/admin/oscal/coverage/gaps", get(oscal_gaps))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn stopped(_: tokio::task::JoinError) -> ServiceError {
    ServiceError::Internal("The request stopped".to_string())
}

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn active(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::active(&caller, &state)).await.map_err(stopped)??))
}

async fn versions(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::versions(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn version(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::version(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn create_draft(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok((StatusCode::CREATED, Json(tokio::task::spawn_blocking(move || policy_admin::create_draft(&caller, &body, &state)).await.map_err(stopped)??)))
}

async fn clone_version(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok((StatusCode::CREATED, Json(tokio::task::spawn_blocking(move || policy_admin::clone_version(&caller, &id, &body, &state)).await.map_err(stopped)??)))
}

async fn run_cases(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::run_cases(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn activate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::activate(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn cases(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::cases(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn add_case(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok((StatusCode::CREATED, Json(tokio::task::spawn_blocking(move || policy_admin::add_case(&caller, &body, &state)).await.map_err(stopped)??)))
}

async fn case(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::case(&caller, &id, &state)).await.map_err(stopped)??))
}

async fn remove_case(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::remove_case(&caller, &id, &body, &state)).await.map_err(stopped)??))
}

async fn simulate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::simulate(&caller, &body, &state)).await.map_err(stopped)??))
}

async fn requirements(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::requirements(&caller, &state)).await.map_err(stopped)??))
}

async fn put_requirement(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(control_id): Path<String>, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::put_requirement(&caller, &control_id, &body, &state)).await.map_err(stopped)??))
}

async fn component_definition(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::component_definition(&caller, &state)).await.map_err(stopped)??))
}

async fn oscal_catalog(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::search_catalog(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn oscal_coverage(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::coverage(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn oscal_gaps(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(tokio::task::spawn_blocking(move || oscal_admin::coverage_gaps(&caller, &q, &state)).await.map_err(stopped)??))
}

async fn validate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, body: Option<Json<Value>>) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let body = body_of(body);
    Ok(Json(tokio::task::spawn_blocking(move || policy_admin::validate(&caller, &body, &state)).await.map_err(stopped)??))
}
