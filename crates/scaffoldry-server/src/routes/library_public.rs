//! Policy library phase 4: the library every signed-in person reads. `service::library_public` does the work.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde_json::Value;
use std::collections::HashMap;

use crate::guard::session_user;
use crate::service::library_public as public;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/library", get(library))
        .route("/library/policies/{key}", get(policy_page))
        // The first segment is named `id` because the review routes under /library/rules/{id}/... share it.
        .route("/library/rules/{id}/{rule_key}", get(rule_page))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
}

async fn library(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || public::library(&state, &c, &q)).await.map(Json)
}

async fn policy_page(State(state): State<SharedState>, headers: HeaderMap, Path(key): Path<String>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || public::policy_page(&state, &c, &key)).await.map(Json)
}

async fn rule_page(State(state): State<SharedState>, headers: HeaderMap, Path((policy_key, rule_key)): Path<(String, String)>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || public::rule_page(&state, &c, &policy_key, &rule_key)).await.map(Json)
}
