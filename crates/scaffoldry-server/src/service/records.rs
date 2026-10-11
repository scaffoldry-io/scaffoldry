//! Sovereign Tabular Record Service Layer with Cedar Policy Enforcement

use scaffoldry_engine::HostRouter;

use crate::service::ServiceError;
use crate::service::access::{authorize_app, AppAction};
use crate::service::history::{write_ctx, Origin};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use chrono::Utc;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::TriggerEvent;
use crate::repository::{IndexedClause, IndexedQuery, IndexedSort, IndexedValue};
use scaffoldry_engine::{AutomationEngine, CompoundFilter, EngineError, FieldSpec, FilterConjunction, FilterOperator, IndexColumn};
use serde_json::Value;

/// True when `field` of the record's table holds one record.
fn mirror_is_one(app_slug: &str, record: &DatasetRecord, field: &str, state: &SharedState) -> bool {
    let table = record.data.get("_table_id").and_then(Value::as_str).unwrap_or("");
    let (fields, _) = table_fields_lenient(app_slug, table, state);
    fields.iter().any(|f| f.name == field && f.link_cardinality == scaffoldry_engine::LinkCardinality::One)
}

/// Checks the link fields of a record about to be written. Each id must name a record of the target
/// table of this app, a `One` field takes at most one id, and no id repeats. A record that is missing,
/// in another table, or in an app the writer has no part in is refused with the same sentence, so the
/// answer does not tell them apart. Every problem names its field.
fn validate_links(app_slug: &str, id: Option<&str>, current: Option<&Value>, data: &Value, state: &SharedState) -> Result<(), ServiceError> {
    use crate::repository_links::{linked_ids, MAX_LINKS_PER_FIELD};
    use scaffoldry_engine::{FieldType, LinkCardinality};
    let table = data.get("_table_id").and_then(Value::as_str).unwrap_or("");
    let (fields, _) = table_fields_lenient(app_slug, table, state);
    let mut bad = std::collections::BTreeMap::new();
    let mut mirror_added: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut to_check: Vec<(String, String, Vec<String>, Option<String>)> = Vec::new();
    for f in fields.iter().filter(|f| f.field_type == FieldType::Link) {
        let value = data.get(&f.name);
        let shaped = match value {
            None | Some(Value::Null) => true,
            Some(Value::String(_)) => true,
            Some(Value::Array(a)) => a.iter().all(Value::is_string),
            Some(_) => false,
        };
        if !shaped {
            bad.insert(f.name.clone(), "must be a record id or a list of record ids".to_string());
            continue;
        }
        let ids = linked_ids(value);
        if f.link_cardinality == LinkCardinality::One && ids.len() > 1 {
            bad.insert(f.name.clone(), format!("holds one record, and {} were given", ids.len()));
            continue;
        }
        if ids.len() > MAX_LINKS_PER_FIELD {
            bad.insert(f.name.clone(), format!("holds at most {MAX_LINKS_PER_FIELD} records, and {} were given", ids.len()));
            continue;
        }
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        if unique.len() != ids.len() {
            bad.insert(f.name.clone(), "names the same record twice".to_string());
            continue;
        }
        if let (false, Some(target)) = (ids.is_empty(), f.link_target_table.clone()) {
            let added: Vec<String> = {
                let before = linked_ids(current.and_then(|c| c.get(&f.name)));
                ids.iter().filter(|x| !before.contains(x)).cloned().collect()
            };
            to_check.push((f.name.clone(), target, ids, f.link_related_field.clone().filter(|_| !added.is_empty())));
            mirror_added.insert(f.name.clone(), added);
        }
    }
    for (name, target, ids, related) in to_check {
        let found = state.existing_record_ids(app_slug, &target, &ids).map_err(ServiceError::Internal)?;
        if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
            bad.insert(name, format!("links to a record that does not exist or cannot be seen: {missing}"));
            continue;
        }
        // A two-way link into a field that holds one record cannot take a record that is taken.
        let Some(related) = related else { continue };
        let Some(this) = id else { continue };
        for other in mirror_added.get(&name).into_iter().flatten() {
            let Some(theirs) = state.find_record(app_slug, other).map_err(ServiceError::Internal)? else { continue };
            let one = !matches!(theirs.data.get(&related), Some(Value::Array(_)));
            let taken = linked_ids(theirs.data.get(&related));
            if one && !taken.is_empty() && taken.iter().any(|x| x != this) && mirror_is_one(app_slug, &theirs, &related, state) {
                bad.insert(name.clone(), format!("links to a record that already links to another record through '{related}': {other}"));
                break;
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(ServiceError::Invalid { message: "A linked record could not be used.".to_string(), fields: bad })
    }
}

pub fn create_record(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    data: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let affiliation = match caller.affiliation.to_lowercase().as_str() {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };

    let realm = caller
        .eppn
        .split('@')
        .nth(1)
        .unwrap_or("university.edu")
        .to_string();

    let identity = EduPersonIdentity {
        eppn: caller.eppn.clone(),
        realm,
        affiliations: vec![affiliation],
    };

    let data_payload = data.get("data").unwrap_or(data);
    let cleaned = strip_lookups(app_slug, data_payload, state);
    let data_payload = &cleaned;
    validate_links(app_slug, None, None, data_payload, state)?;

    let submitted = {
        let engine = state
            .engine
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let labels = state.labels.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        let categories = state.sensitivity_categories();
        engine
            .submit_record_labelled(&identity, app_slug, data_payload, &labels, &categories)
            .map_err(|e| match e {
                EngineError::AccessDenied(msg) => ServiceError::Forbidden {
                    message: format!("403 Forbidden: Cedar Policy denies record mutation: {msg}"),
                    reasons: vec![msg],
                    diagnostics: Vec::new(),
                    policy: None,
                },
                other => ServiceError::BadRequest(other.to_string()),
            })?
    };

    let record = DatasetRecord {
        id: submitted.id.to_string(),
        app_slug: app_slug.to_string(),
        data: submitted.data,
        ceds_mapping: submitted.ceds_mapping,
        is_ferpa_sensitive: submitted.is_ferpa_sensitive,
        created_at: Utc::now().to_rfc3339(),
        created_by: Some(caller.eppn.clone()),
        version: 1,
    };

    let ctx = write_ctx(&caller.eppn, origin, app_slug, &record.data, state);
    state
        .insert_record(&record, &ctx)
        .map_err(|e| ServiceError::Internal(format!("Failed to store the record: {e}")))?;

    let mut record_val = record.data.clone();
    if let Some(map) = record_val.as_object_mut() {
        map.insert("id".to_string(), Value::String(record.id.clone()));
    }
    let mut applied_rule_ids = Vec::new();
    run_automations(
        state,
        app_slug,
        TriggerEvent::RecordCreated,
        &record_val,
        &Actor { identity: &identity, department: &caller.department },
        0,
        &mut applied_rule_ids,
    );

    // Reload the record in case automations updated it
    let final_record = get_record(caller, app_slug, &record.id, state).unwrap_or(record);

    Ok(final_record)
}

pub fn list_records(
    caller: &AuthUser,
    app_slug: &str,
    state: &SharedState,
) -> Result<Vec<DatasetRecord>, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let rows = state.list_app_records(app_slug).map_err(ServiceError::Internal)?;
    Ok(redact_records(caller, app_slug, rows, state))
}

