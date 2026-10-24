//! Access model phase 7: the explanation of one person's access to one object.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::guard::session_user;
use crate::service::access_review as review;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/access/explain", get(explain))
        .route("/admin/access/people/{eppn}", get(person))
        .route("/admin/access/objects/{kind}/{*id}", get(object))
        .route("/admin/access/changes/{id}/impact", post(impact))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

#[derive(Debug, Deserialize)]
struct ExplainParams {
    person: Option<String>,
    object: String,
}

async fn explain(State(state): State<SharedState>, headers: HeaderMap, Query(p): Query<ExplainParams>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let person = p.person.unwrap_or_else(|| c.eppn.clone());
    tokio::task::spawn_blocking(move || review::explain(&state, &c, &person, &p.object))
        .await
        .map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
        .map(Json)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<Json<T>, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?.map(Json)
}

async fn person(State(state): State<SharedState>, headers: HeaderMap, Path(eppn): Path<String>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || review::review_person(&state, &c, &eppn)).await
}

async fn object(State(state): State<SharedState>, headers: HeaderMap, Path((kind, id)): Path<(String, String)>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || review::review_object(&state, &c, &format!("{kind}:{id}"))).await
}

async fn impact(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let id = uuid::Uuid::parse_str(&id).map_err(|_| crate::service::access_grants::missing(&state, &c, "Change"))?;
    blocking(move || review::impact(&state, &c, id)).await
}
