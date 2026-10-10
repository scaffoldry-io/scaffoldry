//! SCIM 2.0 Identity Provisioning Endpoints (RFC 7643 / RFC 7644)

use crate::state::{OrganizationNode, RoleRow, ScimGroup, ScimUser, SharedState};
use scaffoldry_core::standards::eduperson::EduPersonAffiliation;
use std::str::FromStr;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/ServiceProviderConfig", get(service_provider_config))
        .route("/ResourceTypes", get(resource_types))
        .route("/Schemas", get(schemas))
        .route("/Users", get(list_users).post(create_user))
        .route("/Users/{id}", get(get_user).put(update_user).delete(delete_user))
        .route("/Groups", get(list_groups).post(create_group))
        .route("/Groups/{id}", get(get_group).put(update_group).delete(delete_group))
}

async fn service_provider_config() -> impl IntoResponse {
    let config = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "documentationUri": "https://scaffoldry.io/docs/scim",
        "patch": { "supported": false },
        "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
        "filter": { "supported": true, "maxResults": 200 },
        "changePassword": { "supported": false },
        "sort": { "supported": false },
        "etag": { "supported": false },
        "authenticationSchemes": [
            {
                "name": "OAuth Bearer Token",
                "description": "Authentication scheme using Bearer tokens for institutional identity federations",
                "specUri": "https://www.rfc-editor.org/rfc/rfc6750.html",
                "type": "oauthbearertoken",
                "primary": true
            }
        ]
    });
    Json(config)
}

async fn resource_types() -> impl IntoResponse {
    let resources = json!([
        {
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
            "id": "User",
            "name": "User",
            "endpoint": "/Users",
            "description": "User Account with Higher Education Enterprise Extensions",
            "schema": "urn:ietf:params:scim:schemas:core:2.0:User",
            "schemaExtensions": [
                {
                    "schema": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User",
                    "required": false
                }
            ]
        },
        {
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
            "id": "Group",
            "name": "Group",
            "endpoint": "/Groups",
            "description": "Group Resource representing Departmental Teams and Affiliations",
            "schema": "urn:ietf:params:scim:schemas:core:2.0:Group"
        }
    ]);
    Json(resources)
}

async fn schemas() -> impl IntoResponse {
    let schemas = json!([
        {
            "id": "urn:ietf:params:scim:schemas:core:2.0:User",
            "name": "User",
            "description": "Core User Schema"
        },
        {
            "id": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User",
            "name": "EnterpriseUser",
            "description": "Enterprise User Schema with Department and eduPerson Attributes"
        },
        {
            "id": "urn:ietf:params:scim:schemas:core:2.0:Group",
            "name": "Group",
            "description": "Core Group Schema"
        }
    ]);
    Json(schemas)
}

async fn list_users(State(state): State<SharedState>) -> impl IntoResponse {
    let users = state.users.read().unwrap_or_else(|p| p.into_inner());
    let resources: Vec<Value> = users
        .values()
        .map(|u| {
            json!({
                "schemas": [
                    "urn:ietf:params:scim:schemas:core:2.0:User",
                    "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
                ],
                "id": u.id,
                "userName": u.user_name,
                "name": u.name,
                "active": u.active,
                "emails": u.emails,
                "roles": u.roles,
                "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": u.enterprise_extension
            })
        })
        .collect();

    let total = resources.len();
    Json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"],
        "totalResults": total,
        "startIndex": 1,
        "itemsPerPage": total,
        "Resources": resources
    }))
}

