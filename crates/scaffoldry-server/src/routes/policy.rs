//! Cedar Policy Evaluation and Simulation Endpoints

use crate::state::SharedState;
use crate::guard::session_user;
use axum::http::HeaderMap;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/policies", get(list_policies))
        .route("/policies/simulate", post(simulate_policy))
}

async fn list_policies() -> impl IntoResponse {
    let policies = json!([
        {
            "id": "policy_01_dept_isolation",
            "name": "Departmental Data Boundary",
            "description": "Permit departmental members to read and write records in their own department",
            "effect": "permit",
            "cedar": "permit (principal, action in [Action::\"read\", Action::\"write\"], resource) when { principal.department == resource.department };"
        },
        {
            "id": "policy_02_ferpa_export_restriction",
            "name": "FERPA Export Restriction",
            "description": "Strict FERPA Policy: Forbid exporting records if sensitive, unless principal holds verified staff or compliance affiliation",
            "effect": "forbid",
            "cedar": "forbid (principal, action == Action::\"export\", resource) when { resource.is_ferpa_sensitive && principal.scoped_affiliation != \"staff\" && principal.scoped_affiliation != \"compliance\" && principal.scoped_affiliation != \"central_admin\" };"
        },
        {
            "id": "policy_03_dept_export_permit",
            "name": "Departmental Non-Restricted Export",
            "description": "Permit authorized departmental members to export non-restricted records",
            "effect": "permit",
            "cedar": "permit (principal, action == Action::\"export\", resource) when { principal.department == resource.department };"
        }
    ]);

    Json(policies)
}

async fn simulate_policy(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    if user.affiliation != "central_admin" && user.affiliation != "compliance" {
        return Err(ServiceError::forbidden("Forbidden: Requires Platform Admin or compliance affiliation").into_pair());
    }
    // The simulation names who it simulates. The server supplies no person, app or department of its own.
    let need = |field: &str| -> Result<&str, (StatusCode, Json<Value>)> {
        payload[field].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| ServiceError::bad_request(format!("{field} is required")).into_pair())
    };
    let eppn = need("eppn")?;
    let affiliation_str = need("affiliation")?;
    let action = need("action")?;
    let app_slug = need("app_slug")?;
    let department = need("department")?;

    // The simulated person's stored department. Defaults to the resource department.
    let principal_department = payload["principal_department"]
        .as_str()
        .unwrap_or(department);

    let is_ferpa_sensitive = payload["is_ferpa_sensitive"]
        .as_bool()
        .unwrap_or(false);

    let affiliation = match affiliation_str {
        "faculty" => "faculty",
        "student" => "student",
        "staff" => "staff",
        "employee" => "employee",
        _ => "member",
    };

    let result = crate::service::access::simulate_record_action(&state, eppn, affiliation, principal_department, action, app_slug, department, is_ferpa_sensitive)
        .map_err(|e| e.into_pair())?;

    let decision_str = if result.allowed { "Allow" } else { "Deny" };

    Ok(Json(json!({
        "decision": decision_str,
        "reasons": result.reasons,
        "diagnostics": result.diagnostics,
        "deciding_policy": result.policy,
        "principal": {
            "eppn": eppn,
            "affiliation": affiliation_str,
            "department": department
        },
        "resource": {
            "app_slug": app_slug,
            "department": department,
            "is_ferpa_sensitive": is_ferpa_sensitive
        },
        "action": action
    })))
}
