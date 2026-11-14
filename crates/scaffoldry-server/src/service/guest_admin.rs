//! Guests phase 4: operating guests. The console's Guests panel, and the job that removes guests nobody used.
//!
//! Everything here is for a Platform Admin. A reason is required for every change, and each change is a ledger entry. The ledger
//! names a guest by id and never holds an address, so erasing a person leaves nothing of them in the audit trail.

use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::{guests, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A guest nobody has used for this long, and who made nothing, is removed.
pub const SILENT_DAYS: i32 = 90;
/// How many failed mails to one address make a bounce worth showing.
pub const BOUNCES: i64 = 3;
/// What the panel shows in place of an erased guest's address.
pub const FORMER: &str = "Former applicant";

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn reason_of(body: &Value) -> &str {
    body.get("reason").and_then(Value::as_str).unwrap_or("")
}

/// `GET /admin/guests`: the forms that are open, the guests, and the signs of abuse.
pub fn overview(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let closed = guests::closed_forms(state);
    let panel = state
        .repository
        .with_client(move |c| {
            let forms: Vec<Value> = c
                .query(
                    "SELECT v.app_slug, v.id, v.name, \
                            (SELECT COUNT(*) FROM dataset_records r WHERE r.app_slug = v.app_slug AND r.data ? '_form' AND r.data->>'_form' = v.id::text), \
                            (SELECT to_char(MAX(r.created_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM dataset_records r WHERE r.app_slug = v.app_slug AND r.data ? '_form' AND r.data->>'_form' = v.id::text), \
                            (SELECT COUNT(*) FROM guest_links l WHERE l.form_id = v.id AND l.expires_at - INTERVAL '15 minutes' > NOW() - INTERVAL '1 day'), \
                            (SELECT COALESCE(SUM(GREATEST(rl.count - $1, 0)), 0) FROM rate_limits rl WHERE rl.key = 'guest-form:' || v.id::text AND rl.window_start > NOW() - INTERVAL '1 day') \
                     FROM app_views v WHERE v.kind = 'form' AND v.deleted_at IS NULL AND v.config->>'audience' = 'Guests' ORDER BY v.app_slug, v.name",
                    &[&guests::PER_FORM],
                )?
                .iter()
                .map(|r| {
                    let id: Uuid = r.get(1);
                    json!({
                        "app": r.get::<_, String>(0), "form_id": id, "name": r.get::<_, String>(2), "submissions": r.get::<_, i64>(3),
                        "last_submission": r.get::<_, Option<String>>(4), "link_requests_today": r.get::<_, i64>(5), "rate_limit_hits": r.get::<_, i64>(6),
                    })
                })
                .collect();
            let people: Vec<Value> = c
                .query(
                    "SELECT g.id, g.email, g.erased_at IS NOT NULL, g.disabled_at IS NOT NULL, \
                            to_char(g.last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                            (SELECT COUNT(*) FROM dataset_records r WHERE r.data ? '_submitter' AND r.data->>'_submitter' = 'guest:' || g.id::text || '@guest.invalid'), \
                            (SELECT COUNT(DISTINCT l.form_id) FROM guest_links l WHERE l.guest_id = g.id) \
                     FROM guests g ORDER BY g.created_at DESC LIMIT 200",
                    &[],
                )?
                .iter()
                .map(|r| {
                    let (erased, disabled) = (r.get::<_, bool>(2), r.get::<_, bool>(3));
                    json!({
                        "id": r.get::<_, Uuid>(0),
                        "address": if erased { FORMER.to_string() } else { r.get::<_, String>(1) },
                        "state": if erased { "Erased" } else if disabled { "Disabled" } else { "Active" },
                        "last_seen": r.get::<_, Option<String>>(4), "submissions": r.get::<_, i64>(5), "forms_used": r.get::<_, i64>(6),
                    })
                })
                .collect();
            // A bounce is a mail to the address that failed. Three of them are worth a look.
            let bounces: Vec<Value> = c
                .query(
                    "SELECT lower(o.to_address), COUNT(*) FROM email_outbox o JOIN guests g ON lower(g.email) = lower(o.to_address) \
                     WHERE o.state = 'failed' GROUP BY lower(o.to_address) HAVING COUNT(*) >= $1 ORDER BY COUNT(*) DESC LIMIT 50",
                    &[&BOUNCES],
                )?
                .iter()
                .map(|r| json!({ "address": r.get::<_, String>(0), "bounces": r.get::<_, i64>(1) }))
                .collect();
            // A hit is a request past the limit. The network and the address limits are read from the counts the limits keep.
            let mut over = |prefix: &str, limit: i32| -> Result<Vec<(String, i64)>, postgres::Error> {
                Ok(c.query(
                    "SELECT substr(key, length($1) + 1), SUM(GREATEST(count - $2, 0))::bigint FROM rate_limits WHERE key LIKE $1 || '%' AND window_start > NOW() - INTERVAL '1 day' \
                     GROUP BY key HAVING SUM(GREATEST(count - $2, 0)) > 0 ORDER BY 2 DESC LIMIT 50",
                    &[&prefix, &limit],
                )?
                .iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1)))
                .collect())
            };
            let networks: Vec<Value> = over("guest-net:", guests::PER_NETWORK)?.into_iter().map(|(network, hits)| json!({ "network": network, "hits": hits })).collect();
            Ok(json!({ "forms": forms, "guests": people, "abuse": { "bounces": bounces, "networks": networks } }))
        })
        .map_err(storage)?;
    let mut panel = panel;
    if let Some(forms) = panel["forms"].as_array_mut() {
        for f in forms {
            let closed_now = f["form_id"].as_str().is_some_and(|id| closed.iter().any(|c| c == id));
            f["closed"] = json!(closed_now);
        }
    }
    Ok(panel)
}

