//! Integrations phase 4: webhooks.
//!
//! A webhook is part of an app's manifest, so it is approved with the app, through a proposal. A write that matches
//! one queues a delivery in `webhook_deliveries` in the write's own transaction, so a write that rolls back sends
//! nothing. The `deliver_webhooks` job claims due deliveries, signs each body, and posts it through `net::egress`,
//! which refuses any host not on `egress.allow`, any private address, and any redirect.
//!
//! The signature is `X-Scaffoldry-Signature: t={unix},v1={hex}`: HMAC-SHA256 over `{t}.{body}` with the webhook's
//! secret. A `2xx` is delivered. Anything else is tried again after 1 minute, 5 minutes, 30 minutes, 2 hours, 6 hours,
//! 12 hours and 24 hours, and then the delivery is `dead`. Twenty failed deliveries in a row turn the webhook off and
//! tell its workspace's owners. A body names the record by id; it carries values only when the webhook was approved
//! with `include_values`, and then only its listed fields.

use crate::net::egress::{Client, EgressError};
use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::SharedState;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use scaffoldry_engine::{HostRouter, Webhook, WebhookEvent};
use serde_json::{json, Map, Value};
use sha2::Sha256;
use uuid::Uuid;

/// The waits after each failed try, in seconds. A delivery that fails once more after the last is `dead`.
pub const DELAYS: [i64; 7] = [60, 300, 1_800, 7_200, 21_600, 43_200, 86_400];
/// The first try and one retry for each delay.
pub const MAX_ATTEMPTS: i32 = DELAYS.len() as i32 + 1;
/// Failed deliveries in a row that turn a webhook off.
pub const DISABLE_AFTER: i32 = 20;
/// Deliveries kept per webhook for its call log.
pub const KEEP_PER_WEBHOOK: i64 = 200;
/// The largest body sent.
pub const MAX_BODY: usize = 256 * 1024;
/// Deliveries one run of the job claims.
pub const BATCH: i64 = 50;
/// How long a claimed delivery is held before another worker may take it.
const LEASE_SECS: i64 = 300;

type HmacSha256 = Hmac<Sha256>;

/// The webhook event a record write is.
fn event_of(action: &str) -> Option<WebhookEvent> {
    match action {
        "create" | "restore" => Some(WebhookEvent::RecordCreated),
        "delete" => Some(WebhookEvent::RecordDeleted),
        "comment" => None,
        _ => Some(WebhookEvent::RecordUpdated),
    }
}

/// The body of one event. It names the record and the changed fields, and holds values only for a webhook approved to
/// send them, and only its listed fields.
#[allow(clippy::too_many_arguments)]
fn payload(hook: &Webhook, event: &str, app: &str, table: &str, record: &str, version: i32, changed: &[String], data: Option<&Value>) -> (Uuid, Value) {
    let id = Uuid::new_v4();
    let mut body = json!({
        "id": id, "event": event, "app": app, "table": table, "record": record, "version": version, "at": Utc::now().to_rfc3339(), "changed": changed,
    });
    if hook.include_values {
        let mut values = Map::new();
        if let Some(data) = data {
            for f in &hook.fields {
                if let Some(v) = data.get(f) {
                    values.insert(f.clone(), v.clone());
                }
            }
        }
        body["values"] = Value::Object(values);
    }
    (id, body)
}

