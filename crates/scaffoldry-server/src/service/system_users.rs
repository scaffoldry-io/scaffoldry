//! Data layer phase 9: system users for automations (`docs/plans/data-layer.md`, "Automations are system users").
//!
//! An automation is a flow, and a flow already acts as the identity `flow:{rule_id}`. This adds what the flow did not have: an owner and
//! grants a person approved. It builds no second identity.
//!
//! - **A system user** is a row for a flow. A flow with a row reads and writes only what its live grants name, and it gets nothing by
//!   default. A flow with no row runs as it did before.
//! - **A grant** names a dataset, an action (`read`, `write` or `create`) and fields. It never borrows the person who started the run.
//! - **The approver holds the access.** Nobody can grant more than they have. The check is in `holds`, and it is the one way a grant is
//!   stored. It asks the same questions a read or a write asks of the approver: the app, the row rules for the whole table and the field
//!   rules. A guest, a view-as session and an outside app's ceiling cannot approve. A sensitive field is never granted.
//! - **The approval** is one `AccessGrantChanged` ledger entry with the reason, committed with the grant.
//! - **The owner** is a person, a group or a position, stored as a workspace membership key. The people are read when they are needed,
//!   so a position owner follows the seat.
//! - **Expiry.** Each grant has a review date the approver chose. A run compares it with the database's clock (`admit`), so access ends
//!   on the date and no job is needed. The run that first meets a lapse tells the owner, once.
//! - **Change.** What a flow reads or writes comes from its steps. A version that needs more than the grants cover is refused until a grant
//!   for it is approved. The name, the notes, the owner and the flow's schedule need no approval.
//!
//! Category gates still close rows to a system user, because it holds no position: see `Admission::Gated`.

use crate::notifications::{notify, Count, DatasetRef, Notice, Recipients};
use crate::repository::AppendDecisionParams;
use crate::repository_catalog::CatalogRow;
use crate::repository_system_users::{NewGrant, Revoked, SystemGrant, SystemUser};
use crate::service::access::{authorize_app, AppAction};
use crate::service::label_changes::reason_of;
use crate::service::members::{self, Kind};
use crate::service::{flows, scope, ServiceError};
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, Duration, Utc};
use scaffoldry_core::access::{RowOp, RowScope};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::workflow::AutomationRule;
use scaffoldry_engine::{effective_sensitivity, HostRouter};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// The furthest ahead a review date may be, in days.
const MAX_AHEAD_DAYS: i64 = 366;
const MAX_FIELDS: usize = 50;
const MAX_NAME: usize = 200;
const MAX_NOTES: usize = 2000;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn missing() -> ServiceError {
    ServiceError::NotFound("System user not found".to_string())
}

/// The identity an automation acts as.
pub fn identity_of(flow_id: &str) -> String {
    flows::actor_of(flow_id)
}

// ---------------------------------------------------------------------------------------------------------------------
// Who
// ---------------------------------------------------------------------------------------------------------------------

/// A person acting as themselves. A guest, a view-as session, an outside app's ceiling and a shared link never register, change or approve.
fn ordinary(caller: &AuthUser) -> Result<(), ServiceError> {
    if caller.affiliation == "guest" || caller.view_as_by.is_some() || caller.via_client.is_some() || caller.shared_link.is_some() {
        return Err(ServiceError::forbidden("Only a signed-in person acting as themselves registers, changes or approves the access of an automation. A guest, a view-as session and an outside app cannot"));
    }
    Ok(())
}

/// Whether the caller owns the system user, or manages the app its flow lives in.
fn manages(caller: &AuthUser, su: &SystemUser, state: &SharedState) -> bool {
    members::people_of_key(&su.owner, state).iter().any(|p| p.eq_ignore_ascii_case(&caller.eppn)) || authorize_app(caller, &su.app_slug, AppAction::Manage, state).is_ok()
}

