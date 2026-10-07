//! Authentication, Session Management, and Administrative Impersonation Endpoints
//! Enforces OAuth 2.1 / OIDC JWTs, Cedar Policy ABAC, and cryptographic audit logging.

use crate::guard::{bearer_token, session_user, unauthorized};
use crate::jwt::{mint_impersonation_jwt, mint_test_jwt, validate_jwt, get_jwt_secret, get_jwt_issuer, TestJwtParams};
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

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/auth/token", post(issue_test_token))
        .route("/auth/me", get(get_current_user))
        .route("/auth/impersonate", post(impersonate_user))
        .route("/auth/stop-impersonate", post(stop_impersonation))
        .route("/auth/logout", post(logout))
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub eppn: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role_title: Option<String>,
    #[serde(default)]
    pub affiliation: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub expires_in_secs: Option<i64>,
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
}

/// Issues a genuine signed OAuth 2.1 / OIDC JWT for dev, testing, and IdP callback workflows.
async fn issue_test_token(Json(payload): Json<TokenRequest>) -> impl IntoResponse {
    if std::env::var("SCAFFOLDRY_ENV").as_deref() == Ok("production") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Test token issuance endpoint is disabled in production"})),
        );
    }

    let (name, role_title, affiliation, department) = resolve_defaults(
        &payload.eppn,
        payload.name,
        payload.role_title,
        payload.affiliation,
        payload.department,
    );

    let user = AuthUser {
        eppn: payload.eppn.clone(),
        name: name.clone(),
        role_title: role_title.clone(),
        affiliation: affiliation.clone(),
        department: department.clone(),
    };

    let token = match mint_test_jwt(TestJwtParams {
        eppn: payload.eppn,
        name,
        role_title,
        affiliation,
        department,
        expires_in_secs: payload.expires_in_secs.unwrap_or(3600),
    }) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Token minting failed: {e}")})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(json!(AuthResponse {
            token,
            user,
            is_impersonating: false,
            original_admin: None,
        })),
    )
}

