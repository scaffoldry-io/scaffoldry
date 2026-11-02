//! Lifecycle phase 3: retention policies and certificates of destruction, for a Platform Admin or `compliance`.
//! The purge itself is `crate::purge`.

use crate::purge::{Policy, PLACEHOLDER_BASIS};
use crate::service::admin::{admin_write, require_platform_admin_or_compliance};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// Every policy, and whether any still has the placeholder basis, which the Overview shows as a banner.
pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let policies = state.repository.list_retention_policies().map_err(internal)?;
    let placeholder = policies.iter().any(|p| p.active && p.basis.trim() == PLACEHOLDER_BASIS);
    Ok(json!({ "policies": policies, "placeholder": placeholder }))
}

/// Adds a policy (`id` None) or changes one, with a reason, through the ledger entry `RetentionPolicyChanged`.
pub fn save(caller: &AuthUser, id: Option<Uuid>, body: &Value, state: &SharedState) -> Result<(Value, bool), ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
    let mut policy: Policy = serde_json::from_value(body.clone()).map_err(|e| ServiceError::BadRequest(format!("The policy is not a retention policy: {e}")))?;
    let before = match id {
        Some(id) => Some(state.repository.get_retention_policy(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound(format!("Retention policy '{id}' not found")))?),
        None => None,
    };
    policy.id = id;
    let problems = policy.problems();
    if !problems.is_empty() {
        return Err(ServiceError::BadRequest(format!("The policy has {} problem(s): {}", problems.len(), problems.join("; "))));
    }
    if policy.scope_kind == "workspace" && !state.workspaces.read().map_err(internal)?.contains_key(policy.workspace_id.as_deref().unwrap_or("")) {
        return Err(ServiceError::BadRequest("'workspace_id' names no workspace".to_string()));
    }
    let payload = json!({ "before": before, "after": policy });
    let saved = admin_write(state, caller, DecisionType::RetentionPolicyChanged, "SI-12", &reason, &payload, || {
        state.repository.save_retention_policy(&policy, &caller.eppn).map_err(|e| match e {
            crate::repository::RepositoryError::Conflict(m) => ServiceError::conflict(m),
            other => internal(other),
        })
    })?;
    let stored = state.repository.get_retention_policy(saved).map_err(internal)?;
    Ok((json!(stored), before.is_none()))
}

/// Queues a count, for each app, of what the next purge would destroy under a proposed policy.
pub fn preview(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let policy: Policy = serde_json::from_value(body.get("policy").cloned().unwrap_or(Value::Null)).map_err(|e| ServiceError::BadRequest(format!("'policy' is not a retention policy: {e}")))?;
    let problems = policy.problems();
    if !problems.is_empty() {
        return Err(ServiceError::BadRequest(format!("The policy has {} problem(s): {}", problems.len(), problems.join("; "))));
    }
    let job = crate::jobs::enqueue(&state.repository, "retention_preview", json!({ "policy": policy }), &caller.eppn, None).map_err(internal)?;
    Ok(json!({ "job_id": job }))
}

pub fn certificates(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    Ok(json!({ "certificates": state.repository.list_purge_certificates(500).map_err(internal)? }))
}

pub fn certificate(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Certificate not found".to_string()))?;
    state.repository.get_purge_certificate(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Certificate not found".to_string()))
}