/// An owner the body names, as the key it is stored under. It is a person, a group or a position, the options an app has.
fn owner_key(v: &Value, state: &SharedState) -> Result<String, ServiceError> {
    let kind = v.get("kind").and_then(Value::as_str).and_then(Kind::parse);
    let id = v.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let unknown = |what: &str| bad(format!("The owner {what} is not known to this server"));
    match kind {
        Some(Kind::User) => {
            let person = crate::service::identity::resolve_user(id, state).filter(|p| p.affiliation != "guest");
            let held = state.repository.users_by_names(&[id.to_string()]).map(|u| !u.is_empty()).unwrap_or(false);
            let person = person.filter(|_| held).ok_or_else(|| unknown("person"))?;
            Ok(members::stored_key(Kind::User, &person.eppn))
        }
        Some(Kind::Group) => {
            state.repository.group_by_id(id).ok().flatten().ok_or_else(|| unknown("group"))?;
            Ok(members::stored_key(Kind::Group, id))
        }
        Some(Kind::Position) => {
            let (key, unit) = id.split_once('@').ok_or_else(|| bad("A position owner is written position-key@unit-id"))?;
            let unit = Uuid::parse_str(unit).map_err(|_| bad("A position owner is written position-key@unit-id"))?;
            let known = state.organizations.read().unwrap_or_else(|p| p.into_inner()).contains_key(&unit) && state.position_types.read().unwrap_or_else(|p| p.into_inner()).contains_key(key);
            if !known {
                return Err(unknown("position"));
            }
            Ok(members::stored_key(Kind::Position, &format!("{key}@{unit}")))
        }
        _ => Err(bad("owner is a person (kind user), a group (kind group) or a position (kind position), with an id")),
    }
}

fn owner_json(owner: &str, state: &SharedState) -> Value {
    let (kind, id) = members::split_principal(owner);
    json!({ "kind": kind.as_str(), "id": id, "people": members::people_of_key(owner, state) })
}

// ---------------------------------------------------------------------------------------------------------------------
// What a flow needs, and what a grant covers
// ---------------------------------------------------------------------------------------------------------------------

/// One thing a flow's steps need: an action on a table, and a field for a read or a write.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Need {
    action: &'static str,
    app: String,
    table: String,
    field: String,
}

/// The reads, writes and creates a rule's stored grants say its steps make. A link is a write of its field.
fn needs_of(rule: &AutomationRule) -> BTreeSet<Need> {
    let mut out = BTreeSet::new();
    for g in &rule.grants {
        let need = |action, field: &str| Need { action, app: g.app_slug.clone(), table: g.table_id.clone(), field: field.to_string() };
        out.extend(g.read.iter().filter(|f| f.as_str() != "id").map(|f| need("read", f)));
        out.extend(g.write.iter().chain(g.link.iter()).map(|f| need("write", f)));
        if g.create {
            out.insert(need("create", ""));
        }
    }
    out
}

fn covers_of(grants: &[SystemGrant], lapsed: bool) -> BTreeSet<Need> {
    let mut out = BTreeSet::new();
    for g in grants.iter().filter(|g| g.status == "Active" && g.lapsed == lapsed) {
        let action = match g.action.as_str() {
            "read" => "read",
            "write" => "write",
            _ => "create",
        };
        if action == "create" {
            out.insert(Need { action, app: g.app_slug.clone(), table: g.table_id.clone(), field: String::new() });
        }
        out.extend(g.fields.iter().map(|f| Need { action, app: g.app_slug.clone(), table: g.table_id.clone(), field: f.clone() }));
    }
    out
}

