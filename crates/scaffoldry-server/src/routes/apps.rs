use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::state::{lock_err, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::{
    AutomationRule, ProcessInstance, ProcessStatus, TriggerEvent, WorkflowExecutionResult,
};
use scaffoldry_engine::{AppManifest, AutomationEngine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use crate::service::ServiceError;

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/workspaces/{id}/apps", post(create_app_in_workspace))
        .route("/me/apps", get(list_my_apps))
        .route("/apps/{slug}", get(get_app).put(update_app))
        .route("/apps/{slug}/publish", post(publish_app))
        .route("/apps/{slug}/pages/{id}", get(get_app_page))
        .route("/apps/{slug}/pages/{id}/call", post(call_app_page))
        .route("/apps/{slug}/automations", get(list_app_automations).post(create_app_automation))
        .route("/apps/{slug}/automations/simulate", post(simulate_app_automation))
        .route("/apps/{slug}/processes", get(list_app_processes))
        .route("/apps/{slug}/processes/{id}/decide", post(decide_app_process))
        .route("/apps/{slug}/access/preview", post(preview_app_access))
        .route("/apps/{slug}/collaborators", get(list_app_collaborators))
}

async fn create_app_in_workspace(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(ws_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    let ws = {
        let ws_guard = state.workspaces.read().map_err(|_| lock_err())?;
        ws_guard.get(&ws_id).cloned().ok_or_else(|| {
            ServiceError::not_found(format!("Workspace '{ws_id}' not found")).into_pair()
        })?
    };

    crate::service::access::authorize_workspace_denying(&user, &ws, "manage_app", "Forbidden: Cedar policy denied app creation in workspace", &state)
        .map_err(|e| e.into_pair())?;

    let slug = payload.get("slug").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if crate::service::proposals::live_app_exists(&slug, &state) {
        return Err(ServiceError::conflict(format!("An app with the slug '{slug}' already exists")).into_pair());
    }
    let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or(&slug).to_string();
    let input = crate::service::proposals::ProposeInput {
        workspace_id: ws_id.clone(),
        app_slug: None,
        manifest: Some(payload),
        rules: vec![],
        summary: format!("Create the app \"{title}\""),
        page_sources: vec![],
        notes: vec![],
    };
    match direct(&user, input, &state).await? {
        crate::service::proposals::Direct::Applied => {
            let manifest = live_manifest_of(&state, &slug)?;
            Ok((StatusCode::CREATED, Json(serde_json::to_value(manifest).unwrap_or(Value::Null))))
        }
        crate::service::proposals::Direct::Pending(p) => Ok((StatusCode::ACCEPTED, Json(pending_body(&p)))),
    }
}

/// Runs a direct change as a proposal, off the async threads.
async fn direct(
    user: &crate::state::AuthUser,
    input: crate::service::proposals::ProposeInput,
    state: &SharedState,
) -> Result<crate::service::proposals::Direct, (StatusCode, Json<Value>)> {
    let (user, state) = (user.clone(), state.clone());
    tokio::task::spawn_blocking(move || crate::service::proposals::submit_direct(&user, input, &state))
        .await
        .map_err(|_| ServiceError::internal("The change stopped before it finished").into_pair())?
        .map_err(ServiceError::into_pair)
}

/// The `202` body: the proposal, and nothing live.
fn pending_body(p: &Value) -> Value {
    json!({ "proposal_id": p["proposal_id"], "status": "Pending", "checks": p["checks"], "changes": p["changes"] })
}

fn live_manifest_of(state: &SharedState, slug: &str) -> Result<AppManifest, (StatusCode, Json<Value>)> {
    use scaffoldry_engine::HostRouter;
    state
        .engine
        .read()
        .map_err(|_| lock_err())?
        .resolve_by_slug(slug)
        .cloned()
        .ok_or_else(|| ServiceError::not_found("App not found").into_pair())
}

/// The page document, inside JSON. The same tool the MCP `get_page` runs, so the same checks apply.
async fn get_app_page(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    run_page_tool(state, headers, "get_page", json!({ "app_slug": slug, "page_id": id })).await
}

/// What a page asks for. The body is the arguments of `page_call` without the app and the page.
async fn call_app_page(State(state): State<SharedState>, headers: HeaderMap, Path((slug, id)): Path<(String, String)>, Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut args = match body {
        Value::Object(map) => map,
        _ => return Err(ServiceError::BadRequest("The body must be an object".into()).into_pair()),
    };
    args.insert("app_slug".into(), json!(slug));
    args.insert("page_id".into(), json!(id));
    run_page_tool(state, headers, "page_call", Value::Object(args)).await
}

async fn run_page_tool(state: SharedState, headers: HeaderMap, tool: &'static str, args: Value) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let origin = crate::guard::read_origin(&state, &headers);
    tokio::task::spawn_blocking(move || crate::service::tools::call(&user, &origin, tool, args, &state))
        .await
        .map_err(|_| ServiceError::internal("The call stopped").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn get_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    use scaffoldry_engine::HostRouter;
    let manifest = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine
            .resolve_by_slug(&slug)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
    };
    Ok(Json(with_version(&state, &manifest)))
}

/// The manifest as JSON, with the version a save must name.
pub(crate) fn with_version(state: &SharedState, manifest: &AppManifest) -> Value {
    let version = state.repository.app_manifest_version(&manifest.slug).ok().flatten().unwrap_or(1);
    let mut v = serde_json::to_value(manifest).unwrap_or(Value::Null);
    if let Some(map) = v.as_object_mut() {
        map.insert("version".to_string(), Value::from(version));
    }
    v
}

async fn update_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    // A save names the version it read. Without one there is nothing to check it against.
    let expected_version = payload
        .get("version")
        .and_then(|v| v.as_i64())
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| {
            ServiceError::Invalid {
                message: "version is required: send the version you read".to_string(),
                fields: std::collections::BTreeMap::from([("version".to_string(), "required".to_string())]),
            }
            .into_pair()
        })?;

    use scaffoldry_engine::HostRouter;
    let existing = {
        let engine = state.engine.read().map_err(|_| lock_err())?;
        engine
            .resolve_by_slug(&slug)
            .cloned()
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
    };

    // The save names the version it read. A newer one is a conflict before anything is proposed.
    {
        let repo = &state.repository;
        match repo.app_manifest_version(&slug).map_err(|e| ServiceError::internal(e.to_string()).into_pair())? {
            Some(live) if live != expected_version => {
                return Err(ServiceError::StaleVersion {
                    message: "Someone else saved this app first. Reload it and try again.".to_string(),
                    version: live,
                }
                .into_pair());
            }
            None => return Err(ServiceError::not_found("App not found").into_pair()),
            _ => {}
        }
    }

    // What the platform owns stays as it is. A save cannot change them.
    let mut body = payload;
    if let Some(map) = body.as_object_mut() {
        map.insert("slug".to_string(), json!(slug));
        map.insert("department".to_string(), json!(existing.department));
        map.insert("custom_domain".to_string(), json!(existing.custom_domain));
        map.remove("workspace_id");
    }
    let ws_id = existing.workspace_id.clone().ok_or_else(|| ServiceError::bad_request("The app has no workspace").into_pair())?;
    let input = crate::service::proposals::ProposeInput {
        workspace_id: ws_id,
        app_slug: Some(slug.clone()),
        manifest: Some(body),
        rules: vec![],
        summary: format!("Update the app \"{}\"", existing.title),
        page_sources: vec![],
        notes: vec![],
    };
    match direct(&user, input, &state).await? {
        crate::service::proposals::Direct::Applied => {
            let manifest = live_manifest_of(&state, &slug)?;
            Ok((StatusCode::OK, Json(with_version(&state, &manifest))))
        }
        crate::service::proposals::Direct::Pending(p) => Ok((StatusCode::ACCEPTED, Json(pending_body(&p)))),
    }
}