/// Queues a delivery for each webhook a record write matches, in the write's transaction. Called from `write_history`,
/// which every record write calls inside its transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_in_tx(
    tx: &mut postgres::Transaction<'_>,
    hooks: &[Webhook],
    app: &str,
    table: &str,
    record: &str,
    version: i32,
    action: &str,
    changes: &Map<String, Value>,
) -> Result<(), postgres::Error> {
    let Some(event) = event_of(action) else { return Ok(()) };
    let mut matching: Vec<&Webhook> = hooks.iter().filter(|h| h.events.contains(&event) && h.tables.iter().any(|t| t == table)).collect();
    if matching.is_empty() {
        return Ok(());
    }
    // A webhook that is off queues nothing. Turning it on again does not send what happened while it was off.
    let off: Vec<String> = tx.query("SELECT webhook_id FROM webhook_state WHERE app_slug = $1 AND disabled_at IS NOT NULL", &[&app])?.iter().map(|r| r.get(0)).collect();
    matching.retain(|h| !off.contains(&h.id));
    if matching.is_empty() {
        return Ok(());
    }
    // The record as this transaction has it, read only when a filter or a value needs it. A deleted record has none.
    let needs = matching.iter().any(|h| h.include_values || h.filter.is_some());
    let data: Option<Value> = if needs && event != WebhookEvent::RecordDeleted {
        tx.query_opt("SELECT data FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&app, &record])?.map(|r| r.get(0))
    } else {
        None
    };
    let mut changed: Vec<String> = changes.keys().filter(|k| !k.starts_with('_')).cloned().collect();
    changed.sort();
    for hook in matching {
        if let (Some(filter), Some(d)) = (&hook.filter, &data) {
            if !scaffoldry_engine::matches_filter(d, filter) {
                continue;
            }
        }
        let (event_id, body) = payload(hook, event.wire_name(), app, table, record, version, &changed, data.as_ref());
        tx.execute(
            "INSERT INTO webhook_deliveries (id, app_slug, webhook_id, event_id, payload) VALUES ($1, $2, $3, $4, $5)",
            &[&Uuid::new_v4(), &app, &hook.id, &event_id, &body],
        )?;
    }
    Ok(())
}

/// Queues one delivery outside a record write: a process step's `WebhookDispatch` (`process.step`), or a decision
/// (`process.decided`). `webhook` names one webhook; `None` sends to every webhook of the app that listens for
/// `ProcessDecided` on the table.
pub fn enqueue_event(state: &SharedState, app: &str, table: &str, record: &str, event: &str, webhook: Option<&str>) {
    let hooks: Vec<Webhook> = state.engine.read().ok().and_then(|e| e.resolve_by_slug(app).map(|m| m.integrations.webhooks.clone())).unwrap_or_default();
    let chosen: Vec<&Webhook> = match webhook {
        Some(id) => hooks.iter().filter(|h| h.id == id).collect(),
        None => hooks.iter().filter(|h| h.events.contains(&WebhookEvent::ProcessDecided) && h.tables.iter().any(|t| t == table)).collect(),
    };
    let rec = state.find_record(app, record).ok().flatten();
    for hook in chosen {
        if disabled(state, app, &hook.id) {
            continue;
        }
        let data = rec.as_ref().map(|r| r.data.clone());
        if let (Some(filter), Some(d)) = (&hook.filter, &data) {
            if !scaffoldry_engine::matches_filter(d, filter) {
                continue;
            }
        }
        let version = rec.as_ref().map(|r| r.version).unwrap_or(0);
        let (event_id, body) = payload(hook, event, app, table, record, version, &[], data.as_ref());
        let (a, h) = (app.to_string(), hook.id.clone());
        let _ = state.repository.with_client(move |c| {
            c.execute("INSERT INTO webhook_deliveries (id, app_slug, webhook_id, event_id, payload) VALUES ($1, $2, $3, $4, $5)", &[&Uuid::new_v4(), &a, &h, &event_id, &body])?;
            Ok(())
        });
    }
}

/// After a process step's actions or a decision commit: queues each `WebhookDispatch` of the actions with the event
/// `process.step`, and, for a decision, `process.decided` to every webhook of the app that listens for it.
pub fn after_process(state: &SharedState, app: &str, record: &str, effects: &[scaffoldry_core::workflow::ActionEffect], decided: bool) {
    let table = state.find_record(app, record).ok().flatten().and_then(|r| r.data.get("_table_id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    for e in effects {
        if let scaffoldry_core::workflow::ActionEffect::Webhook { webhook_id } = e {
            enqueue_event(state, app, &table, record, "process.step", Some(webhook_id));
        }
    }
    if decided {
        enqueue_event(state, app, &table, record, "process.decided", None);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn mac(secret: &[u8], t: i64, body: &[u8]) -> HmacSha256 {
    let mut m = <HmacSha256 as KeyInit>::new_from_slice(secret).expect("HMAC takes a key of any length");
    m.update(format!("{t}.").as_bytes());
    m.update(body);
    m
}

/// The signature header's value for a body sent at `t`.
pub fn signature(secret: &[u8], t: i64, body: &[u8]) -> String {
    format!("t={t},v1={}", hex(&mac(secret, t, body).finalize().into_bytes()))
}

/// Whether `header` is the signature of `body` with `secret`. A receiver checks this, and should also refuse a `t`
/// that is far from now.
pub fn verify(secret: &[u8], header: &str, body: &[u8]) -> bool {
    let mut t = None;
    let mut v1 = None;
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => v1 = unhex(v),
            _ => {}
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else { return false };
    mac(secret, t, body).verify_slice(&v1).is_ok()
}

struct Due {
    id: Uuid,
    app: String,
    webhook: String,
    payload: Value,
    attempts: i32,
}

/// Claims up to `BATCH` due deliveries of webhooks that are on, holding them for a lease so another worker skips them.
fn claim(repo: &PostgresRepository, now: DateTime<Utc>) -> Result<Vec<Due>, RepositoryError> {
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        let rows = tx.query(
            "SELECT d.id, d.app_slug, d.webhook_id, d.payload, d.attempts FROM webhook_deliveries d \
             WHERE d.state = 'queued' AND d.next_attempt_at <= $1 \
               AND NOT EXISTS (SELECT 1 FROM webhook_state s WHERE s.app_slug = d.app_slug AND s.webhook_id = d.webhook_id AND s.disabled_at IS NOT NULL) \
             ORDER BY d.next_attempt_at, d.created_at LIMIT $2 FOR UPDATE SKIP LOCKED",
            &[&now, &BATCH],
        )?;
        let due: Vec<Due> = rows.iter().map(|r| Due { id: r.get(0), app: r.get(1), webhook: r.get(2), payload: r.get(3), attempts: r.get(4) }).collect();
        let ids: Vec<Uuid> = due.iter().map(|d| d.id).collect();
        let lease = now + chrono::Duration::seconds(LEASE_SECS);
        tx.execute("UPDATE webhook_deliveries SET next_attempt_at = $2 WHERE id = ANY($1)", &[&ids, &lease])?;
        tx.commit()?;
        Ok(due)
    })
}

/// Why a try failed, and the status the receiver answered, when it answered.
struct Failure {
    status: Option<i32>,
    error: String,
}

fn try_once(state: &SharedState, client: &Client, hook: &Webhook, body: &[u8], event_id: &str, now: DateTime<Utc>) -> Result<i32, Failure> {
    let fail = |error: String| Failure { status: None, error };
    let allow_http = state.settings.read().ok().and_then(|s| s.get("webhooks.allow_http").and_then(Value::as_bool)).unwrap_or(false);
    if !hook.url.starts_with("https://") && !(allow_http && hook.url.starts_with("http://")) {
        return Err(fail("The webhook's address is not https".to_string()));
    }
    if body.len() > MAX_BODY {
        return Err(fail(format!("The body is {} bytes, over the limit of {MAX_BODY}", body.len())));
    }
    let secret = crate::secrets::get(state, &hook.secret).map_err(|e| fail(format!("The signing secret '{}' cannot be read: {e}", hook.secret)))?;
    let sig = signature(secret.expose(), now.timestamp(), body);
    let headers = [("Content-Type", "application/json"), ("X-Scaffoldry-Signature", sig.as_str()), ("X-Scaffoldry-Event-Id", event_id)];
    match client.post(&hook.url, &headers, body) {
        Ok(resp) if (200..300).contains(&resp.status) => Ok(resp.status as i32),
        Ok(resp) => Err(Failure { status: Some(resp.status as i32), error: format!("The receiver answered {}", resp.status) }),
        Err(EgressError::Redirect(to)) => Err(fail(format!("The receiver answered with a redirect to {to}, which is a failure"))),
        Err(e) => Err(fail(e.to_string())),
    }
}

/// Sends the deliveries due at `now` through `client`. Returns how many were tried.
pub fn deliver_due(state: &SharedState, client: &Client, now: DateTime<Utc>) -> Result<usize, String> {
    let due = claim(&state.repository, now).map_err(|e| e.to_string())?;
    let tried = due.len();
    for d in due {
        let hook = state.engine.read().ok().and_then(|e| e.resolve_by_slug(&d.app).and_then(|m| m.integrations.webhooks.iter().find(|h| h.id == d.webhook).cloned()));
        let Some(hook) = hook else {
            finish(state, &d, "dead", None, Some("The webhook is no longer in the app".to_string()), now);
            continue;
        };
        // A webhook turned off during this batch sends nothing more.
        if disabled(state, &d.app, &d.webhook) {
            release(state, &d, now);
            continue;
        }
        let body = serde_json::to_vec(&d.payload).unwrap_or_default();
        let event_id = d.payload.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        match try_once(state, client, &hook, &body, &event_id, now) {
            Ok(status) => {
                finish(state, &d, "delivered", Some(status), None, now);
                succeeded(state, &d.app, &d.webhook);
                disclose(state, &hook, &d, now);
            }
            Err(f) => {
                let attempts = d.attempts + 1;
                if attempts >= MAX_ATTEMPTS {
                    finish(state, &d, "dead", f.status, Some(f.error), now);
                } else {
                    let next = now + chrono::Duration::seconds(DELAYS[(attempts - 1) as usize]);
                    retry_at(state, &d, f.status, &f.error, next);
                }
                failed(state, &d.app, &d.webhook);
            }
        }
    }
    Ok(tried)
}

fn finish(state: &SharedState, d: &Due, outcome: &str, status: Option<i32>, error: Option<String>, now: DateTime<Utc>) {
    let (id, outcome) = (d.id, outcome.to_string());
    let delivered = (outcome == "delivered").then_some(now);
    let _ = state.repository.with_client(move |c| {
        c.execute(
            "UPDATE webhook_deliveries SET state = $2, attempts = attempts + 1, last_status = $3, last_error = $4, delivered_at = $5, finished_at = NOW() WHERE id = $1",
            &[&id, &outcome, &status, &error, &delivered],
        )?;
        Ok(())
    });
}

fn retry_at(state: &SharedState, d: &Due, status: Option<i32>, error: &str, next: DateTime<Utc>) {
    let (id, error) = (d.id, error.to_string());
    let _ = state.repository.with_client(move |c| {
        c.execute("UPDATE webhook_deliveries SET attempts = attempts + 1, last_status = $2, last_error = $3, next_attempt_at = $4 WHERE id = $1", &[&id, &status, &error, &next])?;
        Ok(())
    });
}

/// Gives a claimed delivery back, untried.
fn release(state: &SharedState, d: &Due, now: DateTime<Utc>) {
    let id = d.id;
    let _ = state.repository.with_client(move |c| {
        c.execute("UPDATE webhook_deliveries SET next_attempt_at = $2 WHERE id = $1", &[&id, &now])?;
        Ok(())
    });
}

fn disabled(state: &SharedState, app: &str, webhook: &str) -> bool {
    let (a, w) = (app.to_string(), webhook.to_string());
    state
        .repository
        .with_client(move |c| Ok(c.query_opt("SELECT 1 FROM webhook_state WHERE app_slug = $1 AND webhook_id = $2 AND disabled_at IS NOT NULL", &[&a, &w])?.is_some()))
        .unwrap_or(false)
}

fn succeeded(state: &SharedState, app: &str, webhook: &str) {
    let (a, w) = (app.to_string(), webhook.to_string());
    let _ = state.repository.with_client(move |c| {
        c.execute(
            "INSERT INTO webhook_state (app_slug, webhook_id, consecutive_failures) VALUES ($1, $2, 0) \
             ON CONFLICT (app_slug, webhook_id) DO UPDATE SET consecutive_failures = 0",
            &[&a, &w],
        )?;
        Ok(())
    });
}

/// Counts a failed delivery. The twentieth in a row turns the webhook off and tells its workspace's owners.
fn failed(state: &SharedState, app: &str, webhook: &str) {
    let (a, w) = (app.to_string(), webhook.to_string());
    let turned_off = state
        .repository
        .with_client(move |c| {
            let n: i32 = c
                .query_one(
                    "INSERT INTO webhook_state (app_slug, webhook_id, consecutive_failures) VALUES ($1, $2, 1) \
                     ON CONFLICT (app_slug, webhook_id) DO UPDATE SET consecutive_failures = webhook_state.consecutive_failures + 1 \
                     RETURNING consecutive_failures",
                    &[&a, &w],
                )?
                .get(0);
            let off = n >= DISABLE_AFTER && c.execute("UPDATE webhook_state SET disabled_at = NOW() WHERE app_slug = $1 AND webhook_id = $2 AND disabled_at IS NULL", &[&a, &w])? == 1;
            Ok(off)
        })
        .unwrap_or(false);
    if turned_off {
        let workspace = state.engine.read().ok().and_then(|e| e.resolve_by_slug(app).and_then(|m| m.workspace_id.clone()));
        if let Some(workspace) = workspace {
            let notice = crate::notifications::Notice::WebhookDisabled { app: crate::notifications::AppRef(app.to_string()) };
            let to = crate::notifications::Recipients::Workspace { workspace, roles: vec!["owner".into()] };
            let _ = crate::notifications::notify(state, &to, None, &notice);
        }
    }
}

/// Turns a webhook on again and clears its count of failures.
pub fn enable(repo: &PostgresRepository, app: &str, webhook: &str) -> Result<(), RepositoryError> {
    let (a, w) = (app.to_string(), webhook.to_string());
    repo.with_client(move |c| {
        c.execute(
            "INSERT INTO webhook_state (app_slug, webhook_id, consecutive_failures, disabled_at) VALUES ($1, $2, 0, NULL) \
             ON CONFLICT (app_slug, webhook_id) DO UPDATE SET consecutive_failures = 0, disabled_at = NULL",
            &[&a, &w],
        )?;
        Ok(())
    })
}

/// Turns a webhook off on an administrator's word. What was queued ends `dead`, with the reason in its error, so the
/// call log keeps it. Nothing more is queued or sent until it is turned on again.
pub fn disable(repo: &PostgresRepository, app: &str, webhook: &str, reason: &str) -> Result<(), RepositoryError> {
    let (a, w, error) = (app.to_string(), webhook.to_string(), format!("Turned off by an administrator: {reason}"));
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        tx.execute(
            "INSERT INTO webhook_state (app_slug, webhook_id, consecutive_failures, disabled_at) VALUES ($1, $2, 0, NOW()) \
             ON CONFLICT (app_slug, webhook_id) DO UPDATE SET disabled_at = COALESCE(webhook_state.disabled_at, NOW())",
            &[&a, &w],
        )?;
        tx.execute(
            "UPDATE webhook_deliveries SET state = 'dead', last_error = $3, finished_at = NOW() WHERE app_slug = $1 AND webhook_id = $2 AND state = 'queued'",
            &[&a, &w, &error],
        )?;
        tx.commit()?;
        Ok(())
    })
}

