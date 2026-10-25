//! Access model phase 11: hierarchies are created as a change a different Platform Admin confirms, and retired with a
//! reason. A hierarchy is never deleted. The change is decided in `access_grants::decide_change`, with the grant changes.

use crate::repository_grants::RetireOutcome;
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};

const KINDS: [&str; 2] = ["units", "people"];
const SOURCES: [&str; 4] = ["api", "scim", "import", "connection"];

fn bad(msg: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(msg.into())
}

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn key_ok(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && key.len() <= 32 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Stores a new hierarchy as a change. It exists once a different Platform Admin confirms it.
pub fn propose(state: &SharedState, caller: &AuthUser, body: &Value) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let key = text("key").ok_or_else(|| bad("key is required"))?;
    if !key_ok(key) {
        return Err(bad("key is 1 to 32 characters: lowercase letters, digits and '-', starting with a letter"));
    }
    let name = text("name").ok_or_else(|| bad("name is required"))?;
    if name.len() > 255 {
        return Err(bad("name may be at most 255 characters"));
    }
    let kind = text("kind").filter(|k| KINDS.contains(k)).ok_or_else(|| bad(format!("kind must be one of {}", KINDS.join(", "))))?;
    let source = text("source").filter(|s| SOURCES.contains(s)).ok_or_else(|| bad(format!("source must be one of {}", SOURCES.join(", "))))?;
    let reason = text("reason").filter(|r| r.len() <= 500).ok_or_else(|| bad("reason is required and may be at most 500 characters"))?;
    if state.repository.get_hierarchy(key).map_err(internal)?.is_some() {
        return Err(bad(format!("a hierarchy named '{key}' exists")));
    }
    let change = json!({ "kind": "hierarchy", "hierarchy": { "key": key, "name": name, "kind": kind, "source": source } });
    let id = state.repository.propose_access_change(&change, false, reason, &caller.eppn).map_err(internal)?;
    Ok(json!({ "change_id": id, "status": "Pending", "message": "A different Platform Admin must confirm this hierarchy before it exists." }))
}

/// Retires a hierarchy. Its grants stop reaching and its units stay. It narrows access, so it applies at once.
pub fn retire(state: &SharedState, caller: &AuthUser, key: &str, body: &Value) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty() && r.len() <= 500).ok_or_else(|| bad("reason is required and may be at most 500 characters"))?;
    let payload = json!({ "action": "hierarchy_retired", "key": key });
    let outcome = admin_write(state, caller, DecisionType::HierarchyChanged, "AC-02", reason, &payload, || state.repository.retire_hierarchy(key).map_err(internal))?;
    match outcome {
        RetireOutcome::Done => Ok(json!({ "key": key, "status": "Retired" })),
        RetireOutcome::Missing => Err(ServiceError::NotFound(format!("Hierarchy '{key}' not found"))),
        RetireOutcome::Protected => Err(ServiceError::conflict("The academic hierarchy is never retired")),
        RetireOutcome::AlreadyRetired => Err(ServiceError::conflict("That hierarchy is already retired")),
    }
}
