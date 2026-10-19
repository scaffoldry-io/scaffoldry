//! Administrative Endpoints
//! Enforces Platform Admin ABAC and provides institutional overview metrics.

use crate::guard::session_user;
use crate::service::admin::require_platform_admin_or_compliance;
use crate::service::ServiceError;
use crate::state::SharedState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use scaffoldry_core::ProcessStatus;
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::jobs::{self, AdminJobRow};
use crate::service::admin::{admin_write, decode_cursor, encode_cursor, require_platform_admin};
use scaffoldry_core::ledger::DecisionType;
use uuid::Uuid;
use axum::routing::post;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/overview", get(get_overview))
        .route("/admin/jobs", get(get_jobs))
        .route("/admin/jobs/{id}", get(get_job_by_id))
        .route("/admin/jobs/{id}/cancel", post(cancel_job_by_id))
        .route("/admin/jobs/{id}/retry", post(retry_job_by_id))
}

async fn get_overview(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(caller) = session_user(&state, &headers) else {
        return ServiceError::unauthorized("Unauthorized").into_response();
    };

    if let Err(e) = require_platform_admin_or_compliance(&caller, &state) {
        return e.into_response();
    }

    // 1. Server
    let version = env!("CARGO_PKG_VERSION");
    let (applied_migrations, worker_count) = { let repo = &state.repository;
        let migs = repo.get_applied_migrations().unwrap_or_default();
        let workers = repo.worker_count();
        (migs, workers)
    };

    // 2. People
    let (active_users, hold_users, inactive_users) = state.repository.people_counts().unwrap_or((0, 0, 0));

    let platform_admins_count = {
        let mut admin_eppns: HashSet<String> = state.repository.platform_admin_names().unwrap_or_default().into_iter().collect();
        // Central admin callers
        if caller.affiliation == "central_admin" {
            admin_eppns.insert(caller.eppn.clone());
        }
        admin_eppns.len()
    };

    let active_tokens_by_kind: HashMap<String, usize> =
        state.repository.active_token_counts().unwrap_or_default().into_iter().map(|(k, n)| (k, n as usize)).collect();

    // 3. Organization
    let unit_count = state.organizations.read().unwrap().len();

    // 4. Workspaces & apps
    let workspace_count = state.workspaces.read().unwrap().len();
    let app_count = state.engine.read().unwrap().manifests_count();

    // 5. Processes
    let waiting_instance_count = state.repository.count_process_instances(ProcessStatus::Waiting).unwrap_or(0);

    // 6. Ledger
    let (ledger_count, head_hash) = {
        let (count, head) = state.repository.ledger_summary().unwrap_or((0, None));
        (count, head.unwrap_or_default())
    };

    // 7. Jobs
    let jobs_overview = { let repo = &state.repository;
        jobs::get_jobs_overview(repo).unwrap_or_default()
    };

    (
        StatusCode::OK,
        Json(json!({
            "server": {
                "version": version,
                "applied_migrations": applied_migrations,
                "database_worker_count": worker_count,
            },
            "people": {
                "active": active_users,
                "on_hold": hold_users,
                "inactive": inactive_users,
                "users_active": active_users,
                "users_on_hold": hold_users,
                "users_inactive": inactive_users,
                "platform_admins": platform_admins_count,
                "active_tokens": active_tokens_by_kind,
                "active_tokens_by_kind": active_tokens_by_kind,
            },
            "organization": {
                "unit_count": unit_count,
                "units": unit_count,
            },
            "workspaces": {
                "workspace_count": workspace_count,
                "workspaces": workspace_count,
                "app_count": app_count,
                "apps": app_count,
            },
            "processes": {
                "waiting_instance_count": waiting_instance_count,
                "waiting": waiting_instance_count,
            },
            "proposals": {
                "pending": state.repository.count_pending_proposals().unwrap_or(0),
            },
            "ledger": {
                "entry_count": ledger_count,
                "head_hash": head_hash,
            },
            "files": crate::scan::overview(
                Some(&*state.repository),
                state.settings.read().ok().and_then(|s| s.get("files.scanner").cloned()).as_ref(),
            ),
            "jobs": {
                "queue_depth": jobs_overview.queue_depth,
                "oldest_queued_age_secs": jobs_overview.oldest_queued_age_secs,
                "failed_last_24h": jobs_overview.failed_last_24h,
            }
        })),
    )
        .into_response()
}


#[derive(serde::Deserialize)]
struct AdminJobsQuery {
    state: Option<String>,
    kind: Option<String>,
    owner: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(serde::Deserialize)]
struct AdminReasonPayload {
    reason: String,
}

async fn get_jobs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<AdminJobsQuery>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, &state)?;
    let repo = &*state.repository;

    let offset = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let rows = jobs::list_admin_jobs(
        repo,
        query.state.as_deref(),
        query.kind.as_deref(),
        query.owner.as_deref(),
        limit + 1,
        offset,
    )
    .map_err(|e| ServiceError::internal(e.to_string()))?;

    let has_more = rows.len() > limit;
    let result_rows: Vec<AdminJobRow> = rows.into_iter().take(limit).collect();
    let next_cursor = if has_more {
        Some(encode_cursor(offset + limit))
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "rows": result_rows,
            "next_cursor": next_cursor,
        })),
    ))
}

async fn get_job_by_id(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, &state)?;
    let uuid = Uuid::parse_str(&id).map_err(|_| ServiceError::not_found("Job not found"))?;
    let repo = &*state.repository;
    let job = jobs::get_job(repo, uuid)
        .map_err(|e| ServiceError::internal(e.to_string()))?
        .ok_or_else(|| ServiceError::not_found("Job not found"))?;
    Ok((StatusCode::OK, Json(job)))
}

async fn cancel_job_by_id(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    body: Option<Json<AdminReasonPayload>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, &state)?;
    let Json(payload) = body.ok_or_else(|| ServiceError::bad_request("A reason is required"))?;
    let uuid = Uuid::parse_str(&id).map_err(|_| ServiceError::not_found("Job not found"))?;
    let updated = admin_write(
        &state,
        &caller,
        DecisionType::JobCancelled,
        "AU-02",
        &payload.reason,
        &json!({ "job_id": uuid }),
        || {
            let repo = &*state.repository;
            jobs::cancel_job(repo, uuid)
                .map_err(|e| ServiceError::internal(e.to_string()))?
                .ok_or_else(|| ServiceError::not_found("Job not found"))
        },
    )?;
    Ok((StatusCode::OK, Json(updated)))
}

async fn retry_job_by_id(
    State(state): State<SharedState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    body: Option<Json<AdminReasonPayload>>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, &state)?;
    let Json(payload) = body.ok_or_else(|| ServiceError::bad_request("A reason is required"))?;
    let trimmed = payload.reason.trim();
    if trimmed.is_empty() || trimmed.len() > 500 {
        return Err(ServiceError::bad_request(
            "A reason between 1 and 500 characters is required",
        ));
    }
    let uuid = Uuid::parse_str(&id).map_err(|_| ServiceError::not_found("Job not found"))?;
    let repo = &*state.repository;
    let job = jobs::retry_job(repo, uuid).map_err(|e| match e {
        jobs::JobsError::Conflict(msg) => ServiceError::conflict(msg),
        jobs::JobsError::NotFound(msg) => ServiceError::not_found(msg),
        other => ServiceError::internal(other.to_string()),
    })?;
    Ok((StatusCode::OK, Json(job)))
}
