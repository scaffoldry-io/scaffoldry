//! Workspace and Collaborator Management Endpoints (REST Adapter)
//! Enforces workspace sharing boundaries, least privilege roles, and OSCAL AC-02/AC-03 controls
//! via the sovereign service layer.

use crate::service::workspaces as workspace_service;
pub use crate::service::workspaces::{
    CollaboratorPayload, CreateWorkspacePayload, UpdateWorkspacePayload, WorkspaceResponse,
};
use crate::service::ServiceError;
use crate::state::{AuthUser, CollaboratorRecord, SharedState, WorkspaceRecord};
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route("/workspaces/{id}", get(get_workspace).put(update_workspace))
        .route(
            "/workspaces/{id}/collaborators",
            get(list_collaborators).post(add_collaborator),
        )
        .route(
            "/workspaces/{id}/collaborators/{eppn}",
            axum::routing::put(update_collaborator).delete(remove_collaborator),
        )
        .route(
            "/workspaces/{id}/guards",
            get(get_workspace_guards).put(put_workspace_guards),
        )
        .route(
            "/workspaces/{id}/guards/impact",
            axum::routing::post(impact_workspace_guards),
        )
        .route(
            "/workspaces/{id}/guards/test",
            axum::routing::post(test_workspace_guards),
        )
}

#[derive(Debug, Deserialize)]
pub struct UpdateRolePayload {
    pub role: String,
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

async fn list_workspaces(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let list = workspace_service::list_workspaces(&caller, &state)?;
    Ok(Json(list))
}

async fn create_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<WorkspaceRecord>), ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;

    let name = payload["name"]
        .as_str()
        .ok_or_else(|| ServiceError::BadRequest("name is required".to_string()))?
        .to_string();

    let code = payload["code"]
        .as_str()
        .ok_or_else(|| ServiceError::BadRequest("code is required".to_string()))?
        .to_string();

    let input = CreateWorkspacePayload {
        name,
        code,
        organization: payload.get("organization").and_then(|v| v.as_str()).map(str::to_string),
        department: payload.get("department").and_then(|v| v.as_str()).map(str::to_string),
        description: payload.get("description").and_then(|v| v.as_str()).map(str::to_string),
        icon: payload.get("icon").and_then(|v| v.as_str()).map(str::to_string),
        visibility: payload.get("visibility").and_then(|v| v.as_str()).map(str::to_string),
        allowed_affiliations: payload
            .get("allowed_affiliations")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()),
        data_classification: payload.get("data_classification").and_then(|v| v.as_str()).map(str::to_string),
        organization_id: payload.get("organization_id").and_then(|v| v.as_str()).and_then(|s| uuid::Uuid::parse_str(s).ok()),
    };

    let ws = workspace_service::create_workspace(&caller, input, &state)?;
    Ok((StatusCode::CREATED, Json(ws)))
}

async fn get_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceResponse>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = workspace_service::get_workspace(&caller, &id, &state)?;
    Ok(Json(ws))
}

async fn update_workspace(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<UpdateWorkspacePayload>,
) -> Result<Json<WorkspaceRecord>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = workspace_service::update_workspace(&caller, &id, payload, &state)?;
    Ok(Json(ws))
}

async fn list_collaborators(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<CollaboratorRecord>>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let collabs = workspace_service::list_collaborators(&caller, &id, &state)?;
    Ok(Json(collabs))
}

async fn add_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<CollaboratorPayload>,
) -> Result<(StatusCode, Json<CollaboratorRecord>), ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let member = workspace_service::add_collaborator(&caller, &id, payload, &state)?;
    Ok((StatusCode::CREATED, Json(member)))
}

async fn update_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
    Json(payload): Json<UpdateRolePayload>,
) -> Result<Json<CollaboratorRecord>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let member = workspace_service::update_collaborator_role(&caller, &id, &eppn, &payload.role, &state)?;
    Ok(Json(member))
}

async fn remove_collaborator(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path((id, eppn)): Path<(String, String)>,
) -> Result<StatusCode, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    workspace_service::remove_collaborator(&caller, &id, &eppn, &state)?;
    Ok(StatusCode::NO_CONTENT)
}


