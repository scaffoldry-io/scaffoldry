//! RFC 9728 OAuth 2.0 Protected Resource Metadata
//! Enables zero-configuration discovery for MCP clients.

use crate::jwt::get_jwt_issuer;
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

async fn get_oauth_protected_resource() -> impl IntoResponse {
    let issuer = get_jwt_issuer();
    Json(json!({
        "resource": "/api/mcp",
        "authorization_servers": [issuer],
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
