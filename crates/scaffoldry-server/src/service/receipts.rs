//! Access model phase 8: receipts. A receipt is written for every denial, every privileged action and every read that
//! returns a protected field. It holds ids, never record values. A job seals them into hash-chained batches and writes one
//! ledger entry for each, so an edit to a receipt shows.

use crate::repository_receipts::{Batch, ReceiptFilter, Verification};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use axum::{extract::State, extract::Request, middleware::Next, response::Response};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// What a receipt is made of before it is written.
#[derive(Debug, Clone)]
pub struct NewReceipt {
    pub actor: String,
    pub acting_as: Option<String>,
    pub action: String,
    pub object_kind: String,
    pub object_id: String,
    pub outcome: &'static str,
    pub grant_ids: Vec<i64>,
    pub policy_ids: Vec<String>,
    pub narrowed_by: Vec<String>,
}

impl NewReceipt {
    /// A receipt for something `user` did. Under view-as the actor is the person looking and `acting_as` the person seen.
    pub fn of(user: &AuthUser, action: impl Into<String>, object: (&str, &str), outcome: &'static str) -> NewReceipt {
        let (actor, acting_as) = match &user.view_as_by {
            Some(viewer) => (viewer.eppn.clone(), Some(user.eppn.clone())),
            None => (user.eppn.clone(), None),
        };
        NewReceipt {
            actor,
            acting_as,
            action: action.into(),
            object_kind: object.0.chars().take(16).collect(),
            object_id: object.1.chars().take(255).collect(),
            outcome,
            grant_ids: Vec::new(),
            policy_ids: Vec::new(),
            narrowed_by: Vec::new(),
        }
    }
}

/// Writes a receipt. A receipt that cannot be written is reported on the log and never stops the request it describes: a
/// refusal stays a refusal, and a read is not held back by the bookkeeping.
pub fn record(state: &SharedState, receipt: &NewReceipt) {
    if let Err(e) = state.repository.insert_access_receipt(receipt) {
        eprintln!("access receipt not written: {e}");
    }
}

// The user a request resolved to, kept for the receipt layer. `session_user` fills it, so the layer needs no second look-up.
tokio::task_local! {
    pub static REQUEST_USER: Arc<Mutex<Option<AuthUser>>>;
}

/// Notes who a request is for, if a receipt layer is waiting for it.
pub fn note_user(user: &AuthUser) {
    let _ = REQUEST_USER.try_with(|slot| {
        if let Ok(mut s) = slot.lock() {
            *s = Some(user.clone());
        }
    });
}

/// The object a path is about, as a kind and an id: `/apps/{slug}/…` is an app, `/workspaces/{id}/…` a workspace.
pub fn object_of_path(path: &str) -> (String, String) {
    let trimmed = path.trim_start_matches("/api/v1").trim_start_matches("/api").trim_start_matches('/');
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["apps", slug, ..] => ("app".to_string(), slug.to_string()),
        ["workspaces", id, ..] => ("workspace".to_string(), id.to_string()),
        ["proposals", id, ..] => ("proposal".to_string(), id.to_string()),
        ["admin", rest @ ..] => ("admin".to_string(), rest.join("/")),
        [first, rest @ ..] => (first.to_string(), rest.join("/")),
        [] => ("system".to_string(), String::new()),
    }
}

/// Whether a request is a privileged action: a write under `/admin`, impersonation or view-as, or a change to a
/// workspace's coordinates.
fn is_privileged(method: &axum::http::Method, path: &str) -> bool {
    let writes = !matches!(*method, axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS);
    writes && (path.contains("/admin/") || path.ends_with("/auth/impersonate") || path.ends_with("/auth/stop-impersonate") || path.ends_with("/view-as") || path.ends_with("/auth/stop-view-as") || path.contains("/coordinates"))
}