// ---------------------------------------------------------------------------------------------
// Workspace Guards Endpoints & Helpers
// ---------------------------------------------------------------------------------------------

use sha2::{Digest, Sha256};
use scaffoldry_core::guards::{compile_rules, GuardRule, WorkspaceGuardRecord, MAX_GUARD_RULES};
use scaffoldry_policy::entities::{PrincipalCtx, RecordCtx, Resource, WorkspaceCtx};

fn check_can_manage_guards(caller: &AuthUser, ws: &WorkspaceRecord, state: &SharedState) -> Result<(), ServiceError> {
    if caller.affiliation == "central_admin"  {
        return Ok(());
    }

    let collabs = {
        let collabs_guard = state.collaborators.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(&ws.id).cloned().unwrap_or_default()
    };
    if let Some(collab) = collabs.iter().find(|c| c.eppn.eq_ignore_ascii_case(&caller.eppn)) {
        let r = collab.role.to_lowercase();
        if r == "owner" || r == "admin" {
            return Ok(());
        }
    }

    if let Some(org_id) = ws.organization_id {
        let orgs: Vec<crate::state::OrganizationNode> = state.organizations.read().map(|g| g.values().cloned().collect()).unwrap_or_default();
        let roles: Vec<crate::state::RoleRow> = state.roles.read().map(|g| g.clone()).unwrap_or_default();
        let org_caller = crate::service::organizations::OrgCaller {
            eppn: caller.eppn.clone(),
            affiliation: caller.affiliation.clone(),
        };
        if crate::service::organizations::unit_in_scope(&org_caller, org_id, &orgs, &roles) {
            return Ok(());
        }
    }

    Err(ServiceError::Forbidden {
        message: "403 Forbidden: Only workspace owner, admin, org unit admin, or platform admin can manage workspace guards".to_string(),
        reasons: vec![],
        diagnostics: vec![],
        policy: None,
    })
}

fn principal_ctx_for(user: &AuthUser, state: &SharedState) -> PrincipalCtx {
    let role_unit_ids: Vec<String> = {
        let r_read = state.roles.read().ok();
        let mut ids = Vec::new();
        if let Some(roles) = r_read {
            for r in roles.iter() {
                if r.eppn.eq_ignore_ascii_case(&user.eppn) {
                    ids.push(r.organization_id.to_string());
                }
            }
        }
        ids
    };

    let org_parents: std::collections::HashMap<String, Option<String>> = {
        let o_read = state.organizations.read().ok();
        let mut map = std::collections::HashMap::new();
        if let Some(orgs) = o_read {
            for (id, node) in orgs.iter() {
                map.insert(id.to_string(), node.parent_id.map(|p| p.to_string()));
            }
        }
        map
    };

    let all_unit_ids = scaffoldry_policy::entities::unit_ids_for(&role_unit_ids, &org_parents);
    let is_platform_admin = user.affiliation == "central_admin" ;

    PrincipalCtx {
        eppn: user.eppn.clone(),
        name: user.name.clone(),
        scoped_affiliation: user.affiliation.clone(),
        department: user.department.clone(),
        unit_ids: all_unit_ids,
        is_platform_admin,
    }
}

fn build_proposed_policy_set(
    workspace_id: &str,
    rules: Option<&[GuardRule]>,
    source: Option<&str>,
    state: &SharedState,
) -> Result<scaffoldry_policy::PolicySet, ServiceError> {
    let mut ps = state.policy_engine.policies().clone();
    let org_names = state.organizations.read().map(|orgs| {
        orgs.iter().map(|(id, node)| (id.to_string(), node.name.clone())).collect()
    }).unwrap_or_default();

    if let Some(rule_list) = rules {
        for (i, rule) in rule_list.iter().enumerate() {
            let text = rule.compile_policy_text(workspace_id, i + 1, &org_names)
                .map_err(ServiceError::bad_request)?;
            let policy_id = format!("guard-{workspace_id}-{}", i + 1);
            let policy = scaffoldry_policy::parse_guard_policy(&policy_id, &text)
                .map_err(|e| ServiceError::bad_request(e.to_string()))?;
            ps.add(policy).map_err(|e| ServiceError::bad_request(e.to_string()))?;
        }
    } else if let Some(src) = source {
        let raw_set = scaffoldry_policy::validate_raw_source(src)
            .map_err(|e| ServiceError::bad_request(e.to_string()))?;
        for p in raw_set.policies() {
            ps.add(p.clone()).map_err(|e| ServiceError::bad_request(e.to_string()))?;
        }
    }
    Ok(ps)
}

