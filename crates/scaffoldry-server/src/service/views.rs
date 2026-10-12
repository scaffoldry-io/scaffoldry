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
    state.repository.as_deref().ok_or_else(|| ServiceError::Internal("Views need PostgreSQL".to_string()))
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

/// The fields a config sorts, filters, groups or lays out by.
fn used_fields(config: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(rules) = config.get("sort_rules").and_then(Value::as_array) {
        out.extend(rules.iter().filter_map(|r| r.get("field_name").and_then(Value::as_str)).map(str::to_string));
    }
    if let Some(clauses) = config.pointer("/filters/clauses").and_then(Value::as_array) {
        out.extend(clauses.iter().filter_map(|c| c.get("field_name").and_then(Value::as_str)).map(str::to_string));
    }
    for key in ["kanban_column_field", "group_by_field", "calendar_date_field"] {
        if let Some(f) = config.get(key).and_then(Value::as_str) {
            out.insert(f.to_string());
        }
    }
    out
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
