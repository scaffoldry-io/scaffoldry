//! The disclosure log (record-history phase 4).
//!
//! A release of sensitive fields is written down at the moment it happens, so a student's request
//! for their record of disclosures can be answered. The log does not decide what is a disclosure.
//! It records candidates: an export, or a bulk read by an agent (an agent forwards what it reads to
//! a model provider, and whether that is a disclosure is counsel's call). An entry is written only
//! when the released fields include an effective-sensitive field. Nothing else is a candidate.
//!
//! Every function that hands record data to a caller who may be an agent calls `record_agent_read`,
//! or is named in `AGENT_RELEASE_EXEMPT` with its reason. A test reads the source and fails on any
//! that is in neither list.

use crate::repository::ActorKind;
use crate::repository_disclosure::{DisclosureFilter, DisclosureRow, NewDisclosure};
use crate::service::access::{authorize_app, AppAction};
use crate::service::admin::{encode_cursor, decode_cursor, require_platform_admin, require_platform_admin_or_compliance};
use crate::service::history::Origin;
use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, RecordDecisionInput, SharedState};
use scaffoldry_core::DecisionType;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// What a release can be. Later briefs add the writers of `share_link`, `webhook`, `sync_out` and
/// `file_download`. This phase writes `agent_read` and takes `export` from the CSV path when it exists.
pub const KINDS: [&str; 6] = ["export", "share_link", "webhook", "agent_read", "sync_out", "file_download"];

/// An agent read is logged when one call returns more than this many records with a sensitive field.
pub const AGENT_READ_THRESHOLD: usize = 25;

/// The most rows one export of the log holds.
pub const EXPORT_ROW_LIMIT: usize = 50_000;

/// The page size of the admin list.
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 200;

/// Functions that return record data and call `record_agent_read`, as `folder/file.rs::function`.
pub const AGENT_RELEASES: &[&str] = &[
    "routes/records.rs::list_table_records",
    "routes/records.rs::list_records",
    "service/tools.rs::run_list_records",
    "service/tools.rs::run_get_view_data",
    "service/views.rs::run",
];

/// Functions that return record data and do not log it, each with the reason.
pub const AGENT_RELEASE_EXEMPT: &[(&str, &str)] = &[
    ("routes/records.rs::get_record", "Returns one record. One record never passes the threshold, so a single read is not logged."),
    ("service/tools.rs::run_get_record", "Returns one record. One record never passes the threshold, so a single read is not logged."),
    ("service/tools.rs::run_update_record", "Reads one record to apply an update and returns the saved record. One record, below the threshold."),
    (
        "routes/files.rs::download",
        "Returns one file of one record, not a list. A download from a sensitive field is logged as `file_download` by this function.",
    ),
    (
        "routes/trash.rs::list",
        "Returns each deleted record's id and the value of its primary field, not the record. Not logged. Log it if a primary field can be sensitive.",
    ),
    (
        "routes/link_options.rs::options",
        "Returns an id and one display value per row for the link picker, not the record. Not logged. Log it if a display field can be sensitive.",
    ),
];

/// What a release holds. `fields` is every field released. Only the sensitive ones are kept.
pub struct Release<'a> {
    pub kind: &'a str,
    pub actor: &'a str,
    pub app_slug: &'a str,
    pub table_id: Option<&'a str>,
    pub record_ids: Vec<String>,
    pub fields: Vec<String>,
    pub recipient: Option<&'a str>,
    pub note: &'a str,
}

