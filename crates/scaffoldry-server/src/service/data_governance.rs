//! Admin console phase 4: data governance.
//!
//! A compliance officer raises a field's sensitivity at once with a label, which can only add protection and needs no proposal.
//! These routes list every field of every app with where it is used, set labels in batches, and classify published datasets.
//! They are for a Platform Admin or a compliance officer.

use crate::repository_labels::{FlagTable, LabelChange};
use crate::repository::AppendDecisionParams;
use crate::service::admin::{decode_cursor, encode_cursor, require_platform_admin_or_compliance};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::standards::ceds::CedsElement;
use scaffoldry_engine::{effective_ferpa_sensitive, AppManifest, DataLabel, FieldSpec, LabelSet, PageKind};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The most changes one request holds.
pub const MAX_CHANGES: usize = 200;
/// The levels a published dataset may have.
pub const LEVELS: [&str; 4] = ["Public", "Directory", "Internal", "Restricted / FERPA"];
const PAGE: usize = 100;
const MAX_PAGE: usize = 500;

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

/// The strings under the named keys, anywhere in a JSON value. A view or a rule names a field by one of a few keys, and this finds
/// them without caring how deep a filter group nests.
fn named(v: &Value, keys: &[&str], out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if keys.contains(&k.as_str()) {
                    match x {
                        Value::String(s) => {
                            out.insert(s.clone());
                        }
                        Value::Array(items) => {
                            for i in items {
                                if let Some(s) = i.as_str() {
                                    out.insert(s.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                named(x, keys, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| named(i, keys, out)),
        _ => {}
    }
}

/// The fields a view lists, sorts, filters, groups by or shows on a form.
fn view_fields(config: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    named(config, &["column_order", "group_by", "field_name", "kanban_column_field", "calendar_date_field"], &mut out);
    for f in config.get("form_fields").and_then(Value::as_array).into_iter().flatten() {
        if let Some(n) = f.get("name").and_then(Value::as_str) {
            out.insert(n.to_string());
        }
    }
    out
}

/// The fields a rule names: in its predicates, in its steps' conditions, and in what it sets or writes.
fn rule_fields(rule: &scaffoldry_core::AutomationRule) -> BTreeSet<String> {
    let json = serde_json::to_value(rule).unwrap_or(Value::Null);
    let mut out = BTreeSet::new();
    named(&json, &["field_name", "field"], &mut out);
    // `SetFields` and `CreateRecord` and `UpdateRecord` hold their fields as `[name, value]` pairs.
    fn pairs(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == "fields" {
                        for p in x.as_array().into_iter().flatten() {
                            if let Some(n) = p.as_array().and_then(|a| a.first()).and_then(Value::as_str) {
                                out.insert(n.to_string());
                            }
                        }
                    }
                    pairs(x, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| pairs(i, out)),
            _ => {}
        }
    }
    pairs(&json, &mut out);
    out
}

fn mentions(expression: &str, name: &str) -> bool {
    expression.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|t| t == name)
}

/// Where a field is used: views that list, sort or filter it, other fields whose formula names it, custom page grants that read or write
/// it, and rules whose predicates or effects name it. A rule names fields without a table, so it counts for each table of its app that
/// has a field of that name.
fn used_in(state: &SharedState, m: &AppManifest, table: &str, field: &str) -> (usize, Value) {
    let views = state
        .app_views
        .read()
        .map(|all| all.get(&m.slug).map(|vs| vs.iter().filter(|v| v.table_id.as_deref() == Some(table) && view_fields(&v.config).contains(field)).count()).unwrap_or(0))
        .unwrap_or(0);
    let formulas = m
        .tables
        .iter()
        .find(|t| t.id == table)
        .map(|t| t.fields.iter().filter(|f| f.name != field && f.formula_expression.as_deref().is_some_and(|e| mentions(e, field))).count())
        .unwrap_or(0);
    let pages = m
        .pages
        .iter()
        .filter(|p| matches!(&p.kind, PageKind::Custom { grants, .. } if grants.iter().any(|g| g.table_id == table && (g.read.iter().any(|f| f == field) || g.write.iter().any(|f| f == field)))))
        .count();
    let rules = state
        .automations
        .read()
        .map(|all| all.get(&m.slug).map(|rs| rs.iter().filter(|r| rule_fields(r).contains(field)).count()).unwrap_or(0))
        .unwrap_or(0);
    (views + formulas + pages + rules, json!({ "views": views, "formulas": formulas, "pages": pages, "rules": rules }))
}

fn label_for<'a>(labels: &'a LabelSet, app: &str, table: &str, field: &str) -> Option<&'a DataLabel> {
    labels.get(&(app.to_string(), table.to_string(), field.to_string()))
}

/// Whether the manifest itself flags the field. With no label and no categories, the one function answers from the flag alone.
fn manifest_flag(f: &FieldSpec) -> bool {
    effective_ferpa_sensitive(f, None, &[]).protected
}

/// Whether the label raises the field. A blank field has no flag of its own, so the answer is the label's.
fn overlay_flag(label: Option<&DataLabel>) -> bool {
    let blank = FieldSpec::simple("x", "x", scaffoldry_engine::FieldType::Text, false, false);
    effective_ferpa_sensitive(&blank, label, &[]).protected
}

fn type_name(f: &FieldSpec) -> String {
    match serde_json::to_value(f.field_type) {
        Ok(Value::String(s)) => s,
        Ok(Value::Object(m)) => m.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// `GET /admin/fields`: one row for each field of every app.
pub fn fields(caller: &AuthUser, q: &std::collections::HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
    let mut manifests = {
        use scaffoldry_engine::HostRouter;
        let _ = std::marker::PhantomData::<&dyn HostRouter>;
        state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests()
    };
    manifests.sort_by(|a, b| a.slug.cmp(&b.slug));
    let flag = |key: &str| q.get(key).map(|v| v == "true");
    let (sensitive, suggested) = (flag("sensitive"), flag("suggested"));
    let search = q.get("search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let mut rows = Vec::new();
    for m in &manifests {
        if q.get("app").is_some_and(|a| !a.is_empty() && *a != m.slug) {
            continue;
        }
        for t in &m.tables {
            if q.get("table").is_some_and(|x| !x.is_empty() && *x != t.id) {
                continue;
            }
            for f in &t.fields {
                let label = label_for(&labels, &m.slug, &t.id, &f.name);
                let effective = effective_ferpa_sensitive(f, label, &categories).protected;
                let ceds = m.ceds_mappings.get(&f.name).cloned();
                let protected_by_standard = ceds.as_deref().and_then(CedsElement::from_code).is_some_and(|e| e.is_ferpa_sensitive());
                let is_suggested = protected_by_standard && !effective;
                if sensitive.is_some_and(|s| s != effective) || suggested.is_some_and(|s| s != is_suggested) {
                    continue;
                }
                let ty = type_name(f);
                if q.get("type").is_some_and(|x| !x.is_empty() && !x.eq_ignore_ascii_case(&ty)) {
                    continue;
                }
                if let Some(s) = &search {
                    if ![&m.slug, &m.title, &t.id, &f.name, &f.label].iter().any(|x| x.to_lowercase().contains(s.as_str())) {
                        continue;
                    }
                }
                let (count, parts) = used_in(state, m, &t.id, &f.name);
                rows.push(json!({
                    "app": m.slug, "app_title": m.title, "table": t.id, "field": f.name, "label": f.label, "type": ty,
                    "manifest_flag": manifest_flag(f), "overlay_flag": overlay_flag(label),
                    "overlay_note": label.map(|l| l.note.clone()), "effective": effective,
                    "ceds_code": ceds, "suggested": is_suggested, "used_in": count, "used_in_parts": parts,
                }));
            }
        }
    }
    let start = q.get("cursor").filter(|c| !c.is_empty()).map(|c| decode_cursor(c)).transpose()?.unwrap_or(0);
    let limit = q.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(PAGE).clamp(1, MAX_PAGE);
    let total = rows.len();
    let page: Vec<Value> = rows.into_iter().skip(start).take(limit).collect();
    let next = (start + page.len() < total).then(|| encode_cursor(start + page.len()));
    Ok(json!({ "fields": page, "total": total, "next_cursor": next }))
}

/// For each touched table, the names of its effective sensitive fields under the labels as they will be. The records' stored flags are set
/// again from these.
pub(crate) fn flag_tables(state: &SharedState, manifests: &[AppManifest], touched: &BTreeSet<(String, String)>, after: &LabelSet) -> Vec<FlagTable> {
    let categories = state.sensitivity_categories();
    touched
        .iter()
        .filter_map(|(app, table)| {
            let t = manifests.iter().find(|m| &m.slug == app)?.tables.iter().find(|t| &t.id == table)?;
            let names = t.fields.iter().filter(|f| effective_ferpa_sensitive(f, label_for(after, app, table, &f.name), &categories).protected).map(|f| f.name.clone()).collect();
            Some(FlagTable { app_slug: app.clone(), table_id: table.clone(), names })
        })
        .collect()
}

/// `PUT /admin/labels`: sets or removes labels in a batch. Every field must exist in its manifest, or the whole request is refused. The
/// labels, the records' flags, the cache counter and one ledger entry commit together.
pub fn put_labels(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let reason = reason_of(body)?;
    let list = body.get("changes").and_then(Value::as_array).ok_or_else(|| bad("Send `changes` as a list"))?;
    if list.is_empty() || list.len() > MAX_CHANGES {
        return Err(bad(format!("Send 1 to {MAX_CHANGES} changes")));
    }
    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let mut changes: Vec<LabelChange> = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let text = |k: &str| c.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        let (Some(app), Some(table), Some(field)) = (text("app_slug"), text("table_id"), text("field")) else {
            return Err(bad(format!("Change {} needs app_slug, table_id and field", i + 1)));
        };
        let sensitive = match c.get("ferpa_sensitive") {
            Some(Value::Bool(b)) => Some(*b),
            Some(Value::Null) => None,
            _ => return Err(bad(format!("Change {} needs ferpa_sensitive as true, false or null", i + 1))),
        };
        let exists = manifests.iter().find(|m| m.slug == app).and_then(|m| m.tables.iter().find(|t| t.id == table)).is_some_and(|t| t.fields.iter().any(|f| f.name == field));
        if !exists {
            return Err(bad(format!("'{field}' is not a field of table '{table}' in app '{app}'. Nothing was changed")));
        }
        if changes.iter().any(|x| x.app_slug == app && x.table_id == table && x.field == field) {
            return Err(bad(format!("'{field}' of '{table}' is named twice")));
        }
        changes.push(LabelChange { app_slug: app, table_id: table, field, sensitive });
    }

    // The labels as they will be, so each touched table's effective sensitive fields can be worked out before anything is written.
    let mut after = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
    for ch in &changes {
        let key = (ch.app_slug.clone(), ch.table_id.clone(), ch.field.clone());
        match ch.sensitive {
            Some(v) => {
                // Setting the flag leaves the categories a label already carries.
                let categories = after.get(&key).map(|l| l.categories.clone()).unwrap_or_default();
                after.insert(key, DataLabel { ferpa_sensitive: v, note: reason.clone(), set_by: caller.eppn.clone(), categories });
            }
            None => {
                after.remove(&key);
            }
        }
    }
    let touched: BTreeSet<(String, String)> = changes.iter().map(|c| (c.app_slug.clone(), c.table_id.clone())).collect();
    let flags = flag_tables(state, &manifests, &touched, &after);

    let summary: Vec<Value> = changes.iter().map(|c| json!({ "app": c.app_slug, "table": c.table_id, "field": c.field, "ferpa_sensitive": c.sensitive })).collect();
    let ledger = AppendDecisionParams {
        principal: caller.eppn.clone(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: None,
        decision_type: DecisionType::DataLabelChanged,
        oscal_control_id: "RA-02".to_string(),
        rationale: reason.clone(),
        payload: json!({ "changes": summary }),
    };
    let n = changes.len();
    state.repository.apply_labels(changes, &reason, &caller.eppn, flags, ledger).map_err(storage)?;
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The labels were saved, but this server could not reload: {e}")))?;
    Ok(json!({ "changed": n }))
}

/// `GET /admin/datasets`.
pub fn datasets(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let all = state.repository.published_datasets().map_err(storage)?;
    let rows: Vec<Value> = all
        .iter()
        .map(|d| json!({ "id": d.id, "name": d.name, "department": d.department, "sensitivity_level": d.sensitivity_level, "record_count": d.record_count }))
        .collect();
    Ok(json!({ "datasets": rows, "levels": LEVELS }))
}

/// `PUT /admin/datasets/{id}/classification`.
pub fn classify(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let level = body.get("sensitivity_level").and_then(Value::as_str).filter(|l| LEVELS.contains(l)).ok_or_else(|| bad(format!("sensitivity_level is one of {}", LEVELS.join(", "))))?.to_string();
    let reason = reason_of(body)?;
    let (principal, dataset, to, why) = (caller.eppn.clone(), id.to_string(), level.clone(), reason);
    let was = state
        .repository
        .classify_dataset(id, &level, move |was| AppendDecisionParams {
            principal,
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::DataLabelChanged,
            oscal_control_id: "RA-02".to_string(),
            rationale: why,
            payload: json!({ "dataset": dataset, "from": was, "to": to }),
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("Dataset not found".to_string()))?;
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The level was saved, but this server could not reload: {e}")))?;
    Ok(json!({ "id": id, "sensitivity_level": level, "was": was }))
}
