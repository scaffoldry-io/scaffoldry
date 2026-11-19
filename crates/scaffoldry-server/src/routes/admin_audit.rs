//! Admin console phase 7: Audit and security. The ledger and its export are for a Platform Admin or a compliance officer.
//! Tokens and the agent controls are a Platform Admin's. The rules are in `service::admin_audit`.

use crate::service::admin_audit as svc;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashMap;

type Reply = Result<Json<Value>, (StatusCode, Json<Value>)>;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/ledger", get(ledger))
        .route("/admin/ledger/verify", post(verify))
        .route("/admin/ledger/export", get(export))
        .route("/admin/tokens", get(list_tokens))
        .route("/admin/tokens/{id}", delete(revoke_token))
        .route("/admin/tools", get(tools))
        .route("/admin/pages", get(pages))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, (StatusCode, Json<Value>)> {
    match user {
        Some(Extension(u)) => Ok(u),
        None => crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("A valid session is required").into_pair()),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ServiceError> + Send + 'static) -> Result<T, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ServiceError::internal("The request stopped").into_pair())?.map_err(ServiceError::into_pair)
}

async fn ledger(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::ledger(&who, &q, &state)).await.map(Json)
}

async fn verify(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::verify(&who, &state)).await.map(Json)
}

async fn export(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Result<Response, (StatusCode, Json<Value>)> {
    let who = caller(user, &headers, &state)?;
    let file = blocking(move || svc::export(&who, &q, &state)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, file.content_type.to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", file.filename)),
        ],
        file.body,
    )
        .into_response())
}

async fn list_tokens(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::list_tokens(&who, &q, &state)).await.map(Json)
}

async fn revoke_token(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Reply {
    let who = caller(user, &headers, &state)?;
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    blocking(move || svc::revoke_token(&who, &id, &body, &state)).await.map(Json)
}

async fn tools(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::tools(&who, &state)).await.map(Json)
}

async fn pages(State(state): State<SharedState>, user: Option<Extension<AuthUser>>, headers: HeaderMap) -> Reply {
    let who = caller(user, &headers, &state)?;
    blocking(move || svc::pages(&who, &state)).await.map(Json)
}