/// What some needs say, in words: `read Applications: email, status; set Awards: note; create records in Awards`.
fn said(state: &SharedState, needs: &[&Need]) -> String {
    let mut by: BTreeMap<(&str, &str, &str), Vec<&str>> = BTreeMap::new();
    for n in needs {
        by.entry((n.action, n.app.as_str(), n.table.as_str())).or_default().push(n.field.as_str());
    }
    by.into_iter()
        .map(|((action, app, table), fields)| {
            let place = flows::place(state, app, table);
            match action {
                "read" => format!("read {place}: {}", fields.join(", ")),
                "write" => format!("set {place}: {}", fields.join(", ")),
                _ => format!("create records in {place}"),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// What a grant says, in a sentence a reviewer reads.
fn sentence(g: &SystemGrant, state: &SharedState) -> String {
    let place = flows::place(state, &g.app_slug, &g.table_id);
    match g.action.as_str() {
        "read" => format!("May read {place}: {}.", g.fields.join(", ")),
        "write" => format!("May set {place}: {}.", g.fields.join(", ")),
        _ => format!("May create records in {place}."),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------------------------------------------------

/// What a run is told before it starts.
pub enum Admission {
    /// The flow has no system user. It runs inside its own grants, as it did before.
    Open,
    /// The system user is granted all its steps need. The run reads each row it addresses as this identity, so a category gate that asks
    /// for a position closes the row to it, as it would to any person who holds none.
    Gated(Box<AuthUser>),
    /// The run is refused, with this reason in its log.
    Refused(String),
}

fn principal_of(su: &SystemUser) -> AuthUser {
    AuthUser { eppn: identity_of(&su.flow_id), name: su.name.clone(), role_title: "Automation".to_string(), affiliation: "service".to_string(), department: "System".to_string(), view_as_by: None, via_client: None, shared_link: None }
}

/// Decides whether a flow may run now. The review dates are compared by the database's clock on this call, so a grant stops reaching
/// anything on its date and no job is needed. A run that finds a grant lapsed is refused, and the owner is told once.
pub fn admit(state: &SharedState, rule: &AutomationRule) -> Admission {
    let su = match state.repository.system_user(&rule.id) {
        Ok(Some(su)) => su,
        Ok(None) => return Admission::Open,
        Err(e) => return Admission::Refused(format!("system_user_unavailable: the flow's grants could not be read, so it did not run: {e}")),
    };
    let grants = match state.repository.system_grants(&rule.id) {
        Ok(g) => g,
        Err(e) => return Admission::Refused(format!("system_user_unavailable: the flow's grants could not be read, so it did not run: {e}")),
    };
    let (live, lapsed) = (covers_of(&grants, false), covers_of(&grants, true));
    let missing: Vec<Need> = needs_of(rule).into_iter().filter(|n| !live.contains(n)).collect();
    if missing.is_empty() {
        return Admission::Gated(Box::new(principal_of(&su)));
    }
    let ended: Vec<&Need> = missing.iter().filter(|n| lapsed.contains(n)).collect();
    if !ended.is_empty() {
        tell_owner(state, &su);
        return Admission::Refused(format!(
            "system_user_lapsed: the access of \"{}\" to {} reached its review date, so the flow did not run. Its owner is told. It stays stopped until someone who holds that access approves a grant with a new date",
            su.name,
            said(state, &ended)
        ));
    }
    let all: Vec<&Need> = missing.iter().collect();
    Admission::Refused(format!(
        "system_user_not_granted: \"{}\" has not been granted {}, so the flow did not run. It stays stopped until someone who holds that access approves it",
        su.name,
        said(state, &all)
    ))
}

/// Tells the owner that grants lapsed. The lapse is claimed first, in one statement, so two runs cannot both send it, and a claim whose
/// alert failed is given back so the next run tries again. A position or a group is read now, so the alert follows the seat. When
/// nobody holds it, the owners and admins of the flow's workspace are told.
fn tell_owner(state: &SharedState, su: &SystemUser) {
    let datasets: Vec<String> = match state.repository.claim_lapse_alerts(&su.flow_id) {
        Ok(d) => d.into_iter().collect::<BTreeSet<_>>().into_iter().collect(),
        Err(e) => {
            eprintln!("Warning: the lapse of the grants of {} could not be claimed: {e}", su.flow_id);
            return;
        }
    };
    let Some(first) = datasets.first() else { return };
    let notice = Notice::SystemGrantLapsed { dataset: DatasetRef(first.clone()), others: Count(u32::try_from(datasets.len() - 1).unwrap_or(u32::MAX)) };
    let people = members::people_of_key(&su.owner, state);
    let to = if people.is_empty() {
        let workspace = state.engine.read().ok().and_then(|e| e.resolve_by_slug(&su.app_slug).and_then(|m| m.workspace_id.clone()));
        match workspace {
            Some(workspace) => Recipients::Workspace { workspace, roles: vec!["owner".to_string(), "admin".to_string()] },
            None => Recipients::People(Vec::new()),
        }
    } else {
        Recipients::People(people)
    };
    if let Err(e) = notify(state, &to, None, &notice) {
        eprintln!("Warning: the owner of {} could not be told its grants lapsed: {e}", su.flow_id);
        let _ = state.repository.release_lapse_alerts(&su.flow_id);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Registering, reading and changing
// ---------------------------------------------------------------------------------------------------------------------

fn view(su: &SystemUser, state: &SharedState) -> Result<Value, ServiceError> {
    let grants = state.repository.system_grants(&su.flow_id).map_err(storage)?;
    Ok(json!({
        "flow_id": su.flow_id, "identity": identity_of(&su.flow_id), "app_slug": su.app_slug, "name": su.name, "notes": su.notes,
        "owner": owner_json(&su.owner, state), "created_by": su.created_by, "created_at": su.created_at,
        "grants": grants.iter().map(|g| grant_json(g, state)).collect::<Vec<_>>(),
    }))
}

fn grant_json(g: &SystemGrant, state: &SharedState) -> Value {
    let status = if g.status == "Revoked" { "Revoked" } else if g.lapsed { "Lapsed" } else { "Active" };
    json!({
        "id": g.id.to_string(), "flow_id": g.flow_id, "dataset": g.dataset_id, "app_slug": g.app_slug, "table_id": g.table_id, "action": g.action,
        "fields": g.fields, "reason": g.reason, "approved_by": g.approved_by, "approved_at": g.approved_at, "review_by": g.review_by,
        "status": status, "sentence": sentence(g, state), "revoked_by": g.revoked_by, "revoke_reason": g.revoke_reason,
    })
}

fn load(flow_id: &str, state: &SharedState) -> Result<SystemUser, ServiceError> {
    state.repository.system_user(flow_id).map_err(storage)?.ok_or_else(missing)
}

/// `POST /data/system-users`. Whoever manages the flow's app makes the flow a system user, with an owner. It has no grants yet, so it
/// reads and writes nothing until a person who holds the access approves some.
pub fn register(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    ordinary(caller)?;
    let flow_id = body.get("flow_id").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let rule = flows::find_flow(state, flow_id).ok_or_else(|| ServiceError::NotFound("Flow not found".to_string()))?;
    if authorize_app(caller, &rule.app_slug, AppAction::Manage, state).is_err() {
        return Err(ServiceError::forbidden("Only an owner or an admin of the flow's workspace, or a Platform Admin, makes a flow a system user"));
    }
    if !flows::is_flow(&rule) {
        return Err(bad("Only a flow that finds, creates, updates or links records reads or writes data, so only a flow has a system user"));
    }
    let owner = owner_key(body.get("owner").unwrap_or(&Value::Null), state)?;
    let name = text(body, "name", MAX_NAME)?.unwrap_or_else(|| rule.name.clone());
    let notes = text(body, "notes", MAX_NOTES)?.unwrap_or_default();
    if !state.repository.insert_system_user(&rule.id, &rule.app_slug, &name, &notes, &owner, &caller.eppn).map_err(storage)? {
        return Err(ServiceError::Conflict("This flow is a system user already".to_string()));
    }
    view(&load(&rule.id, state)?, state)
}

fn text(body: &Value, key: &str, most: usize) -> Result<Option<String>, ServiceError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().chars().count() <= most && (key != "name" || !s.trim().is_empty()) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(bad(format!("{key} is text of at most {most} characters{}", if key == "name" { ", and not empty" } else { "" }))),
    }
}

/// `GET /data/system-users/{flow_id}`, for its owner, whoever manages its app, and a Platform Admin.
pub fn get(caller: &AuthUser, flow_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let su = load(flow_id, state)?;
    if !manages(caller, &su, state) {
        return Err(missing());
    }
    view(&su, state)
}

/// `PATCH /data/system-users/{flow_id}`. The owner, or whoever manages the flow's app, changes the name, the notes or the owner. None of
/// these is a decision about data, so none needs an approval and none writes a ledger entry.
pub fn change(caller: &AuthUser, flow_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    ordinary(caller)?;
    let su = load(flow_id, state)?;
    if !manages(caller, &su, state) {
        return Err(ServiceError::forbidden("Only the owner of this automation, or an owner or an admin of its workspace, changes it"));
    }
    let name = text(body, "name", MAX_NAME)?;
    let notes = text(body, "notes", MAX_NOTES)?;
    let owner = body.get("owner").filter(|v| !v.is_null()).map(|v| owner_key(v, state)).transpose()?;
    if name.is_none() && notes.is_none() && owner.is_none() {
        return Err(bad("Name the name, the notes or the owner to change"));
    }
    state.repository.update_system_user(flow_id, name.as_deref(), notes.as_deref(), owner.as_deref(), &caller.eppn).map_err(storage)?;
    view(&load(flow_id, state)?, state)
}

// ---------------------------------------------------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------------------------------------------------

fn ledger(caller: &str, row: &CatalogRow, reason: &str, payload: Value) -> AppendDecisionParams {
    AppendDecisionParams {
        principal: caller.to_string(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: Some(row.app_slug.clone()),
        decision_type: DecisionType::AccessGrantChanged,
        oscal_control_id: "AC-02".to_string(),
        rationale: reason.chars().take(500).collect(),
        payload,
    }
}

fn grant_payload(op: &str, g: &SystemGrant, extra: Value) -> Value {
    let mut p = json!({
        "op": op, "source": "system_grant", "grant": g.id.to_string(), "flow": g.flow_id, "holder": identity_of(&g.flow_id), "dataset": g.dataset_id,
        "access": g.action, "fields": g.fields, "valid_until": g.review_by, "approved_by": g.approved_by,
    });
    if let (Some(base), Some(more)) = (p.as_object_mut(), extra.as_object()) {
        base.extend(more.clone());
    }
    p
}

/// What an access is, in words: `read access to Financial Aid / Awards: amount`.
fn describe(action: &str, place: &str, fields: &[String]) -> String {
    match action {
        "create" => format!("the right to create records in {place}"),
        "write" => format!("write access to {place}: {}", fields.join(", ")),
        _ => format!("read access to {place}: {}", fields.join(", ")),
    }
}

/// Whether the approver holds the access they would grant. They must open the app, reach every row of the table for the operation, and
/// see (or change, for a write) each field, by their own rights. This is the same answer a read or a write of theirs would get, so it
/// cannot be more generous. A rule that narrows them to some rows means they do not hold the table, and cannot grant it.
fn holds(caller: &AuthUser, row: &CatalogRow, action: &str, fields: &[String], state: &SharedState) -> Result<(), ServiceError> {
    let place = flows::place(state, &row.app_slug, &row.table_id);
    let lacks = || {
        ServiceError::forbidden(format!(
            "You do not hold {} yourself. Nobody can grant more access than they hold. Ask someone who holds it to approve this",
            describe(action, &place, fields)
        ))
    };
    let app_action = if action == "read" { AppAction::Read } else { AppAction::WriteRecords };
    if authorize_app(caller, &row.app_slug, app_action, state).is_err() {
        return Err(lacks());
    }
    let op = match action {
        "read" => RowOp::Read,
        "write" => RowOp::Update,
        _ => RowOp::Create,
    };
    if scope::scope_of(caller, &row.app_slug, &row.table_id, op, state) != RowScope::AllowAll {
        return Err(lacks());
    }
    let seen = scope::field_visibility_of(caller, &row.app_slug, &row.table_id, state);
    let closed = |f: &String| {
        seen.hidden.iter().any(|d| &d.field == f)
            || (action == "read" && seen.masked.iter().any(|(d, _)| &d.field == f))
            || (action == "write" && seen.read_only.iter().any(|d| &d.field == f))
    };
    if fields.iter().any(closed) {
        return Err(lacks());
    }
    Ok(())
}

/// The review date of a body: ahead of now, and at most a year away.
fn review_by(body: &Value) -> Result<DateTime<Utc>, ServiceError> {
    let at = body.get("review_by").and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|d| d.with_timezone(&Utc));
    let now = Utc::now();
    match at {
        Some(at) if at > now && at <= now + Duration::days(MAX_AHEAD_DAYS) => Ok(at),
        _ => Err(bad(format!("review_by is a date and time (RFC 3339) ahead of now and at most {MAX_AHEAD_DAYS} days away"))),
    }
}

/// `POST /data/system-users/{flow_id}/grants`. A person approves one grant: a dataset, an action (`read`, `write` or `create`), the
/// fields, a `reason` and a `review_by` date. The approver must hold the access. The grant and one `AccessGrantChanged` ledger entry
/// commit together. A sensitive field is never granted, and a computed field is never written.
pub fn approve(caller: &AuthUser, flow_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    ordinary(caller)?;
    let su = load(flow_id, state)?;
    let reason = reason_of(body)?;
    let dataset = body.get("dataset").and_then(Value::as_str).unwrap_or("");
    let row = state.repository.catalog_row(dataset).map_err(storage)?.ok_or_else(|| bad("dataset is the id of a dataset this server holds"))?;
    if row.published_id.is_some() {
        return Err(bad("A grant names the table of an app. A published dataset is read through its own routes"));
    }
    let action = match body.get("action").and_then(Value::as_str) {
        Some(a @ ("read" | "write" | "create")) => a,
        _ => return Err(bad("action is read, write or create")),
    };
    let mut fields: Vec<String> = match body.get("fields") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list.iter().map(|f| f.as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())).collect::<Option<_>>().ok_or_else(|| bad("fields is a list of field names"))?,
        Some(_) => return Err(bad("fields is a list of field names")),
    };
    fields.sort();
    fields.dedup();
    if action == "create" {
        fields.clear();
    } else if fields.is_empty() || fields.len() > MAX_FIELDS {
        return Err(bad(format!("Name 1 to {MAX_FIELDS} fields")));
    }
    let table_fields = scope::fields_of(&row.app_slug, &row.table_id, state);
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner()).clone();
    for name in &fields {
        let Some(spec) = table_fields.iter().find(|f| &f.name == name) else { return Err(bad(format!("'{}' is not a field of this table", name.chars().take(64).collect::<String>()))) };
        if action == "write" && spec.field_type.is_computed() {
            return Err(bad(format!("'{name}' is computed, so nobody writes it")));
        }
        if effective_sensitivity(spec, labels.get(&(row.app_slug.clone(), row.table_id.clone(), name.clone())), &categories).is_protected() {
            return Err(bad(format!("'{name}' is a sensitive field. An automation is not given a sensitive field")));
        }
    }
    let until = review_by(body)?;
    holds(caller, &row, action, &fields, state)?;
    let grant = SystemGrant {
        id: Uuid::new_v4(),
        flow_id: su.flow_id.clone(),
        dataset_id: row.id.clone(),
        app_slug: row.app_slug.clone(),
        table_id: row.table_id.clone(),
        action: action.to_string(),
        fields: fields.clone(),
        reason: reason.clone(),
        approved_by: caller.eppn.clone(),
        approved_at: String::new(),
        review_by: until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        status: "Active".to_string(),
        lapsed: false,
        revoked_by: None,
        revoke_reason: None,
    };
    let entry = ledger(&caller.eppn, &row, &reason, grant_payload("create", &grant, json!({})));
    let new = NewGrant {
        id: grant.id,
        flow_id: grant.flow_id.clone(),
        dataset_id: grant.dataset_id.clone(),
        app_slug: grant.app_slug.clone(),
        table_id: grant.table_id.clone(),
        action: grant.action.clone(),
        fields,
        reason,
        approved_by: grant.approved_by.clone(),
        review_by: grant.review_by.clone(),
    };
    state.repository.approve_system_grant(new, entry).map_err(storage)?;
    Ok(grant_json(&state.repository.system_grant(grant.id).map_err(storage)?.ok_or_else(missing)?, state))
}

/// `POST /data/system-users/{flow_id}/grants/{grant_id}/revoke`. The owner, or whoever manages the flow's app, ends a grant early with a
/// `reason`. The automation stops reaching what it named at once, and one ledger entry says who and why.
pub fn revoke(caller: &AuthUser, flow_id: &str, grant_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    ordinary(caller)?;
    let su = load(flow_id, state)?;
    if !manages(caller, &su, state) {
        return Err(ServiceError::forbidden("Only the owner of this automation, or an owner or an admin of its workspace, ends a grant"));
    }
    let grant = Uuid::parse_str(grant_id).ok().and_then(|id| state.repository.system_grant(id).ok().flatten()).filter(|g| g.flow_id == flow_id).ok_or_else(|| ServiceError::NotFound("Grant not found".to_string()))?;
    let reason = reason_of(body)?;
    let row = state.repository.catalog_row(&grant.dataset_id).map_err(storage)?.ok_or_else(missing)?;
    let entry = ledger(&caller.eppn, &row, &reason, grant_payload("end", &grant, json!({ "reason": "revoked", "ended_by": caller.eppn })));
    match state.repository.revoke_system_grant(grant.id, &caller.eppn, &reason, entry).map_err(storage)? {
        Revoked::Missing => Err(ServiceError::NotFound("Grant not found".to_string())),
        Revoked::Not(s) => Err(ServiceError::Conflict(format!("This grant is {}, so it cannot be ended now", s.to_lowercase()))),
        Revoked::Done => Ok(grant_json(&state.repository.system_grant(grant.id).map_err(storage)?.ok_or_else(missing)?, state)),
    }
}
