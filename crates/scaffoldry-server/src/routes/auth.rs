//! Authentication, Token Management, and Administrative Impersonation Endpoints
//! Enforces stored api_tokens rows, Cedar Policy ABAC, and cryptographic audit logging.

use crate::guard::{bearer_token, session_user, unauthorized};
use crate::service::identity::{generate_raw_token, hash_token, resolve_user};
use crate::state::{ApiToken, AuthUser, RecordDecisionInput, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use scaffoldry_core::DecisionType;
use scaffoldry_policy::PolicyDecision;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/auth/tokens", get(list_tokens).post(create_token))
        .route("/auth/tokens/{id}", delete(revoke_token))
        .route("/auth/me", get(get_current_user))
        .route("/auth/impersonate", post(impersonate_user))
        .route("/auth/stop-impersonate", post(stop_impersonation))
        .route("/auth/logout", post(logout))
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenResponse {
    pub id: Uuid,
    pub token: String,
    pub label: String,
    pub kind: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct TokenSummary {
    pub id: Uuid,
    pub label: String,
    pub kind: String,
    pub created_at: String,
    pub expires_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImpersonatePayload {
    pub target_eppn: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub affiliation: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: AuthUser,
    pub is_impersonating: bool,
    pub original_admin: Option<AuthUser>,
    pub is_platform_admin: bool,
}

async fn list_tokens(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user = match session_user(&state, &headers) {
        Some(u) => u,
        None => return unauthorized(),
    };

    let tokens = state.list_api_tokens(&user.eppn);
    let summaries: Vec<TokenSummary> = tokens
        .into_iter()
        .map(|t| TokenSummary {
            id: t.id,
            label: t.label,
            kind: t.kind,
            created_at: t.created_at.to_rfc3339(),
            expires_at: t.expires_at.to_rfc3339(),
            last_used_at: t.last_used_at.map(|d| d.to_rfc3339()),
        })
        .collect();

    (StatusCode::OK, Json(summaries)).into_response()
}

async fn create_token(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    let raw_bearer = match bearer_token(&headers) {
        Some(t) => t,
        None => return unauthorized(),
    };
    let caller_hash = hash_token(raw_bearer);
    let caller_row = match state.get_api_token(&caller_hash) {
        Some(r) => r,
        None => return unauthorized(),
    };

    if caller_row.revoked_at.is_some() || caller_row.expires_at < Utc::now() {
        return unauthorized();
    }

    // Agent tokens cannot mint tokens
    if caller_row.kind == "agent" {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Agent tokens cannot mint tokens" })),
        )
            .into_response();
    }

    let caller_user = match resolve_user(&caller_row.eppn, &state) {
        Some(u) => u,
        None => return unauthorized(),
    };

    let requested_kind = payload.kind.as_deref().unwrap_or("agent");
    if requested_kind != "agent" && requested_kind != "scim" {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid token kind: must be agent or scim" })),
        )
            .into_response();
    }

    if requested_kind == "scim" && caller_user.affiliation != "central_admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only Platform Admins can mint SCIM tokens" })),
        )
            .into_response();
    }

    if requested_kind == "agent" {
        if let Ok(settings) = state.settings.read() {
            if let Some(enabled) = settings.get("tokens.agent_enabled").and_then(|v| v.as_bool()) {
                if !enabled {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({ "error": "Agent token creation is disabled by platform policy" })),
                    ).into_response();
                }
            }
        }
    }

    let max_days = state.settings.read().ok()
        .and_then(|s| s.get("tokens.max_days").and_then(|v| v.as_i64()))
        .unwrap_or(90)
        .clamp(1, 365);
    let days = payload.days.unwrap_or(30).clamp(1, max_days);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::days(days);
    let raw_token = generate_raw_token();
    let token_hash = hash_token(&raw_token);
    let id = Uuid::new_v4();

    let new_token = ApiToken {
        token_hash,
        id,
        kind: requested_kind.to_string(),
        eppn: caller_user.eppn.clone(),
        label: payload.label.clone(),
        original_admin: None,
        created_at: now,
        expires_at,
        last_used_at: None,
        revoked_at: None,
    };

    if let Err(e) = state.persist_api_token(&new_token) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to persist token: {e}") })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(json!(CreateTokenResponse {
            id,
            token: raw_token,
            label: payload.label,
            kind: requested_kind.to_string(),
            created_at: now.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        })),
    )
        .into_response()
}

async fn revoke_token(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let caller = match session_user(&state, &headers) {
        Some(u) => u,
        None => return unauthorized(),
    };
    let is_platform_admin = caller.affiliation == "central_admin";

    match state.revoke_api_token(id, &caller.eppn, is_platform_admin) {
        Ok(true) => (StatusCode::OK, Json(json!({ "status": "revoked" }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Token not found" }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to revoke token: {e}") })),
        )
            .into_response(),
    }
}

async fn get_current_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let token = match bearer_token(&headers) {
        Some(t) => t,
        None => return unauthorized(),
    };
    let hash = hash_token(token);
    let row = match state.get_api_token(&hash) {
        Some(r) => r,
        None => return unauthorized(),
    };
    if row.revoked_at.is_some() || row.expires_at < Utc::now() {
        return unauthorized();
    }
    let user = match resolve_user(&row.eppn, &state) {
        Some(u) => u,
        None => return unauthorized(),
    };

    let is_impersonating = row.kind == "impersonation" && row.original_admin.is_some();
    let original_admin = row.original_admin.as_ref().and_then(|admin_eppn| resolve_user(admin_eppn, &state));
    let is_platform_admin = crate::service::admin::require_platform_admin(&user, &state).is_ok();

    (
        StatusCode::OK,
        Json(json!(AuthResponse {
            token: token.to_string(),
            user,
            is_impersonating,
            original_admin,
            is_platform_admin,
        })),
    )
        .into_response()
}

