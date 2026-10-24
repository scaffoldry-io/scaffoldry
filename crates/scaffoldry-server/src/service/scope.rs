//! The one place that turns an app's row rules into what a person may reach.
//!
//! `scope_of` gives a `RowScope`. A list applies it in SQL (`plan_of`), a single record is checked in
//! Rust (`allows`). A rule only narrows: with no rule, a workspace member reads every row, as before.
//! Every function that returns record data, or anything derived from it, is named in
//! `RECORD_READERS`, and a test requires each one to apply the scope.

use crate::service::members;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use scaffoldry_core::access::{never, row_scope, Person, Pred, RowOp, RowRule, RowScope};
use scaffoldry_engine::{FieldSpec, HostRouter};
use serde_json::Value;

/// Every service function that returns record data, or anything derived from it. Adding a reader
/// without adding it here fails `every_service_function_that_loads_records_is_listed`.
pub const RECORD_READERS: &[&str] = &[
    "list_records",
    "get_record",
    "list_page",
    "page",
    "app_history",
    "record_history",
    "list_comments",
    "link_options",
    "list_trash",
    "list_waiting_decisions",
    "decide_process",
    "list_app_processes",
];

/// Functions that load records but return none of it to the caller: they write, or they read a
/// record only to decide something else. Each one checks the scope where it matters.
pub const RECORD_INTERNALS: &[&str] = &[
    // Writes. Each checks the create, update, or delete predicate on the stored and the new record.
    "create_record",
    "update_record",
    "delete_record",
    "delete_record_checked",
    "restore_record",
    "restore",
    "undo_batch",
    "validate_links",
    // Reads a record only for its table id, to check a limited token's scope. It returns no data.
    "table_of_record",
    // Helpers that sit under a listed reader and apply the scope before they return.
    "find",
    "link_names",
    "fill_lookups",
    "redact_records",
    "reader",
    "context",
    "run_automations",
    "refresh_no_approver",
    "caller_can_decide",
    "post_comment",
    "edit_comment",
    "remove_comment",
    // Reads a count for the admin inventory, which only a Platform Admin reaches.
    "get_records_per_table",
];

/// The row rules in the manifest the app runs, or `None` when the engine holds no such app. The manifest is the
/// only place rules live, so a restart, or a second instance, applies what was approved.
fn rules_loaded(app_slug: &str, state: &SharedState) -> Option<Vec<RowRule>> {
    use scaffoldry_engine::HostRouter;
    state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(app_slug).map(|m| m.access.rows.clone())
}

/// The rules an app has set. An app the engine does not hold has none to give. Its callers reach it only
/// through a route that found the app first, and the scope functions below treat it as closed.
pub fn rules_of(app_slug: &str, state: &SharedState) -> Vec<RowRule> {
    rules_loaded(app_slug, state).unwrap_or_default()
}

/// Replaces an app's row rules in its manifest, stored and running, the way an approved manifest would. This is for a
/// test or a tool that builds a state by hand. The approved manifest is the source everywhere else. It panics when
/// the app is not held or the manifest no longer validates, so a rule that was not applied is never mistaken for no rule.
pub fn set_rules(app_slug: &str, rules: Vec<RowRule>, state: &SharedState) {
    edit_manifest(app_slug, state, |m| m.access.rows = rules);
}

fn edit_manifest(app_slug: &str, state: &SharedState, edit: impl FnOnce(&mut scaffoldry_engine::AppManifest)) {
    use scaffoldry_engine::HostRouter;
    // The stored manifest is the one to change, so a reload from the database keeps the rules. An app only the engine
    // holds is stored as it runs, without the views that live in their own table.
    let mut manifest = match state.repository.get_app_manifest(app_slug).unwrap_or_else(|e| panic!("the stored manifest of '{app_slug}' cannot be read: {e}")) {
        Some(stored) => stored,
        None => {
            let mut running = state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(app_slug).cloned().unwrap_or_else(|| panic!("the engine holds no app '{app_slug}'"));
            let kept: std::collections::HashSet<String> =
                state.app_views.read().unwrap_or_else(|p| p.into_inner()).get(app_slug).map(|vs| vs.iter().map(|v| v.id.to_string()).collect()).unwrap_or_default();
            running.views.retain(|v| !kept.contains(&v.id));
            running
        }
    };
    edit(&mut manifest);
    state.persist_app_manifest(manifest).unwrap_or_else(|e| panic!("the manifest of '{app_slug}' did not save: {e}"));
}

