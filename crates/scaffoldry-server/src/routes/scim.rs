//! SCIM 2.0 Identity Provisioning Endpoints (RFC 7643 / RFC 7644)

use crate::state::{ScimGroup, ScimUser, SharedState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use uuid::Uuid;

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
        "patch": { "supported": true },
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

async fn create_user(
    State(state): State<SharedState>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user_name = payload["userName"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "userName is required"}))))?
        .to_string();

    let id = Uuid::new_v4().to_string();
    let name = payload.get("name").cloned().unwrap_or(json!({}));
    let active = payload.get("active").and_then(|v| v.as_bool()).unwrap_or(true);
    let emails = payload.get("emails").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let roles = payload.get("roles").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let enterprise = payload.get("urn:ietf:params:scim:schemas:extension:enterprise:2.0:User").cloned();

    let scim_user = ScimUser {
        id: id.clone(),
        user_name: user_name.clone(),
        name: name.clone(),
        active,
        emails: emails.clone(),
        roles: roles.clone(),
        enterprise_extension: enterprise.clone(),
    };

    state.users.write().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), scim_user);

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

async fn delete_user(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut users = state.users.write().unwrap_or_else(|p| p.into_inner());
    if users.remove(&id).is_some() {
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
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "displayName is required"}))))?
        .to_string();

    let id = Uuid::new_v4().to_string();
    let members = payload.get("members").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let group = ScimGroup {
        id: id.clone(),
        display_name: display_name.clone(),
        members: members.clone(),
    };

    state.groups.write().unwrap_or_else(|p| p.into_inner()).insert(id.clone(), group);

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
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