pub fn get_record(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let record = state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;
    Ok(redact_records(caller, app_slug, vec![record], state).remove(0))
}

pub fn update_record(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    id: &str,
    payload: &Value,
    state: &SharedState,
) -> Result<DatasetRecord, ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;

    // A save names the version it read. Without one there is nothing to check it against.
    let expected_version = match payload.get("version") {
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| version_required("version must be a whole number"))?,
        None => return Err(version_required("version is required: send the version you read")),
    };

    let current = state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;

    let mut new_data = match payload.get("data") {
        Some(d) => d.clone(),
        None => {
            let mut rest = payload.clone();
            if let Some(map) = rest.as_object_mut() {
                map.remove("version");
            }
            rest
        }
    };
    if let (Some(tid), Some(map)) = (current.data.get("_table_id").cloned(), new_data.as_object_mut()) {
        map.entry("_table_id".to_string()).or_insert(tid);
    }

    let new_data = strip_lookups(app_slug, &new_data, state);
    validate_links(app_slug, Some(id), Some(&current.data), &new_data, state)?;
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &new_data, state);
    let (outcome, saved) = state
        .save_record_checked(app_slug, id, &new_data, expected_version, &ctx)
        .map_err(ServiceError::Internal)?;
    let updated_rec = match (outcome, saved) {
        (crate::repository::RecordWrite::Saved(_), Some(rec)) => rec,
        (crate::repository::RecordWrite::Conflict(version), _) => {
            return Err(ServiceError::StaleVersion {
                message: "Someone else saved this record first. Reload it and try again.".to_string(),
                version,
            })
        }
        _ => return Err(ServiceError::NotFound(format!("Record '{id}' not found"))),
    };

    let affiliation = match caller.affiliation.to_lowercase().as_str() {
        "faculty" => EduPersonAffiliation::Faculty,
        "student" => EduPersonAffiliation::Student,
        "staff" => EduPersonAffiliation::Staff,
        "employee" => EduPersonAffiliation::Employee,
        _ => EduPersonAffiliation::Member,
    };
    let realm = caller
        .eppn
        .split('@')
        .nth(1)
        .unwrap_or("university.edu")
        .to_string();
    let identity = EduPersonIdentity {
        eppn: caller.eppn.clone(),
        realm,
        affiliations: vec![affiliation],
    };

    let mut record_val = updated_rec.data.clone();
    if let Some(map) = record_val.as_object_mut() {
        map.insert("id".to_string(), Value::String(updated_rec.id.clone()));
    }
    let mut applied_rule_ids = Vec::new();
    run_automations(
        state,
        app_slug,
        TriggerEvent::RecordUpdated,
        &record_val,
        &Actor { identity: &identity, department: &caller.department },
        0,
        &mut applied_rule_ids,
    );

    let final_rec = get_record(caller, app_slug, id, state).unwrap_or(updated_rec);
    Ok(final_rec)
}