/// Writes an entry when the release includes an effective-sensitive field, and returns its id. A
/// release of nothing sensitive writes nothing and returns `None`. With no database there is no log.
pub fn log_disclosure(state: &SharedState, release: Release<'_>) -> Result<Option<Uuid>, ServiceError> {
    if !KINDS.contains(&release.kind) {
        return Err(ServiceError::BadRequest(format!("'{}' is not a kind of disclosure", release.kind)));
    }
    let sensitive: BTreeSet<String> = match release.table_id {
        Some(table) => crate::service::records::sensitive_fields_of(release.app_slug, table, state).into_iter().collect(),
        None => {
            let tables: Vec<String> = state
                .engine
                .read()
                .ok()
                .and_then(|e| e.resolve_by_slug(release.app_slug).map(|m| m.tables.iter().map(|t| t.id.clone()).collect()))
                .unwrap_or_default();
            tables.iter().flat_map(|t| crate::service::records::sensitive_fields_of(release.app_slug, t, state)).collect()
        }
    };
    let kept: Vec<String> = release.fields.iter().filter(|f| sensitive.contains(*f)).cloned().collect::<BTreeSet<_>>().into_iter().collect();
    if kept.is_empty() {
        return Ok(None);
    }
    let Some(repo) = state.repository.as_deref() else { return Ok(None) };
    repo.insert_disclosure(NewDisclosure {
        kind: release.kind.to_string(),
        actor: release.actor.to_string(),
        app_slug: release.app_slug.to_string(),
        table_id: release.table_id.map(str::to_string),
        record_ids: release.record_ids,
        fields: kept,
        recipient: release.recipient.map(str::to_string),
        note: release.note.to_string(),
    })
    .map(Some)
    .map_err(|e| ServiceError::Internal(format!("The release could not be recorded in the disclosure log: {e}")))
}

/// Called by every function that returns a list of records to a caller who may be an agent, with the
/// records as they are returned (after redaction). It logs a read through an agent token that
/// returns more than `AGENT_READ_THRESHOLD` records holding a sensitive field. A person's session
/// writes nothing. If the entry cannot be written the call fails, so no read goes unrecorded.
pub fn record_agent_read(
    caller: &AuthUser,
    origin: &Origin,
    app_slug: &str,
    table_hint: Option<&str>,
    rows: &[DatasetRecord],
    state: &SharedState,
) -> Result<(), ServiceError> {
    if origin.kind != ActorKind::Agent || rows.len() <= AGENT_READ_THRESHOLD {
        return Ok(());
    }
    let mut by_table: std::collections::BTreeMap<String, Vec<&DatasetRecord>> = std::collections::BTreeMap::new();
    for row in rows {
        let table = row.data.get("_table_id").and_then(Value::as_str).or(table_hint).unwrap_or("").to_string();
        by_table.entry(table).or_default().push(row);
    }
    for (table, group) in by_table {
        let sensitive = crate::service::records::sensitive_fields_of(app_slug, &table, state);
        if sensitive.is_empty() {
            continue;
        }
        let mut present: BTreeSet<String> = BTreeSet::new();
        let mut ids: Vec<String> = Vec::new();
        for row in group {
            let held: Vec<&String> = sensitive.iter().filter(|f| row.data.get(f.as_str()).is_some_and(|v| !v.is_null())).collect();
            if !held.is_empty() {
                present.extend(held.into_iter().cloned());
                ids.push(row.id.clone());
            }
        }
        if ids.len() > AGENT_READ_THRESHOLD {
            log_disclosure(
                state,
                Release {
                    kind: "agent_read",
                    actor: &caller.eppn,
                    app_slug,
                    table_id: Some(&table),
                    record_ids: ids,
                    fields: present.into_iter().collect(),
                    recipient: origin.token_label.as_deref(),
                    note: "read by an agent",
                },
            )?;
        }
    }
    Ok(())
}

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    state.repository.as_deref().ok_or_else(|| ServiceError::Internal("The disclosure log needs PostgreSQL".to_string()))
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn time_of(name: &str, value: Option<String>) -> Result<Option<String>, ServiceError> {
    value
        .map(|v| {
            chrono::DateTime::parse_from_rfc3339(&v)
                .map(|t| t.to_rfc3339())
                .map_err(|_| ServiceError::BadRequest(format!("'{name}' must be a time such as 2026-03-03T14:00:00Z")))
        })
        .transpose()
}

/// The query string of the admin list and its export.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Query {
    pub record: Option<String>,
    pub actor: Option<String>,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    pub format: Option<String>,
}

