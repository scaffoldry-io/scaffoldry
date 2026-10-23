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
const PUBLIC_PATHS: [&str; 6] = [
    "/healthz",
    "/readyz",
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

    let mut user = resolve_user(&row.eppn, state)?;
    if row.kind == "viewas" {
        // The viewer must still resolve. Rights are the target's intersected with theirs.
        let viewer = row.original_admin.as_deref().and_then(|a| resolve_user(a, state))?;
        user.view_as_by = Some(Box::new(viewer));
    }
    // A preview works while its proposal is pending, and builds the preview app in this server.
    if row.kind == "preview" && !crate::service::preview::admit(&row, state) {
        return None;
    }
    Some(user)
}

/// The stored `viewas` or `preview` token behind this request, if that is what it carries. Both reach one app and only
/// read, so they share the gates below.
pub fn view_as_token(state: &SharedState, headers: &HeaderMap) -> Option<crate::state::ApiToken> {
    let row = state.get_api_token(&hash_token(bearer_token(headers)?))?;
    matches!(row.kind.as_str(), "viewas" | "preview").then_some(row)
}

/// The stored token behind this request.
pub fn token_row(state: &SharedState, headers: &HeaderMap) -> Option<crate::state::ApiToken> {
    state.get_api_token(&hash_token(bearer_token(headers)?))
}

/// What a read-only session says when it is asked to change something.
fn read_only(row: &crate::state::ApiToken) -> ServiceError {
    if row.kind == "preview" {
        ServiceError::PreviewReadOnly("You are previewing a proposed change. A preview is read-only. Nothing here is live.".to_string())
    } else {
        ServiceError::ViewAsReadOnly("You are viewing as someone else. This view is read-only.".to_string())
    }
}

fn one_app_only(row: &crate::state::ApiToken) -> ServiceError {
    if row.kind == "preview" {
        ServiceError::forbidden("This preview reaches one app only. Stop it to open anything else.")
    } else {
        ServiceError::forbidden("This view reaches one app only. Exit it to open anything else.")
    }
}

/// What a view-as session may reach: reads in its own app, the way out, and the tool endpoint
/// (where each tool is checked on its own). Everything else is refused before a handler runs.
fn view_as_gate(row: &crate::state::ApiToken, method: &axum::http::Method, path: &str) -> Option<ServiceError> {
    use axum::http::Method;
    if method == Method::POST && matches!(path, "/api/v1/auth/stop-view-as" | "/api/v1/mcp" | "/api/mcp") {
        return None;
    }
    if method != Method::GET && method != Method::HEAD {
        return Some(read_only(row));
    }
    if path.starts_with("/api/v1/auth/") {
        return None;
    }
    let slug = path.split_once("/apps/").and_then(|(_, rest)| rest.split('/').next());
    match (slug, row.app_slug.as_deref()) {
        (Some(asked), Some(allowed)) if asked == allowed => None,
        _ => Some(one_app_only(row)),
    }
}

/// The same limits for a tool call: it must read only, and name the app being viewed.
pub fn view_as_tool_gate(row: &crate::state::ApiToken, name: &str, args: &serde_json::Value) -> Result<(), ServiceError> {
    if let Some(tool) = crate::service::tools::TOOLS.iter().find(|t| t.name == name) {
        // A page reads and writes through one tool. Its reads are reads.
        let page_read = name == "page_call" && matches!(args.get("op").and_then(serde_json::Value::as_str), Some("list" | "get" | "settings_get"));
        if !tool.read_only && !page_read {
            return Err(read_only(row));
        }
    }
    if args.get("app_slug").and_then(serde_json::Value::as_str) != row.app_slug.as_deref() {
        return Err(one_app_only(row));
    }
    Ok(())
}

/// Whether this request carries an agent token, and its label. A read has no batch, so a bad batch
/// header cannot hide the agent: this never refuses.
pub fn read_origin(state: &SharedState, headers: &HeaderMap) -> crate::service::history::Origin {
    use crate::repository::ActorKind;
    let mut origin = crate::service::history::Origin::user();
    if let Some(row) = bearer_token(headers).and_then(|t| state.get_api_token(&hash_token(t))) {
        if row.kind == "agent" {
            origin.kind = ActorKind::Agent;
            origin.token_label = Some(row.label);
        }
    }
    origin
}

/// How this request reached the server: an agent token or a person, and the batch the caller
/// named. A bad batch header is refused, not ignored, so a typo cannot split a paste in two.
pub fn origin(state: &SharedState, headers: &HeaderMap) -> Result<crate::service::history::Origin, ServiceError> {
    let mut origin = read_origin(state, headers);
    if let Some(value) = headers.get("x-scaffoldry-batch") {
        let batch = value
            .to_str()
            .ok()
            .and_then(|v| uuid::Uuid::parse_str(v.trim()).ok())
            .ok_or_else(|| ServiceError::BadRequest("x-scaffoldry-batch must be a UUID".to_string()))?;
        origin.batch_id = Some(batch);
    }
    Ok(origin)
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
            if let Some(row) = view_as_token(&state, req.headers()) {
                if let Some(refusal) = view_as_gate(&row, req.method(), req.uri().path()) {
                    return refusal.into_response();
                }
            }
            // A limited token reaches the record routes its scope allows and the tool endpoint, where each tool is checked.
            if let Some(scope) = token_row(&state, req.headers()).and_then(|r| r.scope) {
                let path = req.uri().path().to_string();
                let tool_endpoint = req.method() == axum::http::Method::POST && matches!(path.as_str(), "/api/v1/mcp" | "/api/mcp");
                if !tool_endpoint {
                    if let Err(refusal) = crate::service::token_scope::check_request(&scope, req.method(), &path, &state) {
                        return refusal.into_response();
                    }
                    if let Some(query) = crate::service::token_scope::capped_query(&scope, req.method(), &path, req.uri().query()) {
                        if let Ok(uri) = format!("{path}?{query}").parse::<axum::http::Uri>() {
                            *req.uri_mut() = uri;
                        }
                    }
                }
            }
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
