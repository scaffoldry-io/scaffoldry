//! Views (views phase 1).
//!
//! A view is presentation and lives apart from the governed manifest, in `app_views`. It cannot grant
//! access, because every read still goes through the row scope and the field visibility rules. Who may
//! change a view follows its ownership: a personal view is its owner's, a collaborative view needs an
//! editor, a locked view needs an admin. Anyone who can read the app may use a collaborative or locked one.
//!
//! Indexes follow views. A save that sorts or filters on a field outside the table's indexed set starts
//! that index in the same transaction, and a table may index at most `MAX_INDEXED_FIELDS` fields.

use crate::repository_views::{materialize, IndexChange, NewView, Saved, StoredView, KINDS, OWNERSHIPS};
use crate::service::access::{authorize_app, AppAction};
use crate::service::{records, scope, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::{indexed_fields, validate_manifest, AppManifest, FieldSpec, HostRouter, MAX_INDEXED_FIELDS};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The largest config a view may hold.
const MAX_CONFIG_BYTES: usize = 64 * 1024;
const MAX_NAME: usize = 120;

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    Ok(&*state.repository)
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn not_found() -> ServiceError {
    ServiceError::NotFound("View not found".to_string())
}

fn is_owner(v: &StoredView, caller: &AuthUser) -> bool {
    v.owner.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&caller.eppn))
}

/// A personal view is its owner's alone. Every other view is for anyone who can read the app.
fn visible(v: &StoredView, caller: &AuthUser) -> bool {
    v.ownership != "personal" || is_owner(v, caller)
}

/// May the caller create or change a view of this ownership? A personal view needs read access and is the
/// owner's. A collaborative view needs an editor. A locked view needs an admin.
fn authorize_change(caller: &AuthUser, app_slug: &str, ownership: &str, owner_ok: bool, state: &SharedState) -> Result<(), ServiceError> {
    match ownership {
        "personal" => {
            authorize_app(caller, app_slug, AppAction::Read, state)?;
            if owner_ok { Ok(()) } else { Err(not_found()) }
        }
        "locked" => authorize_app(caller, app_slug, AppAction::Manage, state),
        _ => authorize_app(caller, app_slug, AppAction::WriteRecords, state),
    }
}

fn body_of(v: &StoredView, index_state: Option<&str>) -> Value {
    let mut out = serde_json::to_value(v).unwrap_or(Value::Null);
    if let Some(state) = index_state {
        out["index_state"] = json!(state);
    }
    out
}

/// Every field a filter group names, at any depth.
fn filter_fields(filter: &Value, out: &mut BTreeSet<String>) {
    if let Some(clauses) = filter.get("clauses").and_then(Value::as_array) {
        out.extend(clauses.iter().filter_map(|c| c.get("field_name").and_then(Value::as_str)).map(str::to_string));
    }
    if let Some(groups) = filter.get("groups").and_then(Value::as_array) {
        for g in groups {
            filter_fields(g, out);
        }
    }
}

