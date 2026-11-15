//! Sensitive content phase 2: the routes of the review. A compliance officer asks for scans, reads the findings, confirms or dismisses
//! them, and says which places may hold which category. A scan only writes findings. Only a person's decision, or a person's
//! designation of an authorized store, raises a label.

use crate::repository::AppendDecisionParams;
use crate::repository_content::{CategoryLabel, Decided, FindingFilter, StoreSpec};
use crate::service::admin::{decode_cursor, encode_cursor, require_platform_admin_or_compliance};
use crate::service::data_governance::flag_tables;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::{DataLabel, HostRouter};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

const PAGE: usize = 50;
const MAX_PAGE: usize = 200;
/// The most fields one designation labels.
const MAX_LABELLED: usize = 5_000;
const SCOPES: [&str; 4] = ["workspace", "app", "table", "field"];

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn reason_of(body: &Value) -> Result<String, ServiceError> {
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(bad("A reason between 1 and 500 characters is required"));
    }
    Ok(reason.to_string())
}

fn ledger(caller: &AuthUser, kind: DecisionType, rationale: String, payload: Value) -> AppendDecisionParams {
    AppendDecisionParams { principal: caller.eppn.clone(), organization_code: "DIV-ADMIN-CONSOLE".to_string(), app_slug: None, decision_type: kind, oscal_control_id: "RA-02".to_string(), rationale, payload }
}

/// `POST /admin/content-scans`: queues a scan of one table, or of every table.
pub fn scan(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    if body.get("all").and_then(Value::as_bool) == Some(true) {
        let n = crate::content_scan::enqueue_all(state, &caller.eppn);
        return Ok(json!({ "queued": n }));
    }
    let text = |k: &str| body.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let (Some(app), Some(table)) = (text("app_slug"), text("table_id")) else { return Err(bad("Name app_slug and table_id, or send all: true")) };
    let exists = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.resolve_by_slug(&app).is_some_and(|m| m.tables.iter().any(|t| t.id == table));
    if !exists {
        return Err(ServiceError::NotFound(format!("There is no table '{table}' in app '{app}'")));
    }
    let batch = body.get("batch_size").and_then(Value::as_i64).filter(|n| (1..=10_000).contains(n));
    let id = crate::content_scan::enqueue(&state.repository, &app, &table, batch, &caller.eppn).map_err(|e| ServiceError::QuotaExceeded(format!("The scan could not be queued: {e:?}")))?;
    Ok(json!({ "job_id": id }))
}

/// `GET /admin/content-findings`.
pub fn findings(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let pick = |k: &str| q.get(k).filter(|v| !v.is_empty()).cloned();
    if pick("status").is_some_and(|s| !["open", "confirmed", "dismissed"].contains(&s.as_str())) {
        return Err(bad("status is open, confirmed or dismissed"));
    }
    let start = q.get("cursor").filter(|c| !c.is_empty()).map(|c| decode_cursor(c)).transpose()?.unwrap_or(0);
    let limit = q.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(PAGE).clamp(1, MAX_PAGE);
    let (rows, total) = state.repository.list_findings(FindingFilter { status: pick("status"), category: pick("category"), app_slug: pick("app_slug") }, start as i64, limit as i64).map_err(storage)?;
    let next = (start + rows.len() < total as usize).then(|| encode_cursor(start + rows.len()));
    Ok(json!({ "findings": rows, "total": total, "next_cursor": next }))
}

