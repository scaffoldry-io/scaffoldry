//! Admin console phase 6: Business processes. Every route is a Platform Admin's. The rules are in `service::admin_processes`.

use crate::service::admin_processes as svc;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashMap;

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/processes", get(list))
        .route("/admin/processes/{rule_id}", get(detail).patch(patch))
        .route("/admin/processes/{rule_id}/versions/{v}", get(version))
        .route("/admin/processes/{rule_id}/simulate", post(simulate))
        .route("/admin/process-instances", get(list_instances))
        .route("/admin/process-instances/{id}", get(instance))
        .route("/admin/process-instances/{id}/approvers", get(approvers))
        .route("/admin/process-instances/{id}/reassign", post(reassign))
        .route("/admin/process-instances/{id}/cancel", post(cancel))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, (StatusCode, Json<Value>)> {
    match user {
        Some(Extension(u)) => Ok(u),
        None => crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("A valid session is required").into_pair()),
    }
}

async fn blocking(f: impl FnOnce() -> Result<Value, ServiceError> + Send + 'static) -> Reply {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped").into_pair())?.map(Json).map_err(ServiceError::into_pair)
}

fn body_of(body: Option<Json<Value>>) -> Value {
    body.map(|Json(b)| b).unwrap_or(Value::Null)
}

async fn list(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::list(&who, &q, &state)).await
}

async fn detail(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(rule_id): Path<String>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::detail(&who, &rule_id, &state)).await
}

async fn version(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path((rule_id, v)): Path<(String, String)>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::version(&who, &rule_id, &v, &state)).await
}

async fn patch(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(rule_id): Path<String>, body: Option<Json<Value>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    let body = body_of(body);
    blocking(move || svc::set_enabled(&who, &rule_id, &body, &state)).await
}

async fn simulate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(rule_id): Path<String>, body: Option<Json<Value>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    let body = body_of(body);
    blocking(move || svc::simulate(&who, &rule_id, &body, &state)).await
}

async fn list_instances(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::list_instances(&who, &q, &state)).await
}

async fn instance(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::instance_detail(&who, &id, &state)).await
}

async fn approvers(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::instance_approvers(&who, &id, &state)).await
}

async fn reassign(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    let body = body_of(body);
    blocking(move || svc::reassign(&who, &id, &body, &state)).await
}

async fn cancel(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    let body = body_of(body);
    blocking(move || svc::cancel(&who, &id, &body, &state)).await
}
