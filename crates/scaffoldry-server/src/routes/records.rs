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
    pub search: Option<String>,
}

pub fn router() -> Router<SharedState> {
    Router::new()
        // App Schema & Metadata
        .route("/apps/{slug}/schema", get(get_app_schema))
        .route("/apps/{slug}/tables/{table_id}/schema", get(get_table_schema))
        // Table Records
        .route(
            "/apps/{slug}/tables/{table_id}/aggregate",
            get(aggregate_table_records),
        )
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

    let mut manifest = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine
            .resolve_by_slug(&slug)
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
            .clone()
    };
    crate::service::records::without_hidden_fields(&user, &slug, &mut manifest, &state);

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

    let mut manifest = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine
            .resolve_by_slug(&slug)
            .ok_or_else(|| ServiceError::not_found("App not found").into_pair())?
            .clone()
    };
    crate::service::records::without_hidden_fields(&user, &slug, &mut manifest, &state);

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
        let page = crate::service::records::list_page(
            &user,
            &slug,
            crate::service::records::PageQuery {
                table_id: table_id.clone(),
                filter: None,
                sort: None,
                limit,
                cursor: query.cursor.clone(),
                max_bytes: None,
                must_exist: false,
                search: query.search.clone(),
            },
            &state,
        )
        .map_err(ServiceError::into_pair)?;
        crate::service::disclosure::record_agent_read(&user, &crate::guard::read_origin(&state, &headers), &slug, Some(&table_id), &page.records, &state)
            .map_err(ServiceError::into_pair)?;
        let mut body = json!({
            "app_slug": slug,
            "table_id": table_id,
            "records": page.records,
            "next_cursor": page.next_cursor,
        });
        crate::service::records::add_field_states(&mut body, &page.calculating);
        return Ok(Json(body));
    }

    // The service applies the row scope and redacts before filtering and sorting, so a viewer
    // cannot probe a field by filtering on it.
    let app_records = crate::service::records::list_records(&user, &slug, &state).map_err(ServiceError::into_pair)?;

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

    crate::service::disclosure::record_agent_read(&user, &crate::guard::read_origin(&state, &headers), &slug, Some(&table_id), &paged_records, &state)
        .map_err(ServiceError::into_pair)?;

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

    // The service applies the row scope and redacts before filtering and sorting, so a viewer
    // cannot probe a field by filtering on it.
    let app_records = crate::service::records::list_records(&user, &slug, &state).map_err(ServiceError::into_pair)?;
    crate::service::disclosure::record_agent_read(&user, &crate::guard::read_origin(&state, &headers), &slug, None, &app_records, &state)
        .map_err(ServiceError::into_pair)?;
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

    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    crate::service::records::create_record(&user, &origin, &slug, &payload, &state)
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

    let record = crate::service::records::get_record(&user, &slug, &id, &state).map_err(ServiceError::into_pair)?;
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

    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    crate::service::records::update_record(&user, &origin, &slug, &id, &payload, &state)
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

    let origin = crate::guard::origin(&state, &headers).map_err(ServiceError::into_pair)?;
    crate::service::records::delete_record(&user, &origin, &slug, &id, &state)
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


// ---------------------------------------------------------------------------
// TABLE AGGREGATE ENDPOINT (TOTALS & GROUPS)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AggregateRequestQuery {
    pub function: String,
    pub field: Option<String>,
    pub filter: Option<String>,
    pub group_by: Option<String>,
}

async fn aggregate_table_records(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path((slug, table_id)): Path<(String, String)>,
    Query(query): Query<AggregateRequestQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = session_user(&state, &headers)
        .ok_or_else(|| ServiceError::unauthorized("Unauthorized").into_pair())?;

    let func = match query.function.to_lowercase().as_str() {
        "count" => crate::service::records::AggregateFunction::Count,
        "sum" => crate::service::records::AggregateFunction::Sum,
        "avg" => crate::service::records::AggregateFunction::Avg,
        "min" => crate::service::records::AggregateFunction::Min,
        "max" => crate::service::records::AggregateFunction::Max,
        other => return Err(ServiceError::BadRequest(format!("Unknown aggregate function '{other}'")).into_pair()),
    };

    let filter = if let Some(ref f) = query.filter {
        if f.trim().is_empty() {
            None
        } else {
            match serde_json::from_str::<scaffoldry_engine::CompoundFilter>(f) {
                Ok(cf) => Some(cf),
                Err(e) => return Err(ServiceError::BadRequest(format!("Invalid filter: {e}")).into_pair()),
            }
        }
    } else {
        None
    };

    let result = crate::service::records::aggregate_records(
        &user,
        &slug,
        &table_id,
        crate::service::records::AggregateQuery {
            function: func,
            field: query.field,
            filter,
            group_by: query.group_by,
        },
        &state,
    )
    .map_err(ServiceError::into_pair)?;

    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}
