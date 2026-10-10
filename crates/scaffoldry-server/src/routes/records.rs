//! Tabular Record Data Ingestion, Retrieval, and Modification

use crate::guard::session_user;
use crate::service::access::{authorize_app, AppAction};
use crate::state::{DatasetRecord, SharedState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use scaffoldry_engine::HostRouter;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::service::ServiceError;

#[derive(Debug, Deserialize, Default)]
pub struct RecordListQuery {
    pub page_size: Option<usize>,
    /// Rows per page. `page_size` is the older name for the same thing.
    pub limit: Option<usize>,
    /// Where the next page starts: the `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// Only the filter, sort and offset paths below read this. It goes when phases 2 and 4 land.
    pub offset: Option<usize>,
    pub view: Option<String>,
    pub filter_by_formula: Option<String>,
    pub sort_field: Option<String>,
    pub sort_direction: Option<String>,
}

pub fn router() -> Router<SharedState> {
    Router::new()
        // App Schema & Metadata
        .route("/apps/{slug}/schema", get(get_app_schema))
        .route("/apps/{slug}/tables/{table_id}/schema", get(get_table_schema))
        // Table Records
        .route(
            "/apps/{slug}/tables/{table_id}/records",
            get(list_table_records).post(create_table_record),
        )
        .route(
            "/apps/{slug}/tables/{table_id}/records/{id}",
            get(get_table_record)
                .patch(update_table_record)
                .delete(delete_table_record),
        )
        // General App Records (backward compatible)
        .route("/apps/{slug}/records", get(list_records).post(create_record))
        .route(
            "/apps/{slug}/records/{id}",
            get(get_record).patch(update_record).delete(delete_record),
        )
}

// ---------------------------------------------------------------------------
// METADATA & SCHEMA ENDPOINTS
// ---------------------------------------------------------------------------

async fn get_app_schema(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    let manifest = engine
        .resolve_by_slug(&slug)
        .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?;

    Ok(Json(json!({
        "app_slug": manifest.slug,
        "title": manifest.title,
        "description": manifest.description,
        "department": manifest.department,
        "organization_code": manifest.organization_code,
        "tables": manifest.tables,
        "views": manifest.views,
        "relationships": manifest.relationships,
        "ceds_mappings": manifest.ceds_mappings,
    })))
}

async fn get_table_schema(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table_id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    let manifest = engine
        .resolve_by_slug(&slug)
        .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?;

    let table = manifest
        .tables
        .iter()
        .find(|t| t.id == table_id || t.slug == table_id)
        .ok_or_else(|| ServiceError::not_found("Table not found").into_pair())?;

    let relationships: Vec<_> = manifest
        .relationships
        .iter()
        .filter(|r| r.source_table_id == table.id || r.target_table_id == table.id)
        .cloned()
        .collect();

    let views: Vec<_> = manifest
        .views
        .iter()
        .filter(|v| v.table_id.as_deref() == Some(&table.id))
        .cloned()
        .collect();

    Ok(Json(json!({
        "app_slug": manifest.slug,
        "table_id": table.id,
        "table": table,
        "views": views,
        "relationships": relationships,
    })))
}

// ---------------------------------------------------------------------------
// TABLE RECORD CRUD WITH FILTERING, SORTING & PAGINATION
// ---------------------------------------------------------------------------

async fn list_table_records(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table_id)): Path<(String, String)>,
    Query(query): Query<RecordListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    // A plain list is one indexed page. Filter, sort and offset have no indexed form until the
    // pivot index exists (row-scale phase 2), so a request that uses one still takes the older
    // path below, which reads the whole app.
    let older_path = query.filter_by_formula.is_some() || query.sort_field.is_some() || query.offset.is_some();
    if !older_path {
        let limit = query.limit.or(query.page_size).unwrap_or(100).clamp(1, 1000);
        let cursor = match query.cursor.as_deref() {
            Some(c) => Some(decode_cursor(c).map_err(ServiceError::into_pair)?),
            None => None,
        };
        // One extra row says whether another page follows.
        let mut rows = state
            .list_records_page(&slug, &table_id, limit + 1, cursor)
            .map_err(|e| ServiceError::internal(e).into_pair())?;
        let next_cursor = if rows.len() > limit {
            rows.truncate(limit);
            rows.last().map(|r| encode_cursor(&r.created_at, &r.id))
        } else {
            None
        };
        return Ok(Json(json!({
            "app_slug": slug,
            "table_id": table_id,
            "records": rows,
            "next_cursor": next_cursor,
        })));
    }

    let app_records = state
        .list_app_records(&slug)
        .map_err(|e| ServiceError::internal(e).into_pair())?;

    // Filter by table_id (match tagged _table_id or match all if untagged single-table)
    let mut table_records: Vec<DatasetRecord> = app_records
        .into_iter()
        .filter(|r| {
            if let Some(t) = r.data.get("_table_id").and_then(|v| v.as_str()) {
                t == table_id
            } else {
                true
            }
        })
        .collect();

    // Filter by formula if requested
    if let Some(ref formula) = query.filter_by_formula {
        table_records.retain(|r| evaluate_formula_filter(&r.data, formula));
    }

    // Sort if requested
    if let Some(ref sort_field) = query.sort_field {
        let is_desc = query
            .sort_direction
            .as_ref()
            .map(|d| d.eq_ignore_ascii_case("desc"))
            .unwrap_or(false);

        table_records.sort_by(|a, b| {
            let va = a.data.get(sort_field);
            let vb = b.data.get(sort_field);
            let ord = match (va, vb) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(a_val), Some(b_val)) => {
                    if let (Some(na), Some(nb)) = (a_val.as_f64(), b_val.as_f64()) {
                        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                    } else if let (Some(sa), Some(sb)) = (a_val.as_str(), b_val.as_str()) {
                        sa.cmp(sb)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                }
            };
            if is_desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    let total = table_records.len();
    let offset = query.offset.unwrap_or(0);
    let page_size = query.page_size.unwrap_or(100).min(1000);

    let paged_records: Vec<DatasetRecord> = table_records
        .into_iter()
        .skip(offset)
        .take(page_size)
        .collect();

    let next_offset = if offset + paged_records.len() < total {
        Some(offset + paged_records.len())
    } else {
        None
    };

    Ok(Json(json!({
        "app_slug": slug,
        "table_id": table_id,
        "total": total,
        "offset": next_offset,
        "records": paged_records
    })))
}

async fn create_table_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table_id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<DatasetRecord>), (StatusCode, Json<Value>)> {
    let mut modified_payload = payload.clone();
    if let Value::Object(ref mut map) = modified_payload {
        if let Some(Value::Object(ref mut data_map)) = map.get_mut("data") {
            if !data_map.contains_key("_table_id") {
                data_map.insert("_table_id".to_string(), Value::String(table_id.clone()));
            }
        }
        if !map.contains_key("_table_id") {
            map.insert("_table_id".to_string(), Value::String(table_id.clone()));
        }
    }

    create_record(State(state), headers, Path(slug), Json(modified_payload)).await
}

