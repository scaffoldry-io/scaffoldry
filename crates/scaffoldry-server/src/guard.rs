//! Session guard. Identity comes only from a valid bearer JWT or genuine active session.
//! Headers such as `x-caller-eppn` and request body fields are never trusted.

use crate::jwt::{get_jwt_issuer, get_jwt_secret, validate_jwt};
use crate::state::{AuthUser, SharedState};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Paths that stay reachable without an authenticated session.
/// SCIM carries its own provisioning credential and is checked separately.
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

    // 1. Refuse legacy seeded tokens immediately
    if token.starts_with("sct_") {
        return None;
    }

    // 2. Validate as signed OAuth 2.1 / OIDC JWT
    let secret = get_jwt_secret();
    let issuer = get_jwt_issuer();
    if let Ok(claims) = validate_jwt(token, &secret, &issuer) {
        return Some(claims.to_auth_user());
    }

    // 3. Fall back to active database/state sessions (e.g. for impersonation sessions)
    let sessions = state.sessions.read().ok()?;
    sessions.get(token).map(|s| s.user.clone())
}

pub fn unauthorized() -> Response {
    let mut resp = (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "401 Unauthorized: a valid bearer JWT or session is required" })),
    )
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
        None => return unauthorized(),
    };
    let token = match bearer_token(req.headers()) {
        Some(t) => t.as_bytes(),
        None => return unauthorized(),
    };
    if constant_time_eq(configured, token) {
        next.run(req).await
    } else {
        unauthorized()
    }
}
