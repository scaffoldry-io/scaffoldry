//! Workflows routes. Phase 7: `POST /flows/validate` checks the flow the editor holds. Phase 9: `POST
//! /flows/{id}/test-run` runs a flow in dry mode. Neither writes anything. The rules are in `service::flow_validate` and
//! `service::flow_test_run`.

use crate::service::{flow_runs, flow_test_run, flow_validate, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashMap;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/flows/validate", post(validate))
        .route("/flows/{id}/test-run", post(test_run))
        // Workflows phase 10: runs, a run's timeline, retry, and versions.
        .route("/flows/{id}/runs", get(runs))
        .route("/flows/runs/{instance_id}", get(timeline))
        .route("/flows/runs/{instance_id}/retry", post(retry))
        .route("/flows/{id}/versions", get(versions))
        .route("/flows/{id}/versions/compare", get(compare))
}

async fn blocking(f: impl FnOnce() -> Result<Value, ServiceError> + Send + 'static) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped").into_pair())?.map(Json).map_err(ServiceError::into_pair)
}

async fn runs(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    blocking(move || flow_runs::runs(&caller, &id, &q, &state)).await
}

async fn timeline(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    blocking(move || flow_runs::timeline(&caller, &id, &state)).await
}

async fn retry(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    blocking(move || flow_runs::retry(&caller, &id, &body, &state)).await
}

async fn versions(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    blocking(move || flow_runs::versions(&caller, &id, &state)).await
}

async fn compare(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    blocking(move || flow_runs::compare(&caller, &id, &q, &state)).await
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, (StatusCode, Json<Value>)> {
    match user {
        Some(Extension(u)) => Ok(u),
        None => crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("A valid session is required").into_pair()),
    }
}

async fn test_run(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    tokio::task::spawn_blocking(move || flow_test_run::test_run(&caller, &id, body, &state))
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn validate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = caller(user, &headers, &state)?;
    tokio::task::spawn_blocking(move || flow_validate::validate(&caller, body, &state))
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}