async fn get_table_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, _table_id, id)): Path<(String, String, String)>,
) -> Result<Json<DatasetRecord>, (StatusCode, Json<Value>)> {
    get_record(State(state), headers, Path((slug, id))).await
}

async fn update_table_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, _table_id, id)): Path<(String, String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<DatasetRecord>, (StatusCode, Json<Value>)> {
    update_record(State(state), headers, Path((slug, id)), Json(payload)).await
}

async fn delete_table_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, _table_id, id)): Path<(String, String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    delete_record(State(state), headers, Path((slug, id))).await
}

// ---------------------------------------------------------------------------
// GENERAL APP RECORD CRUD
// ---------------------------------------------------------------------------

async fn list_records(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let app_records = state
        .list_app_records(&slug)
        .map_err(|e| ServiceError::internal(e).into_pair())?;
    let total = app_records.len();

    Ok(Json(json!({
        "app_slug": slug,
        "total": total,
        "records": app_records
    })))
}

async fn create_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<DatasetRecord>), (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::WriteRecords, &state)
        .map_err(ServiceError::into_pair)?;

    crate::service::records::create_record(&user, &slug, &payload, &state)
        .map(|rec| (StatusCode::CREATED, Json(rec)))
        .map_err(ServiceError::into_pair)
}

async fn get_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
) -> Result<Json<DatasetRecord>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::Read, &state)
        .map_err(ServiceError::into_pair)?;

    let record = state
        .find_record(&slug, &id)
        .map_err(|e| ServiceError::internal(e).into_pair())?
        .ok_or_else(|| ServiceError::not_found("Record not found").into_pair())?;
    Ok(Json(record))
}