/// The fields a config sorts, filters, groups or lays out by. Footers and color rules are left out on purpose:
/// a footer or a rule on a field the caller cannot see is dropped when the view runs, and does not stop it.
fn used_fields(config: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(rules) = config.get("sort_rules").and_then(Value::as_array) {
        out.extend(rules.iter().filter_map(|r| r.get("field_name").and_then(Value::as_str)).map(str::to_string));
    }
    if let Some(filter) = config.get("filters") {
        filter_fields(filter, &mut out);
    }
    if let Some(levels) = config.get("group_by").and_then(Value::as_array) {
        out.extend(levels.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if let Some(aggs) = config.get("aggregations").and_then(Value::as_array) {
        out.extend(aggs.iter().filter_map(|a| a.get("field").and_then(Value::as_str)).map(str::to_string));
    }
    for key in ["kanban_column_field", "group_by_field", "calendar_date_field"] {
        if let Some(f) = config.get(key).and_then(Value::as_str) {
            out.insert(f.to_string());
        }
    }
    out
}

/// A summary view groups by one or two fields and computes at least one measure for each group.
fn valid_summary(config: &Value) -> Result<(), ServiceError> {
    let groups = config.get("group_by").and_then(Value::as_array).map_or(0, Vec::len);
    if groups == 0 || groups > scaffoldry_engine::MAX_SUMMARY_GROUPS {
        return Err(ServiceError::BadRequest(format!("A summary groups by 1 to {} fields", scaffoldry_engine::MAX_SUMMARY_GROUPS)));
    }
    if config.get("aggregations").and_then(Value::as_array).is_none_or(Vec::is_empty) {
        return Err(ServiceError::BadRequest("A summary needs at least one aggregation".to_string()));
    }
    Ok(())
}

/// A view that names a field the caller cannot see is refused, the way a sort or filter on it is. Using a
/// shared view later is checked again on every read, so this only keeps a person from saving what they could
/// not run.
fn check_visible(caller: &AuthUser, app_slug: &str, table_id: &str, config: &Value, state: &SharedState) -> Result<(), ServiceError> {
    let (fields, table) = records::table_fields_lenient(app_slug, table_id, state);
    let hidden = records::hidden_fields_for(caller, app_slug, &table, &fields, state);
    for name in used_fields(config) {
        if hidden.contains(&name) {
            let rule = scope::field_visibility_of(caller, app_slug, &table, state).hidden.into_iter().find(|d| d.field == name);
            return Err(ServiceError::FieldHidden {
                message: format!("You cannot use '{name}' in a view because you cannot see it."),
                reason: rule.as_ref().map_or_else(|| format!("'{name}' is hidden from you."), |d| d.reason.clone()),
                remedy: rule.map_or_else(|| "Choose a field you can see.".to_string(), |d| d.remedy),
            });
        }
    }
    Ok(())
}

/// A view over a field the caller cannot see is refused when it is run, the way a sort or filter on that field
/// is. Otherwise it would still show the field's name and options.
pub fn check_runnable(caller: &AuthUser, app_slug: &str, view: &scaffoldry_engine::AppView, state: &SharedState) -> Result<(), ServiceError> {
    match view.table_id.as_deref() {
        Some(table) => check_visible(caller, app_slug, table, &serde_json::to_value(view).unwrap_or(Value::Null), state),
        None => Ok(()),
    }
}

fn valid_config(config: &Value) -> Result<(), ServiceError> {
    if !config.is_object() {
        return Err(ServiceError::BadRequest("config must be an object".to_string()));
    }
    if config.to_string().len() > MAX_CONFIG_BYTES {
        return Err(ServiceError::TooLarge(format!("A view's config may hold {} KB at most", MAX_CONFIG_BYTES / 1024)));
    }
    Ok(())
}

fn valid_name(name: &str) -> Result<String, ServiceError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ServiceError::BadRequest(format!("A view's name is 1 to {MAX_NAME} characters")));
    }
    Ok(name.to_string())
}

/// The id of the table, from its id or its slug.
fn canonical_table(manifest: &AppManifest, table: &str) -> Result<String, ServiceError> {
    manifest
        .tables
        .iter()
        .find(|t| t.id == table || t.slug == table)
        .map(|t| t.id.clone())
        .ok_or_else(|| ServiceError::BadRequest(format!("'{table}' is not a table of this app")))
}

fn runtime_manifest(state: &SharedState, app_slug: &str) -> Result<AppManifest, ServiceError> {
    state
        .engine
        .read()
        .map_err(internal)?
        .resolve_by_slug(app_slug)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))
}

enum Change {
    Upsert(Box<StoredView>),
    Remove(Uuid),
}

struct Plan {
    index: Option<IndexChange>,
    /// The fields the saved view needs indexed, to ask whether they are still being built.
    used: Vec<String>,
    table_id: Option<String>,
}

