//! Scaffoldry Sovereign API Server (Layer 1 / Layer 2 / Layer 3 Bridge)

pub mod guard;
pub mod jwt;
pub mod repository;
pub mod routes;
pub mod service;
pub mod state;

use axum::Router;
use state::{ServerState, SharedState};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub fn validate_production_configuration() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if std::env::var("SCAFFOLDRY_ENV").as_deref() == Ok("production") {
        let secret = std::env::var("SCAFFOLDRY_JWT_SECRET").unwrap_or_default();
        if secret.trim().is_empty() || secret == jwt::DEFAULT_SECRET {
            return Err("Production configuration error: SCAFFOLDRY_JWT_SECRET must be configured with a non-default secret in production".into());
        }
    }
    Ok(())
}

pub fn build_app() -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    validate_production_configuration()?;
    let state = Arc::new(ServerState::new()?);
    build_app_with_state(state)
}

pub fn build_app_with_state(state: SharedState) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let allowed_origins_str = std::env::var("SCAFFOLDRY_ALLOWED_ORIGINS").unwrap_or_else(|_| {
        "http://localhost:5173,http://127.0.0.1:5173,http://localhost:3000,http://127.0.0.1:3000".to_string()
    });

    let origins: Vec<axum::http::HeaderValue> = allowed_origins_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
            axum::http::Method::PATCH,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
            axum::http::header::ORIGIN,
            axum::http::HeaderName::from_static("x-requested-with"),
        ]);

    let router = routes::api_router(state.clone())
        .layer(axum::middleware::from_fn_with_state(state, guard::require_session))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http());
    Ok(router)
}
