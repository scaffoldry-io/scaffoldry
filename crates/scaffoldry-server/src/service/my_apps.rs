//! The apps a person can reach, and how they reach each one. A person with an app-only membership has no
//! workspace to open, so this list is how they find the app.

use crate::service::access::{authorize_app, AppAction};
use crate::service::members;
use crate::state::{AuthUser, SharedState};
use serde_json::{json, Value};

/// How the caller reaches an app: `admin` for a Platform Admin, `workspace` through a workspace-wide
/// membership, or `app` through a membership limited to that app.
pub fn my_apps(caller: &AuthUser, state: &SharedState) -> Vec<Value> {
    let manifests = state.engine.read().map(|e| e.list_manifests()).unwrap_or_default();
    let mut rows: Vec<Value> = manifests
        .into_iter()
        .filter_map(|m| {
            let workspace = m.workspace_id.clone()?;
            authorize_app(caller, &m.slug, AppAction::Read, state).ok()?;
            let scope = if caller.affiliation == "central_admin" {
                "admin"
            } else if members::member_role(caller, &workspace, state).is_some() {
                "workspace"
            } else {
                "app"
            };
            Some(json!({
                "slug": m.slug, "title": m.title, "description": m.description,
                "department": m.department, "workspace_id": workspace, "scope": scope,
            }))
        })
        .collect();
    rows.sort_by_key(|r| r["title"].as_str().unwrap_or("").to_lowercase());
    rows
}