fn resolve_defaults(
    eppn: &str,
    name: Option<String>,
    role_title: Option<String>,
    affiliation: Option<String>,
    department: Option<String>,
) -> (String, String, String, String) {
    if eppn == "jordan.lee@state.edu" {
        (
            name.unwrap_or_else(|| "Jordan Lee".to_string()),
            role_title.unwrap_or_else(|| "Central Enterprise Administrator".to_string()),
            "central_admin".to_string(),
            department.unwrap_or_else(|| "Central IT & Institutional Governance".to_string()),
        )
    } else if eppn.contains("curie") {
        (
            name.unwrap_or_else(|| "Dr. Marie Curie".to_string()),
            role_title.unwrap_or_else(|| "Professor & Lab Director".to_string()),
            "faculty".to_string(),
            department.unwrap_or_else(|| "biology".to_string()),
        )
    } else if eppn.contains("sarah") || eppn.contains("connor") {
        (
            name.unwrap_or_else(|| "Dr. Sarah Connor".to_string()),
            role_title.unwrap_or_else(|| "Department Chair & Professor".to_string()),
            "faculty".to_string(),
            department.unwrap_or_else(|| "Computer Science".to_string()),
        )
    } else if eppn.contains("vance") {
        (
            name.unwrap_or_else(|| "Marcus Vance".to_string()),
            role_title.unwrap_or_else(|| "Senior Research Administrator".to_string()),
            "staff".to_string(),
            department.unwrap_or_else(|| "Office of Sponsored Programs".to_string()),
        )
    } else if eppn.contains("rodriguez") {
        (
            name.unwrap_or_else(|| "Elena Rodriguez".to_string()),
            role_title.unwrap_or_else(|| "IRB & Research Compliance Analyst".to_string()),
            "compliance".to_string(),
            department.unwrap_or_else(|| "Institutional Review Board".to_string()),
        )
    } else if eppn.contains("einstein") {
        (
            name.unwrap_or_else(|| "Albert Einstein".to_string()),
            role_title.unwrap_or_else(|| "Physics Research Fellow".to_string()),
            "faculty".to_string(),
            department.unwrap_or_else(|| "physics".to_string()),
        )
    } else if eppn.contains("student") {
        (
            name.unwrap_or_else(|| "Alex Smith".to_string()),
            role_title.unwrap_or_else(|| "Graduate Research Assistant".to_string()),
            "student".to_string(),
            department.unwrap_or_else(|| "biology".to_string()),
        )
    } else {
        // Disallow arbitrary escalation to central_admin
        let safe_affiliation = affiliation
            .filter(|a| a != "central_admin")
            .unwrap_or_else(|| "member".to_string());
        (
            name.unwrap_or_else(|| eppn.to_string()),
            role_title.unwrap_or_else(|| "Member".to_string()),
            safe_affiliation,
            department.unwrap_or_else(|| "general".to_string()),
        )
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

    // Check if token is a JWT
    let secret = get_jwt_secret();
    let issuer = get_jwt_issuer();
    if let Ok(claims) = validate_jwt(token, &secret, &issuer) {
        return (
            StatusCode::OK,
            Json(json!(AuthResponse {
                token: token.to_string(),
                user: claims.to_auth_user(),
                is_impersonating: claims.original_admin.is_some(),
                original_admin: claims.original_admin,
            })),
        )
            .into_response();
    }

    // Check if token is in active sessions (e.g. database persisted session)
    if let Ok(sessions) = state.sessions.read() {
        if let Some(session) = sessions.get(token) {
            return (
                StatusCode::OK,
                Json(json!(AuthResponse {
                    token: session.token.clone(),
                    user: session.user.clone(),
                    is_impersonating: session.original_admin.is_some(),
                    original_admin: session.original_admin.clone(),
                })),
            )
                .into_response();
        }
    }

    unauthorized()
}

async fn impersonate_user(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ImpersonatePayload>,
) -> impl IntoResponse {
    let caller_user = match session_user(&state, &headers) {
        Some(u) => u,
        None => return unauthorized(),
    };

    // Check if currently already in an impersonation session
    let token = bearer_token(&headers).unwrap_or_default();
    let secret = get_jwt_secret();
    let issuer = get_jwt_issuer();
    let active_original_admin = if let Ok(claims) = validate_jwt(token, &secret, &issuer) {
        claims.original_admin
    } else if let Ok(sessions) = state.sessions.read() {
        sessions.get(token).and_then(|s| s.original_admin.clone())
    } else {
        None
    };

    let real_admin = active_original_admin.unwrap_or(caller_user);

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

    // 3. Resolve target user identity
    let (target_name, target_role, target_aff, target_dept) = resolve_defaults(
        &payload.target_eppn,
        payload.name,
        None,
        payload.affiliation,
        payload.department,
    );

    let target_user = AuthUser {
        eppn: payload.target_eppn.clone(),
        name: target_name,
        role_title: target_role,
        affiliation: target_aff,
        department: target_dept,
    };

    let imp_token = match mint_impersonation_jwt(&target_user, &real_admin, 3600) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Impersonation token minting failed: {e}")})),
            )
                .into_response();
        }
    };

    // Record session in state and repository for stateful tracking
    let imp_session = AuthSession {
        token: imp_token.clone(),
        user: target_user.clone(),
        original_admin: Some(real_admin.clone()),
        created_at: Utc::now().to_rfc3339(),
    };

    if let Ok(mut sessions) = state.sessions.write() {
        sessions.insert(imp_token.clone(), imp_session.clone());
    }
    if let Some(ref repo) = state.repository {
        let _ = repo.upsert_session(&imp_session);
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

    let secret = get_jwt_secret();
    let issuer = get_jwt_issuer();
    let (target_eppn, target_name, original_admin) = if let Ok(claims) = validate_jwt(token, &secret, &issuer) {
        match claims.original_admin {
            Some(admin) => (claims.sub, claims.name.unwrap_or_default(), admin),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "Current session is not an impersonation session" })),
                )
                    .into_response();
            }
        }
    } else if let Ok(sessions) = state.sessions.read() {
        match sessions.get(token) {
            Some(s) => match &s.original_admin {
                Some(admin) => (s.user.eppn.clone(), s.user.name.clone(), admin.clone()),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "Current session is not an impersonation session" })),
                    )
                        .into_response();
                }
            },
            None => return unauthorized(),
        }
    } else {
        return unauthorized();
    };

    // Remove impersonation session
    if let Ok(mut sessions) = state.sessions.write() {
        sessions.remove(token);
    }
    if let Some(ref repo) = state.repository {
        let _ = repo.delete_session(token);
    }

    // Mint restored admin JWT
    let restore_token = match mint_test_jwt(TestJwtParams {
        eppn: original_admin.eppn.clone(),
        name: original_admin.name.clone(),
        role_title: original_admin.role_title.clone(),
        affiliation: original_admin.affiliation.clone(),
        department: original_admin.department.clone(),
        expires_in_secs: 3600,
    }) {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Restore token minting failed: {e}")})),
            )
                .into_response();
        }
    };

    // Append termination audit entry to cryptographic decision ledger
    let audit_payload = json!({
        "admin_eppn": original_admin.eppn,
        "impersonated_eppn": target_eppn,
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
            original_admin.name, target_name
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
        .into_response()
}

async fn logout(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = bearer_token(&headers) {
        if let Ok(mut sessions) = state.sessions.write() {
            sessions.remove(token);
        }
        if let Some(ref repo) = state.repository {
            let _ = repo.delete_session(token);
        }
    }
    (StatusCode::OK, Json(json!({ "status": "logged_out" })))
}
