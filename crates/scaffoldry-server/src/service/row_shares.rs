//! Data layer phase 10: row shares (`docs/plans/data-layer.md`, "Sharing a row").
//!
//! - **A share** names rows of one table, one person, what they may do (read, or read and write), the fields it covers, a justification
//!   and an expiry. The grantee is a person. A position or a unit is not a grantee in version 1.
//! - **The three cases.** A share inside what the grantee already reads (and, for a write, already writes) needs nothing and is not stored.
//!   A share beyond it needs a justification and an expiry, and a steward approves. A share of a row whose label asks something the grantee
//!   does not meet is restricted: the approver must meet it too. Nobody approves their own request, and the grantee never approves theirs.
//! - **The assessment.** The server calls no model. It stores a deterministic pre-check on the request: counts, field names, category ids,
//!   the days asked against the setting `access.request_max_days`, and what is unusual. It has a slot for a narrative an agent or a
//!   person adds later. It holds no record value, and it never approves or refuses.
//! - **The read.** An approved share that has not reached its date is one more way into the rule. `scope::plan_reading` adds it to a list
//!   or a count, `scope::share_cover` to a single row and to a write. It reads a small lookup keyed by the grantee and the row, and checks
//!   the date itself, so a share closes on its date with no job. See `repository::ScopePlan::with_share`.
//! - **The end.** The job `end_row_shares` ends each approved share whose date has come, writes its ledger entry and tells the owners.
//!   A share ended early by a person writes the same entry.
//!
//! Not built: a position or a unit as grantee, and the narrative of the assessment.

use crate::notifications::{notify, Count, DatasetRef, Flag, Notice, PersonRef, Recipients};
use crate::repository::AppendDecisionParams;
use crate::repository_catalog::CatalogRow;
use crate::repository_row_shares::{Decided, Ended};
use crate::service::access::{authorize_app, AppAction};
use crate::service::label_changes::{is_admin_or_compliance, may_decide, reason_of, refused, stewards_of};
use crate::service::scope::{self, Reach};
use crate::service::{row_labels, ServiceError};
use crate::state::{AuthUser, DatasetRecord, SharedState};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use scaffoldry_core::access::RowOp;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_data::shares::{NewShare, Share};
use scaffoldry_engine::{effective_label, effective_sensitivity, HostRouter};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The most rows one share names, and the most fields.
pub const MAX_ROWS: usize = 100;
const MAX_FIELDS: usize = 50;
/// The furthest ahead a share may end, in days. The setting `access.request_max_days` is the length the assessment calls too long.
const MAX_AHEAD_DAYS: i64 = 366;
/// A share of more rows than this is called unusual.
const MANY_ROWS: usize = 25;
/// A justification shorter than this is called short.
const SHORT_REASON: usize = 20;
const LISTED: i64 = 200;
const BATCH: i64 = 50;
const JOB_ACTOR: &str = "system";

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(&Utc))
}

fn missing() -> ServiceError {
    ServiceError::NotFound("Share not found".to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// Who
// ---------------------------------------------------------------------------------------------------------------------

/// The workspace's owners and admins: the people who own the rows of an app.
fn owners_of(app: &str, state: &SharedState) -> BTreeSet<String> {
    let workspace = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine.resolve_by_slug(app).and_then(|m| m.workspace_id.clone())
    };
    let Some(ws) = workspace else { return BTreeSet::new() };
    crate::service::members::people_with_roles(&ws, state).into_iter().filter(|(_, r)| *r >= crate::service::members::Role::Admin).map(|(p, _)| p.to_lowercase()).collect()
}

/// Whether the caller owns the rows of this dataset: an owner or an admin of its workspace, its steward, a Platform Admin or a compliance
/// officer.
fn owns(caller: &AuthUser, row: &CatalogRow, state: &SharedState) -> bool {
    may_decide(caller, row, state) || owners_of(&row.app_slug, state).contains(&caller.eppn.to_lowercase())
}

// ---------------------------------------------------------------------------------------------------------------------
// What is asked
// ---------------------------------------------------------------------------------------------------------------------

/// A request read and checked: the rows, the grantee and what they may do.
struct Ask {
    row: CatalogRow,
    records: Vec<DatasetRecord>,
    grantee: AuthUser,
    write: bool,
    fields: Vec<String>,
}

fn strings(body: &Value, key: &str, most: usize) -> Result<Vec<String>, ServiceError> {
    let Some(list) = body.get(key).and_then(Value::as_array) else { return Err(bad(format!("{key} is a list of text"))) };
    let mut out: Vec<String> = list.iter().map(|s| s.as_str().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())).collect::<Option<_>>().ok_or_else(|| bad(format!("{key} is a list of text")))?;
    out.sort();
    out.dedup();
    if out.is_empty() || out.len() > most {
        return Err(bad(format!("Name 1 to {most} in `{key}`")));
    }
    Ok(out)
}