/// What a change does to the index, and whether the result is a valid view. Nothing is written.
fn plan(state: &SharedState, app_slug: &str, change: &Change) -> Result<Plan, ServiceError> {
    let base = runtime_manifest(state, app_slug)?;
    let mut next = base.clone();
    let (id, table_id, config) = match change {
        Change::Upsert(v) => (v.id, v.table_id.clone(), Some(&v.config)),
        Change::Remove(id) => (*id, None, None),
    };
    let table_id = match change {
        Change::Remove(_) => base.views.iter().find(|v| v.id == id.to_string()).and_then(|v| v.table_id.clone()),
        _ => table_id,
    };
    next.views.retain(|v| v.id != id.to_string());
    if let Change::Upsert(v) = change {
        let view = materialize(v).ok_or_else(|| ServiceError::BadRequest("config is not a view".to_string()))?;
        next.views.push(view);
        // Only what concerns this view is reported. The rest of the manifest was valid when it was approved.
        if let Err(problems) = validate_manifest(&next) {
            let mine: Vec<String> = problems.into_iter().filter(|p| p.contains(&format!("view '{id}'")) || p.starts_with("an app may have")).collect();
            if !mine.is_empty() {
                return Err(ServiceError::BadRequest(mine.join("; ")));
            }
        }
    }

    let Some(table) = table_id.as_deref().and_then(|t| base.tables.iter().find(|x| x.id == t)) else {
        return Ok(Plan { index: None, used: Vec::new(), table_id });
    };
    let before = indexed_fields(&base, &table.id);
    let after = indexed_fields(&next, &table.id);
    if after.len() > MAX_INDEXED_FIELDS {
        let in_a_view: BTreeSet<String> = next.views.iter().filter(|v| v.table_id.as_deref() == Some(table.id.as_str())).flat_map(|v| used_fields(&serde_json::to_value(v).unwrap_or(Value::Null))).collect();
        let unused: Vec<String> = table.fields.iter().filter(|f| f.indexed && !in_a_view.contains(&f.name)).map(|f| f.name.clone()).collect();
        return Err(ServiceError::TooManyIndexes {
            message: format!("A table may index {MAX_INDEXED_FIELDS} fields, and '{}' would index {}.", table.id, after.len()),
            unused,
        });
    }
    let joined: Vec<FieldSpec> = table.fields.iter().filter(|f| after.contains(&f.name) && !before.contains(&f.name)).cloned().collect();
    let left: Vec<String> = before.difference(&after).cloned().collect();
    let used: Vec<String> = config.map(|c| used_fields(c).into_iter().filter(|f| after.contains(f)).collect()).unwrap_or_default();
    let index = (!joined.is_empty() || !left.is_empty()).then(|| IndexChange { table_id: table.id.clone(), left, joined });
    Ok(Plan { index, used, table_id: Some(table.id.clone()) })
}

/// Puts a saved or removed view into what runs: the cache, and the engine's copy of the manifest.
fn apply(state: &SharedState, app_slug: &str, id: Uuid, saved: Option<&StoredView>) -> Result<(), ServiceError> {
    {
        let mut cache = state.app_views.write().map_err(internal)?;
        let list = cache.entry(app_slug.to_string()).or_default();
        list.retain(|v| v.id != id);
        if let Some(v) = saved {
            list.push(v.clone());
        }
    }
    let mut engine = state.engine.write().map_err(internal)?;
    let Some(mut manifest) = engine.resolve_by_slug(app_slug).cloned() else { return Ok(()) };
    manifest.views.retain(|v| v.id != id.to_string());
    if let Some(view) = saved.and_then(materialize) {
        manifest.views.push(view);
    }
    engine.register_manifest(manifest).map_err(internal)
}

/// Whether a field the saved view needs is still being built.
fn index_state(state: &SharedState, app_slug: &str, plan: &Plan) -> Result<&'static str, ServiceError> {
    let Some(table) = plan.table_id.as_deref().filter(|_| !plan.used.is_empty()) else { return Ok("ready") };
    let building = repo(state)?.index_building_percent(app_slug, table, &plan.used).map_err(internal)?;
    Ok(if building.is_some() { "building" } else { "ready" })
}

