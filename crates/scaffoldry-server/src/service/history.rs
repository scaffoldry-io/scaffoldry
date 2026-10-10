//! Record history: who changed what, read back for a person who may see the record.
//!
//! The rows are written by the repository in the same transaction as the record write. This
//! module attributes a write (`Origin`) and reads the rows back, masking what the reader may not see.

use crate::repository::{ActorKind, HistoryEntry, HistoryFilter, WriteCtx};
use crate::service::access::{authorize_app, AppAction};
use crate::service::records::{hidden_fields_for, table_fields_lenient};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use serde_json::{json, Value};

/// How a request reached the server, as far as attribution goes.
#[derive(Debug, Clone)]
pub struct Origin {
    pub kind: ActorKind,
    /// The label of the agent token, when one was used.
    pub token_label: Option<String>,
    /// A caller who writes several records as one action names the batch.
    pub batch_id: Option<uuid::Uuid>,
}

impl Origin {
    pub fn user() -> Self {
        Self { kind: ActorKind::User, token_label: None, batch_id: None }
    }
}

/// The page size of a history read.
pub const DEFAULT_PAGE: usize = 50;
pub const MAX_PAGE: usize = 200;

/// The write context for a record whose data is `data`.
pub fn write_ctx(actor: &str, origin: &Origin, app_slug: &str, data: &Value, state: &SharedState) -> WriteCtx {
    let mut ctx = ctx_for(actor, origin.kind, app_slug, data, state);
    ctx.token_label = origin.token_label.clone();
    if let Some(batch) = origin.batch_id {
        ctx.batch_id = batch;
    }
    ctx
}

/// The context for a write that has no request behind it, such as an automation.
pub fn ctx_for(actor: &str, kind: ActorKind, app_slug: &str, data: &Value, state: &SharedState) -> WriteCtx {
    let mut ctx = WriteCtx::new(actor, kind);
    let table = data.get("_table_id").and_then(|v| v.as_str()).unwrap_or("");
    ctx.computed_fields = crate::service::records::computed_fields(app_slug, table, state);
    ctx
}

/// What `reader` may see of an entry. A field the reader may not read shows that it changed and
/// nothing else. Both routes and the tool call this.
pub fn visible_changes(entry: &HistoryEntry, hidden: &[String]) -> Value {
    let Some(map) = entry.changes.as_object() else { return entry.changes.clone() };
    let mut out = serde_json::Map::new();
    for (field, change) in map {
        if hidden.iter().any(|h| h == field) {
            out.insert(field.clone(), json!({ "changed": true }));
        } else {
            out.insert(field.clone(), change.clone());
        }
    }
    Value::Object(out)
}

pub struct HistoryPage {
    pub entries: Vec<Value>,
    pub next_cursor: Option<String>,
}

pub struct HistoryQuery {
    pub filter: HistoryFilter,
    pub cursor: Option<String>,
    pub limit: usize,
}

fn parse_cursor(cursor: Option<&str>) -> Result<Option<i64>, ServiceError> {
    match cursor {
        None => Ok(None),
        Some(c) => c
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or_else(|| ServiceError::BadRequest("cursor is not one this server issued".to_string())),
    }
}

fn parse_time(name: &str, value: Option<String>) -> Result<Option<String>, ServiceError> {
    match value {
        None => Ok(None),
        Some(v) => chrono::DateTime::parse_from_rfc3339(&v)
            .map(|t| Some(t.to_rfc3339()))
            .map_err(|_| ServiceError::BadRequest(format!("'{name}' must be an RFC 3339 time"))),
    }
}

fn page(caller: &AuthUser, app_slug: &str, mut query: HistoryQuery, state: &SharedState) -> Result<HistoryPage, ServiceError> {
    let repo = state
        .repository
        .as_deref()
        .ok_or_else(|| ServiceError::Internal("History needs PostgreSQL".to_string()))?;
    query.filter.from = parse_time("from", query.filter.from.take())?;
    query.filter.to = parse_time("to", query.filter.to.take())?;
    let before = parse_cursor(query.cursor.as_deref())?;
    let limit = query.limit.clamp(1, MAX_PAGE);
    let mut rows = repo
        .list_history(app_slug, &query.filter, before, limit + 1)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more { rows.last().map(|r| r.id.to_string()) } else { None };

    // One lookup of the hidden fields per table, and one of the people.
    let mut hidden: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in &rows {
        let hide = hidden.entry(row.table_id.clone()).or_insert_with(|| {
            let (fields, canonical) = table_fields_lenient(app_slug, &row.table_id, state);
            hidden_fields_for(caller, app_slug, &canonical, &fields, state)
        });
        let name = names
            .entry(row.actor.clone())
            .or_insert_with(|| {
                crate::service::identity::resolve_user(&row.actor, state)
                    .map(|u| u.name)
                    .unwrap_or_else(|| row.actor.clone())
            })
            .clone();
        entries.push(json!({
            "id": row.id,
            "record_id": row.record_id,
            "table_id": row.table_id,
            "version": row.version,
            "action": row.action,
            "changes": visible_changes(row, hide),
            "actor": row.actor,
            "actor_name": name,
            "actor_kind": row.actor_kind,
            "token_label": row.token_label,
            "batch_id": row.batch_id,
            "reverts_batch": row.reverts_batch,
            "proposal_id": row.proposal_id,
            "at": row.at,
        }));
    }
    Ok(HistoryPage { entries, next_cursor })
}

/// The history of one record. A person who cannot read the record gets `NotFound`, so the
/// existence of the record is not disclosed. A deleted record's history stays readable by
/// anyone who can read the app.
pub fn record_history(caller: &AuthUser, app_slug: &str, record_id: &str, mut query: HistoryQuery, state: &SharedState) -> Result<HistoryPage, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state).map_err(hide_denial)?;
    query.filter.record_id = Some(record_id.to_string());
    let result = page(caller, app_slug, query, state)?;
    if result.entries.is_empty() && state.find_record(app_slug, record_id).map_err(ServiceError::Internal)?.is_none() {
        return Err(ServiceError::NotFound(format!("Record '{record_id}' not found")));
    }
    Ok(result)
}

/// The app-wide feed, for an app owner or admin and a Platform Admin.
pub fn app_history(caller: &AuthUser, app_slug: &str, query: HistoryQuery, state: &SharedState) -> Result<HistoryPage, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state).map_err(hide_denial)?;
    page(caller, app_slug, query, state)
}

fn hide_denial(err: ServiceError) -> ServiceError {
    match err {
        ServiceError::Forbidden { .. } => ServiceError::NotFound("Not found".to_string()),
        other => other,
    }
}
