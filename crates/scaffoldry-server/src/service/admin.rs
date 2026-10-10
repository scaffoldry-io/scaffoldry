//! Administrative Service: Role Enforcement, Cursors, and Audited Writes

use crate::service::organizations::{is_platform_admin, OrgCaller};
use crate::service::ServiceError;
use crate::state::{AuthUser, OrganizationNode, RecordDecisionInput, RoleRow, SharedState};
use scaffoldry_core::ledger::DecisionType;

pub const ADMIN_ROUTES: &[(&str, &str)] = &[
    ("GET", "/admin/overview"),
];

pub fn require_platform_admin(caller: &AuthUser, state: &SharedState) -> Result<(), ServiceError> {
    let orgs: Vec<OrganizationNode> = state.organizations.read().unwrap().values().cloned().collect();
    let roles: Vec<RoleRow> = state.roles.read().unwrap().clone();
    let org_caller = OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };
    if is_platform_admin(&org_caller, &orgs, &roles) {
        Ok(())
    } else {
        Err(ServiceError::forbidden("Platform Admin role required"))
    }
}

pub fn require_platform_admin_or_compliance(caller: &AuthUser, state: &SharedState) -> Result<(), ServiceError> {
    if caller.affiliation == "compliance" {
        return Ok(());
    }
    require_platform_admin(caller, state)
}

pub fn encode_cursor(offset: usize) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(format!("offset:{offset}"))
}

pub fn decode_cursor(cursor: &str) -> Result<usize, ServiceError> {
    use base64::Engine;
    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(cursor)
        .map_err(|_| ServiceError::BadRequest("Invalid cursor format".to_string()))?;
    let decoded_str = String::from_utf8(decoded_bytes)
        .map_err(|_| ServiceError::BadRequest("Invalid cursor string".to_string()))?;
    if let Some(num_str) = decoded_str.strip_prefix("offset:") {
        num_str
            .parse::<usize>()
            .map_err(|_| ServiceError::BadRequest("Invalid cursor offset".to_string()))
    } else {
        Err(ServiceError::BadRequest("Invalid cursor content".to_string()))
    }
}

pub fn admin_write<F, R>(
    state: &SharedState,
    admin: &AuthUser,
    decision_type: DecisionType,
    oscal_control_id: &str,
    reason: &str,
    payload: &serde_json::Value,
    op: F,
) -> Result<R, ServiceError>
where
    F: FnOnce() -> Result<R, ServiceError>,
{
    let trimmed = reason.trim();
    if trimmed.is_empty() || trimmed.len() > 500 {
        return Err(ServiceError::BadRequest(
            "A reason between 1 and 500 characters is required for administrative actions".to_string(),
        ));
    }

    state
        .append_ledger_entry(RecordDecisionInput {
            principal: admin.eppn.clone(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type,
            oscal_control_id: oscal_control_id.to_string(),
            rationale: trimmed.to_string(),
            payload,
        })
        .map_err(|e| ServiceError::Internal(format!("Failed to record audit ledger: {e}")))?;

    op()
}
