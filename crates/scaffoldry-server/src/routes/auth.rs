//! Authentication, Session Management, and Administrative Impersonation Endpoints
//! Enforces Cedar Policy Attribute-Based Access Control and cryptographic audit logging.

use crate::state::{AuthSession, AuthUser, RecordDecisionInput, SharedState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
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
        .route("/auth/login", post(login))
        .route("/auth/me", get(get_current_user))
        .route("/auth/impersonate", post(impersonate_user))
        .route("/auth/stop-impersonate", post(stop_impersonation))
        .route("/auth/logout", post(logout))
        .route("/auth/directory", get(list_directory_users))
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub eppn: String,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImpersonatePayload {
    pub target_eppn: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: AuthUser,
    pub is_impersonating: bool,
    pub original_admin: Option<AuthUser>,
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|val| {
            val.strip_prefix("Bearer ")
                .map(|stripped| stripped.trim().to_string())
        })
}

async fn login(
    State(state): State<SharedState>,
    Json(payload): Json<LoginPayload>,
) -> impl IntoResponse {
    let directory = state.directory.read().unwrap();
    let user_match = directory.iter().find(|u| u.eppn == payload.eppn);

    match user_match {
        Some(user) => {
            let token = format!("sct_{}", Uuid::new_v4().simple());
            let session = AuthSession {
                token: token.clone(),
                user: user.clone(),
                original_admin: None,
                created_at: Utc::now().to_rfc3339(),
            };

            let mut sessions = state.sessions.write().unwrap();
            sessions.insert(token.clone(), session);

            (
                StatusCode::OK,
                Json(json!(AuthResponse {
                    token,
                    user: user.clone(),
                    is_impersonating: false,
                    original_admin: None,
                })),
            )
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Invalid institutional credentials or unknown identity" })),
        ),
    }
}

async fn get_current_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Missing or malformed Authorization header" })),
            );
        }
    };

    let sessions = state.sessions.read().unwrap();
    match sessions.get(&token) {
        Some(session) => (
            StatusCode::OK,
            Json(json!(AuthResponse {
                token: session.token.clone(),
                user: session.user.clone(),
                is_impersonating: session.original_admin.is_some(),
                original_admin: session.original_admin.clone(),
            })),
        ),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Session expired or invalid" })),
        ),
    }
}

async fn impersonate_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ImpersonatePayload>,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Authorization token required for administrative impersonation" })),
            );
        }
    };

    // 1. Resolve active session
    let (caller_user, active_original_admin) = {
        let sessions = state.sessions.read().unwrap();
        match sessions.get(&token) {
            Some(s) => (s.user.clone(), s.original_admin.clone()),
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "Invalid session" })),
                );
            }
        }
    };

    // If currently impersonating, the real admin is active_original_admin, otherwise caller_user
    let real_admin = active_original_admin.unwrap_or(caller_user.clone());

    // 2. Cedar Policy Authorization Check
    let auth_check = state.policy_engine.authorize_institutional_action(
        &real_admin.eppn,
        &real_admin.affiliation,
        &real_admin.department,
        "impersonate",
        &payload.target_eppn,
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
                );
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Policy evaluation error: {e}") })),
            );
        }
    }

    // 3. Locate target user in directory
    let target_user = {
        let directory = state.directory.read().unwrap();
        match directory.iter().find(|u| u.eppn == payload.target_eppn) {
            Some(u) => u.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "Target user not found in institutional directory" })),
                );
            }
        }
    };

    // 4. Create new impersonation session
    let imp_token = format!("sct_imp_{}", Uuid::new_v4().simple());
    let imp_session = AuthSession {
        token: imp_token.clone(),
        user: target_user.clone(),
        original_admin: Some(real_admin.clone()),
        created_at: Utc::now().to_rfc3339(),
    };

    {
        let mut sessions = state.sessions.write().unwrap();
        sessions.insert(imp_token.clone(), imp_session);
    }

    // 5. Append immutable audit entry to cryptographic decision ledger
    let audit_payload = json!({
        "admin_eppn": real_admin.eppn,
        "admin_name": real_admin.name,
        "target_eppn": target_user.eppn,
        "target_name": target_user.name,
        "action": "impersonate_start",
    });

    let _ = state.append_ledger_entry(RecordDecisionInput {
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
    });

    (
        StatusCode::OK,
        Json(json!(AuthResponse {
            token: imp_token,
            user: target_user,
            is_impersonating: true,
            original_admin: Some(real_admin),
        })),
    )
}

async fn stop_impersonation(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Authorization token required" })),
            );
        }
    };

    let session = {
        let sessions = state.sessions.read().unwrap();
        match sessions.get(&token) {
            Some(s) => s.clone(),
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "Invalid session" })),
                );
            }
        }
    };

    let original_admin = match session.original_admin {
        Some(admin) => admin,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Current session is not an impersonation session" })),
            );
        }
    };

    // Remove impersonation token
    {
        let mut sessions = state.sessions.write().unwrap();
        sessions.remove(&token);
    }

    // Create or retrieve restore admin token
    let restore_token = format!("sct_{}", Uuid::new_v4().simple());
    let restore_session = AuthSession {
        token: restore_token.clone(),
        user: original_admin.clone(),
        original_admin: None,
        created_at: Utc::now().to_rfc3339(),
    };

    {
        let mut sessions = state.sessions.write().unwrap();
        sessions.insert(restore_token.clone(), restore_session);
    }

    // Append termination audit entry to cryptographic decision ledger
    let audit_payload = json!({
        "admin_eppn": original_admin.eppn,
        "impersonated_eppn": session.user.eppn,
        "action": "impersonate_end",
    });

    let _ = state.append_ledger_entry(RecordDecisionInput {
        principal: original_admin.eppn.clone(),
        organization_code: "DIV-SECURITY-CENTRAL".to_string(),
        app_slug: None,
        decision_type: DecisionType::ImpersonationSessionEnded,
        oscal_control_id: "AC-02".to_string(),
        rationale: format!(
            "Enterprise administrator {} concluded impersonation of {}",
            original_admin.name, session.user.name
        ),
        payload: &audit_payload,
    });

    (
        StatusCode::OK,
        Json(json!(AuthResponse {
            token: restore_token,
            user: original_admin,
            is_impersonating: false,
            original_admin: None,
        })),
    )
}

async fn logout(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = extract_bearer_token(&headers) {
        let mut sessions = state.sessions.write().unwrap();
        sessions.remove(&token);
    }
    (StatusCode::OK, Json(json!({ "status": "logged_out" })))
}

async fn list_directory_users(State(state): State<SharedState>) -> impl IntoResponse {
    let directory = state.directory.read().unwrap();
    (StatusCode::OK, Json(json!(*directory)))
}