fn filter_of(q: &Query) -> Result<DisclosureFilter, ServiceError> {
    if let Some(kind) = &q.kind {
        if !KINDS.contains(&kind.as_str()) {
            return Err(ServiceError::BadRequest(format!("'{kind}' is not a kind of disclosure")));
        }
    }
    Ok(DisclosureFilter {
        record: q.record.clone(),
        actor: q.actor.clone(),
        kind: q.kind.clone(),
        app: q.app.clone(),
        from: time_of("from", q.from.clone())?,
        to: time_of("to", q.to.clone())?,
    })
}

/// `GET /admin/disclosures`: every entry that matches, a page at a time.
pub fn list(caller: &AuthUser, q: Query, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let filter = filter_of(&q)?;
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let offset = q.cursor.as_deref().map(decode_cursor).transpose()?.unwrap_or(0);
    let mut rows = repo(state)?
        .list_disclosures(filter, i64::try_from(limit + 1).unwrap_or(i64::MAX), i64::try_from(offset).unwrap_or(0))
        .map_err(internal)?;
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(json!({ "entries": rows, "next_cursor": more.then(|| encode_cursor(offset + limit)) }))
}

/// `GET /apps/{slug}/records/{id}/disclosures`: every entry that lists this record, newest first.
/// An app owner or admin, a Platform Admin, and compliance may read it. Anyone else gets 404, so a
/// stranger cannot learn that the record was released.
pub fn for_record(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let allowed = caller.affiliation == "compliance"
        || require_platform_admin(caller, state).is_ok()
        || authorize_app(caller, app_slug, AppAction::Manage, state).is_ok();
    if !allowed {
        return Err(ServiceError::NotFound("Record not found".to_string()));
    }
    let rows: Vec<DisclosureRow> = repo(state)?.record_disclosures(app_slug, record_id).map_err(internal)?;
    Ok(json!({ "app_slug": app_slug, "record_id": record_id, "disclosures": rows }))
}

const EXPORT_HEADERS: [&str; 11] =
    ["id", "at", "actor", "kind", "app_slug", "table_id", "record_count", "fields", "recipient", "note", "records_truncated"];

/// What an export of the log is: its media type, its body, and whether it stopped at the cap.
pub struct Exported {
    pub content_type: &'static str,
    pub body: String,
    pub truncated: bool,
}

/// `GET /admin/disclosures/export`: CSV or JSON, at most `EXPORT_ROW_LIMIT` entries. The export is
/// itself a ledger entry, written before anything is returned.
pub fn export(caller: &AuthUser, q: Query, state: &SharedState) -> Result<Exported, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let as_json = match q.format.as_deref() {
        None | Some("csv") => false,
        Some("json") => true,
        Some(other) => return Err(ServiceError::BadRequest(format!("'{other}' is not a format. Use csv or json"))),
    };
    let filter = filter_of(&q)?;
    let mut rows = repo(state)?.list_disclosures(filter, i64::try_from(EXPORT_ROW_LIMIT + 1).unwrap_or(i64::MAX), 0).map_err(internal)?;
    let truncated = rows.len() > EXPORT_ROW_LIMIT;
    rows.truncate(EXPORT_ROW_LIMIT);

    let payload = json!({
        "rows": rows.len(),
        "truncated": truncated,
        "format": if as_json { "json" } else { "csv" },
        "filters": { "record": q.record, "actor": q.actor, "kind": q.kind, "app": q.app, "from": q.from, "to": q.to },
    });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: None,
            decision_type: DecisionType::DisclosureLogExported,
            oscal_control_id: "AU-02".to_string(),
            rationale: format!("{} exported {} entries of the disclosure log", caller.name, rows.len()),
            payload: &payload,
        })
        .map_err(|e| ServiceError::Internal(format!("Could not record the ledger entry: {e}")))?;

    if as_json {
        let body = json!({ "entries": rows, "truncated": truncated }).to_string();
        return Ok(Exported { content_type: "application/json", body, truncated });
    }
    let values: Vec<Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap_or(Value::Null)).collect();
    Ok(Exported { content_type: "text/csv; charset=utf-8", body: scaffoldry_engine::export_records_to_csv(&EXPORT_HEADERS, &values), truncated })
}
