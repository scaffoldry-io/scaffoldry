//! Session guard. Identity comes only from a valid bearer session.
//! Headers such as `x-caller-eppn` and request body fields are never trusted.

use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Paths that stay reachable without a session.
/// SCIM carries its own provisioning credential and is checked separately.
const PUBLIC_PATHS: [&str; 2] = ["/healthz", "/api/v1/auth/login"];

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

pub fn session_user(state: &SharedState, headers: &HeaderMap) -> Option<AuthUser> {
    let token = bearer_token(headers)?;
    let sessions = state.sessions.read().ok()?;
    sessions.get(token).map(|s| s.user.clone())
}

pub fn unauthorized() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "401 Unauthorized: a valid session is required" })),
    )
}

pub async fn require_session(
    State(state): State<SharedState>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if PUBLIC_PATHS.contains(&path) || path.starts_with("/scim/v2/") {
        return next.run(req).await;
    }
    match session_user(&state, req.headers()) {
        Some(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None => unauthorized().into_response(),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn require_scim_credential(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Response {
    let configured = match &state.scim_token {
        Some(t) => t.as_bytes(),
        None => return unauthorized().into_response(),
    };
    let token = match bearer_token(req.headers()) {
        Some(t) => t.as_bytes(),
        None => return unauthorized().into_response(),
    };
    if constant_time_eq(configured, token) {
        next.run(req).await
    } else {
        unauthorized().into_response()
    }
}