/// `GET /apps/{slug}/views`: the caller's personal views and every collaborative and locked one.
pub fn list(caller: &AuthUser, app_slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let views: Vec<StoredView> = repo(state)?.list_app_views(app_slug).map_err(internal)?.into_iter().filter(|v| visible(v, caller)).collect();
    Ok(json!({ "views": views }))
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub ownership: Option<String>,
    #[serde(default)]
    pub table_id: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// `POST /apps/{slug}/views`.
pub fn create(caller: &AuthUser, app_slug: &str, body: CreateBody, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    let name = valid_name(&body.name)?;
    if !KINDS.contains(&body.kind.as_str()) {
        return Err(ServiceError::BadRequest(format!("kind must be one of {}", KINDS.join(", "))));
    }
    let ownership = body.ownership.unwrap_or_else(|| "collaborative".to_string());
    if !OWNERSHIPS.contains(&ownership.as_str()) {
        return Err(ServiceError::BadRequest(format!("ownership must be one of {}", OWNERSHIPS.join(", "))));
    }
    let config = body.config.unwrap_or_else(|| json!({}));
    valid_config(&config)?;
    if body.kind == "summary" {
        valid_summary(&config)?;
    }
    authorize_change(caller, app_slug, &ownership, true, state)?;
    let table_id = body.table_id.as_deref().map(|t| canonical_table(&runtime_manifest(state, app_slug)?, t)).transpose()?;
    if let Some(table) = &table_id {
        check_visible(caller, app_slug, table, &config, state)?;
    }

    let new = NewView {
        id: Uuid::new_v4(),
        app_slug: app_slug.to_string(),
        table_id,
        name,
        kind: body.kind,
        ownership: ownership.clone(),
        owner: (ownership == "personal").then(|| caller.eppn.clone()),
        config,
        actor: caller.eppn.clone(),
    };
    let candidate = StoredView {
        id: new.id,
        app_slug: new.app_slug.clone(),
        table_id: new.table_id.clone(),
        name: new.name.clone(),
        kind: new.kind.clone(),
        ownership: new.ownership.clone(),
        owner: new.owner.clone(),
        config: new.config.clone(),
        version: 1,
        created_by: new.actor.clone(),
        created_at: String::new(),
        updated_by: new.actor.clone(),
        updated_at: String::new(),
    };
    let plan = plan(state, app_slug, &Change::Upsert(Box::new(candidate)))?;
    let saved = repo(state)?.create_app_view(new, plan.index.clone()).map_err(internal)?;
    apply(state, app_slug, saved.id, Some(&saved))?;
    Ok(body_of(&saved, Some(index_state(state, app_slug, &plan)?)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    #[serde(default)]
    pub name: Option<String>,
    pub config: Value,
    pub version: i32,
}

fn existing(caller: &AuthUser, app_slug: &str, id: Uuid, state: &SharedState) -> Result<StoredView, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state)?;
    repo(state)?.get_app_view(app_slug, id).map_err(internal)?.filter(|v| visible(v, caller)).ok_or_else(not_found)
}

/// `PUT /apps/{slug}/views/{id}`. A stale version is a conflict.
pub fn update(caller: &AuthUser, app_slug: &str, id: Uuid, body: UpdateBody, state: &SharedState) -> Result<Value, ServiceError> {
    let current = existing(caller, app_slug, id, state)?;
    authorize_change(caller, app_slug, &current.ownership, is_owner(&current, caller), state)?;
    let name = valid_name(body.name.as_deref().unwrap_or(&current.name))?;
    valid_config(&body.config)?;
    if current.kind == "summary" {
        valid_summary(&body.config)?;
    }
    if let Some(table) = &current.table_id {
        check_visible(caller, app_slug, table, &body.config, state)?;
    }
    let mut candidate = current.clone();
    candidate.name = name.clone();
    candidate.config = body.config.clone();
    let plan = plan(state, app_slug, &Change::Upsert(Box::new(candidate)))?;
    match repo(state)?.update_app_view(app_slug, id, name, body.config, body.version, &caller.eppn, plan.index.clone()).map_err(internal)? {
        Saved::Done(saved) => {
            apply(state, app_slug, saved.id, Some(&saved))?;
            Ok(body_of(&saved, Some(index_state(state, app_slug, &plan)?)))
        }
        Saved::Stale(version) => Err(ServiceError::StaleVersion { message: "Someone else changed this view first. Reload it and try again.".to_string(), version }),
        Saved::Missing => Err(not_found()),
    }
}

/// `DELETE /apps/{slug}/views/{id}`: a soft delete. The index of a field only this view needed goes with it.
pub fn delete(caller: &AuthUser, app_slug: &str, id: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    let current = existing(caller, app_slug, id, state)?;
    authorize_change(caller, app_slug, &current.ownership, is_owner(&current, caller), state)?;
    let plan = plan(state, app_slug, &Change::Remove(id))?;
    if !repo(state)?.delete_app_view(app_slug, id, &caller.eppn, plan.index.clone()).map_err(internal)? {
        return Err(not_found());
    }
    apply(state, app_slug, id, None)?;
    Ok(json!({ "deleted": true, "id": id }))
}

/// `POST /apps/{slug}/views/{id}/duplicate`: a personal copy for the caller.
pub fn duplicate(caller: &AuthUser, app_slug: &str, id: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    let current = existing(caller, app_slug, id, state)?;
    let name: String = format!("{} (copy)", current.name).chars().take(MAX_NAME).collect();
    create(
        caller,
        app_slug,
        CreateBody { name, kind: current.kind, ownership: Some("personal".to_string()), table_id: current.table_id, config: Some(current.config) },
        state,
    )
}

// ---------------------------------------------------------------------------------------------
// Running a view (views phase 2)
// ---------------------------------------------------------------------------------------------

/// What a run asks for. A grouped view answers with the groups of the next level until `group_path` names a
/// group of the last level, and then it pages that group's rows.
#[derive(Debug, Default, Deserialize)]
pub struct RunQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// The key of the group chosen at each level so far. An empty string is the group with no value.
    #[serde(default)]
    pub group_path: Vec<String>,
    /// Open the view at this row instead of at a cursor. Needs a sort on an indexed field.
    #[serde(default)]
    pub offset: Option<usize>,
    /// The last rows of the view, in order.
    #[serde(default)]
    pub last: bool,
    /// `exact` asks for the row count, which says `at_least` when it runs out of time.
    #[serde(default)]
    pub count: Option<String>,
}