pub fn sync_org_roles(
    user: &ScimUser,
    orgs: &[OrganizationNode],
) -> Vec<RoleRow> {
    if !user.active {
        return Vec::new();
    }

    // 1. Scoped affiliation from roles[]
    let mut scoped_affiliation = "member".to_string();
    for r in &user.roles {
        if let Some(typ) = r.get("type").and_then(|v| v.as_str()) {
            if typ.eq_ignore_ascii_case("eduPersonScopedAffiliation") {
                if let Some(val) = r.get("value").and_then(|v| v.as_str()) {
                    let aff_str = val.split('@').next().unwrap_or(val);
                    if let Ok(aff) = EduPersonAffiliation::from_str(aff_str) {
                        scoped_affiliation = match aff {
                            EduPersonAffiliation::Faculty => "faculty",
                            EduPersonAffiliation::Student => "student",
                            EduPersonAffiliation::Staff => "staff",
                            EduPersonAffiliation::Employee => "employee",
                            EduPersonAffiliation::Member => "member",
                            EduPersonAffiliation::Affiliate => "affiliate",
                            EduPersonAffiliation::Alum => "alum",
                        }.to_string();
                        break;
                    }
                }
            }
        }
    }

    let role_title = user.title.clone().unwrap_or_default();

    // 2. Enterprise Extension matching: department, division, organization
    let mut dept_match = None;
    let mut div_match = None;
    let mut org_match = None;

    if let Some(ref ent) = user.enterprise_extension {
        let match_node = |val: &str| -> Option<Uuid> {
            if let Some(node) = orgs.iter().find(|o| o.code.eq_ignore_ascii_case(val)) {
                return Some(node.id);
            }
            if let Some(node) = orgs.iter().find(|o| o.name.eq_ignore_ascii_case(val)) {
                return Some(node.id);
            }
            None
        };

        if let Some(dept_str) = ent.get("department").and_then(|v| v.as_str()) {
            dept_match = match_node(dept_str);
        }
        if let Some(div_str) = ent.get("division").and_then(|v| v.as_str()) {
            div_match = match_node(div_str);
        }
        if let Some(org_str) = ent.get("organization").and_then(|v| v.as_str()) {
            org_match = match_node(org_str);
        }
    }

    let deepest_id = dept_match.or(div_match).or(org_match);

    let mut matched_units = Vec::new();
    if let Some(id) = dept_match {
        matched_units.push(id);
    }
    if let Some(id) = div_match {
        if !matched_units.contains(&id) {
            matched_units.push(id);
        }
    }
    if let Some(id) = org_match {
        if !matched_units.contains(&id) {
            matched_units.push(id);
        }
    }

    let mut new_roles = Vec::new();
    let person_id = Uuid::new_v4();

    for unit_id in matched_units {
        let is_primary = Some(unit_id) == deepest_id;
        new_roles.push(RoleRow {
            id: Uuid::new_v4(),
            person_id,
            eppn: user.user_name.clone(),
            organization_id: unit_id,
            role_title: role_title.clone(),
            scoped_affiliation: scoped_affiliation.clone(),
            is_primary,
            source: "scim".to_string(),
        });
    }

    // 3. Scaffoldry roles
    for r in &user.roles {
        let typ = r.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let val = r.get("value").and_then(|v| v.as_str()).unwrap_or("");
        if typ.eq_ignore_ascii_case("scaffoldry") {
            if val.eq_ignore_ascii_case("unit_admin") {
                if let Some(deepest) = deepest_id {
                    new_roles.push(RoleRow {
                        id: Uuid::new_v4(),
                        person_id,
                        eppn: user.user_name.clone(),
                        organization_id: deepest,
                        role_title: role_title.clone(),
                        scoped_affiliation: "unit_admin".to_string(),
                        is_primary: false,
                        source: "scim".to_string(),
                    });
                }
            } else if val.eq_ignore_ascii_case("platform_admin") {
                if let Some(root_node) = orgs.iter().find(|o| o.parent_id.is_none()) {
                    new_roles.push(RoleRow {
                        id: Uuid::new_v4(),
                        person_id,
                        eppn: user.user_name.clone(),
                        organization_id: root_node.id,
                        role_title: role_title.clone(),
                        scoped_affiliation: "platform_admin".to_string(),
                        is_primary: false,
                        source: "scim".to_string(),
                    });
                }
            }
        }
    }

    new_roles
}

async fn create_user(
    State(state): State<SharedState>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user_name = payload["userName"]
        .as_str()
        .ok_or_else(|| ServiceError::bad_request("userName is required").into_pair())?
        .to_string();

    let id = Uuid::new_v4().to_string();
    let name = payload.get("name").cloned().unwrap_or(json!({}));
    let active = payload.get("active").and_then(|v| v.as_bool()).unwrap_or(true);
    let emails = payload.get("emails").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let roles = payload.get("roles").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let enterprise = payload.get("urn:ietf:params:scim:schemas:extension:enterprise:2.0:User").cloned();
    let title = payload.get("title").and_then(|v| v.as_str()).map(str::to_string);

    let scim_user = ScimUser {
        id: id.clone(),
        user_name: user_name.clone(),
        name: name.clone(),
        active,
        emails: emails.clone(),
        roles: roles.clone(),
        enterprise_extension: enterprise.clone(),
        title,
    };

    state.users.write().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), scim_user.clone());
    if let Some(ref repo) = state.repository {
        repo.upsert_scim_user(&scim_user)
            .map_err(|_| ServiceError::internal("Failed to persist SCIM user").into_pair())?;
    }

    let all_orgs: Vec<OrganizationNode> = state.organizations.read().unwrap().values().cloned().collect();
    let new_roles = sync_org_roles(&scim_user, &all_orgs);
    let _ = state.delete_scim_roles(&scim_user.user_name);
    for r in new_roles {
        let _ = state.persist_role(&r);
    }

    let resp = json!({
        "schemas": [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
        ],
        "id": id,
        "userName": user_name,
        "name": name,
        "active": active,
        "emails": emails,
        "roles": roles,
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": enterprise
    });

    Ok((StatusCode::CREATED, Json(resp)))
}

