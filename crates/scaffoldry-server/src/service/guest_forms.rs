//! Guests phase 2: a form for guests.
//!
//! Two things live here. The first is the proposal that opens a form to guests, which a second person decides. The second is
//! what a guest session may do once the form is open: make a record, read and change the ones it made, and send a file. Every
//! one of those goes through `context`, which checks the session names this app and this form before anything else.
//!
//! The row rule is built in. A record made through a guest form stores the guest's address in `_submitter`, and a guest reads
//! or changes a record only when that is their own address. It does not depend on the table's access rules, or on anyone
//! remembering to write one: `scope::allows_data` applies it to every guest, and `scope::scope_for` shows a guest no rows to
//! any query.

use crate::repository_views::StoredView;
use crate::service::access::{authorize_app, AppAction};
use crate::service::{forms, guests, records, views, ServiceError};
use crate::state::{AuthUser, SharedState};
use crate::state::RecordDecisionInput;
use scaffoldry_core::DecisionType;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Submissions one guest may make through one form in a day, and in all, unless the form says otherwise.
pub const PER_DAY: i64 = 3;
pub const IN_TOTAL: i64 = 10_000;
/// The size of a file a guest may send, in MB, unless `guests.max_file_mb` says otherwise.
pub const FILE_MB: u64 = 10;

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn digest(config: &Value) -> String {
    Sha256::digest(config.to_string().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn record(state: &SharedState, caller: &AuthUser, app: &str, rationale: String, payload: Value) -> Result<(), ServiceError> {
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: Some(app.to_string()),
            decision_type: DecisionType::GuestExposureChanged,
            oscal_control_id: "AC-3".to_string(),
            rationale,
            payload: &payload,
        })
        .map(|_| ())
        .map_err(|e| ServiceError::Internal(format!("Failed to record audit ledger: {e}")))
}

fn exposure_json(r: &postgres::Row) -> Value {
    let at = |i: usize| r.get::<_, Option<chrono::DateTime<chrono::Utc>>>(i).map(|t| t.to_rfc3339());
    let checks: Value = r.get(5);
    let blocked = checks.as_array().is_some_and(|l| l.iter().any(|c| c["outcome"] == "fail"));
    json!({
        "id": r.get::<_, Uuid>(0), "form_id": r.get::<_, Uuid>(1), "status": r.get::<_, String>(2), "proposed_by": r.get::<_, String>(3),
        "created_at": at(4), "checks": checks, "change": r.get::<_, String>(6), "decided_by": r.get::<_, Option<String>>(7),
        "decided_at": at(8), "rationale": r.get::<_, Option<String>>(9),
        // The most exposed surface in the product. A second person always decides it, and a failed check cannot be approved.
        "needs_second_person": true, "blocked": blocked,
    })
}

const EXPOSURE_COLUMNS: &str = "id, form_id, status, proposed_by, created_at, checks, change, decided_by, decided_at, rationale";

fn guest_form_of(caller: &AuthUser, app: &str, form: Uuid, state: &SharedState) -> Result<StoredView, ServiceError> {
    authorize_app(caller, app, AppAction::Manage, state)?;
    let stored = views::existing(caller, app, form, state)?;
    if stored.kind != "form" || stored.ownership == "personal" {
        return Err(bad("Only a shared form can be opened to guests"));
    }
    Ok(stored)
}