pub fn delete_record(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let current = state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;
    check_not_restricted(app_slug, &current, state)?;
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &current.data, state);
    if state.remove_record(app_slug, id, &ctx).map_err(ServiceError::Internal)? {
        Ok(())
    } else {
        Err(ServiceError::NotFound(format!("Record '{id}' not found")))
    }
}

/// Deletes a record, but only at the version the caller read.
pub fn delete_record_checked(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    id: &str,
    version: i32,
    state: &SharedState,
) -> Result<(), ServiceError> {
    use crate::repository::RecordWrite;
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    let current = state
        .find_record(app_slug, id)
        .map_err(ServiceError::Internal)?
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))?;
    check_not_restricted(app_slug, &current, state)?;
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &current.data, state);
    match state.remove_record_checked(app_slug, id, version, &ctx).map_err(ServiceError::Internal)? {
        RecordWrite::Saved(_) => Ok(()),
        RecordWrite::Conflict(current) => Err(ServiceError::StaleVersion {
            message: "Someone else saved this record first. Reload it and try again.".to_string(),
            version: current,
        }),
        RecordWrite::Missing => Err(ServiceError::NotFound(format!("Record '{id}' not found"))),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a page. The route and the tool both come here.
// ---------------------------------------------------------------------------------------------

pub const UNINDEXED_ROW_LIMIT: i64 = 10_000;
pub const TOO_LARGE: &str = "This table is too large to filter or sort on a field that is not indexed.";

pub struct SortSpec {
    pub field_name: String,
    pub descending: bool,
}

pub struct PageQuery {
    pub table_id: String,
    pub filter: Option<CompoundFilter>,
    pub sort: Option<SortSpec>,
    pub limit: usize,
    pub cursor: Option<String>,
    /// Stop short of this many bytes of rows, and say where to continue.
    pub max_bytes: Option<usize>,
    /// True makes a table the manifest does not name a `NotFound`.
    pub must_exist: bool,
}

pub struct RecordPage {
    pub records: Vec<DatasetRecord>,
    pub next_cursor: Option<String>,
}

/// A page cursor is the last row's `(created_at, id)`, and its sort value when sorted. It is
/// opaque to the caller.
pub fn encode_cursor(row: &DatasetRecord, sort_field: Option<&str>) -> String {
    let key = sort_field.map(|field| row.data.get(field).cloned().unwrap_or(Value::Null));
    encode_cursor_key(row, key)
}

fn encode_cursor_key(row: &DatasetRecord, key: Option<Value>) -> String {
    use base64::Engine;
    let mut body = serde_json::json!({ "created_at": row.created_at, "id": row.id });
    if let Some(key) = key {
        body["key"] = key;
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body.to_string())
}

pub fn decode_cursor(cursor: &str) -> Result<crate::repository::RecordCursor, ServiceError> {
    use base64::Engine;
    let bad = || ServiceError::BadRequest("cursor is not one this server issued".to_string());
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cursor).map_err(|_| bad())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let created_at = body.get("created_at").and_then(|v| v.as_str()).ok_or_else(bad)?;
    let id = body.get("id").and_then(|v| v.as_str()).ok_or_else(bad)?;
    // The timestamp is compared as a timestamp in SQL, so refuse anything that is not one.
    if created_at.len() > 40 || !created_at.chars().all(|c| c.is_ascii_digit() || " :-.+TZ".contains(c)) {
        return Err(bad());
    }
    Ok(crate::repository::RecordCursor { created_at: created_at.to_string(), id: id.to_string(), key: body.get("key").cloned() })
}

/// The fields of a table, and the table id the rows carry. An app with no tables is one untabled
/// table whose rows carry no table id.
fn table_fields(app_slug: &str, table_id: &str, must_exist: bool, state: &SharedState) -> Result<(Vec<FieldSpec>, String), ServiceError> {
    let engine = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
    let manifest = engine
        .resolve_by_slug(app_slug)
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    if manifest.tables.is_empty() {
        let mut fields: Vec<FieldSpec> = Vec::new();
        for f in manifest.views.iter().flat_map(|v| v.fields.iter()) {
            if !fields.iter().any(|x| x.name == f.name) {
                fields.push(f.clone());
            }
        }
        // A tool asks for the one untabled table. A route asks for the table id in its path.
        return Ok((fields, if must_exist { String::new() } else { table_id.to_string() }));
    }
    match manifest.tables.iter().find(|t| t.id == table_id || t.slug == table_id) {
        Some(table) => Ok((table.fields.clone(), table.id.clone())),
        None if must_exist => Err(ServiceError::NotFound(format!("Table '{table_id}' not found"))),
        // A route may ask for a table the manifest does not name. It has no fields to filter on.
        None => Ok((Vec::new(), table_id.to_string())),
    }
}

fn is_viewer(caller: &AuthUser, app_slug: &str, state: &SharedState) -> bool {
    let workspace = state
        .engine
        .read()
        .ok()
        .and_then(|e| e.resolve_by_slug(app_slug).and_then(|m| m.workspace_id.clone()));
    let Some(workspace) = workspace else { return false };
    state
        .collaborators
        .read()
        .ok()
        .and_then(|c| c.get(&workspace).and_then(|list| list.iter().find(|m| m.eppn == caller.eppn).map(|m| m.role.eq_ignore_ascii_case("viewer"))))
        .unwrap_or(false)
}