/// A delivered body that carried an effectively sensitive value adds to the disclosure log: one entry per webhook and
/// hour, with the receiver's host as the recipient, the count of deliveries, and the record ids up to the cap.
fn disclose(state: &SharedState, hook: &Webhook, d: &Due, now: DateTime<Utc>) {
    let Some(values) = d.payload.get("values").and_then(Value::as_object) else { return };
    let table = d.payload.get("table").and_then(Value::as_str).unwrap_or_default().to_string();
    let sensitive = crate::service::records::sensitive_fields_of(&d.app, &table, state);
    let fields: Vec<String> = values.iter().filter(|(k, v)| !v.is_null() && sensitive.contains(*k)).map(|(k, _)| k.clone()).collect();
    if fields.is_empty() {
        return;
    }
    let host = hook.url.split("://").nth(1).and_then(|r| r.split(['/', ':', '?']).next()).unwrap_or_default().to_string();
    let record = d.payload.get("record").and_then(Value::as_str).unwrap_or_default().to_string();
    let (app, note, actor) = (d.app.clone(), format!("webhook {}", hook.id), format!("webhook:{}", hook.id));
    let hour = now.date_naive().and_hms_opt(now.time().format("%H").to_string().parse().unwrap_or(0), 0, 0).map(|t| t.and_utc()).unwrap_or(now);
    let cap = crate::repository_disclosure::MAX_RECORDS_PER_ENTRY as i64;
    let _ = state.repository.with_client(move |c| {
        let mut tx = c.transaction()?;
        // One writer at a time per webhook and receiver, so an hour never gets two entries.
        tx.execute("SELECT pg_advisory_xact_lock(hashtext($1 || '|' || $2))", &[&note, &host])?;
        let existing = tx.query_opt(
            "SELECT id, record_count FROM disclosure_log WHERE kind = 'webhook' AND app_slug = $1 AND note = $2 AND recipient = $3 AND at >= $4 AND at < $4 + INTERVAL '1 hour' LIMIT 1",
            &[&app, &note, &host, &hour],
        )?;
        let id: Uuid = match existing {
            Some(r) => {
                let (id, count): (Uuid, i64) = (r.get(0), r.get(1));
                tx.execute(
                    "UPDATE disclosure_log SET record_count = record_count + 1, fields = ARRAY(SELECT DISTINCT unnest(fields || $2::text[]) ORDER BY 1), records_truncated = records_truncated OR $3 WHERE id = $1",
                    &[&id, &fields, &(count + 1 > cap)],
                )?;
                id
            }
            None => {
                let id = Uuid::new_v4();
                tx.execute(
                    "INSERT INTO disclosure_log (id, at, actor, kind, app_slug, table_id, record_count, fields, recipient, note) VALUES ($1, $2, $3, 'webhook', $4, $5, 1, $6, $7, $8)",
                    &[&id, &hour, &actor, &app, &table, &fields, &host, &note],
                )?;
                id
            }
        };
        if !record.is_empty() {
            tx.execute(
                "INSERT INTO disclosure_records (disclosure_id, record_id) SELECT $1, $2 WHERE (SELECT records_truncated FROM disclosure_log WHERE id = $1) = FALSE ON CONFLICT DO NOTHING",
                &[&id, &record],
            )?;
        }
        tx.commit()?;
        Ok(())
    });
}