/// `POST /admin/content-findings/{id}/decide`.
pub fn decide(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let confirm = match body.get("decision").and_then(Value::as_str) {
        Some("confirm") => true,
        Some("dismiss") => false,
        _ => return Err(bad("decision is confirm or dismiss")),
    };
    let reason = reason_of(body)?;
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Finding not found".to_string()))?;
    let finding = state.repository.get_finding(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("Finding not found".to_string()))?;
    let text = |k: &str| finding[k].as_str().unwrap_or("").to_string();
    let (app, table, field, category) = (text("app_slug"), text("table_id"), text("field"), text("category"));

    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let mut label = None;
    let mut flags = Vec::new();
    if confirm {
        let exists = manifests.iter().find(|m| m.slug == app).and_then(|m| m.tables.iter().find(|t| t.id == table)).is_some_and(|t| t.fields.iter().any(|f| f.name == field));
        if !exists {
            return Err(ServiceError::Conflict("The field is gone, so there is nothing to protect. Dismiss the finding".to_string()));
        }
        let mut after = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
        let key = (app.clone(), table.clone(), field.clone());
        let mut entry = after.get(&key).cloned().unwrap_or(DataLabel { ferpa_sensitive: false, note: reason.clone(), set_by: caller.eppn.clone(), categories: Vec::new() });
        if !entry.categories.contains(&category) {
            entry.categories.push(category.clone());
        }
        after.insert(key, entry);
        flags = flag_tables(state, &manifests, &BTreeSet::from([(app.clone(), table.clone())]), &after);
        label = Some((CategoryLabel { app_slug: app.clone(), table_id: table.clone(), field: field.clone() }, category.clone()));
    }
    // The ledger names the finding, the field and the detector. It holds no value, because the finding holds none.
    let entry = ledger(
        caller,
        DecisionType::ContentFindingDecided,
        reason.clone(),
        json!({ "finding": id, "decision": if confirm { "confirm" } else { "dismiss" }, "app": app, "table": table, "field": field, "detector": text("detector_id"), "category": category }),
    );
    match state.repository.decide_finding(id, confirm, &caller.eppn, &reason, label, flags, entry).map_err(storage)? {
        Decided::Missing => Err(ServiceError::NotFound("Finding not found".to_string())),
        Decided::Already(status) => Err(ServiceError::Conflict(format!("This finding was already decided: {status}"))),
        Decided::Done => {
            state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The decision was saved, but this server could not reload: {e}")))?;
            Ok(json!({ "id": id, "status": if confirm { "confirmed" } else { "dismissed" } }))
        }
    }
}

/// `GET /admin/authorized-stores`.
pub fn stores(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    Ok(json!({ "stores": state.repository.list_stores().map_err(storage)? }))
}

/// A store as the body names it, checked against the apps there are.
fn store_of(body: &Value, state: &SharedState) -> Result<(StoreSpec, Vec<CategoryLabel>), ServiceError> {
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let category = text("category");
    if !state.sensitivity_categories().iter().any(|c| c.id == category) {
        return Err(bad(format!("'{category}' is not a category of this institution")));
    }
    let scope = text("scope");
    if !SCOPES.contains(&scope.as_str()) {
        return Err(bad(format!("scope is one of {}", SCOPES.join(", "))));
    }
    let spec = StoreSpec {
        category,
        scope: scope.clone(),
        app_slug: if scope == "workspace" { String::new() } else { text("app_slug") },
        table_id: if matches!(scope.as_str(), "table" | "field") { text("table_id") } else { String::new() },
        field: if scope == "field" { text("field") } else { String::new() },
        workspace_id: if scope == "workspace" { text("workspace_id") } else { String::new() },
    };
    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let mut labels = Vec::new();
    for m in &manifests {
        let in_scope = match scope.as_str() {
            "workspace" => m.workspace_id.as_deref() == Some(spec.workspace_id.as_str()) && !spec.workspace_id.is_empty(),
            _ => m.slug == spec.app_slug && !spec.app_slug.is_empty(),
        };
        if !in_scope {
            continue;
        }
        for t in &m.tables {
            if matches!(scope.as_str(), "table" | "field") && t.id != spec.table_id {
                continue;
            }
            for f in &t.fields {
                if scope == "field" && f.name != spec.field {
                    continue;
                }
                labels.push(CategoryLabel { app_slug: m.slug.clone(), table_id: t.id.clone(), field: f.name.clone() });
            }
        }
    }
    if labels.is_empty() {
        return Err(bad(format!("Nothing in this {scope} exists, so there is nothing to designate")));
    }
    if labels.len() > MAX_LABELLED {
        return Err(bad(format!("This {scope} holds {} fields. Designate a smaller place", labels.len())));
    }
    Ok((spec, labels))
}

