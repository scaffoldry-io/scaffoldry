use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::organizations::{unit_in_scope, OrgCaller};
use crate::service::ServiceError;
use crate::state::{AuthUser, OrganizationNode, RoleRow, SharedState};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/orgs", get(list_orgs).post(create_org))
        .route("/orgs/{id}", patch(patch_org))
        .route("/orgs/{id}/appointments", post(create_appointment))
        .route("/orgs/{id}/members", get(list_members))
}

fn resolve_caller(
    user: Option<Extension<AuthUser>>,
    headers: &HeaderMap,
    state: &SharedState,
) -> Result<AuthUser, ServiceError> {
    if let Some(Extension(u)) = user {
        return Ok(u);
    }
    crate::guard::session_user(state, headers)
        .ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))
}

fn is_platform_admin(caller: &AuthUser, state: &SharedState) -> bool {
    if caller.affiliation == "central_admin" {
        return true;
    }
    let orgs = state.organizations.read().unwrap();
    let roles = state.roles.read().unwrap();
    roles.iter().any(|r| {
        r.eppn == caller.eppn
            && r.scoped_affiliation == "platform_admin"
            && orgs.iter().any(|(oid, o)| oid == &r.organization_id && o.parent_id.is_none())
    })
}

async fn list_orgs(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let org_caller = OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };

    let orgs_map = state.organizations.read().unwrap();
    let all_orgs: Vec<OrganizationNode> = orgs_map.values().cloned().collect();
    let roles: Vec<RoleRow> = state.roles.read().unwrap().clone();

    let mut in_scope_orgs = Vec::new();
    for org in &all_orgs {
        if unit_in_scope(&org_caller, org.id, &all_orgs, &roles) {
            in_scope_orgs.push(org.clone());
        }
    }

    Ok(Json(in_scope_orgs))
}

#[derive(Debug, Deserialize)]
pub struct CreateOrgPayload {
    pub name: String,
    pub code: String,
    pub org_type: String,
    pub parent_id: Option<Uuid>,
}

const ALLOWED_ORG_TYPES: [&str; 5] = [
    "Institution",
    "College",
    "Department",
    "Center",
    "Program",
];

async fn create_org(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrgPayload>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    if !is_platform_admin(&caller, &state) {
        return Err(ServiceError::forbidden(
            "Only Platform Admin may create organizations",
        ));
    }

    if !ALLOWED_ORG_TYPES.contains(&payload.org_type.as_str()) {
        return Err(ServiceError::BadRequest(format!(
            "Invalid org_type: '{}'. Allowed: {:?}",
            payload.org_type, ALLOWED_ORG_TYPES
        )));
    }

    let orgs_map = state.organizations.read().unwrap();

    // Check code uniqueness
    if orgs_map.values().any(|o| o.code.eq_ignore_ascii_case(&payload.code)) {
        return Err(ServiceError::BadRequest(format!(
            "Organization code '{}' already exists",
            payload.code
        )));
    }

    if payload.org_type == "Institution" || payload.parent_id.is_none() {
        if payload.org_type != "Institution" || payload.parent_id.is_some() {
            return Err(ServiceError::BadRequest(
                "Root node must have org_type 'Institution' and no parent_id".to_string(),
            ));
        }
        if orgs_map.values().any(|o| o.parent_id.is_none()) {
            return Err(ServiceError::BadRequest(
                "A second root institution is not allowed".to_string(),
            ));
        }
    } else if let Some(parent_id) = payload.parent_id {
        if !orgs_map.contains_key(&parent_id) {
            return Err(ServiceError::BadRequest(format!(
                "Parent organization '{parent_id}' does not exist"
            )));
        }
    }

    let node = OrganizationNode {
        id: Uuid::new_v4(),
        parent_id: payload.parent_id,
        name: payload.name,
        code: payload.code,
        org_type: payload.org_type,
    };

    drop(orgs_map);
    state
        .persist_organization(&node)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(node)))
}

#[derive(Debug, Deserialize)]
pub struct PatchOrgPayload {
    pub name: Option<String>,
    pub parent_id: Option<Option<Uuid>>,
}

