//! Jobs API: read, list, and cancel. A job is visible to its creator and to a Platform Admin.
//! Anyone else gets 404, so a stranger cannot learn that a job exists.

use crate::jobs::{self, Job};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/jobs", get(list_jobs))
        .route("/jobs/{id}", get(read_job))
        .route("/jobs/{id}/cancel", post(cancel_job))
}

#[derive(Deserialize)]
struct Page {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers)
        .ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The job as JSON. A failed job carries the error code `job_failed`.
fn view(job: &Job) -> Value {
    let mut value = serde_json::to_value(job).unwrap_or(Value::Null);
    if job.state == "failed" {
        value["code"] = json!("job_failed");
    }
    value
}

/// Loads a job the caller may see, or 404.
fn visible_job(state: &SharedState, caller: &AuthUser, raw_id: &str) -> Result<Job, ServiceError> {
    let not_found = || ServiceError::NotFound("Job not found".to_string());
    let id = Uuid::parse_str(raw_id).map_err(|_| not_found())?;
    let repo = state
        .repository
        .as_deref()
        .ok_or_else(|| ServiceError::Internal("Jobs need PostgreSQL".to_string()))?;
    let job = jobs::get_job(repo, id).map_err(internal)?.ok_or_else(not_found)?;
    if job.created_by.eq_ignore_ascii_case(&caller.eppn)
        || crate::service::admin::require_platform_admin(caller, state).is_ok() {
        Ok(job)
    } else {
        Err(not_found())
    }
}

async fn read_job(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let job = visible_job(&state, &caller, &id)?;
    Ok(Json(view(&job)))
}

async fn cancel_job(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let job = visible_job(&state, &caller, &id)?;
    let repo = state
        .repository
        .as_deref()
        .ok_or_else(|| ServiceError::Internal("Jobs need PostgreSQL".to_string()))?;
    let updated = jobs::cancel_job(repo, job.id)
        .map_err(internal)?
        .ok_or_else(|| ServiceError::NotFound("Job not found".to_string()))?;
    Ok(Json(view(&updated)))
}

async fn list_jobs(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let repo = state
        .repository
        .as_deref()
        .ok_or_else(|| ServiceError::Internal("Jobs need PostgreSQL".to_string()))?;
    let limit = page.limit.unwrap_or(50).clamp(1, 200);
    let offset = page.offset.unwrap_or(0).max(0);
    let found = jobs::list_jobs(repo, &caller.eppn, limit, offset).map_err(internal)?;
    Ok(Json(json!({
        "jobs": found.iter().map(view).collect::<Vec<_>>(),
        "limit": limit,
        "offset": offset,
    })))
}
