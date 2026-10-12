//! Route Tree Aggregation

pub mod admin;
pub mod formula;
pub mod admin_people;
pub mod positions;
pub mod proposals;
pub mod admin_workspaces;
pub mod apps;
pub mod auth;
pub mod datasets;
pub mod discovery;
pub mod comments;
pub mod disclosures;
pub mod history;
pub mod link_options;
pub mod calc;
pub mod view_as;
pub mod views;
pub mod trash;
pub mod framework;
pub mod governance;
pub mod jobs;
pub mod mcp;
pub mod policy;
pub mod records;
pub mod scim;
pub mod organizations;
pub mod workspaces;
pub mod settings;

use crate::state::SharedState;
use axum::{
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

pub fn api_router(state: SharedState) -> Router {
    let api_v1 = Router::new()
        .merge(auth::router())
        .merge(formula::router())
        .merge(admin::router())
        .merge(admin_people::router())
        .merge(positions::router())
        .merge(proposals::router())
        .merge(admin_workspaces::router())
        .merge(workspaces::router())
        .merge(organizations::router())
        .merge(apps::router())
        .merge(records::router())
        .merge(history::router())
        .merge(trash::router())
        .merge(calc::router())
        .merge(view_as::router())
        .merge(views::router())
        .merge(link_options::router())
        .merge(comments::router())
        .merge(disclosures::router())
        .merge(datasets::router())
        .merge(framework::router())
        .merge(policy::router())
        .merge(governance::router())
        .merge(jobs::router())
        .merge(settings::router())
        .merge(mcp::router())
        .with_state(state.clone());

    let scim_v2 = scim::router()
        .with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::guard::require_scim_credential,
        ));
    let mcp_root = mcp::router().with_state(state.clone());
    let discovery_routes = discovery::router().with_state(state.clone());
    let ws_root = workspaces::router().with_state(state.clone());
    let admin_root = admin::router().merge(admin_workspaces::router()).with_state(state);

    Router::new()
        .route("/healthz", get(health_check))
        .merge(discovery_routes)
        .merge(admin_root)
        .merge(ws_root)
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
