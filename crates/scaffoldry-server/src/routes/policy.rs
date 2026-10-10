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
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
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
    let eppn = payload["eppn"]
        .as_str()
        .unwrap_or("sim.user@university.edu");

    let affiliation_str = payload["affiliation"]
        .as_str()
        .unwrap_or("faculty");

    let action = payload["action"]
        .as_str()
        .unwrap_or("read");

    let app_slug = payload["app_slug"]
        .as_str()
        .unwrap_or("physics-admissions");

    let department = payload["department"]
        .as_str()
        .unwrap_or("physics");

    // The simulated person's stored department. Defaults to the resource department.
    let principal_department = payload["principal_department"]
        .as_str()
        .unwrap_or(department);

    let is_ferpa_sensitive = payload["is_ferpa_sensitive"]
        .as_bool()
        .unwrap_or(false);

    let affiliation = match affiliation_str {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };

    let realm = eppn.split('@').nth(1).unwrap_or("university.edu").to_string();

    let caller = EduPersonIdentity {
        eppn: eppn.to_string(),
        realm,
        affiliations: vec![affiliation],
    };

    let result = state
        .policy_engine
        .authorize_record_action(
            &caller,
            principal_department,
            action,
            app_slug,
            department,
            is_ferpa_sensitive,
        )
        .map_err(|e| ServiceError::internal(e.to_string()).into_pair())?;

    let decision_str = match result.decision {
        scaffoldry_policy::PolicyDecision::Allow => "Allow",
        scaffoldry_policy::PolicyDecision::Deny => "Deny",
    };

    Ok(Json(json!({
        "decision": decision_str,
        "reasons": result.reasons,
        "diagnostics": result.diagnostics,
        "deciding_policy": result.deciding_policy,
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