/// The view's filter, narrowed to one group.
fn narrowed(base: Option<scaffoldry_engine::CompoundFilter>, levels: &[String], path: &[String]) -> Option<scaffoldry_engine::CompoundFilter> {
    use scaffoldry_engine::{CompoundFilter, FilterClause, FilterConjunction, FilterOperator};
    if path.is_empty() {
        return base;
    }
    let clauses: Vec<FilterClause> = path
        .iter()
        .zip(levels)
        .enumerate()
        .map(|(i, (key, field))| FilterClause {
            id: format!("group-{i}"),
            field_name: field.clone(),
            operator: if key.is_empty() { FilterOperator::IsEmpty } else { FilterOperator::Equals },
            value: key.clone(),
        })
        .collect();
    Some(match base {
        None => CompoundFilter { conjunction: FilterConjunction::And, clauses, groups: Vec::new() },
        Some(mut f) if f.conjunction == FilterConjunction::And => {
            f.clauses.extend(clauses);
            f
        }
        Some(f) => CompoundFilter { conjunction: FilterConjunction::And, clauses, groups: vec![f] },
    })
}

/// A footer for each field the caller can see. A hidden field offers none, and neither does a function the
/// field's type cannot answer.
fn footers(caller: &AuthUser, app_slug: &str, table_id: &str, view: &scaffoldry_engine::AppView, hidden: &[String], state: &SharedState) -> Result<Value, ServiceError> {
    use records::AggregateFunction as F;
    let (fields, _) = records::table_fields_lenient(app_slug, table_id, state);
    let mut out = serde_json::Map::new();
    for (field, function) in &view.column_summary {
        let Some(spec) = fields.iter().find(|f| &f.name == field) else { continue };
        if hidden.contains(field) {
            continue;
        }
        let numeric = scaffoldry_engine::index_column(spec.field_type) == scaffoldry_engine::IndexColumn::Num;
        let (func, uses_field) = match function.as_str() {
            "count" => (F::Count, false),
            "filled" => (F::Filled, true),
            "empty" => (F::Empty, true),
            "unique" => (F::Unique, true),
            "sum" if numeric => (F::Sum, true),
            "avg" if numeric => (F::Avg, true),
            "min" if numeric => (F::Min, true),
            "max" if numeric => (F::Max, true),
            _ => {
                out.insert(field.clone(), json!({ "function": function, "value": Value::Null, "unsupported": true }));
                continue;
            }
        };
        let result = records::aggregate_records(
            caller,
            app_slug,
            table_id,
            records::AggregateQuery { function: func, field: uses_field.then(|| field.clone()), filter: view.filters.clone(), group_by: None, ..Default::default() },
            state,
        )?;
        out.insert(field.clone(), json!({ "function": function, "value": result.value }));
    }
    Ok(Value::Object(out))
}

