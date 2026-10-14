//! The trash: what was deleted, who may see it, and how it comes back.
//!
//! Delete moves a record here. Only a retention job destroys it. An editor sees and restores what
//! they deleted themselves. An owner or admin sees and restores all of it.

use crate::repository::TrashRow;
use crate::service::access::{authorize_app, AppAction};
use crate::service::history::{write_ctx, Origin};
use crate::service::records::redact_records;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use serde_json::{json, Value};
use std::collections::HashMap;

/// How long a record stays in trash before a retention job may destroy it. A placeholder: the records
/// officer sets the real number (`lifecycle.keep_trash_days` in the platform settings).
pub const DEFAULT_KEEP_TRASH_DAYS: i64 = 30;
pub const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 200;

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    Ok(&*state.repository)
}

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn hide(err: ServiceError) -> ServiceError {
    match err {
        ServiceError::Forbidden { .. } => ServiceError::NotFound("Not found".to_string()),
        other => other,
    }
}

fn keep_days(state: &SharedState) -> i64 {
    state
        .settings
        .read()
        .ok()
        .and_then(|s| s.get("lifecycle.keep_trash_days").and_then(Value::as_i64))
        .filter(|d| *d >= 0)
        .unwrap_or(DEFAULT_KEEP_TRASH_DAYS)
}

fn primary_field(app_slug: &str, table_id: &str, state: &SharedState) -> Option<String> {
    use scaffoldry_engine::HostRouter;
    let engine = state.engine.read().ok()?;
    let manifest = engine.resolve_by_slug(app_slug)?;
    manifest.tables.iter().find(|t| t.id == table_id)?.primary_field.clone()
}

pub struct TrashPage {
    pub entries: Vec<Value>,
    pub next_cursor: Option<String>,
}

fn cursor_of(row: &TrashRow) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json!({ "at": row.deleted_at, "id": row.record.id }).to_string())
}

fn parse_cursor(cursor: Option<&str>) -> Result<Option<(String, String)>, ServiceError> {
    use base64::Engine;
    let Some(raw) = cursor else { return Ok(None) };
    let bad = || ServiceError::BadRequest("cursor is not one this server issued".to_string());
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).map_err(|_| bad())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let at = v.get("at").and_then(Value::as_str).ok_or_else(bad)?;
    let id = v.get("id").and_then(Value::as_str).ok_or_else(bad)?;
    if at.len() > 40 || !at.chars().all(|c| c.is_ascii_digit() || ":-.TZ ".contains(c)) {
        return Err(bad());
    }
    Ok(Some((at.to_string(), id.to_string())))
}

fn shown(row: &TrashRow, redacted: &crate::state::DatasetRecord, state: &SharedState) -> Value {
    let table = row.record.data.get("_table_id").and_then(Value::as_str).unwrap_or("");
    let title = primary_field(&row.record.app_slug, table, state)
        .and_then(|f| redacted.data.get(&f).cloned())
        .filter(|v| !v.is_null() && v != "")
        .unwrap_or_else(|| Value::String(row.record.id.clone()));
    let name = crate::service::identity::resolve_user(&row.deleted_by, state).map(|u| u.name).unwrap_or_else(|| row.deleted_by.clone());
    json!({
        "record_id": row.record.id,
        "table_id": table,
        "title": title,
        "deleted_by": row.deleted_by,
        "deleted_by_name": name,
        "deleted_at": row.deleted_at,
        "delete_batch": row.delete_batch,
        "days_left": row.days_left,
    })
}

/// The trash, newest first. A person who cannot change records sees an empty trash.
pub fn list_trash(caller: &AuthUser, app_slug: &str, limit: Option<usize>, cursor: Option<&str>, state: &SharedState) -> Result<TrashPage, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state).map_err(hide)?;
    if authorize_app(caller, app_slug, AppAction::WriteRecords, state).is_err() {
        return Ok(TrashPage { entries: Vec::new(), next_cursor: None });
    }
    let all = authorize_app(caller, app_slug, AppAction::Manage, state).is_ok();
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let before = parse_cursor(cursor)?;
    let only = if all { None } else { Some(caller.eppn.as_str()) };
    let mut rows = repo(state)?.trash_page(app_slug, only, keep_days(state), before, i64::try_from(limit + 1).unwrap_or(i64::MAX)).map_err(internal)?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more { rows.last().map(cursor_of) } else { None };
    // A deleted record the caller could not read is left out. A page can come back short.
    rows.retain(|r| crate::service::scope::allows(caller, app_slug, &r.record, scaffoldry_core::access::RowOp::Read, state));
    // The person sees what they could see before the deletion.
    let redacted = redact_records(caller, app_slug, rows.iter().map(|r| r.record.clone()).collect(), state);
    let entries = rows.iter().zip(redacted.iter()).map(|(row, red)| shown(row, red, state)).collect();
    Ok(TrashPage { entries, next_cursor })
}

/// Brings back a record, or every record of a batch deleted together.
pub fn restore(caller: &AuthUser, origin: &Origin, app_slug: &str, selector: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let repo = repo(state)?;
    let rows = repo.trashed_by_selector(app_slug, selector, keep_days(state)).map_err(internal)?;
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| crate::service::scope::allows(caller, app_slug, &r.record, scaffoldry_core::access::RowOp::Update, state))
        .collect();
    if rows.is_empty() {
        return Err(ServiceError::NotFound("Nothing in trash matches that".to_string()));
    }
    let admin = authorize_app(caller, app_slug, AppAction::Manage, state).is_ok();
    if !admin && rows.iter().any(|r| !r.deleted_by.eq_ignore_ascii_case(&caller.eppn)) {
        return Err(ServiceError::forbidden("You can restore what you deleted. An owner or admin can restore the rest."));
    }
    let batch = uuid::Uuid::new_v4();
    let mut tables: HashMap<String, crate::repository::WriteCtx> = HashMap::new();
    for row in &rows {
        let table = row.record.data.get("_table_id").and_then(Value::as_str).unwrap_or("").to_string();
        tables.entry(table.clone()).or_insert_with(|| {
            let mut ctx = write_ctx(&caller.eppn, origin, app_slug, &json!({ "_table_id": table }), state);
            ctx.batch_id = batch;
            ctx
        });
    }
    let restored = repo.restore_trash(app_slug, selector, tables).map_err(internal)?;
    Ok(json!({ "restored": restored.len(), "records": restored, "batch_id": batch }))
}
