//! Route Tree Aggregation

pub mod admin;
pub mod formula;
pub mod admin_people;
pub mod positions;
pub mod proposals;
pub mod access_grants;
pub mod access_requests;
pub mod authority;
pub mod library;
pub mod library_public;
pub mod library_review;
pub mod access_review;
pub mod receipts;
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
pub mod bundle;
pub mod clients;
pub mod ownership;
pub mod templates;
pub mod framework;
pub mod governance;
pub mod jobs;
pub mod mcp;
pub mod oauth;
pub mod policy;
pub mod files;
pub mod records;
pub mod scim;
pub mod organizations;
pub mod workspaces;
pub mod settings;
pub mod imports;
pub mod exports;

use crate::state::SharedState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

pub fn api_router(state: SharedState) -> Router {
    let state_for_receipts = state.clone();
    let api_v1 = Router::new()
        .merge(auth::router())
        .merge(formula::router())
        .merge(admin::router())
        .merge(admin_people::router())
        .merge(positions::router())
        .merge(proposals::router())
        .merge(admin_workspaces::router())
        .merge(access_grants::router())
        .merge(access_requests::router())
        .merge(access_requests::admin_router())
        .merge(library::router())
        .merge(authority::router())
        .merge(library_review::router())
        .merge(library_public::router())
        .merge(access_review::router())
        .merge(receipts::router())
        .merge(workspaces::router())
        .merge(organizations::router())
        .merge(apps::router())
        .merge(records::router())
        .merge(files::router())
        .merge(history::router())
        .merge(trash::router())
        .merge(calc::router())
        .merge(view_as::router())
        .merge(views::router())
        .merge(link_options::router())
        .merge(comments::router())
        .merge(disclosures::router())
        .merge(datasets::router())
        .merge(clients::router())
        .merge(bundle::router())
        .merge(ownership::router())
        .merge(templates::router())
        .merge(oauth::me_router())
        .merge(framework::router())
        .merge(policy::router())
        .merge(governance::router())
        .merge(jobs::router())
        .merge(settings::router())
        .merge(imports::router())
        .merge(exports::router())
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
    let oauth_root = oauth::root_router().with_state(state.clone());
    let ws_root = workspaces::router().with_state(state.clone());
    let admin_root = admin::router().merge(admin_workspaces::router()).merge(access_grants::router()).merge(access_requests::admin_router()).merge(library::router()).merge(authority::router()).with_state(state.clone());

    Router::new()
        .route("/healthz", get(health_check))
        .merge(Router::new().route("/readyz", get(ready_check)).with_state(state))
        .merge(discovery_routes)
        .merge(oauth_root)
        .merge(admin_root)
        .merge(ws_root)
        .nest("/api/v1", api_v1)
        .nest("/api", mcp_root)
        .nest("/scim/v2", scim_v2)
        // Last, so it sees every route's answer. It writes a receipt for a denial, a privileged action and a request under view-as.
        .layer(axum::middleware::from_fn_with_state(state_for_receipts, crate::service::receipts::layer))
}

/// Ready when this instance can read its database: `refresh` succeeds. A load balancer sends traffic only to a ready
/// instance. `/healthz` below checks nothing, so a database outage does not make an orchestrator restart the process.
async fn ready_check(State(state): State<SharedState>) -> impl IntoResponse {
    match state.refresh() {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ready" }))).into_response(),
        Err(e) => crate::service::ServiceError::StorageUnavailable(format!("The server cannot read its database: {e}")).into_response(),
    }
}

async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "service": "scaffoldry-api",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
