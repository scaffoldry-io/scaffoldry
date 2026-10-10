//! Admin console: Workspaces & Apps inventory, updates, and ownership transfers.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::guard::session_user;
use crate::service::admin::{
    admin_write, decode_cursor, encode_cursor, require_platform_admin,
};
use crate::service::identity::resolve_user;
use crate::service::workspaces::{self as workspace_service, UpdateWorkspacePayload};
use crate::service::ServiceError;
use crate::state::{AuthUser, CollaboratorRecord, DatasetRecord, SharedState, WorkspaceRecord};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::AppManifest;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/admin/workspaces", get(list_workspaces))
        .route(
            "/admin/workspaces/{id}",
            get(get_workspace).patch(patch_workspace),
        )
        .route(
            "/admin/workspaces/{id}/transfer-ownership",
            post(transfer_workspace_ownership),
        )
        .route("/admin/apps", get(list_apps))
}

fn admin(state: &SharedState, headers: &HeaderMap) -> Result<AuthUser, ServiceError> {
    let caller = session_user(state, headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized"))?;
    require_platform_admin(&caller, state)?;
    Ok(caller)
}

#[derive(Debug, Deserialize)]
pub struct AdminWorkspacesQuery {
    pub search: Option<String>,
    pub unit: Option<String>,
    pub classification: Option<String>,
    pub visibility: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct AdminWorkspaceRow {
    pub id: String,
    pub name: String,
    pub code: String,
    pub unit_name: String,
    pub classification: String,
    pub visibility: String,
    pub owners: Vec<String>,
    pub collaborator_count: usize,
    pub app_count: usize,
    pub record_count: usize,
    pub created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminWorkspaceDetail {
    #[serde(flatten)]
    pub row: AdminWorkspaceRow,
    pub collaborators: Vec<CollaboratorRecord>,
    pub apps: Vec<AdminAppRow>,
}

#[derive(Debug, Deserialize)]
pub struct TransferOwnershipPayload {
    pub eppn: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct AdminAppsQuery {
    pub search: Option<String>,
    pub workspace: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct AdminAppRow {
    pub slug: String,
    pub title: String,
    pub workspace: String,
    pub version: String,
    pub table_count: usize,
    pub page_count: usize,
    pub custom_page_count: usize,
    pub record_count_per_table: HashMap<String, usize>,
    pub updated: String,
}

fn get_workspace_unit_name(state: &SharedState, ws: &WorkspaceRecord) -> String {
    if let Some(org_id) = ws.organization_id {
        if let Ok(orgs) = state.organizations.read() {
            if let Some(org) = orgs.get(&org_id) {
                return org.name.clone();
            }
        }
    }
    if !ws.organization.is_empty() {
        ws.organization.clone()
    } else {
        ws.department.clone()
    }
}

async fn list_workspaces(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<AdminWorkspacesQuery>,
) -> Result<impl axum::response::IntoResponse, ServiceError> {
    let _caller = admin(&state, &headers)?;

    let offset = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let all_workspaces: Vec<WorkspaceRecord> = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        ws_guard.values().cloned().collect()
    };

    let mut filtered: Vec<WorkspaceRecord> = all_workspaces
        .into_iter()
        .filter(|ws| {
            if let Some(ref s) = query.search {
                let term = s.to_lowercase();
                let matches_name = ws.name.to_lowercase().contains(&term);
                let matches_code = ws.code.to_lowercase().contains(&term);
                let matches_id = ws.id.to_lowercase().contains(&term);
                if !matches_name && !matches_code && !matches_id {
                    return false;
                }
            }
            if let Some(ref u) = query.unit {
                let unit_name = get_workspace_unit_name(&state, ws);
                let org_id_str = ws.organization_id.map(|id| id.to_string()).unwrap_or_default();
                if !unit_name.eq_ignore_ascii_case(u)
                    && !org_id_str.eq_ignore_ascii_case(u)
                    && !ws.organization.eq_ignore_ascii_case(u)
                {
                    return false;
                }
            }
            if let Some(ref c) = query.classification {
                if !ws.data_classification.eq_ignore_ascii_case(c) {
                    return false;
                }
            }
            if let Some(ref v) = query.visibility {
                if !ws.visibility.eq_ignore_ascii_case(v) {
                    return false;
                }
            }
            true
        })
        .collect();

    filtered.sort_by_key(|a| a.name.to_lowercase());

    let total = filtered.len();
    let has_more = total > offset + limit;
    let page: Vec<WorkspaceRecord> = filtered.into_iter().skip(offset).take(limit).collect();

    let all_manifests: Vec<AppManifest> = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        engine.list_manifests()
    };

    let mut manifests_by_ws: HashMap<String, Vec<AppManifest>> = HashMap::new();
    for m in all_manifests {
        if let Some(ref wid) = m.workspace_id {
            manifests_by_ws.entry(wid.clone()).or_default().push(m);
        }
    }

    let mut page_app_slugs = Vec::new();
    for ws in &page {
        if let Some(apps) = manifests_by_ws.get(&ws.id) {
            for app in apps {
                page_app_slugs.push(app.slug.clone());
            }
        }
    }

    let record_counts: HashMap<String, usize> = if let Some(ref repo) = state.repository {
        repo.count_records_by_app_slugs(&page_app_slugs)
            .map_err(|e| ServiceError::internal(e.to_string()))?
    } else {
        let mut map = HashMap::new();
        for slug in &page_app_slugs {
            let n = state.count_app_records(slug).map_err(ServiceError::internal)?;
            if n > 0 {
                map.insert(slug.clone(), n);
            }
        }
        map
    };

    let collabs_guard = state
        .collaborators
        .read()
        .map_err(|e| ServiceError::internal(e.to_string()))?;

    let mut rows: Vec<AdminWorkspaceRow> = Vec::with_capacity(page.len());
    for ws in page {
        let unit_name = get_workspace_unit_name(&state, &ws);
        let collabs = collabs_guard.get(&ws.id).cloned().unwrap_or_default();
        let owners: Vec<String> = collabs
            .iter()
            .filter(|c| c.role == "owner")
            .map(|c| c.eppn.clone())
            .collect();
        let collaborator_count = collabs.len();

        let ws_apps = manifests_by_ws.get(&ws.id).cloned().unwrap_or_default();
        let app_count = ws_apps.len();
        let record_count: usize = ws_apps
            .iter()
            .map(|m| record_counts.get(&m.slug).copied().unwrap_or(0))
            .sum();

        rows.push(AdminWorkspaceRow {
            id: ws.id,
            name: ws.name,
            code: ws.code,
            unit_name,
            classification: ws.data_classification,
            visibility: ws.visibility,
            owners,
            collaborator_count,
            app_count,
            record_count,
            created: ws.created_at,
            organization_id: ws.organization_id,
            description: Some(ws.description),
        });
    }

    let next_cursor = if has_more {
        Some(encode_cursor(offset + limit))
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "rows": rows,
            "next_cursor": next_cursor,
        })),
    ))
}

