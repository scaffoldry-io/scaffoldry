//! Reading record history. The writes are made by the repository, with the records.

use crate::guard::session_user;
use crate::repository::HistoryFilter;
use crate::service::history::{app_history, record_history, HistoryPage, HistoryQuery, DEFAULT_PAGE};
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
        .route("/apps/{slug}/records/{id}/history", get(get_record_history))
        .route("/apps/{slug}/history", get(get_app_history))
        .route("/apps/{slug}/history/{batch_id}/undo", post(post_undo))
        .route("/apps/{slug}/records/{id}/restore", post(post_restore))
}

#[derive(Debug, Deserialize, Default)]
struct HistoryParams {
    limit: Option<usize>,
    cursor: Option<String>,
    table: Option<String>,
    actor: Option<String>,
    kind: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

fn body(page: HistoryPage) -> Json<Value> {
    Json(json!({ "entries": page.entries, "next_cursor": page.next_cursor }))
}

async fn get_record_history(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Query(p): Query<HistoryParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    // Only the paging applies to one record.
    let query = HistoryQuery { filter: HistoryFilter::default(), cursor: p.cursor, limit: p.limit.unwrap_or(DEFAULT_PAGE) };
    let page = tokio::task::spawn_blocking(move || record_history(&user, &slug, &id, query, &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(body(page))
}

async fn get_app_history(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(p): Query<HistoryParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let query = HistoryQuery {
        filter: HistoryFilter { table_id: p.table, actor: p.actor, kind: p.kind, from: p.from, to: p.to, record_id: None },
        cursor: p.cursor,
        limit: p.limit.unwrap_or(DEFAULT_PAGE),
    };
    let page = tokio::task::spawn_blocking(move || app_history(&user, &slug, query, &state))
        .await
        .map_err(|_| ServiceError::internal("The read stopped").into_pair())?
        .map_err(ServiceError::into_pair)?;
    Ok(body(page))
}

async fn post_undo(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, batch_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    tokio::task::spawn_blocking(move || crate::service::undo::undo_batch(&user, &origin, &slug, &batch_id, &state))
        .await
        .map_err(|_| ServiceError::internal("The undo stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn post_restore(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<crate::state::DatasetRecord>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    tokio::task::spawn_blocking(move || crate::service::undo::restore_record(&user, &origin, &slug, &id, &payload, &state))
        .await
        .map_err(|_| ServiceError::internal("The restore stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}