/// What the rules know about a person. Their units are the same set Cedar sees, with every ancestor. Each position
/// they hold comes twice, as the bare key and as `key@unit-id`, so a rule can tell Chemistry's chair from Physics's.
pub fn person_of(caller: &AuthUser, state: &SharedState) -> Person {
    let roles = state.roles_of(&caller.eppn);
    let group_ids = state.group_ids_of(&caller.eppn);
    let mut position_keys: Vec<String> = Vec::new();
    for r in &roles {
        if let Some(key) = &r.position_key {
            if !position_keys.contains(key) {
                position_keys.push(key.clone());
            }
        }
    }
    for r in &roles {
        if let Some(key) = &r.position_key {
            position_keys.push(format!("{key}@{}", r.organization_id));
        }
    }
    Person {
        address: caller.eppn.clone(),
        department: caller.department.clone(),
        affiliation: caller.affiliation.clone(),
        unit_ids: crate::service::access::unit_ids_of(state, &roles),
        position_keys,
        group_ids,
    }
}

fn workspace_of(app_slug: &str, state: &SharedState) -> Option<String> {
    state.engine.read().ok().and_then(|e| e.resolve_by_slug(app_slug).and_then(|m| m.workspace_id.clone()))
}

/// What `caller` may reach on `table` for `op`. A Platform Admin skips every rule.
///
/// In view-as the rights are the target's intersected with the viewer's, so the result never shows
/// more than the viewer could see.
pub fn scope_of(caller: &AuthUser, app_slug: &str, table: &str, op: RowOp, state: &SharedState) -> RowScope {
    let mut own = scope_for(caller, app_slug, table, op, state);
    // A preview of a proposal shows live rows through the proposed rules and the live ones together.
    if crate::service::preview::is_preview(app_slug) {
        own = own.and(scope_for(caller, crate::service::preview::stored_slug(app_slug), table, op, state));
    }
    match &caller.view_as_by {
        Some(viewer) => own.and(scope_for(viewer, app_slug, table, op, state)),
        None => own,
    }
}

fn scope_for(caller: &AuthUser, app_slug: &str, table: &str, op: RowOp, state: &SharedState) -> RowScope {
    // An app the engine does not hold, such as one whose stored manifest no longer validates, shows no rows.
    let Some(rules) = rules_loaded(app_slug, state) else { return RowScope::DenyAll };
    let by_rules = if rules.is_empty() {
        RowScope::AllowAll
    } else {
        let role = workspace_of(app_slug, state).and_then(|w| members::member_role_in_app(caller, &w, app_slug, state)).map(|r| r.as_str());
        row_scope(&person_of(caller, state), role, caller.affiliation == "central_admin", &rules, table, op)
    };
    // The rules narrow what the grants reach. A rule never widens it.
    by_rules.and(reach_scope(caller, app_slug, table, state))
}

/// For each hierarchy, the field that places a row of this table in it. Empty for a table that places no rows.
pub fn row_place_of(app_slug: &str, table: &str, state: &SharedState) -> std::collections::BTreeMap<String, String> {
    use scaffoldry_engine::HostRouter;
    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    engine.resolve_by_slug(app_slug).and_then(|m| m.tables.iter().find(|t| t.id == table || t.slug == table)).map(|t| t.row_place.clone()).unwrap_or_default()
}

