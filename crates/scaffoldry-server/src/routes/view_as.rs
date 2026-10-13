//! `POST /apps/{slug}/view-as` and `POST /auth/stop-view-as`. The rules are in `service::view_as`.

use crate::guard::{bearer_token, session_user};
use crate::service::identity::hash_token;
use crate::service::{view_as, ServiceError};
use crate::state::SharedState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new().route("/apps/{slug}/view-as", post(start)).route("/auth/stop-view-as", post(stop))
}

#[derive(Deserialize)]
struct StartBody {
    eppn: String,
}

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

async fn start(State(state): State<SharedState>, headers: HeaderMap, Path(slug): Path<String>, Json(body): Json<StartBody>) -> Reply {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let out = tokio::task::spawn_blocking(move || view_as::start(&user, &slug, &body.eppn, &state))
        .await
        .map_err(|_| ServiceError::internal("The view could not start").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(Json(out))
}

async fn stop(State(state): State<SharedState>, headers: HeaderMap) -> Reply {
    let row = bearer_token(&headers)
        .and_then(|t| state.get_api_token(&hash_token(t)))
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    view_as::stop(&row, &state).map(Json).map_err(ServiceError::into_pair)
}