/// `GET /apps/{slug}/views/{id}/data`: the view's rows or groups, with its footers on the first page and a tone on
/// each row a color rule matches. The server filters, sorts, groups and totals. Nothing here widens access: the
/// rows are the ones the person's row scope and column rules already allow.
pub fn run(caller: &AuthUser, origin: &crate::service::history::Origin, app_slug: &str, id: Uuid, query: RunQuery, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = existing(caller, app_slug, id, state)?;
    let view = materialize(&stored).ok_or_else(|| ServiceError::BadRequest("config is not a view".to_string()))?;
    let table_id = view.table_id.clone().ok_or_else(|| ServiceError::BadRequest("This view is not over a table".to_string()))?;
    check_runnable(caller, app_slug, &view, state)?;
    if stored.kind == "summary" {
        return run_summary(caller, app_slug, &stored, &view, &table_id, state);
    }

    let levels: Vec<String> = if view.group_by.is_empty() { view.group_by_field.iter().cloned().collect() } else { view.group_by.clone() };
    if query.group_path.len() > levels.len() {
        return Err(ServiceError::BadRequest(format!("This view groups by {} fields, and the path names {}", levels.len(), query.group_path.len())));
    }
    let filter = narrowed(view.filters.clone(), &levels, &query.group_path);
    let (fields, table) = records::table_fields_lenient(app_slug, &table_id, state);
    let hidden = records::hidden_fields_for(caller, app_slug, &table, &fields, state);
    let first_page = query.cursor.is_none();
    let footer = if first_page { Some(footers(caller, app_slug, &table_id, &view, &hidden, state)?) } else { None };

    if query.group_path.len() < levels.len() {
        let level = query.group_path.len();
        let result = records::aggregate_records(
            caller,
            app_slug,
            &table_id,
            records::AggregateQuery { function: records::AggregateFunction::Count, field: None, filter, group_by: Some(levels[level].clone()), ..Default::default() },
            state,
        )?;
        let mut items: Vec<Value> = result.groups.unwrap_or_default().into_iter().map(|g| json!({ "key": g.key, "count": g.value })).collect();
        items.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        let mut body = json!({
            "view_id": stored.id,
            "groups": { "level": level, "field": levels[level], "items": items, "has_more": result.has_more.unwrap_or(false) },
        });
        if let Some(f) = footer {
            body["footers"] = f;
        }
        return Ok(body);
    }

    let sort = view.sort_rules.first().map(|r| records::SortSpec { field_name: r.field_name.clone(), descending: r.direction == scaffoldry_engine::SortDirection::Desc });
    let limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let count = match query.count.as_deref() {
        None => None,
        Some("exact") => {
            let counted = records::count_rows(
                caller,
                app_slug,
                records::WindowQuery { table_id: table_id.clone(), filter: filter.clone(), sort: sort.clone(), offset: 0, limit: 1, reverse: false, view_id: Some(stored.id) },
                state,
            )?;
            Some(match counted {
                records::RowCount::Exact(n) => json!({ "value": n, "exact": true }),
                records::RowCount::AtLeast(n) => json!({ "at_least": n }),
            })
        }
        Some(other) => return Err(ServiceError::BadRequest(format!("count must be 'exact', not '{other}'"))),
    };
    let (mut rows, next_cursor, window) = if query.offset.is_some() || query.last {
        let offset = query.offset.unwrap_or(0);
        let page = records::window_page(
            caller,
            app_slug,
            records::WindowQuery { table_id: table_id.clone(), filter, sort, offset, limit, reverse: query.last, view_id: Some(stored.id) },
            state,
        )?;
        (page.records, None, Some(json!({ "offset": if query.last { Value::Null } else { json!(offset) }, "strategy": page.strategy })))
    } else {
        let page = records::list_page(
            caller,
            app_slug,
            records::PageQuery { table_id: table_id.clone(), filter, sort, limit, cursor: query.cursor, max_bytes: None, must_exist: true, search: None },
            state,
        )?;
        (page.records, page.next_cursor, None)
    };
    crate::service::disclosure::record_agent_read(caller, origin, app_slug, Some(&table_id), &rows, state)?;

    // A rule that reads a field the person cannot see is dropped for them, so a tone never says what they may not know.
    let (now, zone) = (chrono::Utc::now(), state.platform_timezone());
    let rules: Vec<scaffoldry_engine::ColorRule> = view
        .color_rules
        .iter()
        .filter(|r| {
            scaffoldry_engine::clauses_of(&r.when).iter().all(|c| !hidden.contains(&c.field_name)) && r.field.as_ref().is_none_or(|f| !hidden.contains(f))
        })
        .map(|r| scaffoldry_engine::ColorRule { when: scaffoldry_engine::resolve_relative_dates(&r.when, now, &zone), ..r.clone() })
        .collect();
    if !rules.is_empty() {
        for row in &mut rows {
            let tones = scaffoldry_engine::tones_of(&row.data, &rules);
            if !tones.is_empty() {
                if let Some(map) = row.data.as_object_mut() {
                    map.insert("_tone".to_string(), serde_json::to_value(tones).map_err(internal)?);
                }
            }
        }
    }

    let mut body = json!({ "view_id": stored.id, "records": rows, "next_cursor": next_cursor, "group_path": query.group_path });
    if let Some(w) = window {
        body["window"] = w;
    }
    if let Some(c) = count {
        body["count"] = c;
    }
    if let Some(f) = footer {
        body["footers"] = f;
    }
    Ok(body)
}

