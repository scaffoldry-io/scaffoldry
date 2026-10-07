//! Scaffoldry Sovereign API Server (Layer 1 / Layer 2 / Layer 3 Bridge)

pub mod guard;
pub mod routes;
pub mod state;

use axum::Router;
use state::{ServerState, SharedState};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub fn build_app() -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(ServerState::new()?);
    build_app_with_state(state)
}

pub fn build_app_with_state(state: SharedState) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = routes::api_router(state.clone())
        .layer(axum::middleware::from_fn_with_state(state, guard::require_session))
        .layer(cors);
    Ok(router)
}
