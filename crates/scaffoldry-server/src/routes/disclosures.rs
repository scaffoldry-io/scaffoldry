//! The disclosure log (record-history phase 4). The log of an app and the log of the institution are
//! for a Platform Admin or `compliance`. The log of one record is also for its app's owner and admins.

use crate::service::disclosure::{self, Query as LogQuery};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    routing::get,
    Extension, Json, Router,
};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/disclosures", get(list_disclosures))
        .route("/admin/disclosures/export", get(export_disclosures))
        .route("/apps/{slug}/records/{id}/disclosures", get(record_disclosures))
}

fn caller(user: Option<Extension<AuthUser>>, headers: &HeaderMap, state: &SharedState) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

async fn list_disclosures(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Query(query): Query<LogQuery>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(disclosure::list(&caller, query, &state)?))
}

async fn export_disclosures(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Query(query): Query<LogQuery>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    let exported = disclosure::export(&caller, query, &state)?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(exported.content_type));
    response_headers.insert("x-truncated", HeaderValue::from_static(if exported.truncated { "true" } else { "false" }));
    Ok((response_headers, exported.body))
}

async fn record_disclosures(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = caller(user, &headers, &state)?;
    Ok(Json(disclosure::for_record(&caller, &slug, &id, &state)?))
}
