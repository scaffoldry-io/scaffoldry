//! Registered outside apps. The rules are in `service::clients`.

use crate::guard::session_user;
use crate::service::clients::{self, Registration};
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/clients", get(list_clients).post(create_client))
        .route("/clients/{id}", get(get_client))
        .route("/clients/{id}/decide", post(decide_client))
        .route("/clients/{id}/disable", post(disable_client))
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

async fn list_clients(State(state): State<SharedState>, headers: HeaderMap) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || clients::list(&user, &state)).await.map(Json)
}

async fn get_client(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || clients::get(&user, &id, &state)).await.map(Json)
}

async fn create_client(State(state): State<SharedState>, headers: HeaderMap, Json(body): Json<Registration>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = user_of(&state, &headers)?;
    let created = blocking(move || clients::propose(&user, body, &state)).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    decision: String,
    #[serde(default)]
    rationale: String,
}

async fn decide_client(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Decision>) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || clients::decide(&user, &id, &body.decision, &body.rationale, &state)).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Disable {
    #[serde(default)]
    reason: String,
}

async fn disable_client(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Disable>) -> Reply {
    let user = user_of(&state, &headers)?;
    blocking(move || clients::disable(&user, &id, &body.reason, &state)).await.map(Json)
}
