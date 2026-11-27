//! Data layer phase 3: who may read a row by its effective label, row tags, and the dataset floor.
//!
//! A row's effective label is the largest of the dataset's floor, the row's tags (`_tags` in its data, copied to a column by a
//! trigger), and the categories of its fields (`scaffoldry_engine::effective_label`). A category may carry a requirement in
//! settings. A reader meets it for a row when they hold one of its positions at a unit that its reach covers for the row. The row's
//! unit is its unit column, or the dataset's own unit when the column is empty. A Platform Admin skips every requirement, as they
//! skip a row rule. Nothing here widens anything: it is one more narrowing, beside the row rules and the reach of the grants.
//!
//! `gates_of` builds the gate for a reader and a table. `scope` carries it to every read: a list compiles it to SQL, a single row
//! is checked in Rust, and both come from the same gate. A reader who meets every requirement everywhere has no gate, and a
//! settings list with no requirement costs one read of settings.

use crate::service::{catalog, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_data::gate::{tags_of, Cover, Gate};
use scaffoldry_engine::sensitivity::{Category, Reach};
use scaffoldry_engine::{effective_label, effective_sensitivity, HostRouter};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The most tags a row carries, and the most categories a floor holds.
pub const MAX_TAGS: usize = 20;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------------------------------------------------

/// What the reader is covered for. `held` is each position the reader holds today, with the unit it is held at. A category the
/// reader meets everywhere is left out. `fixed` is the categories every row of the table carries.
///
/// Returns the covers every row needs, and for each other category with a requirement the cover a row needs when it is tagged.
pub fn needs(categories: &[Category], fixed: &BTreeSet<String>, held: &[(String, Uuid)]) -> (Vec<Cover>, Vec<(String, Cover)>) {
    let (mut always, mut by_tag) = (Vec::new(), Vec::new());
    for category in categories.iter().filter(|c| !c.requires.is_empty()) {
        let mut cover = Cover::default();
        let mut everywhere = false;
        for requirement in &category.requires {
            for (_, unit) in held.iter().filter(|(k, _)| *k == requirement.position) {
                match requirement.reach {
                    Reach::Anywhere => everywhere = true,
                    Reach::Unit => cover.exact.push(*unit),
                    Reach::UnitAndBelow => cover.below.push(*unit),
                }
            }
        }
        if everywhere {
            continue;
        }
        cover.exact.sort();
        cover.exact.dedup();
        cover.below.sort();
        cover.below.dedup();
        if fixed.contains(&category.id) {
            always.push(cover);
        } else {
            by_tag.push((category.id.clone(), cover));
        }
    }
    (always, by_tag)
}

/// True when settings hold any requirement. A server with none gates nothing, and does no other work to find out.
pub fn in_force(state: &SharedState) -> bool {
    state.sensitivity_categories().iter().any(|c| !c.requires.is_empty())
}

/// The positions a person holds today, each with the unit it is held at. A holding outside its dates is not counted. A holding that
/// cannot be read counts as none, so the gate closes.
fn held_by(eppn: &str, state: &SharedState) -> Vec<(String, Uuid)> {
    let now = chrono::Utc::now().timestamp();
    state
        .repository
        .holdings_of(eppn)
        .unwrap_or_default()
        .into_iter()
        .filter(|(h, _)| h.effective.is_none_or(|e| e <= now) && h.end.is_none_or(|e| e >= now))
        .filter_map(|(h, _)| h.position_key.map(|k| (k, h.unit)))
        .collect()
}

/// The floor of a dataset, read now. A table with no dataset row has none. A read that fails gives no floor to a reader who has
/// no gate, so a failure here is surfaced by the caller as a closed gate (`None`).
fn floor_of(app_slug: &str, table: &str, state: &SharedState) -> Option<Vec<String>> {
    let (app, table) = (app_slug.to_string(), table.to_string());
    state.repository.with_client(move |c| Ok(scaffoldry_data::labels::floor_of(c, &app, &table)?)).ok()
}

/// The gate of one reader on one table, or `None` when the reader has nothing to meet. A Platform Admin has none, and neither has a
/// guest, who reads only the record they made and never by a category.
fn gate_for(who: &AuthUser, app_slug: &str, table: &str, categories: &[Category], state: &SharedState) -> Option<Gate> {
    if who.affiliation == "central_admin" || who.affiliation == "guest" {
        return None;
    }
    let stored = crate::service::preview::stored_slug(app_slug);
    let (table_id, unit_field, unit_required) = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        let spec = engine.resolve_by_slug(app_slug).and_then(|m| m.tables.iter().find(|t| t.id == table || t.slug == table));
        (spec.map_or_else(|| table.to_string(), |t| t.id.clone()), spec.and_then(|t| t.unit_field.clone()), spec.is_some_and(|t| t.unit_required))
    };
    // A floor that cannot be read is a floor nobody is known to meet: the gate closes on every category with a requirement.
    let floor = floor_of(stored, &table_id, state);
    let fields = crate::service::scope::fields_of(app_slug, table, state);
    let fixed: BTreeSet<String> = {
        let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
        let sens: Vec<_> = fields.iter().map(|f| effective_sensitivity(f, labels.get(&(stored.to_string(), table_id.clone(), f.name.clone())), categories)).collect();
        effective_label(floor.as_deref().unwrap_or_default(), &[], &sens, categories).categories
    };
    let (mut always, by_tag) = needs(categories, &fixed, &held_by(&who.eppn, state));
    if floor.is_none() {
        always.push(Cover::default());
    }
    if always.is_empty() && by_tag.is_empty() {
        return None;
    }
    let dataset_unit = catalog::unit_of_app(state, stored);
    // Whether each cover reaches the dataset's own unit is one fact for the table. The statement carries it, so it tests no row for it.
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    let mark = |c: &mut Cover| c.at_dataset_unit = c.covers(dataset_unit, &|a, u| tree.contains(a, u));
    always.iter_mut().for_each(mark);
    let mut by_tag = by_tag;
    by_tag.iter_mut().for_each(|(_, c)| mark(c));
    Some(Gate { unit_field, dataset_unit, unit_required, always, by_tag })
}