/// Keeps the newest `KEEP_PER_WEBHOOK` finished deliveries of each webhook and removes the rest. Returns how many.
pub fn purge_deliveries(repo: &PostgresRepository) -> Result<u64, RepositoryError> {
    repo.with_client(move |c| {
        Ok(c.execute(
            "DELETE FROM webhook_deliveries WHERE id IN (SELECT id FROM (SELECT id, row_number() OVER (PARTITION BY app_slug, webhook_id ORDER BY created_at DESC) AS n \
             FROM webhook_deliveries WHERE state IN ('delivered', 'dead')) kept WHERE n > $1)",
            &[&KEEP_PER_WEBHOOK],
        )?)
    })
}

/// A webhook's newest deliveries for its call log: state, tries, the last answer, and times. Never a body.
pub fn call_log(repo: &PostgresRepository, app: &str, webhook: &str) -> Result<Vec<Value>, RepositoryError> {
    let (a, w) = (app.to_string(), webhook.to_string());
    repo.with_client(move |c| {
        Ok(c.query(
            "SELECT json_build_object('id', id, 'event', payload->>'event', 'record', payload->>'record', 'state', state, 'attempts', attempts, \
                    'last_status', last_status, 'last_error', last_error, 'created_at', created_at, 'next_attempt_at', next_attempt_at, 'delivered_at', delivered_at) \
             FROM webhook_deliveries WHERE app_slug = $1 AND webhook_id = $2 ORDER BY created_at DESC LIMIT $3",
            &[&a, &w, &KEEP_PER_WEBHOOK],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect())
    })
}

/// The `deliver_webhooks` job: sends what is due through the server's allowlist.
pub fn deliver_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let state = ctx.state().ok_or_else(|| crate::jobs::JobError::Permanent("delivering webhooks needs the server's state, and this runner has none".to_string()))?;
    let tried = deliver_due(state, &Client::for_state(state), Utc::now()).map_err(crate::jobs::JobError::Retry)?;
    Ok(json!({ "tried": tried }))
}