async fn get_workspace_guards(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    // Check read access on workspace
    let _ = workspace_service::get_workspace(&caller, &id, &state)?;

    let org_names = state.organizations.read().map(|orgs| {
        orgs.iter().map(|(id, node)| (id.to_string(), node.name.clone())).collect()
    }).unwrap_or_default();

    let history: Vec<WorkspaceGuardRecord> = {
        let g_guard = state.guards.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        g_guard.get(&id).cloned().unwrap_or_default()
    };

    let current = history.last().map(|rec| {
        let sentences: Vec<String> = if let Ok(rules) = serde_json::from_value::<Vec<GuardRule>>(rec.rules.clone()) {
            rules.iter().map(|r| r.sentence(&org_names)).collect()
        } else {
            Vec::new()
        };
        serde_json::json!({
            "workspace_id": rec.workspace_id,
            "version": rec.version,
            "rules": rec.rules,
            "compiled": rec.compiled,
            "sentences": sentences,
            "reason": rec.reason,
            "created_by": rec.created_by,
            "created_at": rec.created_at,
        })
    });

    Ok(Json(serde_json::json!({
        "current": current,
        "history": history,
    })))
}

async fn put_workspace_guards(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = {
        let ws_guard = state.workspaces.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.get(&id).cloned().ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?
    };

    check_can_manage_guards(&caller, &ws, &state)?;

    let reason = payload.get("reason")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ServiceError::bad_request("reason is required and cannot be empty"))?
        .to_string();

    let org_names = state.organizations.read().map(|orgs| {
        orgs.iter().map(|(id, node)| (id.to_string(), node.name.clone())).collect()
    }).unwrap_or_default();

    let (compiled_text, rules_val, sentences) = if let Some(source) = payload.get("source").and_then(|v| v.as_str()) {
        if caller.affiliation != "central_admin" && false {
            return Err(ServiceError::forbidden("Raw Cedar source requires platform admin"));
        }
        let _ = scaffoldry_policy::validate_raw_source(source)
            .map_err(|e| ServiceError::bad_request(e.to_string()))?;
        (source.to_string(), Value::Null, Vec::new())
    } else if let Some(rules_arr) = payload.get("rules") {
        let rules: Vec<GuardRule> = serde_json::from_value(rules_arr.clone())
            .map_err(|e| ServiceError::bad_request(format!("Invalid rules format: {e}")))?;

        if rules.len() > MAX_GUARD_RULES {
            return Err(ServiceError::bad_request(format!("Rules count {} exceeds maximum allowed of {}", rules.len(), MAX_GUARD_RULES)));
        }

        // Validate unit_ids exist in organizations:
        {
            let orgs = state.organizations.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
            for rule in &rules {
                for uid in &rule.unit_ids {
                    let parsed = uuid::Uuid::parse_str(uid);
                    if parsed.is_err() || !orgs.contains_key(&parsed.unwrap()) {
                        return Err(ServiceError::bad_request(format!("Unit id '{uid}' does not exist")));
                    }
                }
            }
        }

        let (text, sentences) = compile_rules(&id, &rules, &org_names)
            .map_err(ServiceError::bad_request)?;

        // Verify that every compiled policy parses and is forbid:
        for (i, rule) in rules.iter().enumerate() {
            let p_text = rule.compile_policy_text(&id, i + 1, &org_names).map_err(ServiceError::bad_request)?;
            let p_id = format!("guard-{id}-{}", i + 1);
            scaffoldry_policy::parse_guard_policy(&p_id, &p_text).map_err(|e| ServiceError::bad_request(e.to_string()))?;
        }

        (text, serde_json::to_value(&rules).unwrap_or(Value::Null), sentences)
    } else {
        return Err(ServiceError::bad_request("Either rules or source is required"));
    };

    let current_version = {
        let g_guard = state.guards.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        g_guard.get(&id).and_then(|h| h.last().map(|r| r.version)).unwrap_or(0)
    };
    let new_version = current_version + 1;

    let mut hasher = Sha256::new();
    hasher.update(compiled_text.as_bytes());
    let compiled_sha256 = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>();

    let ledger_payload = serde_json::json!({
        "workspace_id": id,
        "version": new_version,
        "sha256": compiled_sha256,
    });

    state.append_ledger_entry(crate::state::RecordDecisionInput {
        principal: caller.eppn.clone(),
        organization_code: "INST".to_string(),
        app_slug: None,
        decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
        oscal_control_id: "AC-03".to_string(),
        rationale: reason.clone(),
        payload: &ledger_payload,
    }).map_err(|e| ServiceError::internal(format!("Ledger append failed: {e}")))?;

    let record = WorkspaceGuardRecord {
        workspace_id: id.clone(),
        version: new_version,
        rules: rules_val.clone(),
        compiled: compiled_text.clone(),
        reason: reason.clone(),
        created_by: caller.eppn.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    if let Some(ref repo) = state.repository {
        repo.insert_workspace_guard(&record).map_err(|e| ServiceError::internal(e.to_string()))?;
    }

    {
        let mut g_write = state.guards.write().map_err(|e| ServiceError::Internal(e.to_string()))?;
        g_write.entry(id.clone()).or_default().push(record.clone());
    }

    {
        let mut c_write = state.guard_policy_cache.write().map_err(|e| ServiceError::Internal(e.to_string()))?;
        c_write.remove(&(id.clone(), new_version));
    }

    Ok(Json(serde_json::json!({
        "current": {
            "workspace_id": id,
            "version": new_version,
            "rules": rules_val,
            "compiled": compiled_text,
            "sentences": sentences,
            "reason": reason,
            "created_by": caller.eppn,
            "created_at": record.created_at,
        }
    })))
}