/// The rows of a table that places its rows (`row_place`) which the caller's grants reach. A table that places no rows
/// is not narrowed. A container grant, a workspace membership and a Platform Admin reach every row. Rows reached by any
/// grant are the union, and a row with no place is reached only by a container grant.
fn reach_scope(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> RowScope {
    use scaffoldry_core::reach::RowReach;
    let placed: std::collections::BTreeMap<String, String> = {
        use scaffoldry_engine::HostRouter;
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        match engine.resolve_by_slug(app_slug).and_then(|m| m.tables.iter().find(|t| t.id == table || t.slug == table)) {
            Some(t) if !t.row_place.is_empty() => t.row_place.clone(),
            _ => return RowScope::AllowAll,
        }
    };
    if caller.affiliation == "central_admin" {
        return RowScope::AllowAll;
    }
    let by_person: std::collections::BTreeSet<String> = fields_of(app_slug, table, state)
        .into_iter()
        .filter(|f| matches!(f.field_type, scaffoldry_engine::FieldType::Collaborator { .. }))
        .map(|f| f.name)
        .collect();
    let Some(workspace) = workspace_of(app_slug, state) else { return RowScope::DenyAll };
    let table_id = state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(app_slug).and_then(|m| m.tables.iter().find(|t| t.id == table || t.slug == table).map(|t| t.id.clone())).unwrap_or_else(|| table.to_string());
    match members::rows_reach(&caller.eppn, &workspace, app_slug, &table_id, state) {
        None => RowScope::DenyAll,
        Some(RowReach::All) => RowScope::AllowAll,
        Some(RowReach::Units(by_hierarchy)) => {
            let parts: Vec<Pred> = by_hierarchy
                .iter()
                .filter(|(_, units)| !units.is_empty())
                .filter_map(|(hierarchy, units)| {
                    let field = placed.get(hierarchy)?;
                    // A row is placed by a unit, or in a hierarchy of people by a person. For people, the reach is the
                    // addresses of those appointed at the units, and the filter is the same shape over that field.
                    let ids: Vec<Value> = if by_person.contains(field) {
                        let units: Vec<uuid::Uuid> = units.iter().copied().collect();
                        state.repository.appointed_addresses(&units).unwrap_or_default().into_iter().map(Value::String).collect()
                    } else {
                        units.iter().map(|u| Value::String(u.to_string())).collect()
                    };
                    if ids.is_empty() {
                        return Some(never());
                    }
                    Some(Pred::Cmp { field: field.clone(), op: scaffoldry_core::access::Op::In, value: scaffoldry_core::access::Operand::Literal(Value::Array(ids)) })
                })
                .collect();
            match parts.len() {
                0 => RowScope::DenyAll,
                1 => RowScope::of(parts.into_iter().next().expect("one part")),
                _ => RowScope::of(Pred::Or(parts)),
            }
        }
    }
}

fn table_of(record: &DatasetRecord) -> String {
    record.data.get("_table_id").and_then(Value::as_str).unwrap_or("").to_string()
}

/// The fields of a table, to read a record's values the way the index does.
pub fn fields_of(app_slug: &str, table: &str, state: &SharedState) -> Vec<FieldSpec> {
    crate::service::records::table_fields_lenient(app_slug, table, state).0
}

/// True when the record is inside `caller`'s scope for `op`.
pub fn allows(caller: &AuthUser, app_slug: &str, record: &DatasetRecord, op: RowOp, state: &SharedState) -> bool {
    allows_data(caller, app_slug, &table_of(record), &record.data, op, state)
}

/// The same check for record data that is not stored yet, such as a record about to be created.
pub fn allows_data(caller: &AuthUser, app_slug: &str, table: &str, data: &Value, op: RowOp, state: &SharedState) -> bool {
    match scope_of(caller, app_slug, table, op, state) {
        RowScope::AllowAll => true,
        RowScope::DenyAll => false,
        RowScope::Mixed(pred) => scaffoldry_engine::access::eval(&pred, data, &fields_of(app_slug, table, state)),
    }
}

/// The records of `rows` inside the caller's read scope.
pub fn retain_readable(caller: &AuthUser, app_slug: &str, rows: Vec<DatasetRecord>, state: &SharedState) -> Vec<DatasetRecord> {
    match rules_loaded(app_slug, state) {
        None => return Vec::new(),
        Some(rules) if rules.is_empty() => return rows,
        Some(_) => {}
    }
    rows.into_iter().filter(|r| allows(caller, app_slug, r, RowOp::Read, state)).collect()
}

/// What a SQL statement over one table applies to stay inside the caller's read scope. `None` means
/// the whole table.
pub fn plan_of(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> Option<crate::repository::ScopePlan> {
    let pred: Pred = match scope_of(caller, app_slug, table, RowOp::Read, state) {
        RowScope::AllowAll => return None,
        RowScope::DenyAll => never(),
        RowScope::Mixed(p) => p,
    };
    Some(crate::repository::ScopePlan { pred, indexed: fields_of(app_slug, table, state) })
}

/// The fields the app's rules read for a table. They are indexed, so a rule compiles to the index.
pub fn rule_fields(app_slug: &str, table: &str, state: &SharedState) -> Vec<String> {
    fn names(p: &Pred, out: &mut Vec<String>) {
        match p {
            Pred::True => {}
            Pred::Cmp { field, .. } if !field.starts_with("person.") => out.push(field.clone()),
            Pred::Cmp { .. } => {}
            Pred::And(ps) | Pred::Or(ps) => ps.iter().for_each(|p| names(p, out)),
            Pred::Not(p) => names(p, out),
        }
    }
    let mut out = Vec::new();
    for rule in rules_of(app_slug, state).iter().filter(|r| r.table_id == table) {
        names(&rule.visible_when, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Of these record ids, those whose record, live or in trash, the caller may read. A record that is
/// gone cannot be checked, so only a Platform Admin sees what is left of it.
pub fn readable_ids(caller: &AuthUser, app_slug: &str, ids: &[String], state: &SharedState) -> std::collections::HashSet<String> {
    match rules_loaded(app_slug, state) {
        None => return std::collections::HashSet::new(),
        Some(rules) if rules.is_empty() || caller.affiliation == "central_admin" => return ids.iter().cloned().collect(),
        Some(_) => {}
    }
    let repo = &state.repository;
    let found = repo.records_with_trashed(app_slug, ids).unwrap_or_default();
    found.into_iter().filter(|r| allows(caller, app_slug, r, RowOp::Read, state)).map(|r| r.id).collect()
}

/// `NotFound` for a record the caller cannot read, in the words used for a record that does not exist.
pub fn require_readable(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Result<(), crate::service::ServiceError> {
    if readable_ids(caller, app_slug, &[record_id.to_string()], state).is_empty() {
        return Err(crate::service::ServiceError::NotFound(format!("Record '{record_id}' not found")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------------------------

/// The column rules in the manifest the app runs. An app the engine does not hold has none.
pub fn column_rules_of(app_slug: &str, state: &SharedState) -> Vec<scaffoldry_core::access::ColumnRule> {
    use scaffoldry_engine::HostRouter;
    state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(app_slug).map(|m| m.access.columns.clone()).unwrap_or_default()
}

/// Replaces an app's column rules the way `set_rules` replaces its row rules.
pub fn set_column_rules(app_slug: &str, rules: Vec<scaffoldry_core::access::ColumnRule>, state: &SharedState) {
    edit_manifest(app_slug, state, |m| m.access.columns = rules);
}

/// The fields of `table` held back from the caller by the app's column rules.
/// In view-as, what either the target or the viewer may not see is held back.
pub fn field_visibility_of(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> scaffoldry_core::access::FieldVisibility {
    let mut own = field_visibility_for(caller, app_slug, table, state);
    // A field the live rules hide is hidden in a preview of a proposal too, and so is one the proposed rules hide.
    if crate::service::preview::is_preview(app_slug) {
        own = own.union(field_visibility_for(caller, crate::service::preview::stored_slug(app_slug), table, state));
    }
    if let Some(viewer) = &caller.view_as_by {
        own = own.union(field_visibility_for(viewer, app_slug, table, state));
    }
    // Through an outside app, what the ceiling does not read is hidden and what it does not write is read-only.
    if let Some(client) = &caller.via_client {
        own = own.union(client_visibility(client, app_slug, table, state));
    }
    own
}

fn client_visibility(client: &crate::state::ViaClient, app_slug: &str, table: &str, state: &SharedState) -> scaffoldry_core::access::FieldVisibility {
    use scaffoldry_core::access::{Denial, FieldVisibility};
    let names: Vec<String> = fields_of(crate::service::preview::stored_slug(app_slug), table, state).into_iter().map(|f| f.name).collect();
    let (unread, unwritten) = crate::service::clients::field_limits(&client.ceiling, crate::service::preview::stored_slug(app_slug), table, &names);
    let denial = |field: String, reason: &str| Denial {
        field,
        rule_id: format!("client:{}", client.client_id),
        reason: reason.to_string(),
        remedy: "Ask the owners of the app to widen its ceiling.".to_string(),
    };
    FieldVisibility {
        hidden: unread.into_iter().map(|f| denial(f, &format!("The app '{}' is not allowed to read this field.", client.name))).collect(),
        read_only: unwritten.into_iter().map(|f| denial(f, &format!("The app '{}' is not allowed to change this field.", client.name))).collect(),
    }
}

fn field_visibility_for(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> scaffoldry_core::access::FieldVisibility {
    let rules = column_rules_of(app_slug, state);
    if rules.is_empty() {
        return Default::default();
    }
    let role = workspace_of(app_slug, state).and_then(|w| members::member_role_in_app(caller, &w, app_slug, state)).map(|r| r.as_str());
    scaffoldry_core::access::field_visibility(&person_of(caller, state), role, caller.affiliation == "central_admin", &rules, table)
}

#[derive(Debug, serde::Deserialize)]
pub struct AccessPreviewInput {
    pub table_id: String,
    #[serde(default)]
    pub person: Option<scaffoldry_core::access::Person>,
    #[serde(default)]
    pub eppn: Option<String>,
    #[serde(default)]
    pub rules: Option<Vec<scaffoldry_core::access::RowRule>>,
    #[serde(default)]
    pub column_rules: Option<Vec<scaffoldry_core::access::ColumnRule>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AccessPreviewOutput {
    pub can_see: u64,
    pub can_add: u64,
    pub can_change: u64,
    pub can_remove: u64,
    pub total_rows: u64,
    pub hidden_columns: Vec<String>,
}

pub fn count_scoped(
    app_slug: &str,
    table_id: &str,
    scope: Option<&crate::repository::ScopePlan>,
    state: &SharedState,
) -> Result<u64, crate::service::ServiceError> {
    let repo = &*state.repository;
    repo.count_scoped_records(app_slug, table_id, scope)
        .map_err(|e| crate::service::ServiceError::Internal(e.to_string()))
}

pub fn preview_access(
    caller: &AuthUser,
    app_slug: &str,
    input: AccessPreviewInput,
    state: &SharedState,
) -> Result<AccessPreviewOutput, crate::service::ServiceError> {
    crate::service::access::authorize_app(caller, app_slug, crate::service::access::AppAction::Manage, state)?;

    let app = state
        .engine
        .read()
        .map_err(|e| crate::service::ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(app_slug)
        .cloned()
        .ok_or_else(|| crate::service::ServiceError::NotFound(format!("App '{app_slug}' not found")))?;

    let ws_id = app.workspace_id.as_deref();

    let person = if let Some(p) = input.person {
        p
    } else if let Some(eppn) = &input.eppn {
        let dummy = AuthUser {
            eppn: eppn.clone(),
            name: eppn.clone(),
            role_title: String::new(),
            department: String::new(),
            affiliation: String::new(),
        view_as_by: None, via_client: None,
        };
        person_of(&dummy, state)
    } else {
        person_of(caller, state)
    };

    let role = ws_id.and_then(|w| members::member_role_of_in_app(&person.address, w, app_slug, state));
    let role_str = role.as_ref().map(|r| r.as_str());
    let is_platform_admin = person.affiliation == "central_admin";

    let rules = input.rules.unwrap_or_else(|| rules_of(app_slug, state));
    let column_rules = input.column_rules.unwrap_or_else(|| column_rules_of(app_slug, state));

    let fields = fields_of(app_slug, &input.table_id, state);
    let total_rows = count_scoped(app_slug, &input.table_id, None, state)?;

    let count_for_op = |op: RowOp| -> Result<u64, crate::service::ServiceError> {
        match row_scope(&person, role_str, is_platform_admin, &rules, &input.table_id, op) {
            RowScope::AllowAll => Ok(total_rows),
            RowScope::DenyAll => Ok(0),
            RowScope::Mixed(pred) => {
                let plan = crate::repository::ScopePlan {
                    pred,
                    indexed: fields.clone(),
                };
                count_scoped(app_slug, &input.table_id, Some(&plan), state)
            }
        }
    };

    let can_see = count_for_op(RowOp::Read)?;
    let can_add = count_for_op(RowOp::Create)?;
    let can_change = count_for_op(RowOp::Update)?;
    let can_remove = count_for_op(RowOp::Delete)?;

    let visibility = scaffoldry_core::access::field_visibility(
        &person,
        role_str,
        is_platform_admin,
        &column_rules,
        &input.table_id,
    );
    let hidden_columns = visibility.hidden_names();

    Ok(AccessPreviewOutput {
        can_see,
        can_add,
        can_change,
        can_remove,
        total_rows,
        hidden_columns,
    })
}