/// The fields a caller may not read in one table. Only a workspace viewer is held back, and the
/// effective flag decides what is sensitive, so a label counts.
pub(crate) fn hidden_fields_for(caller: &AuthUser, app_slug: &str, table: &str, fields: &[FieldSpec], state: &SharedState) -> Vec<String> {
    hidden_fields(caller, app_slug, table, fields, state)
}

/// The fields of a table, or none when the app or table is unknown. For readers that must not fail.
pub(crate) fn table_fields_lenient(app_slug: &str, table_id: &str, state: &SharedState) -> (Vec<FieldSpec>, String) {
    table_fields(app_slug, table_id, false, state).unwrap_or_else(|_| (Vec::new(), table_id.to_string()))
}

/// The fields of a table that follow from others. A history row does not record them.
pub(crate) fn computed_fields(app_slug: &str, table_id: &str, state: &SharedState) -> Vec<String> {
    let (fields, _) = table_fields_lenient(app_slug, table_id, state);
    fields
        .into_iter()
        .filter(|f| f.field_type.is_computed())
        .map(|f| f.name)
        .collect()
}

/// The indexed fields of a table. A write keeps their index rows current.
pub(crate) fn indexed_fields_of(app_slug: &str, table_id: &str, state: &SharedState) -> Vec<FieldSpec> {
    let (fields, table) = table_fields_lenient(app_slug, table_id, state);
    let names = indexed_names(app_slug, &table, state);
    fields.into_iter().filter(|f| names.contains(&f.name)).collect()
}

/// The `Link` fields of a table, each with the field that mirrors it on the other table. Every write
/// keeps their `record_links` rows current and mirrors the two-way ones.
pub(crate) fn link_specs_of(app_slug: &str, table_id: &str, state: &SharedState) -> Vec<crate::repository_links::LinkSpec> {
    use scaffoldry_engine::{FieldType, HostRouter, LinkCardinality};
    let Ok(engine) = state.engine.read() else { return Vec::new() };
    let Some(manifest) = engine.resolve_by_slug(app_slug) else { return Vec::new() };
    let Some(table) = manifest.tables.iter().find(|t| t.id == table_id) else { return Vec::new() };
    table
        .fields
        .iter()
        .filter(|f| f.field_type == FieldType::Link)
        .map(|f| {
            let mirror = f.link_related_field.as_deref().and_then(|name| {
                let target = manifest.tables.iter().find(|t| Some(t.id.as_str()) == f.link_target_table.as_deref())?;
                target.fields.iter().find(|x| x.name == name)
            });
            crate::repository_links::LinkSpec {
                field: f.name.clone(),
                related: mirror.map(|m| m.name.clone()),
                related_one: mirror.is_some_and(|m| m.link_cardinality == LinkCardinality::One),
            }
        })
        .collect()
}

/// The link fields, on any table of the app, that refuse the delete of a record of `table_id` while
/// records link to it. Each is `(field, name of the table that holds it)`.
fn restricting_fields(app_slug: &str, table_id: &str, state: &SharedState) -> Vec<(String, String, String)> {
    use scaffoldry_engine::{FieldType, HostRouter, LinkOnDelete};
    let Ok(engine) = state.engine.read() else { return Vec::new() };
    let Some(manifest) = engine.resolve_by_slug(app_slug) else { return Vec::new() };
    let target_name = manifest.tables.iter().find(|t| t.id == table_id).map(|t| t.name.clone()).unwrap_or_default();
    manifest
        .tables
        .iter()
        .flat_map(|t| t.fields.iter().map(move |f| (t, f)))
        .filter(|(_, f)| f.field_type == FieldType::Link && f.link_on_delete == LinkOnDelete::Restrict && f.link_target_table.as_deref() == Some(table_id))
        .map(|(t, f)| (f.name.clone(), t.name.clone(), target_name.clone()))
        .collect()
}

fn singular(name: &str) -> String {
    let n = name.to_lowercase();
    if let Some(stem) = n.strip_suffix("ies") {
        format!("{stem}y")
    } else if n.ends_with('s') && !n.ends_with("ss") {
        n[..n.len() - 1].to_string()
    } else {
        n
    }
}

/// Refuses the delete of a record that records still link to through a `Restrict` field.
fn check_not_restricted(app_slug: &str, record: &DatasetRecord, state: &SharedState) -> Result<(), ServiceError> {
    let table = record.data.get("_table_id").and_then(Value::as_str).unwrap_or("");
    for (field, holder, target) in restricting_fields(app_slug, table, state) {
        let n = state.count_links_to(app_slug, &record.id, std::slice::from_ref(&field)).map_err(ServiceError::Internal)?;
        if n > 0 {
            let who = if n == 1 { singular(&holder) } else { holder.to_lowercase() };
            let verb = if n == 1 { "still links" } else { "still link" };
            return Err(ServiceError::ConstraintViolated(format!("{n} {who} {verb} to this {}.", singular(&target))));
        }
    }
    Ok(())
}