/// The rows of the dataset the body names, live and of its own table.
fn rows_of(row: &CatalogRow, ids: &[String], state: &SharedState) -> Result<Vec<DatasetRecord>, ServiceError> {
    let mut out = Vec::new();
    for id in ids {
        let found = state.find_record(&row.app_slug, id).map_err(ServiceError::Internal)?;
        match found {
            Some(r) if r.data.get("_table_id").and_then(Value::as_str) == Some(row.table_id.as_str()) => out.push(r),
            _ => return Err(bad(format!("'{}' is not a live row of this dataset", id.chars().take(64).collect::<String>()))),
        }
    }
    Ok(out)
}

/// The dataset, and the rows, grantee and fields of a body, checked. The requester must read what they share, and write it to share a write.
fn ask_of(caller: &AuthUser, dataset: &str, body: &Value, state: &SharedState) -> Result<Ask, ServiceError> {
    let row = state.repository.catalog_row(dataset).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("Dataset not found".to_string()))?;
    // A person who cannot open the app is told no more than a person who asks for a dataset that does not exist.
    if row.published_id.is_some() || authorize_app(caller, &row.app_slug, AppAction::Read, state).is_err() {
        return Err(ServiceError::NotFound("Dataset not found".to_string()));
    }
    let ids = strings(body, "records", MAX_ROWS)?;
    let fields = strings(body, "fields", MAX_FIELDS)?;
    let write = match body.get("access").and_then(Value::as_str) {
        Some("read") => false,
        Some("write") => true,
        _ => return Err(bad("access is read or write")),
    };
    let who = body.get("grantee").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let grantee = (!who.is_empty()).then(|| crate::service::identity::resolve_user(who, state)).flatten().ok_or_else(|| bad("grantee is the address of a person this server knows"))?;
    if grantee.eppn.eq_ignore_ascii_case(&caller.eppn) {
        return Err(bad("A share is for another person. You already read what you read"));
    }
    if grantee.affiliation == "guest" {
        return Err(bad("A guest reads only the record they made, so no share is for a guest"));
    }
    let table_fields = scope::fields_of(&row.app_slug, &row.table_id, state);
    for name in &fields {
        let Some(spec) = table_fields.iter().find(|f| &f.name == name) else { return Err(bad(format!("'{}' is not a field of this table", name.chars().take(64).collect::<String>()))) };
        if write && spec.field_type.is_computed() {
            return Err(bad(format!("'{name}' is computed, so nobody writes it")));
        }
    }
    let visibility = scope::field_visibility_of(caller, &row.app_slug, &row.table_id, state);
    if let Some(name) = fields.iter().find(|f| visibility.hidden.iter().any(|d| &d.field == *f)) {
        return Err(bad(format!("You cannot share '{name}' because you cannot see it")));
    }
    if write {
        if let Some(name) = fields.iter().find(|f| visibility.read_only.iter().any(|d| &d.field == *f)) {
            return Err(bad(format!("You cannot share the right to write '{name}' because you cannot change it")));
        }
    }
    let records = rows_of(&row, &ids, state)?;
    // A person cannot hand on what they cannot reach. A row they may not read is the same answer as a row that is not there.
    for r in &records {
        let op = if write { RowOp::Update } else { RowOp::Read };
        if !scope::allows(caller, &row.app_slug, r, op, state) || !scope::allows(caller, &row.app_slug, r, RowOp::Read, state) {
            return Err(ServiceError::NotFound(format!("Record '{}' not found", r.id)));
        }
    }
    // A share reaches rows through the app, so the grantee must be able to open it. A share gives no seat in an app.
    if authorize_app(&grantee, &row.app_slug, AppAction::Read, state).is_err() {
        return Err(ServiceError::Conflict("This person cannot open the app, and a share does not give access to an app. Ask for them to be given a place in it first".to_string()));
    }
    Ok(Ask { row, records, grantee, write, fields })
}

