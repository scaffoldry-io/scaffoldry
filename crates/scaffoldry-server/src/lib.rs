//! Scaffoldry Sovereign API Server (Layer 1 / Layer 2 / Layer 3 Bridge)

pub mod guard;
pub mod jobs;
pub mod jwt;
pub mod repository;
pub mod routes;
pub mod service;
pub mod state;

use axum::Router;
pub use state::{ServerState, SharedState};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub fn build_app() -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(ServerState::new()?);
    build_app_with_state(state)
}

pub fn build_app_with_state(state: SharedState) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let state_cors = state.clone();
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(
            move |origin: &axum::http::HeaderValue, _req: &axum::http::request::Parts| {
                let origin_str = match origin.to_str() {
                    Ok(s) => s,
                    Err(_) => return false,
                };
                if origin_str == "http://localhost:5173"
                    || origin_str == "http://127.0.0.1:5173"
                    || origin_str == "http://localhost:3000"
                    || origin_str == "http://127.0.0.1:3000"
                {
                    return true;
                }
                if let Ok(settings) = state_cors.settings.read() {
                    if let Some(val) = settings.get("cors.allowed_origins") {
                        if let Some(arr) = val.as_array() {
                            return arr.iter().any(|v| v.as_str() == Some(origin_str));
                        }
                    }
                }
                false
            },
        ))
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