async fn patch_org(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Json(payload): Json<PatchOrgPayload>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    if !is_platform_admin(&caller, &state) {
        return Err(ServiceError::forbidden(
            "Only Platform Admin may update organizations",
        ));
    }

    let orgs_map = state.organizations.read().unwrap().clone();
    let mut node = orgs_map
        .get(&id)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("Organization '{id}' not found")))?;

    if let Some(name) = payload.name {
        node.name = name;
    }

    if let Some(new_parent_opt) = payload.parent_id {
        if let Some(new_parent_id) = new_parent_opt {
            if new_parent_id == id {
                return Err(ServiceError::BadRequest(
                    "Cannot set parent to the node itself".to_string(),
                ));
            }
            if !orgs_map.contains_key(&new_parent_id) {
                return Err(ServiceError::BadRequest(format!(
                    "Parent organization '{new_parent_id}' does not exist"
                )));
            }

            // Descendant cycle check: walk up from new_parent_id to root
            let mut curr = new_parent_id;
            let mut steps = 0;
            while steps < 32 {
                steps += 1;
                if curr == id {
                    return Err(ServiceError::BadRequest(
                        "Cannot set parent to a descendant node (cycle detected)".to_string(),
                    ));
                }
                if let Some(p) = orgs_map.get(&curr).and_then(|o| o.parent_id) {
                    curr = p;
                } else {
                    break;
                }
            }

            node.parent_id = Some(new_parent_id);
        } else {
            // Unparenting to root is only allowed if it is Institution
            if node.org_type != "Institution" {
                return Err(ServiceError::BadRequest(
                    "Only Institution org_type may have a null parent".to_string(),
                ));
            }
            node.parent_id = None;
        }
    }

    state
        .persist_organization(&node)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok(Json(node))
}

#[derive(Debug, Deserialize)]
pub struct AppointmentPayload {
    pub eppn: String,
    pub scoped_affiliation: String,
}

async fn create_appointment(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Json(payload): Json<AppointmentPayload>,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let org_caller = OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };

    let all_orgs: Vec<OrganizationNode> = state.organizations.read().unwrap().values().cloned().collect();
    let roles = state.roles.read().unwrap().clone();

    if !all_orgs.iter().any(|o| o.id == id) {
        return Err(ServiceError::NotFound(format!("Organization '{id}' not found")));
    }

    if !is_platform_admin(&caller, &state) && !unit_in_scope(&org_caller, id, &all_orgs, &roles) {
        return Err(ServiceError::forbidden(
            "Caller is not in scope for this organization unit",
        ));
    }

    if payload.scoped_affiliation != "unit_admin" && payload.scoped_affiliation != "platform_admin" {
        return Err(ServiceError::BadRequest(
            "scoped_affiliation must be 'unit_admin'".to_string(),
        ));
    }

    let role = RoleRow {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        eppn: payload.eppn,
        organization_id: id,
        role_title: "Org Unit Admin".to_string(),
        scoped_affiliation: payload.scoped_affiliation,
        is_primary: true,
        source: "api".to_string(),
    };

    state
        .persist_role(&role)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(role)))
}

#[derive(Debug, Serialize)]
pub struct MemberResponse {
    pub eppn: String,
    pub role_title: String,
    pub scoped_affiliation: String,
    pub is_primary: bool,
    pub source: String,
}

async fn list_members(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let org_caller = OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };

    let all_orgs: Vec<OrganizationNode> = state.organizations.read().unwrap().values().cloned().collect();
    let roles = state.roles.read().unwrap().clone();

    if !all_orgs.iter().any(|o| o.id == id) {
        return Err(ServiceError::NotFound(format!("Organization '{id}' not found")));
    }

    if !is_platform_admin(&caller, &state) && !unit_in_scope(&org_caller, id, &all_orgs, &roles) {
        return Err(ServiceError::forbidden(
            "Caller is not in scope for this organization unit",
        ));
    }

    let members: Vec<MemberResponse> = roles
        .into_iter()
        .filter(|r| r.organization_id == id)
        .map(|r| MemberResponse {
            eppn: r.eppn,
            role_title: r.role_title,
            scoped_affiliation: r.scoped_affiliation,
            is_primary: r.is_primary,
            source: r.source,
        })
        .collect();

    Ok(Json(members))
}
