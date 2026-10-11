//! `GET /apps/{slug}/tables/{table_id}/link-options`. The rules are in `service::link_options`.

use crate::guard::session_user;
use crate::service::link_options;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router() -> Router<SharedState> {
    Router::new().route("/apps/{slug}/tables/{table_id}/link-options", get(options))
}

#[derive(Debug, Deserialize, Default)]
struct Params {
    q: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

async fn options(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table_id)): Path<(String, String)>,
    Query(p): Query<Params>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let page = tokio::task::spawn_blocking(move || {
        link_options::link_options(&user, &slug, &table_id, p.q.as_deref().unwrap_or(""), p.limit, p.cursor.as_deref(), &state)
    })
    .await
    .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
    .map_err(ServiceError::into_pair)?;
    Ok(Json(json!({ "options": page.options, "next_cursor": page.next_cursor })))
}