/// Every `Rollup` and `Count` of the app, with what a write needs to recompute it.
pub(crate) fn computed_specs_of(app_slug: &str, state: &SharedState) -> Vec<crate::repository_links::ComputedSpec> {
    use scaffoldry_engine::{FieldType, HostRouter};
    let Ok(engine) = state.engine.read() else { return Vec::new() };
    let Some(manifest) = engine.resolve_by_slug(app_slug) else { return Vec::new() };
    let mut out = Vec::new();
    for table in &manifest.tables {
        let indexed = scaffoldry_engine::indexed_fields(manifest, &table.id);
        for f in &table.fields {
            let (true, Some(via)) = (matches!(f.field_type, FieldType::Rollup | FieldType::Count), f.computed_via.clone()) else { continue };
            out.push(crate::repository_links::ComputedSpec {
                owner_table: table.id.clone(),
                field: f.name.clone(),
                via,
                count: f.field_type == FieldType::Count,
                function: f.rollup_function.clone().unwrap_or_else(|| "sum".to_string()),
                target: f.target_display_field.clone(),
                indexed: indexed.contains(&f.name),
            });
        }
    }
    out
}

/// `(field, expression)` pairs: the formulas stored at write time, and the formulas added at read time.
pub(crate) type FormulaSplit = (Vec<(String, String)>, Vec<(String, String)>);

/// The formulas of a table, split by when they are computed. A formula that names a lookup field is
/// computed when a page is read, because the lookup is. Every other formula is stored when its
/// record is written. Each list holds `(field, expression)` in field order.
pub(crate) fn formula_specs_of(app_slug: &str, table_id: &str, state: &SharedState) -> FormulaSplit {
    use scaffoldry_engine::FieldType;
    let (fields, _) = table_fields_lenient(app_slug, table_id, state);
    let lookups: Vec<&str> = fields.iter().filter(|f| f.field_type == FieldType::Lookup).map(|f| f.name.as_str()).collect();
    let (mut at_write, mut at_read) = (Vec::new(), Vec::new());
    for f in fields.iter().filter(|f| f.field_type == FieldType::Formula) {
        let Some(expression) = f.formula_expression.clone() else { continue };
        let refs = scaffoldry_engine::formula_field_refs(&expression).unwrap_or_default();
        if refs.iter().any(|r| lookups.contains(&r.as_str())) {
            at_read.push((f.name.clone(), expression));
        } else {
            at_write.push((f.name.clone(), expression));
        }
    }
    (at_write, at_read)
}

/// A lookup is never stored: it is added to the rows of a page when they are read. A value a caller sends
/// under a lookup field's name is dropped.
fn strip_lookups(app_slug: &str, data: &Value, state: &SharedState) -> Value {
    let table = data.get("_table_id").and_then(Value::as_str).unwrap_or("");
    let (fields, _) = table_fields_lenient(app_slug, table, state);
    let mut out = data.clone();
    if let Some(map) = out.as_object_mut() {
        for f in fields.iter().filter(|f| f.field_type == scaffoldry_engine::FieldType::Lookup) {
            map.remove(&f.name);
        }
        for (name, _) in formula_specs_of(app_slug, table, state).1 {
            map.remove(&name);
        }
    }
    out
}

/// Adds the lookup values to the rows of a page. One statement for each lookup field, however many
/// rows the page has. A value the reader may not see is not added.
fn fill_lookups(caller: &AuthUser, app_slug: &str, rows: &mut [DatasetRecord], state: &SharedState) {
    use scaffoldry_engine::FieldType;
    let Some(repo) = state.repository.as_deref() else { return };
    let mut by_table: std::collections::BTreeMap<String, Vec<usize>> = std::collections::BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_table.entry(r.data.get("_table_id").and_then(Value::as_str).unwrap_or("").to_string()).or_default().push(i);
    }
    for (table, positions) in by_table {
        let (fields, _) = table_fields_lenient(app_slug, &table, state);
        for f in fields.iter().filter(|f| f.field_type == FieldType::Lookup) {
            let (Some(via), Some(target)) = (f.computed_via.as_deref(), f.target_display_field.as_deref()) else { continue };
            let Some(target_table) = fields.iter().find(|x| x.name == via).and_then(|x| x.link_target_table.clone()) else { continue };
            let (target_fields, canonical) = table_fields_lenient(app_slug, &target_table, state);
            if hidden_fields(caller, app_slug, &canonical, &target_fields, state).iter().any(|h| h == target) {
                continue;
            }
            let owners: Vec<String> = positions.iter().map(|i| rows[*i].id.clone()).collect();
            let Ok(found) = repo.lookup_values(&owners, via, target) else { continue };
            let mut by_owner: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
            for (owner, value) in found {
                if !value.is_null() {
                    by_owner.entry(owner).or_default().push(value);
                }
            }
            for i in &positions {
                let Some(mut values) = by_owner.remove(&rows[*i].id) else { continue };
                let shown = if values.len() == 1 { values.remove(0) } else { Value::Array(values) };
                if let Some(map) = rows[*i].data.as_object_mut() {
                    map.insert(f.name.clone(), shown);
                }
            }
        }
        // A formula over a lookup is added now that the lookups are in the rows.
        for (name, expression) in formula_specs_of(app_slug, &table, state).1 {
            for i in &positions {
                let value = scaffoldry_engine::evaluate_formula(&expression, &rows[*i].data);
                if let (false, Some(map)) = (value.is_null(), rows[*i].data.as_object_mut()) {
                    map.insert(name.clone(), value);
                }
            }
        }
    }
}