async fn update_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<DatasetRecord>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::WriteRecords, &state)
        .map_err(ServiceError::into_pair)?;

    crate::service::records::update_record(&user, &slug, &id, &payload, &state)
        .map(Json)
        .map_err(ServiceError::into_pair)
}

async fn delete_record(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    authorize_app(&user, &slug, AppAction::WriteRecords, &state)
        .map_err(ServiceError::into_pair)?;

    crate::service::records::delete_record(&user, &slug, &id, &state)
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(ServiceError::into_pair)
}

fn evaluate_formula_filter(record_data: &Value, formula: &str) -> bool {
    let f = formula.trim();
    if f.is_empty() {
        return true;
    }

    if let Some((left, right)) = f.split_once('>') {
        let field = left.trim().trim_matches(|c| c == '{' || c == '}').trim();
        let val_num = right.trim().parse::<f64>().ok();
        let field_num = record_data
            .get(field)
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
        if let (Some(fn_val), Some(vn_val)) = (field_num, val_num) {
            return fn_val > vn_val;
        }
    }

    if let Some((left, right)) = f.split_once('<') {
        let field = left.trim().trim_matches(|c| c == '{' || c == '}').trim();
        let val_num = right.trim().parse::<f64>().ok();
        let field_num = record_data
            .get(field)
            .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
        if let (Some(fn_val), Some(vn_val)) = (field_num, val_num) {
            return fn_val < vn_val;
        }
    }

    if let Some((left, right)) = f.split_once('=') {
        let field = left.trim().trim_matches(|c| c == '{' || c == '}').trim();
        let target_str = right.trim().trim_matches(|c| c == '\'' || c == '"').trim();
        if let Some(actual) = record_data.get(field) {
            match actual {
                Value::String(s) => return s.eq_ignore_ascii_case(target_str),
                Value::Number(n) => return n.to_string() == target_str,
                Value::Bool(b) => return b.to_string() == target_str,
                _ => return false,
            }
        }
    }

    if let Some(actual) = record_data.as_object() {
        return actual.values().any(|v| {
            if let Value::String(s) = v {
                s.to_lowercase().contains(&f.to_lowercase())
            } else {
                false
            }
        });
    }

    true
}

/// A page cursor is the last row's `(created_at, id)`, opaque to the caller.
fn encode_cursor(created_at: &str, id: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{created_at}\n{id}"))
}

fn decode_cursor(cursor: &str) -> Result<(String, String), ServiceError> {
    use base64::Engine;
    let bad = || ServiceError::bad_request("cursor is not one this server issued");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cursor).map_err(|_| bad())?;
    let text = String::from_utf8(bytes).map_err(|_| bad())?;
    let (created_at, id) = text.split_once('\n').ok_or_else(bad)?;
    // The timestamp is compared as a timestamp in SQL, so refuse anything that is not one.
    if created_at.len() > 40 || !created_at.chars().all(|c| c.is_ascii_digit() || " :-.+TZ".contains(c)) {
        return Err(bad());
    }
    Ok((created_at.to_string(), id.to_string()))
}