/// A summary view: its groups, and one column for each measure. Each measure is one call to the aggregate route, ordered by
/// key so every column has the same groups, and the rows are put together by key. At most 200 groups, and a flag when there
/// are more. A summary is read-only: it releases totals, never records.
fn run_summary(caller: &AuthUser, app_slug: &str, stored: &StoredView, view: &scaffoldry_engine::AppView, table_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    use records::AggregateFunction as F;
    let groups = &view.group_by;
    let mut columns: Vec<String> = groups.clone();
    columns.extend(view.aggregations.iter().map(scaffoldry_engine::Aggregation::label));
    // Each group is its keys and then one value for each measure.
    let mut rows: Vec<(Vec<String>, Vec<Value>)> = Vec::new();
    let mut has_more = false;
    for (i, a) in view.aggregations.iter().enumerate() {
        let function = match a.function.as_str() {
            "count" => F::Count,
            "sum" => F::Sum,
            "avg" => F::Avg,
            "min" => F::Min,
            "max" => F::Max,
            "count_distinct" => F::Unique,
            other => return Err(ServiceError::BadRequest(format!("'{other}' is not an aggregation"))),
        };
        let result = records::aggregate_records(
            caller,
            app_slug,
            table_id,
            records::AggregateQuery {
                function,
                field: a.field.clone(),
                filter: view.filters.clone(),
                group_by: groups.first().cloned(),
                group_by2: groups.get(1).cloned(),
                order_by_key: true,
                ..Default::default()
            },
            state,
        )?;
        let found = result.groups.unwrap_or_default();
        let keys = |g: &records::AggregateGroup| -> Vec<String> { std::iter::once(g.key.clone()).chain(g.key2.clone()).collect() };
        if i == 0 {
            has_more = result.has_more.unwrap_or(false);
            rows = found.iter().map(|g| (keys(g), vec![json!(g.value)])).collect();
        } else {
            let by_key: std::collections::HashMap<Vec<String>, f64> = found.iter().map(|g| (keys(g), g.value)).collect();
            for (key, values) in &mut rows {
                values.push(by_key.get(key).map_or(Value::Null, |v| json!(v)));
            }
        }
    }
    let groups: Vec<Value> = rows.into_iter().map(|(key, values)| json!({ "key": key, "values": values })).collect();
    Ok(json!({ "view_id": stored.id, "summary": { "columns": columns, "groups": groups, "has_more": has_more } }))
}
