//! Route Tree Aggregation

pub mod apps;
pub mod datasets;
pub mod governance;
pub mod mcp;
pub mod policy;
pub mod records;
pub mod scim;
pub mod workspaces;

use crate::state::SharedState;
use axum::{
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

pub fn api_router(state: SharedState) -> Router {
    let api_v1 = Router::new()
        .merge(workspaces::router())
        .merge(apps::router())
        .merge(records::router())
        .merge(datasets::router())
        .merge(policy::router())
        .merge(governance::router())
        .merge(mcp::router())
        .with_state(state.clone());

    let scim_v2 = scim::router().with_state(state.clone());
    let mcp_root = mcp::router().with_state(state);

    Router::new()
        .route("/healthz", get(health_check))
        .nest("/api/v1", api_v1)
        .nest("/api", mcp_root)
        .nest("/scim/v2", scim_v2)
}

async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "scaffoldry-api",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
