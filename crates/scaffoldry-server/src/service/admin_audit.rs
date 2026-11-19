//! Admin console phase 7: Audit and security.
//!
//! The ledger is read a page at a time by the database, newest first, with the payload each entry was stored with. One
//! action walks the whole chain and says where it first breaks. An export reads at most 50,000 entries. Tokens are listed
//! without their hash, and a revoke writes its ledger entry first. The agent controls are two settings that only take
//! capability away: tools that may not run, and custom pages that may not run.

use crate::repository_audit::{LedgerFilter, LedgerRow, TokenFilter};
use crate::service::admin::{admin_write, require_platform_admin, require_platform_admin_or_compliance, require_reason};
use crate::service::admin_processes::{page_args, page_of};
use crate::service::tools::{Scope, TOOLS};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::{catalog, LedgerEntry};
use scaffoldry_engine::{PageGrant, PageKind};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The most entries one export holds.
pub const EXPORT_LIMIT: i64 = 50_000;
/// What a tool call answers when an administrator turned the tool off.
pub const TOOL_DISABLED: &str = "Tool disabled by administrator";
/// What a person sees where a page that an administrator turned off would have been.
pub const PAGE_DISABLED: &str = "This page was disabled by an administrator.";

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

// -------------------------------------------------------------------------------------------------------------------
// Agent controls
// -------------------------------------------------------------------------------------------------------------------

fn setting_strings(state: &SharedState, key: &str) -> Vec<String> {
    state
        .settings
        .read()
        .ok()
        .and_then(|s| s.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// Whether an administrator turned this tool off (`mcp.disabled_tools`).
pub fn tool_disabled(state: &SharedState, name: &str) -> bool {
    setting_strings(state, "mcp.disabled_tools").iter().any(|t| t == name)
}

/// Whether an administrator turned this custom page off (`pages.disabled`, entries `app_slug/page_id`).
pub fn page_disabled(state: &SharedState, slug: &str, page_id: &str) -> bool {
    let key = format!("{slug}/{page_id}");
    setting_strings(state, "pages.disabled").contains(&key)
}

fn scope_name(scope: &Scope) -> &'static str {
    match scope {
        Scope::SignedIn => "signed_in",
        Scope::App(_) => "app",
        Scope::Workspace(_) => "workspace",
        Scope::Proposal(_) => "proposal",
        Scope::PlatformAdmin => "platform_admin",
    }
}

/// `GET /admin/tools`: every tool in the registry, with whether an administrator turned it off.
pub fn tools(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let off = setting_strings(state, "mcp.disabled_tools");
    let rows: Vec<Value> = TOOLS
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "read_only": t.read_only, "scope": scope_name(&t.scope), "disabled": off.iter().any(|n| n == t.name) }))
        .collect();
    Ok(json!({ "tools": rows }))
}

fn list_words(items: &[String], none: &str) -> String {
    if items.is_empty() {
        none.to_string()
    } else {
        items.join(", ")
    }
}