/// The gates a reader of `table` must pass: their own, the live app's when this is a preview of a proposal, and, in view-as, the
/// viewer's. The row is read only when every one admits it.
pub fn gates_of(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> Vec<Gate> {
    let categories = state.sensitivity_categories();
    if !categories.iter().any(|c| !c.requires.is_empty()) {
        return Vec::new();
    }
    let mut out: Vec<Gate> = gate_for(caller, app_slug, table, &categories, state).into_iter().collect();
    if crate::service::preview::is_preview(app_slug) {
        out.extend(gate_for(caller, crate::service::preview::stored_slug(app_slug), table, &categories, state));
    }
    if let Some(viewer) = &caller.view_as_by {
        out.extend(gate_for(viewer, app_slug, table, &categories, state));
    }
    out
}

/// Whether every gate admits a row's data. No gate admits everything.
pub fn admits_with(gates: &[Gate], data: &Value, state: &SharedState) -> bool {
    if gates.is_empty() {
        return true;
    }
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    gates.iter().all(|g| g.admits(data, &|ancestor, unit| tree.contains(ancestor, unit)))
}

/// Whether the caller's gates admit this row's data, for a row stored or about to be.
pub fn admits(caller: &AuthUser, app_slug: &str, table: &str, data: &Value, state: &SharedState) -> bool {
    admits_with(&gates_of(caller, app_slug, table, state), data, state)
}

// ---------------------------------------------------------------------------------------------------------------------
// Row tags
// ---------------------------------------------------------------------------------------------------------------------

fn is_admin_or_compliance(caller: &AuthUser, state: &SharedState) -> bool {
    crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok()
}

fn is_platform_admin(caller: &AuthUser, state: &SharedState) -> bool {
    crate::service::admin::require_platform_admin(caller, state).is_ok()
}

/// Sets the `_tags` of a record about to be written. `stored` is the record as it is now, when there is one.
///
/// - A write that leaves `_tags` out keeps the stored tags. Only a write that names them changes them.
/// - Tags are a list of category ids that settings define, at most [`MAX_TAGS`]. They are kept sorted, without repeats.
/// - A tag is added by anyone who may write the row, and who still reads it afterwards: the caller's gate checks the new data.
/// - A tag is removed by a Platform Admin or a compliance officer when its category is protected or carries a requirement,
///   because removing it widens who reads the row. Any other tag may be removed by a writer.
/// - A guest sets none and removes none.
///
/// Whatever the dataset floor holds is not in the row's tags and cannot be removed here.
pub fn prepare_tags(caller: &AuthUser, stored: Option<&Value>, data: &mut Value, state: &SharedState) -> Result<(), ServiceError> {
    let stored_tags: Vec<String> = stored.map(tags_of).unwrap_or_default();
    let Some(map) = data.as_object_mut() else { return Ok(()) };
    let named = map.remove("_tags");
    let keep = |map: &mut serde_json::Map<String, Value>, tags: Vec<String>| {
        if !tags.is_empty() {
            map.insert("_tags".to_string(), json!(tags));
        }
    };
    if caller.affiliation == "guest" {
        keep(map, stored_tags);
        return Ok(());
    }
    let Some(named) = named else {
        keep(map, stored_tags);
        return Ok(());
    };
    let list = named.as_array().ok_or_else(|| bad("_tags is a list of category ids"))?;
    let mut tags: Vec<String> = list.iter().map(|t| t.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(|| bad("_tags is a list of category ids"))?;
    tags.sort();
    tags.dedup();
    if tags.len() > MAX_TAGS {
        return Err(bad(format!("A row carries at most {MAX_TAGS} tags")));
    }
    let categories = state.sensitivity_categories();
    if let Some(unknown) = tags.iter().find(|t| !categories.iter().any(|c| &c.id == *t)) {
        // A tag a row already carries may name a category settings dropped since. It stays, and it is never read as open.
        if !stored_tags.contains(unknown) {
            return Err(bad(format!("'{}' is not a category of this organization", unknown.chars().take(64).collect::<String>())));
        }
    }
    for removed in stored_tags.iter().filter(|t| !tags.contains(t)) {
        let protective = categories.iter().find(|c| &c.id == removed).is_none_or(|c| c.protected || !c.requires.is_empty());
        if protective && !is_admin_or_compliance(caller, state) {
            return Err(ServiceError::forbidden(format!("Removing the tag '{removed}' widens who reads this row. A Platform Admin or a compliance officer does that.")));
        }
    }
    if tags.is_empty() && stored_tags.is_empty() {
        return Ok(());
    }
    map.insert("_tags".to_string(), json!(tags));
    Ok(())
}

/// After an update: a row that now carries a protected label is flagged for Cedar. The flag only goes up here, as it did.
pub fn flag_after_update(app_slug: &str, table: &str, id: &str, data: &Value, was_flagged: bool, state: &SharedState) {
    if was_flagged {
        return;
    }
    let categories = state.sensitivity_categories();
    let floor = floor_of(crate::service::preview::stored_slug(app_slug), table, state).unwrap_or_default();
    let tags = tags_of(data);
    if effective_label(&floor, &tags, &[], &categories).protected {
        let _ = state.repository.raise_flag(id);
    }
}

/// The floor to submit a new record under.
pub fn floor_for(app_slug: &str, table: &str, state: &SharedState) -> Vec<String> {
    floor_of(crate::service::preview::stored_slug(app_slug), table, state).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------------
// The floor
// ---------------------------------------------------------------------------------------------------------------------

/// `PUT /admin/datasets/{id}/label-floor`. The body names `floor`, a list of category ids that every row of the dataset carries, and
/// a `reason`. It replaces the floor. A Platform Admin or a compliance officer may raise it. Lowering it, which widens who reads, is
/// a Platform Admin's, until the label-change process of `data-layer.md` "Changing a label" is built. The change, the flag Cedar
/// reads on each row, and the ledger entry commit together.
pub fn set_floor(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::admin::require_platform_admin_or_compliance(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(bad("A reason between 1 and 500 characters is required"));
    }
    let row = state.repository.catalog_row(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("Dataset not found".to_string()))?;
    if row.published_id.is_some() {
        return Err(bad("A published dataset holds no rows, so it has no floor"));
    }
    let list = body.get("floor").and_then(Value::as_array).ok_or_else(|| bad("floor is a list of category ids"))?;
    let mut floor: Vec<String> = list.iter().map(|t| t.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(|| bad("floor is a list of category ids"))?;
    floor.sort();
    floor.dedup();
    if floor.len() > MAX_TAGS {
        return Err(bad(format!("A floor holds at most {MAX_TAGS} categories")));
    }
    let categories = state.sensitivity_categories();
    if let Some(unknown) = floor.iter().find(|t| !categories.iter().any(|c| &c.id == *t)) {
        return Err(bad(format!("'{}' is not a category of this organization", unknown.chars().take(64).collect::<String>())));
    }
    let was = floor_of(&row.app_slug, &row.table_id, state).ok_or_else(|| ServiceError::StorageUnavailable("The floor cannot be read".to_string()))?;
    if was.iter().any(|c| !floor.contains(c)) && !is_platform_admin(caller, state) {
        return Err(ServiceError::forbidden("Lowering a floor widens who reads every row of the dataset. A Platform Admin does that, and a compliance officer may raise it."));
    }
    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let touched: BTreeSet<(String, String)> = std::iter::once((row.app_slug.clone(), row.table_id.clone())).collect();
    let labels = state.labels.read().map_err(|e| ServiceError::Internal(e.to_string()))?.clone();
    let flag = crate::service::data_governance::flag_tables(state, &manifests, &touched, &labels).into_iter().next();
    let keep: Vec<String> = categories.iter().filter(|c| c.protected).map(|c| c.id.clone()).collect();
    let names = flag.map(|f| f.names).unwrap_or_default();
    let (principal, why, app, dataset) = (caller.eppn.clone(), reason.to_string(), row.app_slug.clone(), row.id.clone());
    let new = floor.clone();
    let now = state
        .repository
        .set_label_floor(&row.id, floor, names, keep, move |was: &[String]| crate::repository::AppendDecisionParams {
            principal,
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: Some(app),
            decision_type: scaffoldry_core::ledger::DecisionType::DataLabelChanged,
            oscal_control_id: "AC-03".to_string(),
            rationale: why,
            payload: json!({ "dataset": dataset, "setting": "label_floor", "from": was, "to": new }),
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("Dataset not found".to_string()))?;
    Ok(json!({ "id": id, "dataset_id": row.id, "floor": now, "was": was }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scaffoldry_engine::sensitivity::Requirement;

    fn category(id: &str, requires: Vec<(&str, Reach)>) -> Category {
        Category {
            id: id.to_string(),
            name: id.to_string(),
            source: String::new(),
            protected: true,
            detectors: vec![],
            default_mask: Default::default(),
            requires: requires.into_iter().map(|(p, r)| Requirement { position: p.to_string(), reach: r }).collect(),
        }
    }

    fn fixed(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_category_with_no_requirement_asks_nothing() {
        let (always, by_tag) = needs(&[category("open", vec![])], &fixed(&["open"]), &[]);
        assert!(always.is_empty() && by_tag.is_empty());
    }

    #[test]
    fn a_category_every_row_carries_asks_the_cover_of_every_row_and_a_tagged_one_only_of_tagged_rows() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let cats = [category("phi", vec![("registrar", Reach::UnitAndBelow)]), category("hr", vec![("registrar", Reach::Unit), ("chair", Reach::Unit)])];
        let held = [("registrar".to_string(), a), ("registrar".to_string(), b), ("registrar".to_string(), a), ("dean".to_string(), b)];
        let (always, by_tag) = needs(&cats, &fixed(&["phi"]), &held);
        assert_eq!(always, vec![Cover { below: vec![a, b], ..Default::default() }]);
        assert_eq!(by_tag, vec![("hr".to_string(), Cover { exact: vec![a, b], ..Default::default() })]);
    }

    #[test]
    fn a_reader_who_holds_no_listed_position_is_covered_by_nobody() {
        let (always, by_tag) = needs(&[category("phi", vec![("registrar", Reach::UnitAndBelow)])], &fixed(&["phi"]), &[("chair".to_string(), Uuid::from_u128(1))]);
        assert_eq!(always, vec![Cover::default()]);
        assert!(by_tag.is_empty());
    }

    #[test]
    fn a_reach_of_anywhere_is_met_by_holding_the_position_at_any_unit_and_needs_no_cover() {
        let cats = [category("phi", vec![("registrar", Reach::Anywhere)])];
        let (always, by_tag) = needs(&cats, &fixed(&["phi"]), &[("registrar".to_string(), Uuid::from_u128(9))]);
        assert!(always.is_empty() && by_tag.is_empty());
        let (always, _) = needs(&cats, &fixed(&["phi"]), &[]);
        assert_eq!(always, vec![Cover::default()], "a reader with no holding is not covered");
    }
}