/// The fields of a table that are protected under the effective rule, whoever reads them.
pub(crate) fn protected_fields_of(app_slug: &str, table_id: &str, state: &SharedState) -> Vec<String> {
    let (fields, canonical) = table_fields_lenient(app_slug, table_id, state);
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    fields
        .iter()
        .filter(|f| scaffoldry_engine::effective_ferpa_sensitive(f, labels.get(&(app_slug.to_string(), canonical.clone(), f.name.clone())), &categories).protected)
        .map(|f| f.name.clone())
        .collect()
}

/// The names of a table's indexed fields: marked, used by a view, or joined on.
fn indexed_names(app_slug: &str, table_id: &str, state: &SharedState) -> std::collections::BTreeSet<String> {
    let Ok(engine) = state.engine.read() else { return Default::default() };
    engine.resolve_by_slug(app_slug).map(|m| scaffoldry_engine::indexed_fields(m, table_id)).unwrap_or_default()
}

fn hidden_fields(caller: &AuthUser, app_slug: &str, table: &str, fields: &[FieldSpec], state: &SharedState) -> Vec<String> {
    if !is_viewer(caller, app_slug, state) {
        return Vec::new();
    }
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    fields
        .iter()
        .filter(|f| {
            let label = labels.get(&(app_slug.to_string(), table.to_string(), f.name.clone()));
            scaffoldry_engine::effective_ferpa_sensitive(f, label, &categories).protected
        })
        .map(|f| f.name.clone())
        .collect()
}

/// Removes, from each row, the fields the caller may not read. Rows of the same table share one
/// lookup.
pub fn redact_records(caller: &AuthUser, app_slug: &str, mut rows: Vec<DatasetRecord>, state: &SharedState) -> Vec<DatasetRecord> {
    if !is_viewer(caller, app_slug, state) {
        return rows;
    }
    let mut by_table: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for row in &mut rows {
        let table = row.data.get("_table_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let hidden = by_table.entry(table.clone()).or_insert_with(|| {
            table_fields(app_slug, &table, false, state)
                .map(|(fields, canonical)| hidden_fields(caller, app_slug, &canonical, &fields, state))
                .unwrap_or_default()
        });
        if let Some(map) = row.data.as_object_mut() {
            for name in hidden.iter() {
                map.remove(name);
            }
        }
    }
    rows
}

/// What a filter or sort needs from the pivot index, or the field that stops it.
fn indexed_plan(
    fields: &[FieldSpec],
    indexed: &std::collections::BTreeSet<String>,
    filter: Option<&CompoundFilter>,
    sort: Option<&SortSpec>,
) -> Result<(Vec<IndexedClause>, Option<IndexedSort>), String> {
    use scaffoldry_engine::FieldType;
    let field = |name: &str| fields.iter().find(|f| f.name == name);
    let mut clauses = Vec::new();
    if let Some(f) = filter {
        if f.conjunction == FilterConjunction::Or && f.clauses.len() > 1 {
            return Err(f.clauses[0].field_name.clone());
        }
        for c in &f.clauses {
            let spec = field(&c.field_name).filter(|s| indexed.contains(&s.name)).ok_or_else(|| c.field_name.clone())?;
            let multi = spec.field_type == FieldType::MultiSelect;
            let supported = match c.operator {
                FilterOperator::Equals => true,
                FilterOperator::NotEquals | FilterOperator::GreaterThan | FilterOperator::LessThan | FilterOperator::IsEmpty | FilterOperator::IsNotEmpty => !multi,
                FilterOperator::Contains | FilterOperator::NotContains => false,
            };
            if !supported {
                return Err(c.field_name.clone());
            }
            let needs_value = !matches!(c.operator, FilterOperator::IsEmpty | FilterOperator::IsNotEmpty);
            if needs_value && c.value.trim().is_empty() {
                continue;
            }
            clauses.push(IndexedClause {
                field: c.field_name.clone(),
                column: scaffoldry_engine::index_column(spec.field_type),
                multi,
                op: c.operator,
                value: c.value.clone(),
            });
        }
    }
    let sort = match sort {
        None => None,
        Some(s) => {
            let spec = field(&s.field_name).filter(|f| indexed.contains(&f.name) && f.field_type != FieldType::MultiSelect).ok_or_else(|| s.field_name.clone())?;
            Some(IndexedSort { field: s.field_name.clone(), column: scaffoldry_engine::index_column(spec.field_type), descending: s.descending })
        }
    };
    Ok((clauses, sort))
}

