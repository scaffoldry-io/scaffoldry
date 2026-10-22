//! The apps a person can reach, and how they reach each one. A person with an app-only membership has no
//! workspace to open, so this list is how they find the app.

use crate::service::access::{authorize_app_with_role, principal_of, AppAction, KnownCaller};
use crate::service::members;
use crate::state::{AuthUser, SharedState};
use serde_json::{json, Value};

/// How the caller reaches an app: `admin` for a Platform Admin, `workspace` through a workspace-wide
/// membership, or `app` through a membership limited to that app.
pub fn my_apps(caller: &AuthUser, state: &SharedState) -> Vec<Value> {
    let manifests: Vec<_> = state
        .engine
        .read()
        .map(|e| e.list_manifests())
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.workspace_id.is_some())
        .collect();
    // The roles come from one read of the person's appointments, groups and grants, for every app and workspace.
    let pairs: Vec<(String, String)> = manifests.iter().filter_map(|m| Some((m.workspace_id.clone()?, m.slug.clone()))).collect();
    let principal = principal_of(state, caller);
    let in_app = members::member_roles_in_apps(&caller.eppn, &pairs, state);
    let mut workspaces: Vec<String> = pairs.iter().map(|(w, _)| w.clone()).collect();
    workspaces.sort();
    workspaces.dedup();
    let in_workspace: std::collections::HashMap<String, bool> = workspaces
        .iter()
        .cloned()
        .zip(members::member_roles(&caller.eppn, &workspaces, state).into_iter().map(|r| r.is_some()))
        .collect();

    let mut rows: Vec<Value> = manifests
        .into_iter()
        .zip(in_app)
        .filter_map(|(m, role)| {
            let workspace = m.workspace_id.clone()?;
            authorize_app_with_role(caller, &m.slug, AppAction::Read, Some(KnownCaller { role, principal: &principal }), state).ok()?;
            let scope = if caller.affiliation == "central_admin" {
                "admin"
            } else if in_workspace.get(&workspace).copied().unwrap_or(false) {
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