async fn impersonate_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ImpersonatePayload>,
) -> impl IntoResponse {
    let real_admin = match session_user(&state, &headers) {
        Some(u) => u,
        None => return unauthorized(),
    };

    // Check Cedar policy: principal must have central_admin affiliation
    let auth_check = state.policy_engine.authorize_institutional_action(
        &real_admin.eppn,
        &real_admin.affiliation,
        &real_admin.department,
        "impersonate",
        "system",
    );

    match auth_check {
        Ok(result) => {
            if result.decision != PolicyDecision::Allow {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({
                        "error": "Forbidden: Cedar policy denies Action::impersonate to non-central_admin principals",
                        "diagnostics": result.diagnostics,
                        "reasons": result.reasons,
                    })),
                )
                    .into_response();
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Policy evaluation error: {e}") })),
            )
                .into_response();
        }
    }

    let target_user = match resolve_user(&payload.target_eppn, &state) {
        Some(u) => u,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Target user not found: {}", payload.target_eppn) })),
            )
                .into_response();
        }
    };

    let raw_token = generate_raw_token();
    let token_hash = hash_token(&raw_token);
    let id = Uuid::new_v4();
    let now = Utc::now();
    let expires_at = now + chrono::Duration::hours(1);

    let imp_token = ApiToken {
        token_hash,
        id,
        kind: "impersonation".to_string(),
        eppn: target_user.eppn.clone(),
        label: format!("Impersonation of {}", target_user.eppn),
        original_admin: Some(real_admin.eppn.clone()),
        created_at: now,
        expires_at,
        last_used_at: None,
        revoked_at: None,
    };

    // Append immutable audit entry to cryptographic decision ledger
    let audit_payload = json!({
        "admin_eppn": real_admin.eppn,
        "admin_name": real_admin.name,
        "target_eppn": target_user.eppn,
        "target_name": target_user.name,
        "action": "impersonate_start",
    });

    if let Err(e) = state.append_ledger_entry(RecordDecisionInput {
        principal: real_admin.eppn.clone(),
        organization_code: "DIV-SECURITY-CENTRAL".to_string(),
        app_slug: None,
        decision_type: DecisionType::ImpersonationSessionStarted,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Enterprise administrator {} ({}) initiated verified user impersonation of {}",
            real_admin.name, real_admin.eppn, target_user.name
        ),
        payload: &audit_payload,
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to record audit ledger entry: {e}") })),
        )
            .into_response();
    }

    if let Err(e) = state.persist_api_token(&imp_token) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to persist impersonation token: {e}") })),
        )
            .into_response();
    }

    let is_target_platform_admin = crate::service::admin::require_platform_admin(&target_user, &state).is_ok();
    (
        StatusCode::OK,
        Json(json!(AuthResponse {
            token: raw_token,
            user: target_user,
            is_impersonating: true,
            original_admin: Some(real_admin),
            is_platform_admin: is_target_platform_admin,
        })),
    )
        .into_response()
}

async fn stop_impersonation(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let token = match bearer_token(&headers) {
        Some(t) => t,
        None => return unauthorized(),
    };
    let hash = hash_token(token);
    let row = match state.get_api_token(&hash) {
        Some(r) => r,
        None => return unauthorized(),
    };

    if row.kind != "impersonation" || row.original_admin.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Current session is not an impersonation session" })),
        )
            .into_response();
    }

    let admin_eppn = row.original_admin.clone().unwrap();
    let original_admin = match resolve_user(&admin_eppn, &state) {
        Some(a) => a,
        None => return unauthorized(),
    };
    let target_eppn = row.eppn.clone();
    let target_user = resolve_user(&target_eppn, &state);
    let target_name = target_user.map(|u| u.name).unwrap_or_else(|| target_eppn.clone());

    // Revoke the impersonation token
    let _ = state.revoke_api_token(row.id, &row.eppn, true);

    // Append termination audit entry to cryptographic decision ledger
    let audit_payload = json!({
        "admin_eppn": original_admin.eppn,
        "impersonated_eppn": target_eppn,
        "action": "impersonate_end",
    });

    if let Err(e) = state.append_ledger_entry(RecordDecisionInput {
        principal: original_admin.eppn.clone(),
        organization_code: "DIV-SECURITY-CENTRAL".to_string(),
        app_slug: None,
        decision_type: DecisionType::ImpersonationSessionEnded,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Enterprise administrator {} concluded impersonation of {}",
            original_admin.name, target_name
        ),
        payload: &audit_payload,
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to record audit ledger entry: {e}") })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "status": "stopped",
            "user": original_admin,
            "is_impersonating": false,
        })),
    )
        .into_response()
}

async fn logout(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = bearer_token(&headers) {
        let hash = hash_token(token);
        let _ = state.revoke_api_token_by_hash(&hash);
    }
    (StatusCode::OK, Json(json!({ "status": "logged_out" })))
}
