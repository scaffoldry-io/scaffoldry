//! Sovereign Tabular Record Service Layer with Cedar Policy Enforcement

use scaffoldry_engine::HostRouter;

use crate::service::ServiceError;
use crate::service::access::{authorize_app, AppAction};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use chrono::Utc;
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_core::TriggerEvent;
use scaffoldry_engine::{AutomationEngine, CompoundFilter, EngineError, FieldSpec, FilterConjunction};
use serde_json::Value;

pub fn create_record(
    caller: &AuthUser,
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

    state
        .insert_record(&record)
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

    let (outcome, saved) = state
        .save_record_checked(app_slug, id, &new_data, expected_version)
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
    app_slug: &str,
    id: &str,
    state: &SharedState,
) -> Result<(), ServiceError> {
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    if state.remove_record(app_slug, id).map_err(ServiceError::Internal)? {
        Ok(())
    } else {
        Err(ServiceError::NotFound(format!("Record '{id}' not found")))
    }
}

/// Deletes a record, but only at the version the caller read.
pub fn delete_record_checked(
    caller: &AuthUser,
    app_slug: &str,
    id: &str,
    version: i32,
    state: &SharedState,
) -> Result<(), ServiceError> {
    use crate::repository::RecordWrite;
    authorize_app(caller, app_slug, AppAction::WriteRecords, state)?;
    match state.remove_record_checked(app_slug, id, version).map_err(ServiceError::Internal)? {
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
    use base64::Engine;
    let mut body = serde_json::json!({ "created_at": row.created_at, "id": row.id });
    if let Some(field) = sort_field {
        body["key"] = row.data.get(field).cloned().unwrap_or(Value::Null);
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

/// One page of a table. With no filter and no sort it walks the paging index. A filter or a sort
/// is allowed only on a table of 10,000 rows or fewer, until the pivot index exists.
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

    let mut rows = if clauses.is_empty() && query.sort.is_none() {
        state
            .list_records_page(app_slug, &table, query.limit + 1, after.map(|c| (c.created_at, c.id)))
            .map_err(ServiceError::Internal)?
    } else {
        if state.table_has_more_than(app_slug, &table, UNINDEXED_ROW_LIMIT).map_err(ServiceError::Internal)? {
            return Err(ServiceError::BadRequest(TOO_LARGE.to_string()));
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
    let next_cursor = if more { rows.last().map(|r| encode_cursor(r, sort_field.as_deref())) } else { None };
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
            let _ = state.overwrite_record_data(app_slug, record_id, &modified);
        }

        if let Some(retrigger_event) = scaffoldry_core::workflow::effects_retrigger(&all_effects) {
            run_automations(
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
