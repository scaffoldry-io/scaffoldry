//! The trash of an app. The rules are in `service::trash`.

use crate::guard::session_user;
use crate::service::trash;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/apps/{slug}/trash", get(list))
        .route("/apps/{slug}/trash/{what}/restore", post(restore))
}

#[derive(Debug, Deserialize, Default)]
struct Params {
    limit: Option<usize>,
    cursor: Option<String>,
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

async fn list(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, Query(p): Query<Params>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let page = tokio::task::spawn_blocking(move || trash::list_trash(&user, &slug, p.limit, p.cursor.as_deref(), &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(Json(json!({ "entries": page.entries, "next_cursor": page.next_cursor })))
}

async fn restore(State(state): State<SharedState>, headers: HeaderMap, Path((slug, what)): Path<(String, String)>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    tokio::task::spawn_blocking(move || trash::restore(&user, &origin, &slug, &what, &state))
        .await
        .map_err(|_| ServiceError::internal("The restore stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}
