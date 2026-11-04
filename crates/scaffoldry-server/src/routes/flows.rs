//! Workflows phase 7 route: `POST /flows/validate`. It checks the flow the editor holds and writes nothing. The rules
//! are in `service::flow_validate`.

use crate::service::{flow_validate, ServiceError};
use crate::state::{AuthUser, SharedState};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new().route("/flows/validate", post(validate))
}

async fn validate(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let caller = match user {
        Some(Extension(u)) => u,
        None => crate::guard::session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("A valid session is required").into_pair())?,
    };
    tokio::task::spawn_blocking(move || flow_validate::validate(&caller, body, &state))
        .await
        .map_err(|_| ServiceError::internal("The request stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}