async fn get_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl axum::response::IntoResponse, ServiceError> {
    let _caller = admin(&state, &headers)?;

    let ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        ws_guard
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::not_found(format!("Workspace '{id}' not found")))?
    };

    let unit_name = get_workspace_unit_name(&state, &ws);

    let collabs = {
        let collabs_guard = state
            .collaborators
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        collabs_guard.get(&id).cloned().unwrap_or_default()
    };
    let owners: Vec<String> = collabs
        .iter()
        .filter(|c| c.role == "owner")
        .map(|c| c.eppn.clone())
        .collect();
    let collaborator_count = collabs.len();

    let manifests: Vec<AppManifest> = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        engine
            .list_manifests()
            .into_iter()
            .filter(|m| m.workspace_id.as_deref() == Some(&id))
            .collect()
    };

    let app_slugs: Vec<String> = manifests.iter().map(|m| m.slug.clone()).collect();
    let record_counts: HashMap<String, usize> = if let Some(ref repo) = state.repository {
        repo.count_records_by_app_slugs(&app_slugs)
            .map_err(|e| ServiceError::internal(e.to_string()))?
    } else {
        let mut map = HashMap::new();
        for slug in &app_slugs {
            let n = state.count_app_records(slug).map_err(ServiceError::internal)?;
            if n > 0 {
                map.insert(slug.clone(), n);
            }
        }
        map
    };

    let mut app_rows = Vec::with_capacity(manifests.len());
    for m in &manifests {
        let (records_per_table, _) = get_records_per_table(&state, m)?;
        app_rows.push(AdminAppRow {
            slug: m.slug.clone(),
            title: m.title.clone(),
            workspace: id.clone(),
            version: "1.0.0".to_string(),
            table_count: m.tables.len(),
            page_count: m.views.len(),
            custom_page_count: 0,
            record_count_per_table: records_per_table,
            updated: Utc::now().to_rfc3339(),
        });
    }

    let app_count = manifests.len();
    let record_count: usize = manifests
        .iter()
        .map(|m| record_counts.get(&m.slug).copied().unwrap_or(0))
        .sum();

    let row = AdminWorkspaceRow {
        id: ws.id,
        name: ws.name,
        code: ws.code,
        unit_name,
        classification: ws.data_classification,
        visibility: ws.visibility,
        owners,
        collaborator_count,
        app_count,
        record_count,
        created: ws.created_at,
        organization_id: ws.organization_id,
        description: Some(ws.description),
    };

    Ok((
        StatusCode::OK,
        Json(AdminWorkspaceDetail {
            row,
            collaborators: collabs,
            apps: app_rows,
        }),
    ))
}

