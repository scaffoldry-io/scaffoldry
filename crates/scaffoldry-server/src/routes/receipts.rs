//! Access model phase 8: reading, checking and replaying receipts. For a Platform Admin or compliance.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::guard::session_user;
use crate::service::receipts as receipts;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/access/receipts", get(list))
        .route("/admin/access/receipts/verify", post(verify))
        .route("/admin/access/receipts/{id}/replay", post(replay))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    actor: Option<String>,
    object: Option<String>,
    outcome: Option<String>,
    since: Option<String>,
    until: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<Json<T>, ServiceError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::Internal("The request stopped before it finished".to_string()))?.map(Json)
}

async fn list(State(state): State<SharedState>, headers: HeaderMap, Query(p): Query<ListParams>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let q = receipts::ListQuery { actor: p.actor, object: p.object, outcome: p.outcome, since: p.since, until: p.until, cursor: p.cursor, limit: p.limit };
    blocking(move || receipts::list(&state, &c, &q)).await
}

async fn verify(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || {
        crate::service::admin::require_platform_admin_or_compliance(&c, &state)?;
        receipts::verify(&state).map(|v| json!(v))
    })
    .await
}

async fn replay(State(state): State<SharedState>, headers: HeaderMap, Path(id): Path<i64>) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    blocking(move || receipts::replay(&state, &c, id)).await
}
