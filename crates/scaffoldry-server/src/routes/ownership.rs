//! When an owner leaves. The rules are in `service::ownership`.

use crate::guard::session_user;
use crate::service::ownership::{self, TransferBody};
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/ownership/orphans", get(orphans))
        .route("/admin/ownership/transfer/preview", post(preview))
        .route("/admin/ownership/transfer", post(transfer))
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

fn user_of(state: &SharedState, headers: &HeaderMap) -> Result<crate::state::AuthUser, (StatusCode, Json<Value>)> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())
}

fn parse(body: &Bytes) -> Result<TransferBody, (StatusCode, Json<Value>)> {
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")).into_pair())
}

async fn orphans(State(state): State<SharedState>, headers: HeaderMap) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || ownership::orphans(&user, &state)).await.map(Json)
}

async fn preview(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = parse(&body)?;
    blocking(move || ownership::preview(&user, &body, &state)).await.map(Json)
}

async fn transfer(State(state): State<SharedState>, headers: HeaderMap, body: Bytes) -> Reply {
    let user = user_of(&state, &headers)?;
    let body = parse(&body)?;
    blocking(move || ownership::transfer(&user, &body, &state)).await.map(Json)
}