async fn patch_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<impl axum::response::IntoResponse, ServiceError> {
    let caller = admin(&state, &headers)?;

    let payload: UpdateWorkspacePayload = serde_json::from_slice(&body)
        .map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")))?;

    let reason = payload.reason.as_deref().unwrap_or("").trim();
    if reason.is_empty() || reason.len() > 500 {
        return Err(ServiceError::bad_request(
            "A reason between 1 and 500 characters is required for administrative updates",
        ));
    }

    let updated = workspace_service::update_workspace(&caller, &id, payload, &state)?;
    Ok((StatusCode::OK, Json(updated)))
}

async fn transfer_workspace_ownership(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    raw_body: axum::body::Bytes,
) -> Result<impl axum::response::IntoResponse, ServiceError> {
    let caller = admin(&state, &headers)?;

    let body: TransferOwnershipPayload = serde_json::from_slice(&raw_body)
        .map_err(|e| ServiceError::bad_request(format!("Invalid request body: {e}")))?;

    let trimmed_reason = body.reason.trim();
    if trimmed_reason.is_empty() || trimmed_reason.len() > 500 {
        return Err(ServiceError::bad_request(
            "A reason between 1 and 500 characters is required for administrative actions",
        ));
    }

    let mut ws = {
        let ws_guard = state
            .workspaces
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        ws_guard
            .get(&id)
            .cloned()
            .ok_or_else(|| ServiceError::not_found(format!("Workspace '{id}' not found")))?
    };

    let target_user = resolve_user(&body.eppn, &state).ok_or_else(|| {
        ServiceError::bad_request("Target user cannot be resolved or is on hold")
    })?;

    let updated_ws = admin_write(
        &state,
        &caller,
        DecisionType::WorkspaceMemberRoleUpdated,
        "AC-02",
        trimmed_reason,
        &json!({
            "workspace_id": id,
            "new_owner": target_user.eppn,
        }),
        || {
            let mut collabs = {
                let collabs_guard = state
                    .collaborators
                    .read()
                    .map_err(|e| ServiceError::internal(e.to_string()))?;
                collabs_guard.get(&id).cloned().unwrap_or_default()
            };

            for c in collabs.iter_mut() {
                if c.role == "owner" {
                    c.role = "admin".to_string();
                    if let Some(ref repo) = state.repository {
                        repo.upsert_collaborator(c)
                            .map_err(|e| ServiceError::internal(e.to_string()))?;
                    }
                }
            }

            if let Some(existing) = collabs.iter_mut().find(|c| c.eppn == target_user.eppn) {
                existing.role = "owner".to_string();
                if let Some(ref repo) = state.repository {
                    repo.upsert_collaborator(existing)
                        .map_err(|e| ServiceError::internal(e.to_string()))?;
                }
            } else {
                let new_collab = CollaboratorRecord {
                    id: Uuid::new_v4().to_string(),
                    workspace_id: id.clone(),
                    eppn: target_user.eppn.clone(),
                    name: target_user.name.clone(),
                    role: "owner".to_string(),
                    scoped_affiliation: target_user.affiliation.clone(),
                    department: target_user.department.clone(),
                    added_at: Utc::now().to_rfc3339(),
                };
                if let Some(ref repo) = state.repository {
                    repo.upsert_collaborator(&new_collab)
                        .map_err(|e| ServiceError::internal(e.to_string()))?;
                }
                collabs.push(new_collab);
            }

            {
                let mut collabs_guard = state
                    .collaborators
                    .write()
                    .map_err(|e| ServiceError::internal(e.to_string()))?;
                collabs_guard.insert(id.clone(), collabs);
            }

            ws.lead = target_user.name.clone();
            if let Some(ref repo) = state.repository {
                repo.upsert_workspace(&ws)
                    .map_err(|e| ServiceError::internal(e.to_string()))?;
            }
            {
                let mut ws_guard = state
                    .workspaces
                    .write()
                    .map_err(|e| ServiceError::internal(e.to_string()))?;
                ws_guard.insert(id.clone(), ws.clone());
            }

            Ok(ws)
        },
    )?;

    Ok((StatusCode::OK, Json(updated_ws)))
}