/// The layer that writes the receipts a request owes: a denial, a privileged action, and everything done under view-as.
pub async fn layer(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let slot: Arc<Mutex<Option<AuthUser>>> = Arc::new(Mutex::new(None));
    let response = REQUEST_USER.scope(slot.clone(), next.run(req)).await;
    let Some(user) = slot.lock().ok().and_then(|mut s| s.take()) else { return response };
    let status = response.status();
    let object = object_of_path(&path);
    let action = format!("{method} {}", path.trim_start_matches("/api/v1").trim_start_matches("/api"));

    if status == axum::http::StatusCode::FORBIDDEN {
        // The error body names the policy that forbade it. Read it, write the receipt, and send the same body on.
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, 256 * 1024).await.unwrap_or_default();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let mut receipt = NewReceipt::of(&user, parsed["action"].as_str().map_or(action, str::to_string), (&object.0, &object.1), "deny");
        if let Some(id) = parsed["policy"]["id"].as_str() {
            receipt.policy_ids.push(id.to_string());
        }
        let writer = state.clone();
        let _ = tokio::task::spawn_blocking(move || record(&writer, &receipt)).await;
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    }
    if (status.is_success() || status == axum::http::StatusCode::ACCEPTED) && (user.view_as_by.is_some() || is_privileged(&method, &path)) {
        let receipt = NewReceipt::of(&user, action, (&object.0, &object.1), "allow");
        let writer = state.clone();
        let _ = tokio::task::spawn_blocking(move || record(&writer, &receipt)).await;
    }
    response
}

/// A denial inside a tool call. A tool call answers 200 with the error inside, so the layer above does not see it.
pub fn record_tool_denial(state: &SharedState, user: &AuthUser, tool: &str, args: &Value, error: &ServiceError) {
    if !matches!(error, ServiceError::Forbidden { .. } | ServiceError::FieldHidden { .. } | ServiceError::TokenScope(_) | ServiceError::ClientScope(_) | ServiceError::ViewAsReadOnly(_)) {
        return;
    }
    let object = if let Some(slug) = args["app_slug"].as_str() {
        ("app", slug.to_string())
    } else if let Some(id) = args["workspace_id"].as_str() {
        ("workspace", id.to_string())
    } else {
        ("tool", tool.to_string())
    };
    let body = error.body();
    let mut receipt = NewReceipt::of(user, body["action"].as_str().map_or_else(|| format!("tool:{tool}"), str::to_string), (object.0, &object.1), "deny");
    if let Some(id) = body["policy"]["id"].as_str() {
        receipt.policy_ids.push(id.to_string());
    }
    record(state, &receipt);
}

/// A read that returned a protected field in full: one receipt for the call, naming the table and no rows.
pub fn record_protected_read(state: &SharedState, user: &AuthUser, app_slug: &str, table: &str) {
    let receipt = NewReceipt::of(user, "read_protected", ("table", &format!("{app_slug}/{table}")), "allow");
    record(state, &receipt);
}

/// The ledger entry that holds a batch's hash.
fn ledger_entry_for(batch: &Batch) -> crate::repository::AppendDecisionParams {
    crate::repository::AppendDecisionParams {
        principal: "system".to_string(),
        organization_code: "SYSTEM".to_string(),
        app_slug: None,
        decision_type: DecisionType::AccessReceiptsSealed,
        oscal_control_id: "AU-09".to_string(),
        rationale: format!("Sealed {} access receipts, {} to {}", batch.receipts, batch.first_id, batch.last_id),
        payload: json!({ "batch_id": batch.id, "first_id": batch.first_id, "last_id": batch.last_id, "receipts": batch.receipts, "hash": batch.hash, "previous_hash": batch.previous_hash }),
    }
}

/// The most receipts one batch holds.
const BATCH_MOST: i64 = 10_000;

/// Seals the receipts waiting into one batch and writes its ledger entry. `None` when none are waiting.
pub fn seal_repository(repo: &crate::repository::PostgresRepository) -> Result<Option<i64>, ServiceError> {
    let internal = |e: crate::repository::RepositoryError| ServiceError::Internal(e.to_string());
    let Some(batch) = repo.seal_access_receipts(BATCH_MOST).map_err(internal)? else { return Ok(None) };
    let entry = repo.append_ledger_decision(ledger_entry_for(&batch)).map_err(internal)?;
    repo.set_batch_ledger_sequence(batch.id, entry.sequence as i64).map_err(internal)?;
    Ok(Some(batch.id))
}