/// The reach of the grantee over each row, for what the share lets them do, by their own rights alone, and the tally.
#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    open: usize,
    beyond: usize,
    restricted: usize,
}

fn tally_of(grantee: &AuthUser, app: &str, records: &[DatasetRecord], write: bool, state: &SharedState) -> Tally {
    let can_write_app = !write || authorize_app(grantee, app, AppAction::WriteRecords, state).is_ok();
    let mut t = Tally::default();
    for r in records {
        let mut reach = scope::reach_of(grantee, app, r, RowOp::Read, state);
        if write && reach == Reach::Open {
            reach = scope::reach_of(grantee, app, r, RowOp::Update, state);
        }
        if write && reach == Reach::Open && !can_write_app {
            reach = Reach::Beyond;
        }
        match reach {
            Reach::Open => t.open += 1,
            Reach::Beyond => t.beyond += 1,
            Reach::Restricted => t.restricted += 1,
        }
    }
    t
}

impl Tally {
    fn kind(self) -> Option<&'static str> {
        if self.restricted > 0 {
            Some("restricted")
        } else if self.beyond > 0 {
            Some("beyond")
        } else {
            None
        }
    }
}

/// The categories the named rows and fields carry: the dataset's floor, the rows' tags and the categories of the fields shared.
fn categories_touched(row: &CatalogRow, records: &[DatasetRecord], fields: &[String], state: &SharedState) -> Vec<String> {
    let categories = state.sensitivity_categories();
    let floor = {
        let (app, table) = (row.app_slug.clone(), row.table_id.clone());
        state.repository.with_client(move |c| Ok(scaffoldry_data::labels::floor_of(c, &app, &table)?)).unwrap_or_default()
    };
    let specs = scope::fields_of(&row.app_slug, &row.table_id, state);
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    let sens: Vec<_> = specs
        .iter()
        .filter(|f| fields.contains(&f.name))
        .map(|f| effective_sensitivity(f, labels.get(&(row.app_slug.clone(), row.table_id.clone(), f.name.clone())), &categories))
        .collect();
    let mut out: BTreeSet<String> = effective_label(&floor, &[], &sens, &categories).categories;
    for r in records {
        out.extend(scaffoldry_data::gate::tags_of(&r.data));
    }
    out.into_iter().collect()
}

/// The fields of the share that are protected by themselves.
fn protected_fields(row: &CatalogRow, fields: &[String], state: &SharedState) -> Vec<String> {
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    scope::fields_of(&row.app_slug, &row.table_id, state)
        .iter()
        .filter(|f| fields.contains(&f.name))
        .filter(|f| effective_sensitivity(f, labels.get(&(row.app_slug.clone(), row.table_id.clone(), f.name.clone())), &categories).protected)
        .map(|f| f.name.clone())
        .collect()
}

/// The longest share the setting allows, in days.
fn max_days(state: &SharedState) -> i64 {
    crate::service::access_requests::max_days(state)
}

