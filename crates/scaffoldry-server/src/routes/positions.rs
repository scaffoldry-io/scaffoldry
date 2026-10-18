//! Position types for Platform Admins, and holders by unit. Each handler checks the caller
//! before it reads a body.

use crate::guard::session_user;
use crate::service::admin::require_platform_admin;
use crate::service::positions::{self, NewPosition, PositionPatch};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/positions", get(list_positions).post(create_position))
        .route("/admin/positions/vacancies", get(vacancies))
        .route("/admin/positions/{key}", axum::routing::patch(patch_position))
        .route("/orgs/{id}/positions", get(unit_positions))
        .route("/orgs/{id}/positions/{key}/holders", post(assign_holder))
        .route("/orgs/{id}/positions/{key}/holders/{eppn}", axum::routing::delete(vacate_holder))
}

fn caller(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    session_user(state, headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized"))
}

fn admin(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    let c = caller(state, headers)?;
    require_platform_admin(&c, state)?;
    Ok(c)
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ServiceError> {
    serde_json::from_slice(body).map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")))
}

async fn list_positions(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    admin(&state, &headers)?;
    Ok(Json(json!({ "positions": positions::list_positions(&state) })))
}

async fn vacancies(State(state): State<SharedState>, headers: HeaderMap) -> Result<Json<Value>, ServiceError> {
    admin(&state, &headers)?;
    Ok(Json(json!({ "vacancies": positions::vacancies(&state) })))
}

#[derive(Debug, Deserialize)]
struct CreateBody {
    key: Option<String>,
    name: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    org_types: Vec<String>,
    max_holders: Option<i64>,
    reason: Option<String>,
}

async fn create_position(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = admin(&state, &headers)?;
    let b: CreateBody = parse(&body)?;
    let new = NewPosition {
        key: b.key.unwrap_or_default(),
        name: b.name.unwrap_or_default(),
        description: b.description,
        org_types: b.org_types,
        max_holders: b.max_holders.unwrap_or(1),
    };
    let position = positions::create_position(&state, &c, new, &b.reason.unwrap_or_default())?;
    Ok((StatusCode::CREATED, Json(json!({ "position": position }))))
}

#[derive(Debug, Deserialize)]
struct PatchBody {
    name: Option<String>,
    description: Option<String>,
    max_holders: Option<i64>,
    org_types: Option<Vec<String>>,
    retired: Option<bool>,
    reason: Option<String>,
}

async fn patch_position(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ServiceError> {
    let c = admin(&state, &headers)?;
    let b: PatchBody = parse(&body)?;
    let patch = PositionPatch {
        name: b.name,
        description: b.description,
        max_holders: b.max_holders,
        org_types: b.org_types,
        retired: b.retired,
    };
    let position = positions::patch_position(&state, &c, &key, patch, &b.reason.unwrap_or_default())?;
    Ok(Json(json!({ "position": position })))
}

async fn unit_positions(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    Ok(Json(positions::unit_positions(&state, &c, id)?))
}

#[derive(Debug, Deserialize)]
struct AssignBody {
    eppn: Option<String>,
    #[serde(default)]
    replace: bool,
    reason: Option<String>,
}

async fn assign_holder(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((id, key)): Path<(Uuid, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServiceError> {
    let c = caller(&state, &headers)?;
    let b: AssignBody = parse(&body)?;
    let eppn = b.eppn.unwrap_or_default();
    let out = positions::assign_holder(&state, &c, id, &key, &eppn, b.replace, &b.reason.unwrap_or_default())?;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Debug, Deserialize, Default)]
struct ReasonBody {
    reason: Option<String>,
}

async fn vacate_holder(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((id, key, eppn)): Path<(Uuid, String, String)>,
    body: Bytes,
) -> Result<Json<Value>, ServiceError> {
    let c = caller(&state, &headers)?;
    let b: ReasonBody = serde_json::from_slice(&body).unwrap_or_default();
    positions::vacate_holder(&state, &c, id, &key, &eppn, &b.reason.unwrap_or_default())?;
    Ok(Json(json!({ "vacated": true })))
}
