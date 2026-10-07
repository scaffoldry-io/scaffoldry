//! RFC 9728 OAuth 2.0 Protected Resource Metadata and OpenID Connect Discovery 1.0 Endpoints
//! Enables zero-configuration discovery for MCP clients and identity providers.

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
        .route("/.well-known/openid-configuration", get(get_openid_configuration))
        .route("/.well-known/jwks.json", get(get_jwks))
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

async fn get_openid_configuration() -> impl IntoResponse {
    let issuer = get_jwt_issuer();
    Json(json!({
        "issuer": issuer,
        "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "response_types_supported": ["code", "token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["HS256", "RS256"],
        "scopes_supported": ["openid", "profile", "email", "eduperson"]
    }))
}

async fn get_jwks() -> impl IntoResponse {
    Json(json!({
        "keys": [
            {
                "kty": "oct",
                "use": "sig",
                "kid": "test-key-1",
                "alg": "HS256"
            }
        ]
    }))
}