/// `PUT /admin/authorized-stores`: designates a place and labels the fields it covers with the category, at once.
pub fn put_store(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let reason = reason_of(body)?;
    let (spec, labels) = store_of(body, state)?;
    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let mut after = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
    for l in &labels {
        let e = after.entry((l.app_slug.clone(), l.table_id.clone(), l.field.clone())).or_insert(DataLabel { ferpa_sensitive: false, note: reason.clone(), set_by: caller.eppn.clone(), categories: Vec::new() });
        if !e.categories.contains(&spec.category) {
            e.categories.push(spec.category.clone());
        }
    }
    let touched: BTreeSet<(String, String)> = labels.iter().map(|l| (l.app_slug.clone(), l.table_id.clone())).collect();
    let flags = flag_tables(state, &manifests, &touched, &after);
    let n = labels.len();
    let entry = ledger(caller, DecisionType::DataLabelChanged, reason.clone(), json!({ "authorized_store": { "category": spec.category, "scope": spec.scope, "app": spec.app_slug, "table": spec.table_id, "field": spec.field, "workspace": spec.workspace_id }, "labelled": n }));
    state.repository.put_store(spec, labels, flags, &reason, &caller.eppn, entry).map_err(storage)?;
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The designation was saved, but this server could not reload: {e}")))?;
    Ok(json!({ "labelled": n }))
}

/// `DELETE /admin/authorized-stores`: removes the designation. The labels it made stay until a person removes them.
pub fn delete_store(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let reason = reason_of(body)?;
    let (spec, _) = store_of(body, state)?;
    let entry = ledger(caller, DecisionType::DataLabelChanged, reason, json!({ "authorized_store_removed": { "category": spec.category, "scope": spec.scope, "app": spec.app_slug, "table": spec.table_id, "field": spec.field, "workspace": spec.workspace_id } }));
    if !state.repository.delete_store(spec, entry).map_err(storage)? {
        return Err(ServiceError::NotFound("That place is not designated".to_string()));
    }
    Ok(json!({ "removed": true }))
}

/// `GET /admin/detector-stats`.
pub fn stats(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    Ok(json!({ "detectors": state.repository.detector_stats().map_err(storage)? }))
}

/// For the Overview: the findings waiting for a person, and the matches the scans expected inside authorized stores.
pub fn counts(state: &SharedState) -> Value {
    let (open, expected) = state.repository.content_counts().unwrap_or((0, 0));
    json!({ "open": open, "expected": expected })
}

/// A detector or a category was saved. Every detector that changed has its dismissals set aside, because the reason for a dismissal may
/// not hold for a detector that changed, and every table is scanned again.
pub fn settings_changed(state: &SharedState, key: &str, was: Option<&Value>, now: &Value, by: &str) {
    let ids = |v: Option<&Value>| -> std::collections::BTreeMap<String, Value> {
        v.and_then(Value::as_array).map(|a| a.iter().filter_map(|d| Some((d.get("id")?.as_str()?.to_string(), d.clone()))).collect()).unwrap_or_default()
    };
    let (before, after) = (ids(was), ids(Some(now)));
    let changed: Vec<String> = after.iter().filter(|(id, spec)| before.get(*id) != Some(*spec)).map(|(id, _)| id.clone()).collect();
    if key == "sensitivity.detectors" && !changed.is_empty() {
        let _ = state.repository.reopen_dismissed(&changed);
    }
    if !changed.is_empty() || (key == "sensitivity.categories" && before.keys().ne(after.keys())) {
        crate::content_scan::enqueue_all(state, by);
    }
}

/// `GET /admin/sensitivity-settings`: the categories and detectors the organization has, and which presets are switched on. A compliance
/// officer reads them here. Changing them is a Platform Admin's, through the settings.
pub fn sensitivity_settings(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let (default_categories, default_detectors) = scaffoldry_engine::sensitivity::default_settings();
    let read = |key: &str| state.settings.read().ok().and_then(|s| s.get(key).cloned());
    let categories = read("sensitivity.categories").unwrap_or_else(|| serde_json::to_value(&default_categories).unwrap_or(Value::Null));
    let detectors = read("sensitivity.detectors").unwrap_or_else(|| serde_json::to_value(&default_detectors).unwrap_or(Value::Null));
    let have: BTreeSet<String> = categories.as_array().into_iter().flatten().filter_map(|c| c["id"].as_str().map(str::to_string)).collect();
    let presets: Vec<Value> = scaffoldry_engine::sensitivity::preset_ids()
        .into_iter()
        .filter_map(|id| scaffoldry_engine::sensitivity::preset(&id)?.ok().map(|p| json!({ "id": id, "name": p.category.name, "switched_on": have.contains(&p.category.id) })))
        .collect();
    Ok(json!({ "categories": categories, "detectors": detectors, "presets": presets }))
}
