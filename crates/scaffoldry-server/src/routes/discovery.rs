//! RFC 9728 OAuth 2.0 Protected Resource Metadata
//! Enables zero-configuration discovery for MCP clients.

use crate::jwt::configured_issuer;
use crate::state::SharedState;
use axum::{
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(get_oauth_protected_resource))
}

async fn get_oauth_protected_resource(axum::extract::State(state): axum::extract::State<SharedState>, headers: axum::http::HeaderMap) -> impl IntoResponse {
    // This server issues tokens for outside apps, and the institution's identity provider issues the others.
    let own = crate::service::oauth::issuer_of(&headers);
    let mut servers = vec![own.clone()];
    // The institution's identity provider, when a Platform Admin has set it. There is no built-in default.
    if let Some(idp) = configured_issuer(&state).filter(|idp| *idp != own) {
        servers.push(idp);
    }
    Json(json!({
        "resource": format!("{own}/api/mcp"),
        "authorization_servers": servers,
        "scopes_supported": [
            "mcp:read",
            "mcp:write",
            "workspaces:read",
            "workspaces:write",
            "records:read",
            "records:write",
            "governance:audit"
        ],
        "bearer_methods_supported": ["header"]
    }))
}