async fn get_user(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let users = state.users.read().unwrap_or_else(|p| p.into_inner());
    let u = users.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "schemas": [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
        ],
        "id": u.id,
        "userName": u.user_name,
        "name": u.name,
        "active": u.active,
        "emails": u.emails,
        "roles": u.roles,
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": u.enterprise_extension
    })))
}

async fn update_user(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let mut users = state.users.write().unwrap_or_else(|p| p.into_inner());
    let u = users.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;

    if let Some(un) = payload.get("userName").and_then(|v| v.as_str()) {
        u.user_name = un.to_string();
    }
    if let Some(n) = payload.get("name") {
        u.name = n.clone();
    }
    if let Some(a) = payload.get("active").and_then(|v| v.as_bool()) {
        u.active = a;
    }
    if let Some(e) = payload.get("emails").and_then(|v| v.as_array()) {
        u.emails = e.clone();
    }
    if let Some(r) = payload.get("roles").and_then(|v| v.as_array()) {
        u.roles = r.clone();
    }
    if let Some(ent) = payload.get("urn:ietf:params:scim:schemas:extension:enterprise:2.0:User") {
        u.enterprise_extension = Some(ent.clone());
    }
    if let Some(t) = payload.get("title").and_then(|v| v.as_str()) {
        u.title = Some(t.to_string());
    }

    if let Some(ref repo) = state.repository {
        repo.upsert_scim_user(u)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    let updated_user = u.clone();
    drop(users);

    let all_orgs: Vec<OrganizationNode> = state.organizations.read().unwrap().values().cloned().collect();
    let new_roles = sync_org_roles(&updated_user, &all_orgs);
    let _ = state.delete_scim_roles(&updated_user.user_name);
    for r in new_roles {
        let _ = state.persist_role(&r);
    }

    Ok(Json(json!({
        "schemas": [
            "urn:ietf:params:scim:schemas:core:2.0:User",
            "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
        ],
        "id": updated_user.id,
        "userName": updated_user.user_name,
        "name": updated_user.name,
        "active": updated_user.active,
        "emails": updated_user.emails,
        "roles": updated_user.roles,
        "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User": updated_user.enterprise_extension
    })))
}

async fn delete_user(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut users = state.users.write().unwrap_or_else(|p| p.into_inner());
    if let Some(u) = users.remove(&id) {
        if let Some(ref repo) = state.repository {
            repo.delete_scim_user(&id)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        drop(users);
        let _ = state.delete_scim_roles(&u.user_name);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn list_groups(State(state): State<SharedState>) -> impl IntoResponse {
    let groups = state.groups.read().unwrap_or_else(|p| p.into_inner());
    let resources: Vec<Value> = groups
        .values()
        .map(|g| {
            json!({
                "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
                "id": g.id,
                "displayName": g.display_name,
                "members": g.members
            })
        })
        .collect();

    let total = resources.len();
    Json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"],
        "totalResults": total,
        "startIndex": 1,
        "itemsPerPage": total,
        "Resources": resources
    }))
}

async fn create_group(
    State(state): State<SharedState>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let display_name = payload["displayName"]
        .as_str()
        .ok_or_else(|| ServiceError::bad_request("displayName is required").into_pair())?
        .to_string();

    let id = Uuid::new_v4().to_string();
    let members = payload.get("members").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let group = ScimGroup {
        id: id.clone(),
        display_name: display_name.clone(),
        members: members.clone(),
    };

    state.groups.write().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), group.clone());
    if let Some(ref repo) = state.repository {
        repo.upsert_scim_group(&group)
            .map_err(|_| ServiceError::internal("Failed to persist SCIM group").into_pair())?;
    }

    let resp = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "id": id,
        "displayName": display_name,
        "members": members
    });

    Ok((StatusCode::CREATED, Json(resp)))
}

async fn get_group(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let groups = state.groups.read().unwrap_or_else(|p| p.into_inner());
    let g = groups.get(&id).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "id": g.id,
        "displayName": g.display_name,
        "members": g.members
    })))
}

async fn update_group(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let mut groups = state.groups.write().unwrap_or_else(|p| p.into_inner());
    let g = groups.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;

    if let Some(dn) = payload.get("displayName").and_then(|v| v.as_str()) {
        g.display_name = dn.to_string();
    }
    if let Some(m) = payload.get("members").and_then(|v| v.as_array()) {
        g.members = m.clone();
    }

    if let Some(ref repo) = state.repository {
        repo.upsert_scim_group(g)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "id": g.id,
        "displayName": g.display_name,
        "members": g.members
    })))
}

async fn delete_group(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut groups = state.groups.write().unwrap_or_else(|p| p.into_inner());
    if groups.remove(&id).is_some() {
        if let Some(ref repo) = state.repository {
            repo.delete_scim_group(&id)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