async fn publish_app(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    let domain = payload["custom_domain"]
        .as_str()
        .ok_or_else(|| ServiceError::bad_request("custom_domain is required").into_pair())?
        .to_string();

    // A manifest proposal that sets the domain. It cannot set the domain as verified.
    let mut manifest = live_manifest_of(&state, &slug)?;
    let ws_id = manifest.workspace_id.clone().ok_or_else(|| ServiceError::bad_request("The app has no workspace").into_pair())?;
    manifest.custom_domain = Some(domain.clone());
    let input = crate::service::proposals::ProposeInput {
        workspace_id: ws_id,
        app_slug: Some(slug.clone()),
        manifest: Some(serde_json::to_value(&manifest).unwrap_or(Value::Null)),
        rules: vec![],
        summary: format!("Publish the app \"{slug}\" at {domain}"),
        page_sources: vec![],
        notes: vec![],
    };
    match direct(&user, input, &state).await? {
        crate::service::proposals::Direct::Applied => {
            let manifest = live_manifest_of(&state, &slug)?;
            Ok((StatusCode::OK, Json(serde_json::to_value(manifest).unwrap_or(Value::Null))))
        }
        crate::service::proposals::Direct::Pending(p) => Ok((StatusCode::ACCEPTED, Json(pending_body(&p)))),
    }
}

async fn list_app_automations(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Vec<AutomationRule>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();
    Ok(Json(rules))
}

async fn create_app_automation(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    // A rules proposal for one rule.
    let rule: AutomationRule = serde_json::from_value(payload.clone())
        .map_err(|e| ServiceError::bad_request(format!("rule_schema: {e}")).into_pair())?;
    let manifest = live_manifest_of(&state, &slug)?;
    let ws_id = manifest.workspace_id.clone().ok_or_else(|| ServiceError::bad_request("The app has no workspace").into_pair())?;
    let input = crate::service::proposals::ProposeInput {
        workspace_id: ws_id,
        app_slug: Some(slug.clone()),
        manifest: None,
        rules: vec![payload],
        summary: format!("Save the rule \"{}\"", rule.name),
        page_sources: vec![],
        notes: vec![],
    };
    match direct(&user, input, &state).await? {
        crate::service::proposals::Direct::Applied => {
            let live = state
                .automations
                .read()
                .map_err(|_| lock_err())?
                .get(&slug)
                .and_then(|rules| rules.iter().find(|r| r.id == rule.id).cloned())
                .ok_or_else(|| ServiceError::internal("The rule was not saved").into_pair())?;
            Ok((StatusCode::CREATED, Json(serde_json::to_value(live).unwrap_or(Value::Null))))
        }
        crate::service::proposals::Direct::Pending(p) => Ok((StatusCode::ACCEPTED, Json(pending_body(&p)))),
    }
}