/// The cursor's key, read as the value type of the column the page walks.
fn indexed_after(after: &crate::repository::RecordCursor, column: IndexColumn) -> Result<(IndexedValue, String), ServiceError> {
    let bad = || ServiceError::BadRequest("cursor is not one this server issued".to_string());
    let value = match (&after.key, column) {
        (None, _) | (Some(Value::Null), _) => IndexedValue::Blank,
        (Some(Value::Number(n)), IndexColumn::Num) => IndexedValue::Num(n.as_f64().ok_or_else(bad)?),
        (Some(Value::String(t)), IndexColumn::Txt) => IndexedValue::Txt(t.clone()),
        (Some(Value::String(t)), IndexColumn::Ts) => IndexedValue::Ts(scaffoldry_engine::parse_instant(t).ok_or_else(bad)?),
        _ => return Err(bad()),
    };
    Ok((value, after.id.clone()))
}

/// The key a row has in the index column of `field`, for the cursor.
fn indexed_key(row: &DatasetRecord, spec: &FieldSpec) -> Value {
    let cols = scaffoldry_engine::index_rows(spec, row.data.get(&spec.name).unwrap_or(&Value::Null)).into_iter().next().unwrap_or((None, None, None));
    match cols {
        (Some(n), _, _) => serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null),
        (_, Some(t), _) => Value::String(t),
        (_, _, Some(t)) => Value::String(t),
        _ => Value::Null,
    }
}

/// One page of a table. With no filter and no sort it walks the paging index. A filter or sort on
/// indexed fields reads the pivot index. Any other filter or sort is allowed only on a table of
/// 10,000 rows or fewer.
pub fn list_page(caller: &AuthUser, app_slug: &str, query: PageQuery, state: &SharedState) -> Result<RecordPage, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let (fields, table) = table_fields(app_slug, &query.table_id, query.must_exist, state)?;
    let hidden = hidden_fields(caller, app_slug, &table, &fields, state);
    let check = |name: &str| -> Result<(), ServiceError> {
        if fields.iter().any(|f| f.name == name) && !hidden.iter().any(|h| h == name) {
            Ok(())
        } else {
            Err(ServiceError::BadRequest(format!("'{name}' is not a field of this table")))
        }
    };
    let clauses: Vec<_> = query
        .filter
        .as_ref()
        .map(|f| f.clauses.iter().map(|c| (c.field_name.clone(), c.operator, c.value.clone())).collect())
        .unwrap_or_default();
    for (name, _, _) in &clauses {
        check(name)?;
    }
    if let Some(sort) = &query.sort {
        check(&sort.field_name)?;
    }
    let after = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let sort_field = query.sort.as_ref().map(|s| s.field_name.clone());

    let plan = indexed_plan(&fields, &indexed_names(app_slug, &table, state), query.filter.as_ref(), query.sort.as_ref());
    let mut walk_spec: Option<FieldSpec> = None;
    let mut rows = if clauses.is_empty() && query.sort.is_none() {
        state
            .list_records_page(app_slug, &table, query.limit + 1, after.map(|c| (c.created_at, c.id)))
            .map_err(ServiceError::Internal)?
    } else if let Some((iclauses, isort)) = plan.as_ref().ok().filter(|(c, s)| !c.is_empty() || s.is_some()) {
        let named: Vec<String> = iclauses.iter().map(|c| c.field.clone()).chain(isort.iter().map(|s| s.field.clone())).collect();
        if let Some(percent) = state.index_building_percent(app_slug, &table, &named).map_err(ServiceError::Internal)? {
            return Err(ServiceError::IndexBuilding(format!("This field is still being prepared for sorting. It is {percent}% done.")));
        }
        let mut iq = IndexedQuery {
            app_slug: app_slug.to_string(),
            table_id: table.clone(),
            clauses: iclauses.clone(),
            sort: isort.clone(),
            after: None,
            limit: query.limit + 1,
        };
        if let Some((name, column, _)) = crate::repository_index::walk_of(&iq) {
            walk_spec = fields.iter().find(|f| f.name == name).cloned();
            if let Some(a) = &after {
                iq.after = Some(indexed_after(a, column)?);
            }
        }
        state.indexed_page(&iq).map_err(ServiceError::Internal)?
    } else {
        if state.table_has_more_than(app_slug, &table, UNINDEXED_ROW_LIMIT).map_err(ServiceError::Internal)? {
            let name = plan.err().unwrap_or_default();
            return Err(ServiceError::BadRequest(format!("{TOO_LARGE} Mark '{name}' as indexed in the app manifest to filter or sort on it.")));
        }
        let q = crate::repository::RecordQuery {
            table_id: table.clone(),
            any: query.filter.as_ref().is_some_and(|f| f.conjunction == FilterConjunction::Or),
            clauses,
            sort: query.sort.as_ref().map(|s| (s.field_name.clone(), s.descending)),
            after,
            limit: query.limit + 1,
        };
        state.query_records(app_slug, &q).map_err(ServiceError::Internal)?
    };

    let mut more = rows.len() > query.limit;
    rows.truncate(query.limit);
    if let Some(max) = query.max_bytes {
        let mut total = 0usize;
        let mut keep = 0usize;
        for row in &rows {
            total += serde_json::to_string(row).map(|s| s.len()).unwrap_or(0);
            if total > max && keep > 0 {
                break;
            }
            keep += 1;
        }
        if keep < rows.len() {
            rows.truncate(keep);
            more = true;
        }
    }
    let next_cursor = if !more {
        None
    } else if let Some(spec) = &walk_spec {
        rows.last().map(|r| encode_cursor_key(r, Some(indexed_key(r, spec))))
    } else {
        rows.last().map(|r| encode_cursor(r, sort_field.as_deref()))
    };
    fill_lookups(caller, app_slug, &mut rows, state);
    Ok(RecordPage { records: redact_records(caller, app_slug, rows, state), next_cursor })
}

