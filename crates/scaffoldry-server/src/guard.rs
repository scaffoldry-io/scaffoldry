//! Session guard. Identity comes only from stored api_tokens rows.
//! Headers such as  and request body fields are never trusted.

use crate::service::identity::{hash_token, resolve_user};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::Next,
    response::{IntoResponse, Response},
};
use crate::service::ServiceError;

/// Paths that stay reachable without an authenticated session.
const PUBLIC_PATHS: [&str; 5] = [
    "/healthz",
    "/.well-known/oauth-protected-resource",
    "/.well-known/openid-configuration",
    "/.well-known/jwks.json",
    "/api/v1/auth/token",
];

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

pub fn session_user(state: &SharedState, headers: &HeaderMap) -> Option<AuthUser> {
    let token = bearer_token(headers)?;
    let hash = hash_token(token);

    let row = match state.get_api_token(&hash) {
        Some(r) => r,
        None => {
            return None;
        }
    };

    // Refuse when revoked or expired
    if row.revoked_at.is_some() {
        return None;
    }
    let now = chrono::Utc::now();
    if row.expires_at < now {
        return None;
    }

    // Update last_used_at at most once a minute for a given row
    let should_update = match row.last_used_at {
        None => true,
        Some(last) => now.signed_duration_since(last).num_seconds() >= 60,
    };
    if should_update {
        let _ = state.update_token_last_used(&hash, now);
    }

    resolve_user(&row.eppn, state)
}

pub fn unauthorized() -> Response {
    let mut resp = ServiceError::unauthorized("401 Unauthorized: a valid bearer token or session is required").into_pair()
        .into_response();
    resp.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(
            r#"Bearer error="invalid_token", error_description="The access token expired or is invalid", resource_metadata="/.well-known/oauth-protected-resource""#,
        ),
    );
    resp
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
        None => unauthorized(),
    }
}

pub async fn require_scim_credential(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Response {
    let token = match bearer_token(req.headers()) {
        Some(t) => t,
        None => return unauthorized(),
    };
    let hash = hash_token(token);
    let valid = if let Some(row) = state.get_api_token(&hash) {
        row.kind == "scim"
            && row.revoked_at.is_none()
            && row.expires_at > chrono::Utc::now()
    } else {
        false
    };

    if valid {
        next.run(req).await
    } else {
        unauthorized()
    }
}