/// `POST /apps/{slug}/views/{id}/guest-exposure`: proposes opening a form to guests. The checks run now, and a second person
/// decides. Nothing is open to guests until they approve.
pub fn propose(caller: &AuthUser, app: &str, form: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = guest_form_of(caller, app, form, state)?;
    if forms::audience(&stored.config) == "Guests" {
        return Err(ServiceError::Conflict("This form is already open to guests".to_string()));
    }
    let table = stored.table_id.clone().ok_or_else(|| bad("This form is not over a table"))?;
    let (fields, name) = records::table_fields_lenient(app, &table, state);
    let check = forms::check_exposure(&stored.config, &fields, &records::sensitive_fields_of(app, &table, state))?;
    let (id, app_s, digest_s, change, by) = (Uuid::new_v4(), app.to_string(), digest(&stored.config), forms::exposure_sentence(&name), caller.eppn.clone());
    let checks = json!([check]);
    let made = state
        .repository
        .with_client({
            let (checks, change, by) = (checks.clone(), change.clone(), by.clone());
            move |c| {
                let mut tx = c.transaction()?;
                tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&format!("form-exposure:{form}")])?;
                let open: i64 = tx.query_one("SELECT COUNT(*) FROM form_exposures WHERE app_slug = $1 AND form_id = $2 AND status = 'Pending'", &[&app_s, &form])?.get(0);
                if open > 0 {
                    return Ok(None);
                }
                tx.execute(
                    "INSERT INTO form_exposures (id, app_slug, form_id, proposed_by, checks, change, config_digest) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[&id, &app_s, &form, &by, &checks, &change, &digest_s],
                )?;
                let row = tx.query_one(&format!("SELECT {EXPOSURE_COLUMNS} FROM form_exposures WHERE id = $1"), &[&id])?;
                tx.commit()?;
                Ok(Some(exposure_json(&row)))
            }
        })
        .map_err(storage)?;
    let made = made.ok_or_else(|| ServiceError::Conflict("This form already has a proposal waiting for a second person".to_string()))?;
    record(state, caller, app, format!("{by} proposed opening the form '{}' to guests. {change}", stored.name), json!({ "form": form, "proposal": id, "action": "proposed", "checks": checks }))?;
    Ok(made)
}

/// `GET /apps/{slug}/views/{id}/guest-exposure`: the proposals for a form, newest first.
pub fn list(caller: &AuthUser, app: &str, form: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    guest_form_of(caller, app, form, state)?;
    let app = app.to_string();
    let rows = state
        .repository
        .with_client(move |c| Ok(c.query(&format!("SELECT {EXPOSURE_COLUMNS} FROM form_exposures WHERE app_slug = $1 AND form_id = $2 ORDER BY created_at DESC LIMIT 50"), &[&app, &form])?.iter().map(exposure_json).collect::<Vec<_>>()))
        .map_err(storage)?;
    Ok(json!({ "proposals": rows }))
}

/// `POST /apps/{slug}/views/{id}/guest-exposure/{xid}/decide`. The proposer cannot decide. A proposal whose checks failed cannot
/// be approved, and neither can one whose form changed after it was proposed.
pub fn decide(caller: &AuthUser, app: &str, form: Uuid, proposal: Uuid, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = guest_form_of(caller, app, form, state)?;
    let decision = match body.get("decision").and_then(Value::as_str) {
        Some(d @ ("approve" | "reject")) => d,
        _ => return Err(bad("decision is approve or reject")),
    };
    let rationale = body.get("rationale").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty() && r.chars().count() <= 1000).ok_or_else(|| bad("Give a reason of 1 to 1000 characters"))?.to_string();
    let found = {
        let app = app.to_string();
        state
            .repository
            .with_client(move |c| Ok(c.query_opt("SELECT status, proposed_by, checks, config_digest FROM form_exposures WHERE id = $1 AND app_slug = $2 AND form_id = $3", &[&proposal, &app, &form])?.map(|r| (r.get::<_, String>(0), r.get::<_, String>(1), r.get::<_, Value>(2), r.get::<_, String>(3)))))
            .map_err(storage)?
    };
    let (status, proposer, checks, was) = found.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    if status != "Pending" {
        return Err(ServiceError::Conflict(format!("This proposal was already decided: {status}")));
    }
    if proposer.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("The person who proposed this cannot decide it. A second person must"));
    }
    if decision == "approve" {
        if checks.as_array().is_some_and(|l| l.iter().any(|c| c["outcome"] == "fail")) {
            return Err(bad("A failed check cannot be approved. Change the form and propose it again"));
        }
        if digest(&stored.config) != was {
            return Err(ServiceError::Conflict("The form changed after it was proposed. Propose it again".to_string()));
        }
    }
    let new_status = if decision == "approve" { "Approved" } else { "Rejected" };
    let (by, r2, app_s) = (caller.eppn.clone(), rationale.clone(), app.to_string());
    let claimed = state
        .repository
        .with_client(move |c| {
            Ok(c.execute(
                "UPDATE form_exposures SET status = $1, decided_by = $2, decided_at = NOW(), rationale = $3 WHERE id = $4 AND app_slug = $5 AND status = 'Pending'",
                &[&new_status, &by, &r2, &proposal, &app_s],
            )?)
        })
        .map_err(storage)?;
    if claimed == 0 {
        return Err(ServiceError::Conflict("This proposal was decided while you were reading it".to_string()));
    }
    if decision == "approve" {
        views::set_audience(app, form, "Guests", &caller.eppn, state)?;
    }
    record(state, caller, app, format!("{} {} opening the form '{}' to guests. {rationale}", caller.eppn, if decision == "approve" { "approved" } else { "rejected" }, stored.name), json!({ "form": form, "proposal": proposal, "action": new_status }))?;
    Ok(json!({ "id": proposal, "status": new_status, "form_id": form }))
}