fn form_app(state: &SharedState, form: Uuid) -> Result<String, ServiceError> {
    state
        .repository
        .with_client(move |c| Ok(c.query_opt("SELECT app_slug FROM app_views WHERE id = $1 AND kind = 'form' AND deleted_at IS NULL", &[&form])?.map(|r| r.get::<_, String>(0))))
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("Form not found".to_string()))
}

/// `POST /admin/guests/forms/{id}/close` and `.../open`. The setting `forms.disabled` is the kill switch. The change takes effect on
/// the next request, on every server, because every server reads the setting.
pub fn set_form_closed(caller: &AuthUser, form: Uuid, closed: bool, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let app = form_app(state, form)?;
    let reason = reason_of(body).to_string();
    let verb = if closed { "closed" } else { "opened" };
    let payload = json!({ "form": form, "app": app, "action": verb });
    admin_write(state, caller, DecisionType::GuestChanged, "AC-3", &reason, &payload, || {
        let mut list = guests::closed_forms(state);
        let id = form.to_string();
        list.retain(|f| *f != id);
        if closed {
            list.push(id);
        }
        let value = json!(list);
        state
            .repository
            .put_platform_setting(
                "forms.disabled",
                &value,
                &caller.eppn,
                crate::repository::AppendDecisionParams {
                    principal: caller.eppn.clone(),
                    organization_code: "INST".to_string(),
                    app_slug: None,
                    decision_type: DecisionType::PolicyRevision,
                    oscal_control_id: "AC-03".to_string(),
                    rationale: format!("Platform setting 'forms.disabled' updated by {}", caller.eppn),
                    payload: json!({ "key": "forms.disabled", "form": form }),
                },
            )
            .map_err(|e| ServiceError::Internal(format!("Database write failed: {e}")))?;
        state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The setting was saved, but this server could not reload: {e}")))?;
        Ok(json!({ "form_id": form, "closed": closed }))
    })
}

fn guest_exists(state: &SharedState, id: Uuid) -> Result<(bool, bool), ServiceError> {
    state
        .repository
        .with_client(move |c| Ok(c.query_opt("SELECT erased_at IS NOT NULL, disabled_at IS NOT NULL FROM guests WHERE id = $1", &[&id])?.map(|r| (r.get::<_, bool>(0), r.get::<_, bool>(1)))))
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("Guest not found".to_string()))
}