async fn impact_workspace_guards(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = {
        let ws_guard = state.workspaces.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.get(&id).cloned().ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?
    };

    check_can_manage_guards(&caller, &ws, &state)?;

    let rules: Option<Vec<GuardRule>> = payload.get("rules").and_then(|v| serde_json::from_value(v.clone()).ok());
    let source: Option<&str> = payload.get("source").and_then(|v| v.as_str());

    let proposed_policy_set = build_proposed_policy_set(&id, rules.as_deref(), source, &state)?;

    let collabs = {
        let collabs_guard = state.collaborators.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };

    let actions = ["access_workspace", "read_app", "write_record", "export", "approve"];
    let sensitivities = [false, true];

    let mut changes = Vec::new();

    for collab in &collabs {
        let collab_user = crate::service::identity::resolve_user(&collab.eppn, &state).unwrap_or_else(|| AuthUser {
            eppn: collab.eppn.clone(),
            name: collab.name.clone(),
            role_title: collab.role.clone(),
            affiliation: collab.scoped_affiliation.split('@').next().unwrap_or("member").to_string(),
            department: collab.department.clone(),
        });
        let princ = principal_ctx_for(&collab_user, &state);

        for action in actions {
            for &sensitive in &sensitivities {
                let resource = match action {
                    "access_workspace" => Resource::Workspace(WorkspaceCtx {
                        workspace_id: id.clone(),
                        department: ws.department.clone(),
                        visibility: ws.visibility.clone(),
                        data_classification: ws.data_classification.clone(),
                        member_role: collab.role.to_lowercase(),
                        unit_id: ws.organization_id.map(|u| u.to_string()).unwrap_or_default(),
                        is_member: true,
                    }),
                    "read_app" => Resource::Record(RecordCtx {
                        app_slug: "app".to_string(),
                        department: ws.department.clone(),
                        workspace_id: id.clone(),
                        is_ferpa_sensitive: sensitive,
                        categories: Default::default(),
                    }),
                    _ => Resource::Record(RecordCtx {
                        app_slug: "app".to_string(),
                        department: ws.department.clone(),
                        workspace_id: id.clone(),
                        is_ferpa_sensitive: sensitive,
                        categories: Default::default(),
                    }),
                };

                let dec_now = crate::service::access::decide(&state, &collab_user, action, &resource);
                let dec_prop = scaffoldry_policy::entities::authorize_with_policy_set(
                    &state.policy_engine,
                    &proposed_policy_set,
                    &princ,
                    action,
                    &resource,
                ).map(|r| r.decision == scaffoldry_policy::PolicyDecision::Allow).unwrap_or(false);

                if dec_now.allowed != dec_prop {
                    changes.push(serde_json::json!({
                        "eppn": collab.eppn,
                        "action": action,
                        "is_ferpa_sensitive": sensitive,
                        "current_allowed": dec_now.allowed,
                        "proposed_allowed": dec_prop,
                    }));
                }
            }
        }
    }

    Ok(Json(serde_json::json!({ "changes": changes })))
}

