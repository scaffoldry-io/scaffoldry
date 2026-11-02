//! Integrations phase 1: the secrets routes' rules. A Platform Admin writes, replaces and deletes secrets, each with a
//! reason and a `SecretChanged` ledger entry that names the secret and never holds its value. Nobody reads a value back.

use crate::secrets::{self, SecretError};
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};

/// The longest value a secret may hold.
const MAX_VALUE_BYTES: usize = 16 * 1024;

fn from_secret(e: SecretError) -> ServiceError {
    match e {
        SecretError::MasterKeyMissing(m) => ServiceError::MasterKeyMissing(m),
        SecretError::NotFound(m) => ServiceError::NotFound(m),
        SecretError::Decrypt(m) => ServiceError::Internal(m),
        SecretError::Storage(m) => ServiceError::StorageUnavailable(m),
    }
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn reason_of(body: &Value) -> String {
    body.get("reason").and_then(Value::as_str).unwrap_or("").to_string()
}

fn check_name(name: &str) -> Result<(), ServiceError> {
    if secrets::valid_name(name) {
        Ok(())
    } else {
        Err(ServiceError::bad_request("A secret's name starts with a lowercase letter and holds 2 to 64 lowercase letters, digits and underscores"))
    }
}

/// Names, dates and how many things use each. Never a value.
pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    secrets::master_key(state).map_err(from_secret)?;
    Ok(json!({ "secrets": secrets::list(&state.repository).map_err(storage)? }))
}

/// Creates or replaces a secret.
pub fn put(caller: &AuthUser, name: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    check_name(name)?;
    let key = secrets::master_key(state).map_err(from_secret)?;
    let value = body.get("value").and_then(Value::as_str).unwrap_or("");
    if value.is_empty() || value.len() > MAX_VALUE_BYTES {
        return Err(ServiceError::bad_request("A secret needs a value of 1 to 16,384 bytes"));
    }
    let existed = secrets::list(&state.repository).map_err(storage)?.iter().any(|s| s.name == name);
    let action = if existed { "replaced" } else { "created" };
    admin_write(state, caller, DecisionType::SecretChanged, "SC-12", &reason_of(body), &json!({ "name": name, "action": action }), || {
        secrets::put(&state.repository, &key, name, value.as_bytes(), &caller.eppn).map_err(|e| match e {
            crate::repository::RepositoryError::Conflict(m) => ServiceError::conflict(m),
            e => storage(e),
        })
    })?;
    Ok(json!({ "name": name, "action": action }))
}

/// Deletes a secret nothing uses. One that something names is refused, and the refusal names what.
pub fn delete(caller: &AuthUser, name: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    check_name(name)?;
    secrets::master_key(state).map_err(from_secret)?;
    if !secrets::list(&state.repository).map_err(storage)?.iter().any(|s| s.name == name) {
        return Err(ServiceError::NotFound(format!("No secret is named {name}")));
    }
    let used_by = secrets::references(&state.repository, name).map_err(storage)?;
    if !used_by.is_empty() {
        return Err(ServiceError::ConstraintViolated(format!("The secret {name} is in use by {}. Change those first", used_by.join(", "))));
    }
    admin_write(state, caller, DecisionType::SecretChanged, "SC-12", &reason_of(body), &json!({ "name": name, "action": "deleted" }), || {
        secrets::delete(&state.repository, name).map_err(storage)
    })?;
    Ok(json!({ "name": name, "action": "deleted" }))
}
