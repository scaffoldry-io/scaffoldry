//! Admin console: People. Every handler checks for a Platform Admin before it reads a body.

use crate::guard::session_user;
use crate::service::admin::require_platform_admin;
use crate::service::people::{self, NewUser, UserQuery};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/users", get(list_users).post(create_user))
        .route("/admin/users/{id}", get(get_user))
        .route("/admin/users/{id}/hold", post(hold_user))
        .route("/admin/users/{id}/revoke-tokens", post(revoke_tokens))
        .route("/admin/groups", get(list_groups))
}

fn admin(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    let caller = session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, state)?;
    Ok(caller)
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ServiceError> {
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")))
}

#[derive(Debug, Deserialize)]
struct ListParams {
    search: Option<String>,
    active: Option<bool>,
    hold: Option<bool>,
    affiliation: Option<String>,
    unit: Option<Uuid>,
    platform_admin: Option<bool>,
    unit_admin: Option<bool>,
    cursor: Option<String>,
    limit: Option<usize>,
}

async fn list_users(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(p): Query<ListParams>,
) -> Result<Json<Value>, ServiceError> {
    admin(&state, &headers)?;
    let (users, next) = people::list_users(
        &state,
        &UserQuery {
            search: p.search,
            active: p.active,
            hold: p.hold,
            affiliation: p.affiliation,
            unit: p.unit,
            platform_admin: p.platform_admin,
            unit_admin: p.unit_admin,
            cursor: p.cursor,
            limit: p.limit,
        },
    )?;
    Ok(Json(json!({ "users": users, "next_cursor": next })))
}

async fn get_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ServiceError> {
    admin(&state, &headers)?;
    Ok(Json(people::user_detail(&state, &id)?))
}

#[derive(Debug, Deserialize)]
struct CreateBody {
    #[serde(rename = "userName")]
    user_name: Option<String>,
    name: Option<String>,
    email: Option<String>,
    affiliation: Option<String>,
    department: Option<String>,
    title: Option<String>,
    reason: Option<String>,
}

async fn create_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(axum::http::StatusCode, Json<Value>), ServiceError> {
    let caller = admin(&state, &headers)?;
    let b: CreateBody = parse(&body)?;
    let user_name = b.user_name.unwrap_or_default();
    let new = NewUser {
        email: b.email.unwrap_or_else(|| user_name.clone()),
        user_name,
        name: b.name.unwrap_or_default(),
        affiliation: b.affiliation.unwrap_or_default(),
        department: b.department.unwrap_or_default(),
        title: b.title.unwrap_or_default(),
    };
    let user = people::create_user(&state, &caller, new, &b.reason.unwrap_or_default())?;
    Ok((axum::http::StatusCode::CREATED, Json(json!({ "user": user }))))
}

#[derive(Debug, Deserialize)]
struct HoldBody {
    hold: bool,
    reason: Option<String>,
}

async fn hold_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ServiceError> {
    let caller = admin(&state, &headers)?;
    let b: HoldBody = parse(&body)?;
    let user = people::set_user_hold(&state, &caller, &id, b.hold, &b.reason.unwrap_or_default())?;
    Ok(Json(json!({ "user": user })))
}

#[derive(Debug, Deserialize)]
struct ReasonBody {
    reason: Option<String>,
}

async fn revoke_tokens(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ServiceError> {
    let caller = admin(&state, &headers)?;
    let b: ReasonBody = parse(&body)?;
    let revoked = people::revoke_user_tokens(&state, &caller, &id, &b.reason.unwrap_or_default())?;
    Ok(Json(json!({ "revoked": revoked })))
}

async fn list_groups(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    admin(&state, &headers)?;
    Ok(Json(json!({ "groups": people::list_groups(&state) })))
}