/// The `purge_webhook_deliveries` job.
pub fn purge_job(ctx: &crate::jobs::JobCtx, _payload: Value) -> Result<Value, crate::jobs::JobError> {
    let removed = purge_deliveries(ctx.repository()).map_err(|e| crate::jobs::JobError::Retry(e.to_string()))?;
    Ok(json!({ "removed": removed }))
}

/// Whether a JSON value is an old webhook action: `WebhookDispatch` naming a URL, from before a webhook was part of the
/// manifest. A rule that names an approved webhook by id is not one.
fn is_url_webhook(action: &Value) -> bool {
    let Some(w) = action.get("WebhookDispatch") else { return false };
    w.get("target_url").is_some() || w.get("webhook_id").and_then(Value::as_str).is_some_and(|id| id.contains("://"))
}

/// Removes old webhook actions from `v`, and the process steps that hold one, and counts what it removed.
fn strip_url_webhooks(v: &mut Value) -> u64 {
    match v {
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !(is_url_webhook(item) || item.pointer("/kind/Service/action").is_some_and(is_url_webhook)));
            let mut removed = (before - items.len()) as u64;
            for item in items.iter_mut() {
                removed += strip_url_webhooks(item);
            }
            removed
        }
        Value::Object(map) => map.values_mut().map(strip_url_webhooks).sum(),
        _ => 0,
    }
}