pub fn seal(state: &SharedState) -> Result<Option<i64>, ServiceError> {
    seal_repository(&state.repository)
}

/// The job that seals every minute.
pub fn seal_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    match seal_repository(ctx.repository()) {
        Ok(batch) => Ok(json!({ "batch_id": batch })),
        Err(e) => Err(crate::jobs::JobError::Retry(e.message().to_string())),
    }
}

pub fn verify(state: &SharedState) -> Result<Verification, ServiceError> {
    state.repository.verify_access_receipts().map_err(|e| ServiceError::Internal(e.to_string()))
}

/// The query of the receipt list.
#[derive(Debug, Default)]
pub struct ListQuery {
    pub actor: Option<String>,
    pub object: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

pub fn list(state: &SharedState, caller: &AuthUser, q: &ListQuery) -> Result<Value, ServiceError> {
    crate::service::admin::require_platform_admin_or_compliance(caller, state)?;
    if q.outcome.as_deref().is_some_and(|o| o != "allow" && o != "deny") {
        return Err(ServiceError::BadRequest("outcome must be allow or deny".to_string()));
    }
    let before = match &q.cursor {
        Some(c) => Some(c.parse::<i64>().map_err(|_| ServiceError::BadRequest("Invalid cursor".to_string()))?),
        None => None,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let filter = ReceiptFilter { actor: q.actor.clone(), object: q.object.clone(), outcome: q.outcome.clone(), since: q.since.clone(), until: q.until.clone(), before, limit };
    let mut rows = state.repository.list_access_receipts(&filter).map_err(|e| ServiceError::Internal(e.to_string()))?;
    let next = (rows.len() as i64 > limit).then(|| rows[limit as usize - 1].id.to_string());
    rows.truncate(limit as usize);
    Ok(json!({ "receipts": rows, "next_cursor": next }))
}

/// Asks the decision again with the receipt's actor, action and object. The outcome is the same while the counters are the
/// same. When a counter has moved the answer says so, since the state the decision was made in is gone.
pub fn replay(state: &SharedState, caller: &AuthUser, id: i64) -> Result<Value, ServiceError> {
    use crate::service::access::{authorize_app, AppAction};
    crate::service::admin::require_platform_admin_or_compliance(caller, state)?;
    let receipt = state.repository.access_receipt(id).map_err(|e| ServiceError::Internal(e.to_string()))?.ok_or_else(|| ServiceError::NotFound("Receipt not found".to_string()))?;
    let counters_match = state.repository.cache_counters().map_err(|e| ServiceError::Internal(e.to_string()))? == receipt.counters;
    let who = receipt.acting_as.clone().unwrap_or_else(|| receipt.actor.clone());
    let user = crate::service::identity::resolve_user(&who, state).ok_or_else(|| ServiceError::NotFound("The person in this receipt is not in the directory any more".to_string()))?;
    let app_action = match receipt.action.as_str() {
        "read_app" => Some(AppAction::Read),
        "write_record" => Some(AppAction::WriteRecords),
        "comment" => Some(AppAction::Comment),
        "manage_app" => Some(AppAction::Manage),
        _ => None,
    };
    let (Some(action), "app") = (app_action, receipt.object_kind.as_str()) else {
        return Ok(json!({ "receipt": receipt.id, "replayable": false, "reason": "Only a refusal by the policy engine on an app can be asked again.", "counters_match": counters_match }));
    };
    let outcome = match authorize_app(&user, &receipt.object_id, action, state) {
        Ok(()) => "allow",
        Err(ServiceError::Forbidden { .. }) => "deny",
        Err(e) => return Err(e),
    };
    Ok(json!({ "receipt": receipt.id, "replayable": true, "original": receipt.outcome, "outcome": outcome, "matches": outcome == receipt.outcome, "counters_match": counters_match }))
}