async fn simulate_app_automation(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Vec<WorkflowExecutionResult>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Manage, &state)
        .map_err(ServiceError::into_pair)?;

    let event: TriggerEvent = serde_json::from_value(payload["event"].clone())
        .map_err(|e| ServiceError::bad_request(format!("Invalid trigger event: {e}")).into_pair())?;
    let record = payload.get("record").cloned().unwrap_or(json!({}));

    let affiliation = EduPersonAffiliation::from_str(&user.affiliation)
        .unwrap_or(EduPersonAffiliation::Staff);
    let realm = user.eppn.split('@').nth(1).unwrap_or("university.edu");
    let identity = EduPersonIdentity {
        eppn: user.eppn.clone(),
        realm: realm.to_string(),
        affiliations: vec![affiliation],
    };

    let auto_engine = AutomationEngine;
    let automations = state.automations.read().map_err(|_| lock_err())?;
    let rules = automations.get(&slug).cloned().unwrap_or_default();

    let results: Vec<WorkflowExecutionResult> = rules
        .iter()
        .map(|r| auto_engine.evaluate_rule(r, &event, &record, &identity, &user.department, 0))
        .collect();

    Ok(Json(results))
}

#[derive(Debug, Deserialize, Default)]
pub struct ProcessListQuery {
    pub status: Option<String>,
    /// Only the instances the caller can decide now.
    pub mine: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessDecisionPayload {
    pub decision: String,
}

async fn list_app_processes(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<ProcessListQuery>,
) -> Result<Json<Vec<Value>>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let mut all: Vec<ProcessInstance> = state
        .repository
        .list_process_instances(Some(&slug), None)
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()).into_pair())?;
    // Who may decide changes as people are appointed, so the flag is refreshed when listed. An instance someone
    // decided since it was read is not written back.
    for inst in all.iter_mut() {
        if crate::service::approvers::refresh_no_approver(&state, inst) {
            let _ = state.repository.update_waiting_instance(inst);
        }
    }
    // An instance on a record the caller cannot read is left out.
    let record_ids: Vec<String> = all.iter().map(|i| i.record_id.clone()).collect();
    let readable = crate::service::scope::readable_ids(&user, &slug, &record_ids, &state);
    let filtered: Vec<Value> = all
        .iter()
        .filter(|inst| {
            if inst.app_slug != slug || !readable.contains(&inst.record_id) {
                return false;
            }
            if let Some(ref st) = query.status {
                let status_str = match inst.status {
                    ProcessStatus::Waiting => "Waiting",
                    ProcessStatus::Completed => "Completed",
                    ProcessStatus::Rejected => "Rejected",
                    ProcessStatus::Failed => "Failed",
                };
                if !status_str.eq_ignore_ascii_case(st) {
                    return false;
                }
            }
            if query.mine.unwrap_or(false)
                && !crate::service::approvers::caller_can_decide(&state, &user, inst)
            {
                return false;
            }
            true
        })
        .map(|inst| crate::service::approvers::instance_view(&state, inst))
        .collect();
    Ok(Json(filtered))
}

async fn decide_app_process(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<ProcessDecisionPayload>,
) -> Result<Json<ProcessInstance>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;

    let (user, state) = (user, state.clone());
    tokio::task::spawn_blocking(move || crate::service::processes::decide_process(&user, &origin, &slug, &id, &payload.decision, &state))
        .await
        .map_err(|_| ServiceError::internal("The decision stopped before it finished").into_pair())?
        .map(Json)
        .map_err(ServiceError::into_pair)
}


async fn preview_app_access(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<crate::service::scope::AccessPreviewInput>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    let output = crate::service::scope::preview_access(&user, &slug, payload, &state)
        .map_err(ServiceError::into_pair)?;

    Ok((StatusCode::OK, Json(serde_json::to_value(output).unwrap())))
}


#[derive(Debug, Deserialize, Default)]
struct CollaboratorQuery {
    q: Option<String>,
}

async fn list_app_collaborators(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(params): Query<CollaboratorQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let collabs = crate::service::people::list_app_collaborators(&slug, params.q.as_deref().unwrap_or(""), &state)
        .map_err(ServiceError::into_pair)?;

    Ok(Json(json!({ "collaborators": collabs })))
}

/// The apps the caller can reach, for the My apps list.
async fn list_my_apps(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers).ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;
    Ok(Json(serde_json::json!({ "apps": crate::service::my_apps::my_apps(&user, &state) })))
}
