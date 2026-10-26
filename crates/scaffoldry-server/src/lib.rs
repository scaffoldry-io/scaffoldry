//! Scaffoldry Sovereign API Server (Layer 1 / Layer 2 / Layer 3 Bridge)

pub mod files;
pub mod guard;
pub mod cache;
pub mod calc_jobs;
pub mod index_jobs;
pub mod jobs;
pub mod jwt;
pub mod repository;
pub mod repository_clients;
pub mod repository_comments;
pub mod repository_disclosure;
pub mod repository_receipts;
pub mod repository_files;
pub mod repository_grants;
pub mod repository_authority;
pub mod repository_access_requests;
pub mod repository_library;
pub mod repository_supervision;
pub mod repository_calc;
pub mod repository_index;
pub mod repository_links;
pub mod repository_people;
pub mod repository_proposals;
pub mod repository_trash;
pub mod repository_undo;
pub mod import_jobs;
pub mod export_jobs;
pub mod repository_imports;
pub mod repository_views;
pub mod routes;
pub mod scan;
pub mod serve;
pub mod service;
pub mod state;
pub mod testing;

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
                    if let Some(arr) = settings.get("cors.allowed_origins").and_then(|v| v.as_array()) {
                        if arr.iter().any(|v| v.as_str() == Some(origin_str)) {
                            return true;
                        }
                    }
                }
                // The origin of an active outside app's redirect address may call too.
                crate::service::oauth::client_origins(&state_cors).iter().any(|o| o == origin_str)
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
            axum::http::HeaderName::from_static("x-file-name"),
        ]);

    let router = routes::api_router(state.clone())
        .layer(axum::middleware::from_fn_with_state(state.clone(), guard::require_session))
        // The first thing every request does. A request is answered from what the database says now.
        .layer(axum::middleware::from_fn_with_state(state, refresh_state))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(tower_http::trace::TraceLayer::new_for_http());
    Ok(router)
}

/// Brings this instance up to date with the database before the request is looked at. When the database cannot be
/// read the request fails with `storage_unavailable`, and the next one tries again.
async fn refresh_state(
    axum::extract::State(state): axum::extract::State<SharedState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Liveness checks nothing, and readiness runs the refresh itself and reports what it finds.
    if matches!(request.uri().path(), "/healthz" | "/readyz") {
        return next.run(request).await;
    }
    match state.refresh() {
        Ok(()) => next.run(request).await,
        Err(e) => service::ServiceError::StorageUnavailable(format!("The server cannot read its database: {e}")).into_response(),
    }
}
