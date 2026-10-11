//! The record picker for a link: ids and labels of the records a person could link to.

use crate::service::access::{authorize_app, AppAction};
use crate::service::records::{decode_cursor, encode_cursor, redact_records, table_fields_lenient};
use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_engine::{FieldType, HostRouter};
use serde_json::{json, Value};

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 50;
/// How many text fields besides the primary one a query reads.
const SEARCHED_TEXT_FIELDS: usize = 3;

pub struct OptionsPage {
    pub options: Vec<Value>,
    pub next_cursor: Option<String>,
}

/// The primary field of a table, and the first text field that is not it.
fn labels_of(app_slug: &str, table: &str, state: &SharedState) -> (Option<String>, Option<String>) {
    let primary = state
        .engine
        .read()
        .ok()
        .and_then(|e| e.resolve_by_slug(app_slug).and_then(|m| m.tables.iter().find(|t| t.id == table).and_then(|t| t.primary_field.clone())));
    let (fields, _) = table_fields_lenient(app_slug, table, state);
    let secondary = fields.iter().find(|f| f.field_type == FieldType::Text && Some(&f.name) != primary.as_ref()).map(|f| f.name.clone());
    (primary, secondary)
}

/// The fields a query reads: the primary field and the first three text fields.
fn searched(app_slug: &str, table: &str, primary: Option<&String>, state: &SharedState) -> Vec<String> {
    let (fields, _) = table_fields_lenient(app_slug, table, state);
    let mut out: Vec<String> = primary.cloned().into_iter().collect();
    for f in fields.iter().filter(|f| f.field_type == FieldType::Text).take(SEARCHED_TEXT_FIELDS) {
        if !out.contains(&f.name) {
            out.push(f.name.clone());
        }
    }
    out
}

fn text_of(row: &DatasetRecord, field: Option<&String>) -> Option<String> {
    field.and_then(|f| row.data.get(f)).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The records of the rows a person may see. Row-level rules (`row_scope`, access-rules phase 2)
/// narrow this one call; until then a reader sees every live row, less the fields hidden from them.
fn in_scope(caller: &AuthUser, app_slug: &str, rows: Vec<DatasetRecord>, state: &SharedState) -> Vec<DatasetRecord> {
    redact_records(caller, app_slug, rows, state)
}

pub fn link_options(
    caller: &AuthUser,
    app_slug: &str,
    table_id: &str,
    query: &str,
    limit: Option<usize>,
    cursor: Option<&str>,
    state: &SharedState,
) -> Result<OptionsPage, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let repo = state.repository.as_deref().ok_or_else(|| ServiceError::Internal("The picker needs PostgreSQL".to_string()))?;
    let (_, table) = table_fields_lenient(app_slug, table_id, state);
    let (primary, secondary) = labels_of(app_slug, &table, state);
    let fields = searched(app_slug, &table, primary.as_ref(), state);
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let after = cursor.map(decode_cursor).transpose()?;
    let mut rows = repo
        .link_option_rows(&crate::repository_links::LinkOptionQuery {
            app_slug,
            table_id: &table,
            fields: &fields,
            needle: query.trim(),
            after: after.as_ref(),
            limit: limit + 1,
            scope: crate::service::scope::plan_of(caller, app_slug, &table, state).as_ref(),
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if more { rows.last().map(|r| encode_cursor(r, None)) } else { None };
    let options = in_scope(caller, app_slug, rows, state)
        .iter()
        .map(|r| json!({ "id": r.id, "label": text_of(r, primary.as_ref()).unwrap_or_else(|| r.id.clone()), "secondary": text_of(r, secondary.as_ref()) }))
        .collect();
    Ok(OptionsPage { options, next_cursor })
}