fn get_records_per_table(
    state: &SharedState,
    manifest: &AppManifest,
) -> Result<(HashMap<String, usize>, Vec<DatasetRecord>), ServiceError> {
    let records: Vec<DatasetRecord> = if let Some(ref repo) = state.repository {
        repo.list_records(&manifest.slug)
            .map_err(|e| ServiceError::internal(e.to_string()))?
    } else {
        state.list_app_records(&manifest.slug).map_err(ServiceError::internal)?
    };

    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in &manifest.tables {
        counts.insert(t.id.clone(), 0);
    }

    for r in &records {
        if let Some(table_id) = r.data.get("_table_id").and_then(|v| v.as_str()) {
            if let Some(entry) = counts.get_mut(table_id) {
                *entry += 1;
            }
        } else if manifest.tables.len() <= 1 {
            if let Some(first) = manifest.tables.first() {
                if let Some(entry) = counts.get_mut(&first.id) {
                    *entry += 1;
                }
            }
        }
    }

    Ok((counts, records))
}

async fn list_apps(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<AdminAppsQuery>,
) -> Result<impl axum::response::IntoResponse, ServiceError> {
    let _caller = admin(&state, &headers)?;

    let offset = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let all_manifests: Vec<AppManifest> = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::internal(e.to_string()))?;
        engine.list_manifests()
    };

    let mut filtered: Vec<AppManifest> = all_manifests
        .into_iter()
        .filter(|m| {
            if let Some(ref s) = query.search {
                let term = s.to_lowercase();
                if !m.title.to_lowercase().contains(&term) && !m.slug.to_lowercase().contains(&term) {
                    return false;
                }
            }
            if let Some(ref ws) = query.workspace {
                if m.workspace_id.as_deref() != Some(ws.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();

    filtered.sort_by(|a, b| a.slug.cmp(&b.slug));

    let total = filtered.len();
    let has_more = total > offset + limit;
    let page: Vec<AppManifest> = filtered.into_iter().skip(offset).take(limit).collect();

    let mut rows: Vec<AdminAppRow> = Vec::with_capacity(page.len());
    for m in page {
        let (records_per_table, _) = get_records_per_table(&state, &m)?;
        rows.push(AdminAppRow {
            slug: m.slug.clone(),
            title: m.title.clone(),
            workspace: m.workspace_id.clone().unwrap_or_default(),
            version: "1.0.0".to_string(),
            table_count: m.tables.len(),
            page_count: m.views.len(),
            custom_page_count: 0,
            record_count_per_table: records_per_table,
            updated: Utc::now().to_rfc3339(),
        });
    }

    let next_cursor = if has_more {
        Some(encode_cursor(offset + limit))
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(json!({
            "rows": rows,
            "next_cursor": next_cursor,
        })),
    ))
}
