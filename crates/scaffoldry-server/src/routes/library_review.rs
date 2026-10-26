//! Policy library phase 3: review, answers, and publishing. The service functions in `service::library_review` do the work.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

use crate::guard::session_user;
use crate::service::library_review as review;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/me/library/reviews", get(my_reviews))
        .route("/library/rules/{id}/submit", post(submit))
        .route("/library/rules/{id}/review", get(review_view))
        .route("/library/rules/{id}/decide", post(decide))
        .route("/library/rules/{id}/retire", post(retire))
        .route("/library/questions/{id}/answer", post(answer))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

/// A body that is not a JSON object reads as null, so an unauthorized caller is refused before the fields are named.
fn parse(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes).ok().filter(Value::is_object).unwrap_or(Value::Null)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?
}

async fn my_reviews(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || review::my_reviews(&state, &c)).await.map(Json)
}

async fn submit(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || review::submit(&state, &c, &id)).await.map(Json)
}

async fn review_view(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || review::review_view(&state, &c, &id)).await.map(Json)
}

async fn decide(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || review::decide(&state, &c, &id, &body)).await.map(Json)
}

async fn retire(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || review::retire(&state, &c, &id, &body)).await.map(Json)
}

async fn answer(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let body = parse(&body);
    blocking(move || review::answer_question(&state, &c, &id, &body)).await.map(Json)
}