/// The server's pre-check (`row-share-assessment.schema.json`). It reads the request and counts. It calls no model, holds no record value,
/// and never approves or refuses.
fn assess(ask: &Ask, tally: Tally, justification: &str, expires: DateTime<Utc>, now: DateTime<Utc>, categories: &[String], state: &SharedState) -> Value {
    let days = ((expires - now).num_seconds() + 86_399).div_euclid(86_400).max(0);
    let longest = max_days(state);
    let protected = protected_fields(&ask.row, &ask.fields, state);
    let every_field = {
        let all: BTreeSet<String> = scope::fields_of(&ask.row.app_slug, &ask.row.table_id, state).into_iter().map(|f| f.name).collect();
        all.iter().all(|f| ask.fields.contains(f))
    };
    let mut unusual: Vec<&str> = Vec::new();
    if ask.write {
        unusual.push("write_access");
    }
    if tally.restricted > 0 {
        unusual.push("restricted_category");
    }
    if days > longest {
        unusual.push("longer_than_setting");
    }
    if ask.records.len() > MANY_ROWS {
        unusual.push("many_rows");
    }
    if every_field {
        unusual.push("covers_every_field");
    }
    if !protected.is_empty() {
        unusual.push("protected_field");
    }
    if justification.chars().count() < SHORT_REASON {
        unusual.push("short_justification");
    }
    json!({
        "version": 1,
        "source": "server_precheck",
        "reason": { "fits": "not_assessed", "characters": justification.chars().count(), "short": justification.chars().count() < SHORT_REASON },
        "exposure": {
            "rows": ask.records.len(), "rows_inside_default": tally.open, "rows_beyond_default": tally.beyond, "rows_restricted": tally.restricted,
            "fields": ask.fields, "access": if ask.write { "write" } else { "read" }, "categories": categories, "protected_fields": protected,
        },
        "duration": { "days": days, "max_days": longest, "too_long": days > longest },
        "unusual": unusual,
        "narrative": null,
        "advisory": "These are the server's own checks, with no model. They are advice. A person decides.",
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------------------------------------------------

fn share_json(s: &Share) -> Value {
    json!({
        "id": s.id, "dataset_id": s.dataset_id, "status": s.status, "kind": s.kind, "grantee": s.grantee, "access": s.access, "fields": s.fields,
        "records": s.records, "categories": s.categories, "justification": s.justification, "requested_by": s.requested_by, "requested_at": s.requested_at,
        "expires_at": s.expires_at, "assessment": s.assessment, "decided_by": s.decided_by, "decided_at": s.decided_at, "decision_note": s.decision_note, "ended_at": s.ended_at,
    })
}

/// `POST /data/datasets/{id}/row-shares`. Returns whether a share was stored, and the answer.
///
/// A share inside what the grantee already does is not stored: `stored` is false and `kind` is `default`. Any other share needs a
/// justification (1 to 500 characters) and `expires_at`, in the future and at most a year ahead. It is stored as `Requested` with the
/// server's assessment, and the dataset's stewards are told. It opens nothing until a person approves it.
pub fn request(caller: &AuthUser, dataset: &str, body: &Value, state: &SharedState) -> Result<(bool, Value), ServiceError> {
    let ask = ask_of(caller, dataset, body, state)?;
    let tally = tally_of(&ask.grantee, &ask.row.app_slug, &ask.records, ask.write, state);
    let Some(kind) = tally.kind() else {
        return Ok((false, json!({ "stored": false, "kind": "default", "why": "This person already reads this, so a share is not needed. Nothing was stored and nothing changed." })));
    };
    let justification = body.get("justification").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if justification.is_empty() || justification.chars().count() > 500 {
        return Err(bad("This share goes beyond what the person reads now. It needs a justification of 1 to 500 characters"));
    }
    let now = Utc::now();
    let expires = body.get("expires_at").and_then(Value::as_str).and_then(parse_time).ok_or_else(|| bad("This share goes beyond what the person reads now. It needs expires_at, a date and time such as 2026-12-30T09:00:00Z"))?;
    if expires <= now {
        return Err(bad("expires_at is in the past"));
    }
    if expires > now + Duration::days(MAX_AHEAD_DAYS) {
        return Err(bad(format!("expires_at is more than {MAX_AHEAD_DAYS} days ahead. Choose an earlier date")));
    }
    let categories = categories_touched(&ask.row, &ask.records, &ask.fields, state);
    let assessment = assess(&ask, tally, justification, expires, now, &categories, state);
    let id = Uuid::new_v4();
    let ids: Vec<String> = ask.records.iter().map(|r| r.id.clone()).collect();
    state
        .repository
        .insert_row_share(NewShare {
            id,
            dataset_id: &ask.row.id,
            app_slug: &ask.row.app_slug,
            table_id: &ask.row.table_id,
            records: &ids,
            grantee: &ask.grantee.eppn,
            access: if ask.write { "write" } else { "read" },
            fields: &ask.fields,
            kind,
            categories: &categories,
            justification,
            requested_by: &caller.eppn,
            expires_at: &rfc3339(expires),
            assessment: &assessment,
        })
        .map_err(storage)?;
    let waiting: Vec<String> = stewards_of(&ask.row, state).into_iter().filter(|p| !p.eq_ignore_ascii_case(&caller.eppn) && !p.eq_ignore_ascii_case(&ask.grantee.eppn)).collect();
    let notice = Notice::AccessRequestWaiting { by: PersonRef(caller.eppn.clone()), dataset: DatasetRef(ask.row.id.clone()), reminder: Flag(false) };
    if let Err(e) = notify(state, &Recipients::People(waiting), Some(&caller.eppn), &notice) {
        eprintln!("Warning: the stewards of a row share could not be told: {e}");
    }
    let stored = state.repository.row_share(id).map_err(storage)?.ok_or_else(|| ServiceError::Internal("The share was not stored".to_string()))?;
    let mut out = share_json(&stored);
    out["next"] = json!(format!("A steward other than you and the grantee decides it: POST /api/v1/data/row-shares/{id}/decide. Nothing opens before then."));
    Ok((true, out))
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------------------------------

/// `GET /data/datasets/{id}/row-shares`: the live shares on the dataset's rows, who asked for each and who approved it. `status` reads
/// another list, such as the requests that wait. `record` keeps the shares that name one row. For the dataset's owners and stewards.
pub fn list(caller: &AuthUser, dataset: &str, query: &std::collections::HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    let row = state.repository.catalog_row(dataset).map_err(storage)?;
    let Some(row) = row.filter(|r| owns(caller, r, state)) else {
        return Err(ServiceError::forbidden("Only the owners of the rows, the dataset's steward, a Platform Admin or a compliance officer list its shares"));
    };
    let status = match query.get("status").map(String::as_str) {
        None | Some("Live") => None,
        Some(s @ ("Requested" | "Approved" | "Declined" | "Withdrawn" | "Ended" | "Lapsed")) => Some(s.to_string()),
        Some(_) => return Err(bad("status is Live, Requested, Approved, Declined, Withdrawn, Ended or Lapsed")),
    };
    let shares = state.repository.row_shares_of(&row.id, status, query.get("record").cloned(), LISTED).map_err(storage)?;
    Ok(json!({ "shares": shares.iter().map(share_json).collect::<Vec<_>>() }))
}

/// `GET /data/row-shares/{id}`, for the person who asked, the grantee and the owners of the rows.
pub fn get(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let share = Uuid::parse_str(id).ok().and_then(|id| state.repository.row_share(id).ok().flatten()).ok_or_else(missing)?;
    let row = state.repository.catalog_row(&share.dataset_id).map_err(storage)?.ok_or_else(missing)?;
    let party = share.requested_by.eq_ignore_ascii_case(&caller.eppn) || share.grantee.eq_ignore_ascii_case(&caller.eppn);
    if !party && !owns(caller, &row, state) {
        return Err(missing());
    }
    Ok(share_json(&share))
}

// ---------------------------------------------------------------------------------------------------------------------
// Deciding
// ---------------------------------------------------------------------------------------------------------------------

fn load(caller: &AuthUser, id: &str, state: &SharedState) -> Result<(Share, CatalogRow), ServiceError> {
    let admin = is_admin_or_compliance(caller, state);
    let gone = || if admin { missing() } else { refused() };
    let share = Uuid::parse_str(id).ok().and_then(|id| state.repository.row_share(id).ok().flatten()).ok_or_else(gone)?;
    let row = state.repository.catalog_row(&share.dataset_id).map_err(storage)?.ok_or_else(gone)?;
    if !may_decide(caller, &row, state) {
        return Err(ServiceError::forbidden("Only the dataset's steward, a Platform Admin or a compliance officer decides a row share"));
    }
    Ok((share, row))
}

fn share_ledger(caller: &str, row: &CatalogRow, decision_type: DecisionType, control: &str, rationale: &str, payload: Value) -> AppendDecisionParams {
    AppendDecisionParams {
        principal: caller.to_string(),
        organization_code: "DIV-ADMIN-CONSOLE".to_string(),
        app_slug: Some(row.app_slug.clone()),
        decision_type,
        oscal_control_id: control.to_string(),
        rationale: rationale.chars().take(500).collect(),
        payload,
    }
}

fn grant_payload(op: &str, share: &Share, extra: Value) -> Value {
    let mut p = json!({
        "op": op, "source": "row_share", "share": share.id.to_string(), "dataset": share.dataset_id, "holder": share.grantee, "access": share.access,
        "rows": share.records.len(), "fields": share.fields, "kind": share.kind, "requested_by": share.requested_by, "valid_until": share.expires_at,
    });
    if let (Some(base), Some(more)) = (p.as_object_mut(), extra.as_object()) {
        base.extend(more.clone());
    }
    p
}

/// `POST /data/row-shares/{id}/decide`. The dataset's steward, a Platform Admin or a compliance officer approves or declines a request with
/// `decision` and a `reason`. Nobody decides a request they made, and the grantee never decides theirs. For a share of a row with a
/// restricted category, the approver must meet the requirement of every category the rows carry. An approval opens the rows and writes
/// one `AccessGrantChanged` entry. A decline writes `AccessRequestDecided`.
pub fn decide(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (share, row) = load(caller, id, state)?;
    let reason = reason_of(body)?;
    let approve = match body.get("decision").and_then(Value::as_str) {
        Some("approve") => true,
        Some("decline") => false,
        _ => return Err(bad("decision is approve or decline")),
    };
    if share.status != "Requested" {
        return Err(ServiceError::Conflict(format!("This share is {}, so it cannot be decided now", share.status.to_lowercase())));
    }
    if share.requested_by.eq_ignore_ascii_case(&caller.eppn) || share.grantee.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("Nobody decides a share they asked for or that is for them. A second person must"));
    }
    if !approve {
        let ledger = share_ledger(
            &caller.eppn,
            &row,
            DecisionType::AccessRequestDecided,
            "AC-03",
            &reason,
            json!({ "outcome": "Declined", "source": "row_share", "share": share.id.to_string(), "dataset": share.dataset_id, "requester": share.requested_by, "grantee": share.grantee }),
        );
        return match state.repository.close_row_share_request(share.id, "Declined", &caller.eppn, &reason, Some(ledger)).map_err(storage)? {
            Decided::Missing => Err(missing()),
            Decided::Not(s) => Err(ServiceError::Conflict(format!("This share is {}, so it cannot be decided now", s.to_lowercase()))),
            Decided::Done => Ok(share_json(&state.repository.row_share(share.id).map_err(storage)?.ok_or_else(missing)?)),
        };
    }
    let expires = parse_time(&share.expires_at).ok_or_else(|| ServiceError::Internal("The share has no date".to_string()))?;
    if expires <= Utc::now() {
        return Err(ServiceError::Conflict("This share has already run out, so it cannot be approved".to_string()));
    }
    // The rows are read again. A row that is gone cannot be shared, and a row that has since gained a restricted label needs the stricter approval.
    let records = rows_of(&row, &share.records, state).map_err(|_| ServiceError::Conflict("A row of this share no longer exists. Decline it and ask again".to_string()))?;
    let grantee = crate::service::identity::resolve_user(&share.grantee, state).ok_or_else(|| ServiceError::Conflict("The person this share is for is no longer known".to_string()))?;
    let tally = tally_of(&grantee, &row.app_slug, &records, share.access == "write", state);
    if tally.restricted > 0 || share.kind == "restricted" {
        let authorized = records.iter().all(|r| row_labels::admits(caller, &row.app_slug, &row.table_id, &r.data, state));
        if !authorized {
            return Err(ServiceError::forbidden("A row of this share carries a category that asks something of its readers. Only a steward who meets that requirement approves it"));
        }
    }
    let ledger = share_ledger(&caller.eppn, &row, DecisionType::AccessGrantChanged, "AC-02", &reason, grant_payload("create", &share, json!({ "approved_by": caller.eppn })));
    match state.repository.approve_row_share(share.id, &caller.eppn, &reason, ledger).map_err(storage)? {
        Decided::Missing => Err(missing()),
        Decided::Not(s) => Err(ServiceError::Conflict(format!("This share is {}, so it cannot be decided now", s.to_lowercase()))),
        Decided::Done => Ok(share_json(&state.repository.row_share(share.id).map_err(storage)?.ok_or_else(missing)?)),
    }
}

/// `POST /data/row-shares/{id}/withdraw`. The person who asked, or an owner of the rows, ends a request or a live share early, with a
/// `reason`. A live share closes at once and writes the same ledger entry the date writes.
pub fn withdraw(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let share = Uuid::parse_str(id).ok().and_then(|id| state.repository.row_share(id).ok().flatten()).ok_or_else(missing)?;
    let row = state.repository.catalog_row(&share.dataset_id).map_err(storage)?.ok_or_else(missing)?;
    let mine = share.requested_by.eq_ignore_ascii_case(&caller.eppn);
    if !mine && !owns(caller, &row, state) {
        return Err(ServiceError::forbidden("Only the person who asked, or an owner of the rows, ends a share"));
    }
    let reason = reason_of(body)?;
    let conflict = |s: &str| ServiceError::Conflict(format!("This share is {}, so it cannot be withdrawn", s.to_lowercase()));
    match share.status.as_str() {
        "Requested" => match state.repository.close_row_share_request(share.id, "Withdrawn", &caller.eppn, &reason, None).map_err(storage)? {
            Decided::Done => {}
            Decided::Missing => return Err(missing()),
            Decided::Not(s) => return Err(conflict(&s)),
        },
        "Approved" => {
            let ledger = share_ledger(&caller.eppn, &row, DecisionType::AccessGrantChanged, "AC-02", &reason, grant_payload("end", &share, json!({ "reason": "withdrawn", "ended_by": caller.eppn })));
            match state.repository.end_row_share(share.id, "Withdrawn", Some(&caller.eppn), &reason, ledger).map_err(storage)? {
                Ended::Done => {}
                Ended::Missing => return Err(missing()),
                Ended::Not(s) => return Err(conflict(&s)),
            }
        }
        other => return Err(conflict(other)),
    }
    Ok(share_json(&state.repository.row_share(share.id).map_err(storage)?.ok_or_else(missing)?))
}

// ---------------------------------------------------------------------------------------------------------------------
// The end
// ---------------------------------------------------------------------------------------------------------------------

/// Ends one approved share that has reached its date: the status, the removal of its lookup rows and one ledger entry commit together.
fn end_one(state: &SharedState, id: Uuid) -> Result<bool, String> {
    let storage = |e: crate::repository::RepositoryError| e.to_string();
    let Some(share) = state.repository.row_share(id).map_err(storage)? else { return Ok(false) };
    let Some(row) = state.repository.catalog_row(&share.dataset_id).map_err(storage)? else { return Ok(false) };
    let ledger = share_ledger(JOB_ACTOR, &row, DecisionType::AccessGrantChanged, "AC-02", "The share reached its date", grant_payload("end", &share, json!({ "reason": "expired" })));
    Ok(matches!(state.repository.end_row_share(id, "Ended", None, "Reached its date", ledger).map_err(storage)?, Ended::Done))
}

/// Tells the owners of an ended share: the person who asked and the owners and admins of the app. A notice names the dataset and a count.
fn tell_owners(state: &SharedState, share: &Share) -> Result<(), String> {
    let mut people: BTreeSet<String> = owners_of(&share.app_slug, state);
    people.insert(share.requested_by.to_lowercase());
    let notice = Notice::RowShareEnded { dataset: DatasetRef(share.dataset_id.clone()), rows: Count(u32::try_from(share.records.len()).unwrap_or(u32::MAX)) };
    notify(state, &Recipients::People(people.into_iter().collect()), None, &notice).map_err(|e| e.to_string())?;
    state.repository.mark_row_share_notified(share.id).map_err(|e| e.to_string())
}

/// Ends each approved share whose date is `now` or earlier, closes the requests nobody decided by their date, and tells the owners of each
/// share that ended. Returns how many shares it ended. Access never waits for this: a read checks the date itself. The time is an argument,
/// so a test never waits for a date.
pub fn end_due(state: &SharedState, now: DateTime<Utc>) -> Result<usize, String> {
    let at = rfc3339(now);
    state.repository.lapse_row_shares(&at).map_err(|e| e.to_string())?;
    let mut ended = 0;
    for id in state.repository.due_row_shares(&at, BATCH).map_err(|e| e.to_string())? {
        match end_one(state, id) {
            Ok(true) => ended += 1,
            Ok(false) => {}
            Err(e) => eprintln!("Warning: row share {id} could not be ended on its date: {e}"),
        }
    }
    // A notice that failed is tried again on the next run, because it is recorded only when it was made.
    for share in state.repository.unnotified_row_shares(BATCH).map_err(|e| e.to_string())? {
        if let Err(e) = tell_owners(state, &share) {
            eprintln!("Warning: the owners of row share {} could not be told it ended: {e}", share.id);
        }
    }
    Ok(ended)
}

/// The `end_row_shares` job, run every minute.
pub fn end_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("ending a share on its date needs the server's state, and this runner has none".to_string()))?;
    let ended = end_due(state, Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "ended": ended }))
}
