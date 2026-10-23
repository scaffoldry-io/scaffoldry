//! View as another user (access-rules phase 6).
//!
//! An app owner or admin, or a Platform Admin, opens a read-only session that shows an app the way
//! another person sees it. The token's rights are that person's, intersected with the starter's
//! (`scope::scope_of` and `scope::field_visibility_of` do the intersection). It reaches one app,
//! only reads, and lasts 30 minutes. `guard::view_as_gate` enforces the first two.

use crate::service::access::{authorize_app, AppAction};
use crate::service::identity::{generate_raw_token, hash_token, resolve_user};
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, RecordDecisionInput, SharedState};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};

pub const VIEW_AS_MINUTES: i64 = 30;

pub fn start(caller: &AuthUser, app_slug: &str, eppn: &str, state: &SharedState) -> Result<Value, ServiceError> {
    if caller.view_as_by.is_some() {
        return Err(ServiceError::ViewAsReadOnly("You are viewing as someone else. Exit first.".to_string()));
    }
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let target = resolve_user(eppn, state).ok_or_else(|| ServiceError::not_found(format!("No one is known as {eppn}.")))?;
    let organization_code = state
        .engine
        .read()
        .ok()
        .and_then(|e| e.resolve_by_slug(app_slug).map(|m| m.organization_code.clone()))
        .ok_or_else(|| ServiceError::not_found("That app does not exist."))?;

    let raw = generate_raw_token();
    let now = chrono::Utc::now();
    let token = ApiToken {
        token_hash: hash_token(&raw),
        id: uuid::Uuid::new_v4(),
        kind: "viewas".to_string(),
        eppn: target.eppn.clone(),
        label: format!("View {app_slug} as {}", target.eppn),
        original_admin: Some(caller.eppn.clone()),
        created_at: now,
        expires_at: now + chrono::Duration::minutes(VIEW_AS_MINUTES),
        last_used_at: None,
        revoked_at: None,
        app_slug: Some(app_slug.to_string()),
    };

    let payload = json!({ "viewer": caller.eppn, "target": target.eppn, "app_slug": app_slug });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code,
            app_slug: Some(app_slug.to_string()),
            decision_type: DecisionType::ViewAsStarted,
            oscal_control_id: "AC-06".to_string(),
            rationale: format!("{} started a read-only view of {app_slug} as {}", caller.name, target.name),
            payload: &payload,
        })
        .map_err(|e| ServiceError::internal(format!("Could not record the ledger entry: {e}")))?;
    state.persist_api_token(&token).map_err(|e| ServiceError::internal(format!("Could not save the token: {e}")))?;

    Ok(json!({
        "token": raw,
        "expires_at": token.expires_at.to_rfc3339(),
        "app_slug": app_slug,
        "target": { "eppn": target.eppn, "name": target.name },
        "read_only": true,
    }))
}

/// Ends the session the token belongs to.
pub fn stop(row: &ApiToken, state: &SharedState) -> Result<Value, ServiceError> {
    if !matches!(row.kind.as_str(), "viewas" | "preview") {
        return Err(ServiceError::bad_request("This is not a view-as or preview session."));
    }
    let _ = state.revoke_api_token(row.id, &row.eppn, true);
    Ok(json!({ "status": "stopped" }))
}