/// Integrations phase 4, at boot: a webhook is part of the app's manifest now, so a live rule that sent to a URL loses
/// that step. One ledger entry says how many were dropped. A rule version already stored is left as it was.
pub fn drop_url_webhook_steps(repo: &PostgresRepository) -> Result<u64, RepositoryError> {
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        let rows = tx.query("SELECT id, rule_json FROM workflow_automations", &[])?;
        let mut dropped = 0u64;
        let mut rules = Vec::new();
        for r in rows {
            let (id, mut json): (String, Value) = (r.get(0), r.get(1));
            let n = strip_url_webhooks(&mut json);
            if n > 0 {
                tx.execute("UPDATE workflow_automations SET rule_json = $2 WHERE id = $1", &[&id, &json])?;
                rules.push(id);
                dropped += n;
            }
        }
        if dropped > 0 {
            crate::repository::append_ledger_in_tx(
                &mut tx,
                crate::repository::AppendDecisionParams {
                    principal: "system".to_string(),
                    organization_code: "institutional".to_string(),
                    app_slug: None,
                    decision_type: scaffoldry_core::DecisionType::ProcessDefinitionChanged,
                    oscal_control_id: "CM-03".to_string(),
                    rationale: format!("{dropped} webhook steps that named a URL were dropped from live rules, because a webhook is now approved with its app"),
                    payload: json!({ "dropped": dropped, "rules": rules }),
                },
            )?;
        }
        tx.commit()?;
        Ok(dropped)
    })
}