/// A guest session, checked against the app and the form it was made for.
pub struct Context {
    pub app: String,
    pub form: Uuid,
    pub guest: AuthUser,
    pub view: StoredView,
}

fn out_of_scope() -> ServiceError {
    ServiceError::TokenScope("This session is for one form. It reaches nothing else.".to_string())
}

/// Reads the session behind a request and checks it names this app and this form, before any other check. A session that is
/// not a guest's, or is for another form, is refused with `token_scope`. A form that was closed since is refused as closed.
pub fn context(state: &SharedState, headers: &axum::http::HeaderMap, app: &str, form: &str) -> Result<Context, ServiceError> {
    let row = crate::guard::token_row(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))?;
    let guest = crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("A valid session is required".to_string()))?;
    let form_id = Uuid::parse_str(form).map_err(|_| out_of_scope())?;
    if row.kind != "guest" || row.app_slug.as_deref() != Some(app) || row.label != format!("guest-form:{form_id}") {
        return Err(out_of_scope());
    }
    if !guests::open_form(state, app, form_id) {
        return Err(ServiceError::forbidden("This form is not open to guests now"));
    }
    let view = state.repository.get_app_view(app, form_id).map_err(storage)?.ok_or_else(|| ServiceError::NotFound("Form not found".to_string()))?;
    Ok(Context { app: app.to_string(), form: form_id, guest, view })
}

fn int(config: &Value, key: &str, default: i64) -> i64 {
    config.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn listed(config: &Value) -> Vec<String> {
    config["form_fields"].as_array().into_iter().flatten().filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_string)).collect()
}

