//! Access model phase 7: the explanation of one person's access to one object.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::guard::session_user;
use crate::service::access_review as review;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new().route("/access/explain", get(explain))
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