async fn test_workspace_guards(
    State(state): State<SharedState>,
    user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ServiceError> {
    let caller = resolve_caller(user, &headers, &state)?;
    let ws = {
        let ws_guard = state.workspaces.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        ws_guard.get(&id).cloned().ok_or_else(|| ServiceError::NotFound(format!("Workspace '{id}' not found")))?
    };

    check_can_manage_guards(&caller, &ws, &state)?;

    let target_eppn = payload.get("user")
        .or_else(|| payload.get("user_eppn"))
        .or_else(|| payload.get("eppn"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| ServiceError::bad_request("user is required"))?;

    let action = payload.get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ServiceError::bad_request("action is required"))?;

    let is_sensitive = payload.get("is_ferpa_sensitive")
        .or_else(|| payload.get("sensitive"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let rules: Option<Vec<GuardRule>> = payload.get("rules").and_then(|v| serde_json::from_value(v.clone()).ok());
    let source: Option<&str> = payload.get("source").and_then(|v| v.as_str());

    let proposed_policy_set = build_proposed_policy_set(&id, rules.as_deref(), source, &state)?;

    let target_user = crate::service::identity::resolve_user(target_eppn, &state).unwrap_or_else(|| AuthUser {
        eppn: target_eppn.to_string(),
        name: target_eppn.to_string(),
        role_title: "Member".to_string(),
        affiliation: "member".to_string(),
        department: ws.department.clone(),
    });
    let princ = principal_ctx_for(&target_user, &state);

    let collabs = {
        let collabs_guard = state.collaborators.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };
    let (is_member, member_role) = match collabs.iter().find(|c| c.eppn.eq_ignore_ascii_case(target_eppn)) {
        Some(c) => (true, c.role.to_lowercase()),
        None => (false, String::new()),
    };

    let resource = match action {
        "access_workspace" => Resource::Workspace(WorkspaceCtx {
            workspace_id: id.clone(),
            department: ws.department.clone(),
            visibility: ws.visibility.clone(),
            data_classification: ws.data_classification.clone(),
            member_role,
            unit_id: ws.organization_id.map(|u| u.to_string()).unwrap_or_default(),
            is_member,
        }),
        _ => Resource::Record(RecordCtx {
            app_slug: "app".to_string(),
            department: ws.department.clone(),
            workspace_id: id.clone(),
            is_ferpa_sensitive: is_sensitive,
            categories: Default::default(),
        }),
    };

    let dec_now = crate::service::access::decide(&state, &target_user, action, &resource);
    let dec_prop = scaffoldry_policy::entities::authorize_with_policy_set(
        &state.policy_engine,
        &proposed_policy_set,
        &princ,
        action,
        &resource,
    );

    let (prop_allowed, prop_pol) = match dec_prop {
        Ok(res) => (res.decision == scaffoldry_policy::PolicyDecision::Allow, res.deciding_policy),
        Err(_) => (false, None),
    };

    Ok(Json(serde_json::json!({
        "current": {
            "allowed": dec_now.allowed,
            "policy": dec_now.policy,
        },
        "proposed": {
            "allowed": prop_allowed,
            "policy": prop_pol,
        }
    })))
}
