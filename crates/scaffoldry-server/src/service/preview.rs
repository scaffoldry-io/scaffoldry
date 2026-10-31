//! Open a proposed app before it is live (proposal-preview phase 2).
//!
//! A preview is an app under the slug `<live slug>~preview~<proposal id>`. Each server builds it from the stored
//! proposal when a request for it arrives, so no server has to be the one that opened it. It holds no record. Reads
//! of its records are reads of the live app's, through the stricter of the live rules and the proposed rules. Its
//! token only reads (`guard::view_as_gate` refuses the rest) and lasts 30 minutes. It stops working when the
//! proposal is decided. NIST SP 800-53 CM-3, CM-3(2), CM-4.

use crate::service::access::authorize_workspace;
use crate::service::identity::{generate_raw_token, hash_token};
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, RecordDecisionInput, SharedState};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::{AppManifest, HostRouter, PREVIEW_MARK};
use serde_json::{json, Value};

pub const PREVIEW_MINUTES: i64 = 30;

/// The slug the preview of `proposal_id` has.
pub fn shadow_slug(live: &str, proposal_id: &str) -> String {
    format!("{live}{PREVIEW_MARK}{proposal_id}")
}

/// The live app a slug stands for. A slug that is not a preview is its own.
pub use scaffoldry_engine::stored_slug;

pub fn is_preview(slug: &str) -> bool {
    slug.contains(PREVIEW_MARK)
}

fn proposal_of(slug: &str) -> Option<uuid::Uuid> {
    slug.split_once(PREVIEW_MARK).and_then(|(_, id)| uuid::Uuid::parse_str(id).ok())
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// The manifest the proposal would make live: its own, or the live one when it only changes rules.
pub(crate) fn proposed_manifest(row: &crate::repository_proposals::ProposalRow, state: &SharedState) -> Result<AppManifest, ServiceError> {
    let live_views = state.app_views.read().unwrap_or_else(|p| p.into_inner()).get(&row.app_slug).cloned().unwrap_or_default();
    let mut manifest = match &row.manifest {
        Some(value) => crate::repository_views::merged(state.repository.parse_manifest(value.clone()).map_err(internal)?, &live_views),
        None => state
            .engine
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .resolve_by_slug(&row.app_slug)
            .cloned()
            .ok_or_else(|| ServiceError::not_found(format!("App '{}' not found", row.app_slug)))?,
    };
    manifest.slug = shadow_slug(&row.app_slug, &row.id.to_string());
    manifest.custom_domain = None;
    manifest.custom_domain_verified = false;
    Ok(manifest)
}

/// Whether this person may open a preview of the proposal: the one who made it, a person who may decide it, or a
/// Platform Admin. `manage_workspace` is the right the decision needs.
pub(crate) fn authorize(caller: &AuthUser, row: &crate::repository_proposals::ProposalRow, state: &SharedState) -> Result<(), ServiceError> {
    if row.proposed_by.eq_ignore_ascii_case(&caller.eppn) {
        return Ok(());
    }
    authorize_workspace(caller, &row.workspace_id, "manage_workspace", state)
}

pub fn start(caller: &AuthUser, proposal_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    if caller.view_as_by.is_some() {
        return Err(ServiceError::ViewAsReadOnly("You are viewing as someone else. Exit first.".to_string()));
    }
    let id = uuid::Uuid::parse_str(proposal_id).map_err(|_| ServiceError::bad_request("Argument 'proposal_id' must be a proposal id"))?;
    let row = state.repository.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Proposal not found"))?;
    authorize(caller, &row, state)?;
    if row.status != "Pending" {
        return Err(ServiceError::Conflict(format!("This proposal is already {}", row.status)));
    }
    let digest = crate::service::proposals::digest_of(&row, state)?;
    let shadow = shadow_slug(&row.app_slug, &row.id.to_string());
    load(&shadow, &row, state)?;

    let raw = generate_raw_token();
    let now = chrono::Utc::now();
    let token = ApiToken {
        token_hash: hash_token(&raw),
        id: uuid::Uuid::new_v4(),
        kind: "preview".to_string(),
        eppn: caller.eppn.clone(),
        label: format!("Preview of proposal {}", row.id),
        original_admin: Some(caller.eppn.clone()),
        created_at: now,
        expires_at: now + chrono::Duration::minutes(PREVIEW_MINUTES),
        last_used_at: None,
        revoked_at: None,
        app_slug: Some(shadow.clone()),
        scope: None,
    };

    let organization_code = state
        .engine
        .read()
        .ok()
        .and_then(|e| e.resolve_by_slug(&row.app_slug).map(|m| m.organization_code.clone()))
        .or_else(|| row.manifest.as_ref().and_then(|m| m["organization_code"].as_str().map(str::to_string)))
        .unwrap_or_default();
    let payload = json!({ "proposal_id": row.id, "digest": digest, "previewer": caller.eppn, "app_slug": row.app_slug });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code,
            app_slug: Some(row.app_slug.clone()),
            decision_type: DecisionType::ProposalPreviewed,
            oscal_control_id: "CM-03".to_string(),
            rationale: format!("{} opened a read-only preview of proposal {} for {}", caller.name, row.id, row.app_slug),
            payload: &payload,
        })
        .map_err(|e| ServiceError::internal(format!("Could not record the ledger entry: {e}")))?;
    state.persist_api_token(&token).map_err(|e| ServiceError::internal(format!("Could not save the token: {e}")))?;

    Ok(json!({
        "token": raw,
        "expires_at": token.expires_at.to_rfc3339(),
        "app_slug": shadow,
        "live_app_slug": row.app_slug,
        "proposal_id": row.id,
        "digest": digest,
        "summary": row.summary,
        "read_only": true,
    }))
}

/// Builds the preview app in this server when it does not hold it yet.
fn load(shadow: &str, row: &crate::repository_proposals::ProposalRow, state: &SharedState) -> Result<(), ServiceError> {
    if state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(shadow).is_some() {
        return Ok(());
    }
    let manifest = proposed_manifest(row, state)?;
    state.hold_preview(manifest).map_err(internal)
}

/// Whether the preview token may be used now. It may while its proposal is pending. This also builds the preview
/// app in this server, and drops it when the proposal has been decided.
pub fn admit(row: &ApiToken, state: &SharedState) -> bool {
    let Some(shadow) = row.app_slug.as_deref() else { return false };
    let Some(id) = proposal_of(shadow) else { return false };
    match state.repository.get_proposal(id) {
        Ok(Some(proposal)) if proposal.status == "Pending" => load(shadow, &proposal, state).is_ok(),
        Ok(_) => {
            state.engine.write().unwrap_or_else(|p| p.into_inner()).remove_preview(shadow);
            false
        }
        Err(_) => false,
    }
}