/// Who triggered an automation: their identity and their stored department.
pub struct Actor<'a> {
    pub identity: &'a EduPersonIdentity,
    pub department: &'a str,
}

pub fn run_automations(
    state: &SharedState,
    app_slug: &str,
    event: TriggerEvent,
    record: &Value,
    actor: &Actor<'_>,
    depth: u8,
    applied_rule_ids: &mut Vec<String>,
) {
    // One run is one batch, including the runs its own writes set off.
    run_automations_in(uuid::Uuid::new_v4(), state, app_slug, event, record, actor, depth, applied_rule_ids)
}

#[allow(clippy::too_many_arguments)]
fn run_automations_in(
    batch: uuid::Uuid,
    state: &SharedState,
    app_slug: &str,
    event: TriggerEvent,
    record: &Value,
    actor: &Actor<'_>,
    depth: u8,
    applied_rule_ids: &mut Vec<String>,
) {
    if depth >= 3 {
        return;
    }

    let rules = {
        let automations = match state.automations.read() {
            Ok(m) => m,
            Err(_) => return,
        };
        automations.get(app_slug).cloned().unwrap_or_default()
    };

    let auto_engine = AutomationEngine;
    let mut modified = record.clone();
    let mut all_effects = Vec::new();

    for rule in &rules {
        if !rule.enabled || applied_rule_ids.contains(&rule.id) {
            continue;
        }

        let res = auto_engine.evaluate_rule(rule, &event, &modified, actor.identity, actor.department, depth);
        if res.trigger_matched && res.conditions_met && res.cedar_authorized {
            let ws_id = {
                let eng = state.engine.read().ok();
                eng.and_then(|e| e.resolve_by_slug(app_slug).and_then(|a| a.workspace_id.clone()))
                    .unwrap_or_default()
            };
            let aff_str = actor.identity.affiliations.first().map(|a| match a {
                scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty => "faculty",
                scaffoldry_core::standards::eduperson::EduPersonAffiliation::Student => "student",
                scaffoldry_core::standards::eduperson::EduPersonAffiliation::Staff => "staff",
                scaffoldry_core::standards::eduperson::EduPersonAffiliation::Employee => "employee",
                _ => "member",
            }).unwrap_or("member");

            let caller_auth = crate::state::AuthUser {
                eppn: actor.identity.eppn.clone(),
                name: actor.identity.eppn.clone(),
                role_title: String::new(),
                affiliation: aff_str.to_string(),
                department: actor.department.to_string(),
            };
            let resource = scaffoldry_policy::entities::Resource::Record(scaffoldry_policy::entities::RecordCtx {
                app_slug: app_slug.to_string(),
                department: actor.department.to_string(),
                workspace_id: ws_id,
                is_ferpa_sensitive: false,
                categories: Default::default(),
            });
            let auto_dec = crate::service::access::decide(state, &caller_auth, "run_automation", &resource);
            if !auto_dec.allowed {
                continue;
            }
            applied_rule_ids.push(rule.id.clone());
            if !res.effects.is_empty() {
                scaffoldry_core::workflow::apply_field_effects(&mut modified, &res.effects);
                all_effects.extend(res.effects);
            }
            if let Some(inst) = res.waiting_instance {
                let should_upsert = match state.process_instances.read() {
                    Ok(map) => match map.get(&inst.id) {
                        Some(existing) => existing.status != scaffoldry_core::ProcessStatus::Waiting,
                        None => true,
                    },
                    Err(_) => false,
                };
                if should_upsert {
                    let mut inst = inst;
                    inst.started_by = Some(actor.identity.eppn.clone());
                    inst.started_at = Utc::now().to_rfc3339();
                    crate::service::approvers::refresh_no_approver(state, &mut inst);
                    let _ = state.persist_process_instance(inst);
                }
            }
        }
    }

    if modified != *record {
        if let Some(record_id) = record.get("id").and_then(|v| v.as_str()) {
            let mut ctx = crate::service::history::ctx_for(&actor.identity.eppn, crate::repository::ActorKind::Automation, app_slug, &modified, state);
            ctx.batch_id = batch;
            let _ = state.overwrite_record_data(app_slug, record_id, &modified, &ctx);
        }

        if let Some(retrigger_event) = scaffoldry_core::workflow::effects_retrigger(&all_effects) {
            run_automations_in(
                batch,
                state,
                app_slug,
                retrigger_event,
                &modified,
                actor,
                depth + 1,
                applied_rule_ids,
            );
        }
    }
}

fn version_required(message: &str) -> ServiceError {
    ServiceError::Invalid {
        message: message.to_string(),
        fields: std::collections::BTreeMap::from([("version".to_string(), "required".to_string())]),
    }
}