/// `POST /admin/guests/{id}/disable` and `.../enable`. A disabled guest's session stops on its next request, because it is read each time.
pub fn set_disabled(caller: &AuthUser, id: Uuid, disabled: bool, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (erased, _) = guest_exists(state, id)?;
    if erased {
        return Err(ServiceError::Conflict("This guest was erased".to_string()));
    }
    let payload = json!({ "guest": id, "action": if disabled { "disabled" } else { "enabled" } });
    admin_write(state, caller, DecisionType::GuestChanged, "AC-2", reason_of(body), &payload, || {
        state
            .repository
            .with_client(move |c| Ok(c.execute("UPDATE guests SET disabled_at = CASE WHEN $2 THEN NOW() ELSE NULL END WHERE id = $1 AND erased_at IS NULL", &[&id, &disabled])?))
            .map_err(storage)?;
        Ok(json!({ "id": id, "state": if disabled { "Disabled" } else { "Active" } }))
    })
}

/// `POST /admin/guests/{id}/erase`: the address becomes a hash, and nothing is left that could send the person mail or let them in. The
/// records stay, with the same submitter, so retention and a request to be forgotten are handled apart.
pub fn erase(caller: &AuthUser, id: Uuid, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (erased, _) = guest_exists(state, id)?;
    if erased {
        return Err(ServiceError::Conflict("This guest was already erased".to_string()));
    }
    let payload = json!({ "guest": id, "action": "erased" });
    admin_write(state, caller, DecisionType::GuestChanged, "AC-2", reason_of(body), &payload, || {
        let address = guests::address_of(id);
        state
            .repository
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let email: String = tx.query_one("SELECT email FROM guests WHERE id = $1 FOR UPDATE", &[&id])?.get(0);
                let hash: String = Sha256::digest(email.to_lowercase().as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
                tx.execute("UPDATE guests SET email = $2, erased_at = NOW(), disabled_at = COALESCE(disabled_at, NOW()) WHERE id = $1", &[&id, &format!("erased:{hash}")])?;
                // Mail that has not gone, links that have not been used, and the choices the address made.
                tx.execute("DELETE FROM email_outbox WHERE lower(to_address) = lower($1) AND state = 'queued'", &[&email])?;
                tx.execute("DELETE FROM guest_links WHERE guest_id = $1", &[&id])?;
                tx.execute("DELETE FROM guest_stops WHERE guest_id = $1", &[&id])?;
                tx.execute("DELETE FROM notification_prefs WHERE eppn = $1", &[&address])?;
                tx.execute("UPDATE api_tokens SET revoked_at = NOW() WHERE kind = 'guest' AND eppn = $1 AND revoked_at IS NULL", &[&address])?;
                tx.commit()?;
                Ok(())
            })
            .map_err(storage)?;
        Ok(json!({ "id": id, "state": "Erased" }))
    })
}

/// Removes guests with no records and no activity for `SILENT_DAYS`. A guest who made a record is kept, because the record's
/// submitter names them. Returns how many were removed.
pub fn purge(repo: &crate::repository::PostgresRepository) -> Result<u64, crate::repository::RepositoryError> {
    repo.with_client(|c| {
        let mut tx = c.transaction()?;
        let gone: Vec<Uuid> = tx
            .query(
                "SELECT g.id FROM guests g WHERE g.erased_at IS NULL AND COALESCE(g.last_seen_at, g.created_at) < NOW() - make_interval(days => $1) \
                 AND NOT EXISTS (SELECT 1 FROM dataset_records r WHERE r.data ? '_submitter' AND r.data->>'_submitter' = 'guest:' || g.id::text || '@guest.invalid')",
                &[&SILENT_DAYS],
            )?
            .iter()
            .map(|r| r.get(0))
            .collect();
        if gone.is_empty() {
            return Ok(0);
        }
        let addresses: Vec<String> = gone.iter().map(|g| guests::address_of(*g)).collect();
        tx.execute("DELETE FROM guest_links WHERE guest_id = ANY($1)", &[&gone])?;
        tx.execute("DELETE FROM guest_stops WHERE guest_id = ANY($1)", &[&gone])?;
        tx.execute("DELETE FROM notification_prefs WHERE eppn = ANY($1)", &[&addresses])?;
        tx.execute("DELETE FROM api_tokens WHERE kind = 'guest' AND eppn = ANY($1)", &[&addresses])?;
        let n = tx.execute("DELETE FROM guests WHERE id = ANY($1)", &[&gone])?;
        tx.commit()?;
        Ok(n)
    })
}

/// The `purge_guests` job, run daily.
pub fn purge_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let n = purge(ctx.repository()).map_err(|e| crate::jobs::JobError::Retry(e.to_string()))?;
    Ok(json!({ "removed": n }))
}