/// What a page may do, in words.
pub fn grants_sentence(grants: &[PageGrant]) -> String {
    if grants.is_empty() {
        return "May use no table.".to_string();
    }
    grants
        .iter()
        .map(|g| {
            let mut s = format!("In {}: reads {}, writes {}", g.table_id, list_words(&g.read, "nothing"), list_words(&g.write, "nothing"));
            s.push_str(match (g.create, g.delete) {
                (true, true) => ", may create and delete records.",
                (true, false) => ", may create records.",
                (false, true) => ", may delete records.",
                (false, false) => ".",
            });
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `GET /admin/pages`: every custom page of every app, with what it may do and whether an administrator turned it off.
/// Who approved a page is not kept with the page, so `approved_by` is empty until a brief stores it.
pub fn pages(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let off = setting_strings(state, "pages.disabled");
    let mut manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    manifests.sort_by(|a, b| a.slug.cmp(&b.slug));
    let mut rows = Vec::new();
    for m in &manifests {
        for p in &m.pages {
            if let PageKind::Custom { grants, .. } = &p.kind {
                rows.push(json!({
                    "app_slug": m.slug, "app_title": m.title, "page_id": p.id, "title": p.title,
                    "grants": grants_sentence(grants), "approved_by": Value::Null,
                    "disabled": off.iter().any(|k| *k == format!("{}/{}", m.slug, p.id)),
                }));
            }
        }
    }
    Ok(json!({ "pages": rows }))
}

// -------------------------------------------------------------------------------------------------------------------
// The ledger
// -------------------------------------------------------------------------------------------------------------------

/// Every way a stored row may spell a control: `ac-2` is `AC-2` and `AC-02`, and `ac-2.1` is `AC-2(1)` and `AC-02(01)`.
/// The control column is indexed, so the filter names the spellings and the database does not read each row.
fn control_spellings(raw: &str) -> Vec<String> {
    let normal = catalog::normalize(raw);
    let mut out = vec![raw.trim().to_string(), raw.trim().to_uppercase(), normal.to_uppercase()];
    let upper = normal.to_uppercase();
    if let Some((family, rest)) = upper.split_once('-') {
        let (base, enhancement) = match rest.split_once('.') {
            Some((b, e)) => (b, Some(e)),
            None => (rest, None),
        };
        let pad = |n: &str| if n.len() == 1 { format!("0{n}") } else { n.to_string() };
        for b in [base.to_string(), pad(base)] {
            match enhancement {
                None => out.push(format!("{family}-{b}")),
                Some(e) => {
                    for e in [e.to_string(), pad(e)] {
                        out.push(format!("{family}-{b}({e})"));
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A date (`2026-03-01`) or a time (RFC 3339). A date used as an upper bound runs to the end of that day.
fn parse_time(raw: &str, key: &str, upper: bool) -> Result<DateTime<Utc>, ServiceError> {
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Ok(t.with_timezone(&Utc));
    }
    let day = NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| ServiceError::BadRequest(format!("'{key}' is a date such as 2026-03-01 or a time such as 2026-03-01T09:00:00Z")))?;
    let time = if upper { day.and_hms_milli_opt(23, 59, 59, 999) } else { day.and_hms_opt(0, 0, 0) };
    time.map(|t| Utc.from_utc_datetime(&t)).ok_or_else(|| ServiceError::BadRequest(format!("'{key}' is not a date")))
}

fn text<'a>(q: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    q.get(key).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// The filters shared by the ledger list and the export: `principal`, `decision_type`, `app`, `control`, `from`, `to`.
pub fn ledger_filter(q: &HashMap<String, String>) -> Result<LedgerFilter, ServiceError> {
    let decision_type = match text(q, "decision_type") {
        Some(d) => Some(DecisionType::parse(d).ok_or_else(|| ServiceError::BadRequest(format!("'{d}' is not a decision type")))?),
        None => None,
    };
    let (from, to) = (
        text(q, "from").map(|t| parse_time(t, "from", false)).transpose()?,
        text(q, "to").map(|t| parse_time(t, "to", true)).transpose()?,
    );
    Ok(LedgerFilter {
        principal: text(q, "principal").map(str::to_string),
        decision_type,
        app: text(q, "app").map(str::to_string),
        controls: text(q, "control").map(control_spellings).unwrap_or_default(),
        from,
        to,
    })
}

/// A ledger entry as the console shows it: its fields, the payload it was stored with, and the title of its control.
pub fn ledger_row_json(row: &LedgerRow) -> Value {
    let mut v = serde_json::to_value(&row.entry).unwrap_or(Value::Null);
    v["payload"] = row.payload.clone();
    v["control_title"] = json!(catalog::title(&row.entry.oscal_control_id));
    v
}

/// `GET /admin/ledger`. Newest first by sequence. The cursor is the sequence the next page starts under.
pub fn ledger(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let filter = ledger_filter(q)?;
    // The cursor is the sequence the next page starts under. It is written like the other cursors.
    let (cursor, limit) = page_args(q)?;
    let before = text(q, "cursor").map(|_| cursor as u64);
    let mut rows = state.repository.ledger_rows(&filter, before, limit as i64 + 1).map_err(storage)?;
    let next = (rows.len() > limit).then(|| rows[limit - 1].entry.sequence as usize);
    rows.truncate(limit);
    Ok(page_of(rows.iter().map(ledger_row_json).collect(), next))
}

/// The answer of a chain check.
fn verify_json(state: &SharedState) -> Result<Value, ServiceError> {
    let check = state.repository.ledger_chain_check().map_err(storage)?;
    let (_, head) = state.repository.ledger_summary().map_err(storage)?;
    Ok(json!({
        "valid": check.first_bad.is_none(),
        "entries": check.entries,
        "head_hash": head.unwrap_or_else(|| scaffoldry_core::GENESIS_PREVIOUS_HASH.to_string()),
        "first_bad_sequence": check.first_bad.as_ref().map(|(s, _)| *s),
        "detail": check.first_bad.as_ref().map(|(_, why)| why.clone()),
    }))
}

/// `POST /admin/ledger/verify`: walks the whole chain. It is a full scan.
pub fn verify(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    verify_json(state)
}

/// A file the browser saves: its type, its name, and its text.
pub struct Download {
    pub content_type: &'static str,
    pub filename: &'static str,
    pub body: String,
}

/// A CSV cell. A cell that a spreadsheet would run as a formula is made text, and one with a comma, a quote or a line is quoted.
fn csv_cell(raw: &str) -> String {
    let safe = if raw.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{raw}") } else { raw.to_string() };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

fn csv_line(entry: &LedgerEntry, payload: &Value) -> String {
    let cells = [
        entry.sequence.to_string(),
        entry.timestamp_iso.clone(),
        entry.principal.clone(),
        entry.organization_code.clone(),
        entry.app_slug.clone().unwrap_or_default(),
        entry.decision_type.as_str().to_string(),
        entry.oscal_control_id.clone(),
        catalog::title(&entry.oscal_control_id).unwrap_or_default().to_string(),
        entry.rationale.clone(),
        payload.to_string(),
        entry.entry_hash.clone(),
        entry.previous_hash.clone(),
        entry.payload_hash.clone(),
    ];
    cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(",")
}

const CSV_HEADER: &str = "sequence,timestamp,principal,organization,app,decision_type,control,control_title,rationale,payload,entry_hash,previous_hash,payload_hash";

/// `GET /admin/ledger/export`: the filtered entries as `json` or `csv`, at most 50,000. A JSON export starts with the head
/// hash and the result of a full chain check. A CSV export is the header and one line for each entry.
pub fn export(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Download, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let as_csv = match text(q, "format").unwrap_or("json") {
        "json" => false,
        "csv" => true,
        other => return Err(ServiceError::BadRequest(format!("'format' is json or csv, not '{other}'"))),
    };
    let filter = ledger_filter(q)?;
    if state.repository.ledger_count_up_to(&filter, EXPORT_LIMIT).map_err(storage)? > EXPORT_LIMIT {
        return Err(ServiceError::BadRequest(format!("More than {EXPORT_LIMIT} entries match. Narrow the filter by decision type, person, app, control or dates, and export again.")));
    }
    let mut body = if as_csv {
        format!("{CSV_HEADER}\n")
    } else {
        let verify = verify_json(state)?;
        format!("{{\"head_hash\":{},\"verify\":{},\"entries\":[", verify["head_hash"], verify)
    };
    let mut before: Option<u64> = None;
    let mut first = true;
    loop {
        let rows = state.repository.ledger_rows(&filter, before, crate::repository::LEDGER_BATCH).map_err(storage)?;
        let Some(last) = rows.last() else { break };
        before = Some(last.entry.sequence);
        let done = (rows.len() as i64) < crate::repository::LEDGER_BATCH;
        for row in &rows {
            if as_csv {
                body.push_str(&csv_line(&row.entry, &row.payload));
                body.push('\n');
            } else {
                if !first {
                    body.push(',');
                }
                body.push_str(&ledger_row_json(row).to_string());
            }
            first = false;
        }
        if done {
            break;
        }
    }
    if !as_csv {
        body.push_str("]}");
    }
    Ok(if as_csv {
        Download { content_type: "text/csv; charset=utf-8", filename: "ledger.csv", body }
    } else {
        Download { content_type: "application/json", filename: "ledger.json", body }
    })
}

/// The newest entries, for the Overview. Each carries the title of its control.
pub fn recent_changes(state: &SharedState, count: i64) -> Vec<Value> {
    state
        .repository
        .ledger_rows(&LedgerFilter::default(), None, count)
        .unwrap_or_default()
        .iter()
        .map(|r| {
            json!({
                "sequence": r.entry.sequence, "timestamp_iso": r.entry.timestamp_iso, "principal": r.entry.principal,
                "decision_type": r.entry.decision_type.as_str(), "oscal_control_id": r.entry.oscal_control_id,
                "control_title": catalog::title(&r.entry.oscal_control_id), "rationale": r.entry.rationale,
            })
        })
        .collect()
}

// -------------------------------------------------------------------------------------------------------------------
// Tokens
// -------------------------------------------------------------------------------------------------------------------

fn token_state(t: &crate::state::ApiToken, now: DateTime<Utc>) -> &'static str {
    if t.revoked_at.is_some() {
        "revoked"
    } else if t.expires_at <= now {
        "expired"
    } else {
        "active"
    }
}

/// `GET /admin/tokens`. Query: `kind`, `owner`, `state`, `cursor`, `limit`. The row never holds the hash.
pub fn list_tokens(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (offset, limit) = page_args(q)?;
    let wanted = text(q, "state");
    if let Some(s) = wanted {
        if !["active", "expired", "revoked"].contains(&s) {
            return Err(ServiceError::BadRequest("'state' is active, expired or revoked".to_string()));
        }
    }
    let filter = TokenFilter { kind: text(q, "kind").map(str::to_string), owner_prefix: text(q, "owner").map(str::to_string), state: wanted.map(str::to_string) };
    let mut page = state.repository.list_tokens_admin(&filter, offset as i64, limit as i64 + 1).map_err(storage)?;
    let next = (page.len() > limit).then_some(offset + limit);
    page.truncate(limit);
    let now = Utc::now();
    let rows = page
        .iter()
        .map(|t| {
            json!({
                "id": t.id, "kind": t.kind, "owner": t.eppn, "label": t.label, "state": token_state(t, now),
                "created_at": t.created_at.to_rfc3339(), "expires_at": t.expires_at.to_rfc3339(),
                "last_used_at": t.last_used_at.map(|d| d.to_rfc3339()), "revoked_at": t.revoked_at.map(|d| d.to_rfc3339()),
                "original_admin": t.original_admin,
            })
        })
        .collect();
    Ok(page_of(rows, next))
}

/// `DELETE /admin/tokens/{id}`. Body: `reason`. The ledger entry is written first. An impersonation session is a token too.
pub fn revoke_token(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = require_reason(body.get("reason").and_then(Value::as_str).unwrap_or(""))?.to_string();
    let id: uuid::Uuid = id.parse().map_err(|_| ServiceError::NotFound("There is no such token".to_string()))?;
    let token = state.repository.api_token_by_id(id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("There is no such token".to_string()))?;
    if token.revoked_at.is_some() {
        return Err(ServiceError::Conflict("That token is already revoked".to_string()));
    }
    let payload = json!({ "action": "revoke_token", "token_id": token.id, "kind": token.kind, "owner": token.eppn, "label": token.label });
    admin_write(state, caller, DecisionType::TokenRevoked, "IA-05", &reason, &payload, || {
        let revoked = state.revoke_api_token(id, &caller.eppn, true).map_err(|e| ServiceError::Internal(e.to_string()))?;
        if !revoked {
            return Err(ServiceError::Conflict("That token is already revoked".to_string()));
        }
        Ok(json!({ "revoked": true, "id": id, "kind": token.kind, "owner": token.eppn }))
    })
}

// -------------------------------------------------------------------------------------------------------------------
// The inventory tool
// -------------------------------------------------------------------------------------------------------------------

/// The kinds `admin_inventory` reads. Each is the same service function as its REST route.
pub const INVENTORY_KINDS: [&str; 8] = ["users", "workspaces", "apps", "fields", "processes", "process_instances", "tokens", "ledger"];

fn flag(q: &HashMap<String, String>, key: &str) -> Result<Option<bool>, ServiceError> {
    crate::service::admin_processes::query_flag(q, key)
}

/// `admin_inventory`: one page of what an admin screen lists, read-only. The filters are the matching route's.
pub fn inventory(caller: &AuthUser, args: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    use crate::routes::admin_workspaces::{apps_page, workspaces_page, AdminAppsQuery, AdminWorkspacesQuery};
    use crate::service::people::{list_users, UserQuery};
    require_platform_admin(caller, state)?;
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("");
    // The screen's filters, as its route reads them, then the page size and cursor.
    let mut q: HashMap<String, String> = HashMap::new();
    let filters = args.get("filter").map(|f| f.as_object().ok_or_else(|| ServiceError::BadRequest("'filter' is an object of the screen's filters".to_string()))).transpose()?;
    let paging = ["limit", "cursor"].iter().filter_map(|k| args.get(*k).map(|v| (k.to_string(), v)));
    for (key, value) in filters.into_iter().flatten().map(|(k, v)| (k.clone(), v)).chain(paging) {
        match value {
            Value::String(s) => q.insert(key, s.clone()),
            Value::Bool(b) => q.insert(key, b.to_string()),
            Value::Number(n) => q.insert(key, n.to_string()),
            Value::Null => None,
            _ => return Err(ServiceError::BadRequest(format!("The filter '{key}' is a string, number or true or false"))),
        };
    }
    if let Some(limit) = q.get("limit") {
        match limit.parse::<usize>() {
            Ok(n) if (1..=200).contains(&n) => {}
            _ => return Err(ServiceError::BadRequest("'limit' is a number from 1 to 200".to_string())),
        }
    }
    let text = |k: &str| q.get(k).filter(|v| !v.is_empty()).cloned();
    let limit = text("limit").and_then(|l| l.parse::<usize>().ok());
    let mut reply = match kind {
        "users" => {
            let (users, next_cursor) = list_users(
                state,
                &UserQuery {
                    search: text("search"),
                    active: flag(&q, "active")?,
                    hold: flag(&q, "hold")?,
                    affiliation: text("affiliation"),
                    unit: match text("unit") {
                        Some(u) => Some(u.parse().map_err(|_| ServiceError::BadRequest("'unit' is a unit id".to_string()))?),
                        None => None,
                    },
                    platform_admin: flag(&q, "platform_admin")?,
                    unit_admin: flag(&q, "unit_admin")?,
                    cursor: text("cursor"),
                    limit,
                },
            )?;
            json!({ "users": users, "next_cursor": next_cursor })
        }
        "workspaces" => workspaces_page(
            state,
            &AdminWorkspacesQuery { search: text("search"), unit: text("unit"), classification: text("classification"), visibility: text("visibility"), cursor: text("cursor"), limit },
        )?,
        "apps" => apps_page(state, &AdminAppsQuery { search: text("search"), workspace: text("workspace"), cursor: text("cursor"), limit })?,
        "fields" => crate::service::data_governance::fields(caller, &q, state)?,
        "processes" => crate::service::admin_processes::list(caller, &q, state)?,
        "process_instances" => crate::service::admin_processes::list_instances(caller, &q, state)?,
        "tokens" => list_tokens(caller, &q, state)?,
        "ledger" => ledger(caller, &q, state)?,
        other => return Err(ServiceError::BadRequest(format!("'kind' is one of {}, not '{other}'", INVENTORY_KINDS.join(", ")))),
    };
    // One shape for every kind: `rows` and `next_cursor`.
    for old in ["users", "fields"] {
        if let Some(rows) = reply.get_mut(old).map(Value::take) {
            reply["rows"] = rows;
            if let Some(o) = reply.as_object_mut() {
                o.remove(old);
            }
        }
    }
    reply["kind"] = json!(kind);
    Ok(reply)
}
