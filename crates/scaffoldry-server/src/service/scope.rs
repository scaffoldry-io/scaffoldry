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

/// The rules an app has set.
pub fn rules_of(app_slug: &str, state: &SharedState) -> Vec<RowRule> {
    state.access_rules.read().unwrap_or_else(|p| p.into_inner()).get(app_slug).cloned().unwrap_or_default()
}

/// Replaces an app's rules.
pub fn set_rules(app_slug: &str, rules: Vec<RowRule>, state: &SharedState) {
    state.access_rules.write().unwrap_or_else(|p| p.into_inner()).insert(app_slug.to_string(), rules);
}

/// What the rules know about a person.
pub fn person_of(caller: &AuthUser, state: &SharedState) -> Person {
    let roles: Vec<crate::state::RoleRow> = state
        .roles
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|r| r.eppn.eq_ignore_ascii_case(&caller.eppn))
        .cloned()
        .collect();
    let group_ids: Vec<String> = {
        let groups = state.groups.read().unwrap_or_else(|p| p.into_inner());
        let users = state.users.read().unwrap_or_else(|p| p.into_inner());
        let my_id = users.values().find(|u| u.user_name.eq_ignore_ascii_case(&caller.eppn)).map(|u| u.id.clone());
        groups
            .values()
            .filter(|g| {
                g.members.iter().filter_map(|m| m.get("value").and_then(Value::as_str)).any(|v| v.eq_ignore_ascii_case(&caller.eppn) || my_id.as_deref() == Some(v))
            })
            .map(|g| g.id.clone())
            .collect()
    };
    Person {
        address: caller.eppn.clone(),
        department: caller.department.clone(),
        affiliation: caller.affiliation.clone(),
        unit_ids: roles.iter().map(|r| r.organization_id.to_string()).collect(),
        position_keys: roles.iter().filter_map(|r| r.position_key.clone()).collect(),
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
    let own = scope_for(caller, app_slug, table, op, state);
    match &caller.view_as_by {
        Some(viewer) => own.and(scope_for(viewer, app_slug, table, op, state)),
        None => own,
    }
}

fn scope_for(caller: &AuthUser, app_slug: &str, table: &str, op: RowOp, state: &SharedState) -> RowScope {
    let rules = rules_of(app_slug, state);
    if rules.is_empty() {
        return RowScope::AllowAll;
    }
    let role = workspace_of(app_slug, state).and_then(|w| members::member_role(caller, &w, state)).map(|r| r.as_str());
    row_scope(&person_of(caller, state), role, caller.affiliation == "central_admin", &rules, table, op)
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
    if rules_of(app_slug, state).is_empty() {
        return rows;
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
    if rules_of(app_slug, state).is_empty() || caller.affiliation == "central_admin" {
        return ids.iter().cloned().collect();
    }
    let Some(repo) = state.repository.as_deref() else { return Default::default() };
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

/// The column rules an app has set.
pub fn column_rules_of(app_slug: &str, state: &SharedState) -> Vec<scaffoldry_core::access::ColumnRule> {
    state.column_rules.read().unwrap_or_else(|p| p.into_inner()).get(app_slug).cloned().unwrap_or_default()
}

/// Replaces an app's column rules.
pub fn set_column_rules(app_slug: &str, rules: Vec<scaffoldry_core::access::ColumnRule>, state: &SharedState) {
    state.column_rules.write().unwrap_or_else(|p| p.into_inner()).insert(app_slug.to_string(), rules);
}

/// The fields of `table` held back from the caller by the app's column rules.
/// In view-as, what either the target or the viewer may not see is held back.
pub fn field_visibility_of(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> scaffoldry_core::access::FieldVisibility {
    let own = field_visibility_for(caller, app_slug, table, state);
    match &caller.view_as_by {
        Some(viewer) => own.union(field_visibility_for(viewer, app_slug, table, state)),
        None => own,
    }
}

fn field_visibility_for(caller: &AuthUser, app_slug: &str, table: &str, state: &SharedState) -> scaffoldry_core::access::FieldVisibility {
    let rules = column_rules_of(app_slug, state);
    if rules.is_empty() {
        return Default::default();
    }
    let role = workspace_of(app_slug, state).and_then(|w| members::member_role(caller, &w, state)).map(|r| r.as_str());
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
    let repo = state
        .repository
        .as_deref()
        .ok_or_else(|| crate::service::ServiceError::Internal("Database required".to_string()))?;
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
        view_as_by: None,
        };
        person_of(&dummy, state)
    } else {
        person_of(caller, state)
    };

    let role = ws_id.and_then(|w| members::member_role_of(&person.address, w, state));
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
