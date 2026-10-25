//! Access model phase 10: SCIM's `manager` attribute becomes a tree of supervisory units in each `people` hierarchy that
//! SCIM keeps. The writes are in `repository_supervision`. This reads the chain of managers and calls them.

use crate::state::{ScimUser, SharedState};

const MAX_DEPTH: usize = 50;

/// The manager a user names. SCIM gives `manager.value` as the manager's id. A user name is accepted too.
fn manager_of(state: &SharedState, user: &ScimUser) -> Option<ScimUser> {
    let value = user.enterprise_extension.as_ref()?.get("manager")?;
    let key = value.get("value").and_then(|v| v.as_str()).or_else(|| value.as_str()).filter(|s| !s.is_empty())?;
    state.user_by_id(key).or_else(|| crate::service::people::find_by_user_name(state, key))
}

/// The user's managers, nearest first, ending at the top or at a loop.
fn chain_of(state: &SharedState, user: &ScimUser) -> Vec<(String, String)> {
    let mut seen = vec![user.user_name.to_lowercase()];
    let mut chain = Vec::new();
    let mut at = user.clone();
    while chain.len() < MAX_DEPTH {
        let Some(manager) = manager_of(state, &at).filter(|m| m.active) else { break };
        if seen.contains(&manager.user_name.to_lowercase()) {
            break;
        }
        seen.push(manager.user_name.to_lowercase());
        let name = manager.name.get("formatted").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| manager.user_name.clone());
        chain.push((manager.user_name.clone(), name));
        at = manager;
    }
    chain
}

/// Keeps the supervision tree current for one user, after the SCIM write that changed them. A failure is logged and does
/// not undo the SCIM write: the next write of the user applies it again.
pub fn sync_user(state: &SharedState, user: &ScimUser) {
    let hierarchies = state.repository.scim_people_hierarchies().unwrap_or_default();
    if hierarchies.is_empty() {
        return;
    }
    let chain = if user.active { chain_of(state, user) } else { Vec::new() };
    for key in hierarchies {
        if let Err(e) = state.repository.apply_supervision(&key, &user.user_name, &chain, user.active) {
            eprintln!("supervision sync in {key} failed: {e:?}");
        }
    }
    if let Err(e) = state.refresh() {
        eprintln!("supervision refresh failed: {e:?}");
    }
}