fn shown(config: &Value) -> Vec<String> {
    let mut names = listed(config);
    names.extend(config.get("read_only_fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    names
}

/// A record as the guest sees it: the form's fields, and the read-only fields the form lists. Nothing else leaves.
fn shown_record(ctx: &Context, r: &crate::state::DatasetRecord) -> Value {
    let names = shown(&ctx.view.config);
    let data: serde_json::Map<String, Value> = r.data.as_object().into_iter().flatten().filter(|(k, _)| names.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    json!({ "id": r.id, "version": r.version, "created_at": r.created_at, "data": data })
}

/// The record a guest made through this form. Any other id, including one that exists, is not found.
fn own(ctx: &Context, id: &str, state: &SharedState) -> Result<crate::state::DatasetRecord, ServiceError> {
    let found = state.find_record(&ctx.app, id).map_err(ServiceError::Internal)?;
    found
        .filter(|r| r.data.get("_submitter").and_then(Value::as_str) == Some(ctx.guest.eppn.as_str()) && r.data.get("_form").and_then(Value::as_str) == Some(ctx.form.to_string().as_str()))
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{id}' not found")))
}

fn counts(ctx: &Context, state: &SharedState) -> Result<(i64, i64), ServiceError> {
    let (app, form, me) = (ctx.app.clone(), ctx.form.to_string(), ctx.guest.eppn.clone());
    state
        .repository
        .with_client(move |c| {
            let day: i64 = c
                .query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND data->>'_form' = $2 AND data->>'_submitter' = $3 AND created_at > NOW() - INTERVAL '1 day'", &[&app, &form, &me])?
                .get(0);
            let all: i64 = c.query_one("SELECT COUNT(*) FROM dataset_records WHERE app_slug = $1 AND data->>'_form' = $2", &[&app, &form])?.get(0);
            Ok((day, all))
        })
        .map_err(storage)
}

/// `POST /guest/{app}/{form}/records`.
pub fn create(ctx: &Context, origin: &crate::service::history::Origin, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (per_day, in_total) = (int(&ctx.view.config, "max_per_day", PER_DAY), int(&ctx.view.config, "max_total", IN_TOTAL));
    let (day, all) = counts(ctx, state)?;
    if day >= per_day {
        return Err(ServiceError::RateLimited(format!("This form takes {per_day} submissions a day from one person, and you have sent {day}. Try again tomorrow.")));
    }
    if all >= in_total {
        return Err(ServiceError::Conflict(format!("This form takes {in_total} submissions in all, and it has taken them.")));
    }
    let mut system = serde_json::Map::new();
    system.insert("_submitter".to_string(), json!(ctx.guest.eppn));
    system.insert("_form".to_string(), json!(ctx.form.to_string()));
    let made = forms::submit_stored(&ctx.guest, origin, &ctx.app, &ctx.view, body, &system, state)?;
    let created: crate::state::DatasetRecord = serde_json::from_value(made["record"].clone()).map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(json!({ "record": shown_record(ctx, &created), "success_message": made["success_message"] }))
}

/// `GET /guest/{app}/{form}/records/{id}`.
pub fn get(ctx: &Context, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(json!({ "record": shown_record(ctx, &own(ctx, id, state)?) }))
}

/// Whether the form's `locks_when` holds for a record: its field has one of the listed values.
fn locked(ctx: &Context, r: &crate::state::DatasetRecord) -> bool {
    let Some(lock) = ctx.view.config.get("locks_when").filter(|l| !l.is_null()) else { return false };
    let (Some(field), Some(values)) = (lock.get("field").and_then(Value::as_str), lock.get("in").and_then(Value::as_array)) else { return false };
    r.data.get(field).is_some_and(|have| values.contains(have))
}

/// `PUT /guest/{app}/{form}/records/{id}`: the guest changes a record of their own, with the form's fields only, until the lock holds.
pub fn update(ctx: &Context, origin: &crate::service::history::Origin, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let current = own(ctx, id, state)?;
    if locked(ctx, &current) {
        return Err(ServiceError::Conflict("This request is being handled and can no longer be changed. Contact the office if something is wrong.".to_string()));
    }
    let data = body.get("data").and_then(Value::as_object).ok_or_else(|| bad("Send the changes as `data`"))?;
    let allowed = listed(&ctx.view.config);
    if let Some(extra) = data.keys().find(|k| !allowed.contains(k)) {
        return Err(bad(format!("'{extra}' is not a field of this form")));
    }
    let version = body.get("version").cloned().ok_or_else(|| bad("version is required: send the version you read"))?;
    // A save replaces the record's data, so the guest's changes go over what is there. `_submitter` and the rest stay as they were.
    let mut merged = current.data.as_object().cloned().unwrap_or_default();
    for (k, v) in data {
        merged.insert(k.clone(), v.clone());
    }
    let saved = records::update_record(&ctx.guest, origin, &ctx.app, id, &json!({ "version": version, "data": merged }), state)?;
    Ok(json!({ "record": shown_record(ctx, &saved) }))
}

/// The limit on a guest's file, in bytes.
pub fn file_limit(state: &SharedState) -> u64 {
    state.settings.read().ok().and_then(|s| s.get("guests.max_file_mb").and_then(Value::as_u64)).unwrap_or(FILE_MB).clamp(1, crate::files::MAX_MAX_MB) * 1024 * 1024
}

/// The field that says where a request stands: the form's `status_field`, else the field `locks_when` reads, else the first
/// read-only field.
fn status_field(config: &Value) -> Option<String> {
    config
        .get("status_field")
        .and_then(Value::as_str)
        .or_else(|| config.get("locks_when").and_then(|l| l.get("field")).and_then(Value::as_str))
        .or_else(|| config.get("read_only_fields").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str))
        .map(str::to_string)
}

/// The label of an option of a select field, or the value as it is.
fn label_of(fields: &[scaffoldry_engine::FieldSpec], field: &str, value: &Value) -> String {
    let raw = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
    fields.iter().find(|f| f.name == field).and_then(|f| f.select_options.iter().find(|o| o.id == raw)).map(|o| o.label.clone()).unwrap_or(raw)
}

/// The plain sentence for where a record stands.
fn standing(ctx: &Context, fields: &[scaffoldry_engine::FieldSpec], r: &crate::state::DatasetRecord) -> String {
    let mut s = match status_field(&ctx.view.config).and_then(|f| r.data.get(&f).filter(|v| !v.is_null()).map(|v| (f, v.clone()))) {
        Some((field, v)) => format!("Where it stands: {}.", label_of(fields, &field, &v)),
        None => "Where it stands: we have it.".to_string(),
    };
    if locked(ctx, r) {
        s.push_str(" It can no longer be changed.");
    }
    s
}

/// `GET /guest/{app}/{form}`: everything the status page needs, in one answer. The guest's own records, newest first, with the
/// form's fields and the read-only ones, and whether they may add another.
pub fn overview(ctx: &Context, state: &SharedState) -> Result<Value, ServiceError> {
    let table = ctx.view.table_id.clone().ok_or_else(|| bad("This form is not over a table"))?;
    let (fields, _) = records::table_fields_lenient(&ctx.app, &table, state);
    let (app, form, me) = (ctx.app.clone(), ctx.form.to_string(), ctx.guest.eppn.clone());
    let ids: Vec<String> = state
        .repository
        .with_client(move |c| Ok(c.query("SELECT id FROM dataset_records WHERE app_slug = $1 AND data->>'_form' = $2 AND data->>'_submitter' = $3 ORDER BY created_at DESC, id LIMIT 100", &[&app, &form, &me])?.iter().map(|r| r.get(0)).collect()))
        .map_err(storage)?;
    let mut rows = Vec::new();
    for id in ids {
        if let Ok(r) = own(ctx, &id, state) {
            let mut shown = shown_record(ctx, &r);
            shown["standing"] = json!(standing(ctx, &fields, &r));
            shown["editable"] = json!(!locked(ctx, &r));
            rows.push(shown);
        }
    }
    let (per_day, in_total) = (int(&ctx.view.config, "max_per_day", PER_DAY), int(&ctx.view.config, "max_total", IN_TOTAL));
    let (day, all) = counts(ctx, state)?;
    let remaining = (per_day - day).max(0);
    let spec = |name: &str, label: Option<&str>, ro: bool| {
        let f = fields.iter().find(|f| f.name == name);
        json!({
            "name": name,
            "label": label.map(str::to_string).or_else(|| f.map(|f| f.label.clone())).unwrap_or_else(|| name.to_string()),
            "type": f.map(|f| serde_json::to_value(f.field_type).unwrap_or(Value::Null)).unwrap_or(Value::Null),
            "required": f.is_some_and(|f| f.required),
            "read_only": ro,
            "options": f.map(|f| f.select_options.iter().map(|o| json!({ "id": o.id, "label": o.label })).collect::<Vec<_>>()).unwrap_or_default(),
        })
    };
    let mut listed_fields: Vec<Value> = ctx.view.config["form_fields"].as_array().into_iter().flatten().filter_map(|f| Some(spec(f.get("name")?.as_str()?, f.get("label").and_then(Value::as_str), false))).collect();
    listed_fields.extend(ctx.view.config.get("read_only_fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|n| spec(n, None, true)));
    Ok(json!({
        "form": { "name": ctx.view.name, "fields": listed_fields, "success_message": ctx.view.config.get("success_message") },
        "records": rows,
        "can_add": remaining > 0 && all < in_total,
        "remaining_today": remaining,
    }))
}

fn random_token() -> String {
    use aes_gcm::aead::{rand_core::RngCore, OsRng};
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A record of a guest form has been saved. When its status changed, the guest is told by mail, with no value in it. A guest who
/// stopped these mails, or whose address was disabled, is told nothing. A failure here never fails the save.
pub fn on_saved(app: &str, before: &crate::state::DatasetRecord, after: &crate::state::DatasetRecord, state: &SharedState) {
    let (Some(who), Some(form)) = (after.data.get("_submitter").and_then(Value::as_str), after.data.get("_form").and_then(Value::as_str).and_then(|f| Uuid::parse_str(f).ok())) else { return };
    let Some(guest) = guests::id_of(who) else { return };
    let Ok(Some(view)) = state.repository.get_app_view(app, form) else { return };
    let Some(field) = status_field(&view.config) else { return };
    if before.data.get(&field) == after.data.get(&field) {
        return;
    }
    let Ok(base) = crate::mail::mailing(&state.settings.read().unwrap_or_else(|p| p.into_inner())) else { return };
    let (raw, address, app_s) = (random_token(), who.to_string(), app.to_string());
    let hash = crate::service::identity::hash_token(&raw);
    let _ = state.repository.with_client(move |c| {
        let mut tx = c.transaction()?;
        let row = tx.query_opt("SELECT email FROM guests WHERE id = $1 AND disabled_at IS NULL AND email_verified_at IS NOT NULL", &[&guest])?;
        let Some(email) = row.map(|r| r.get::<_, String>(0)) else { return Ok(()) };
        let stopped: i64 = tx.query_one("SELECT count(*) FROM notification_prefs WHERE eppn = $1 AND kind = 'guest_status' AND channel = 'off'", &[&address])?.get(0);
        if stopped > 0 {
            return Ok(());
        }
        tx.execute("INSERT INTO guest_stops (token_hash, guest_id) VALUES ($1, $2)", &[&hash, &guest])?;
        let text = format!(
            "There is an update on your request.\n\nSign in to see it: {base}/g/{app_s}/{form}\n\nTo stop these mails: {base}/g/stop/{raw}\n"
        );
        crate::mail::queue_guest_update(&mut tx, &email, "An update on your request", &text)?;
        tx.commit()?;
        Ok(())
    });
}

/// `POST /guest/stop`: the link in a mail. It turns off status mail for the guest the link was made for. The link keeps working,
/// so stopping twice is fine, and a token that is not one is not found.
pub fn stop(state: &SharedState, body: &Value) -> Result<Value, ServiceError> {
    let not_found = || ServiceError::NotFound("This link does not work.".to_string());
    let raw = body.get("token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 200).ok_or_else(not_found)?;
    let hash = crate::service::identity::hash_token(raw);
    let found = state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(guest) = tx.query_opt("SELECT guest_id FROM guest_stops WHERE token_hash = $1", &[&hash])?.map(|r| r.get::<_, Uuid>(0)) else { return Ok(false) };
            tx.execute(
                "INSERT INTO notification_prefs (eppn, kind, channel) VALUES ($1, 'guest_status', 'off') ON CONFLICT (eppn, kind) DO UPDATE SET channel = 'off'",
                &[&guests::address_of(guest)],
            )?;
            tx.commit()?;
            Ok(true)
        })
        .map_err(storage)?;
    if !found {
        return Err(not_found());
    }
    Ok(json!({ "message": "You will not get mail about your requests any more." }))
}
