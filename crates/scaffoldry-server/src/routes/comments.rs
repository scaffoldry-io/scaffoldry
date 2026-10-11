//! Comments on records. The rules are in `service::comments`.

use crate::guard::session_user;
use crate::service::comments;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
    Json, Router,
};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/records/{id}/comments", get(list).post(create))
        .route("/apps/{slug}/comments/{id}", patch(edit).delete(remove))
        .route("/apps/{slug}/mentionable", get(mentionable))
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn user_of(state: &SharedState, headers: &HeaderMap) -> Result<crate::state::AuthUser, (StatusCode, Json<Value>)> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())
}

async fn run<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map_err(ServiceError::into_pair)
}

async fn list(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Reply {
    let user = user_of(&state, &headers)?;
    run(move || comments::list_comments(&user, &slug, &id, &state)).await.map(Json)
}

async fn create(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = user_of(&state, &headers)?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    run(move || comments::post_comment(&user, &origin, &slug, &id, &payload, &state)).await.map(|v| (StatusCode::CREATED, Json(v)))
}

async fn edit(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Reply {
    let user = user_of(&state, &headers)?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    run(move || comments::edit_comment(&user, &origin, &slug, &id, &payload, &state)).await.map(Json)
}

async fn remove(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Reply {
    let user = user_of(&state, &headers)?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    run(move || comments::remove_comment(&user, &origin, &slug, &id, &state)).await.map(Json)
}

async fn mentionable(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>) -> Reply {
    let user = user_of(&state, &headers)?;
    run(move || comments::mentionable(&user, &slug, &state)).await.map(Json)
}
