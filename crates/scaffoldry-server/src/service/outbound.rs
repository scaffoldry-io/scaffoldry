//! Integrations phase 5: the rules behind the console's outbound panels. A Platform Admin edits the outbound allowlist,
//! tests a host against it, reads every webhook across apps with its recent deliveries, and turns a webhook off or on.
//! Each change carries a reason and a ledger entry.

use crate::net::egress::{resolve_and_check, Allow, EgressError, Resolver, SystemResolver};
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn reason_of(body: &Value) -> String {
    body.get("reason").and_then(Value::as_str).unwrap_or("").to_string()
}

/// The allowlist as saved. An empty list when none is.
pub fn egress(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let allow = state.settings.read().map_err(|e| ServiceError::Internal(e.to_string()))?.get("egress.allow").cloned().unwrap_or_else(|| json!([]));
    Ok(json!({ "allow": allow }))
}

/// Saves the whole list, with the reason. It is the setting `egress.allow`, so the change is the same ledger entry any
/// setting makes, and it names the reason.
pub fn egress_save(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = reason_of(body);
    let reason = reason.trim();
    if reason.is_empty() || reason.len() > 500 {
        return Err(ServiceError::bad_request("A reason between 1 and 500 characters is required for administrative actions"));
    }
    let allow = body.get("allow").cloned().unwrap_or(Value::Null);
    Allow::parse(&allow).map_err(|m| ServiceError::bad_request(format!("The allowlist is not valid: {m}")))?;
    let digest: String = Sha256::digest(serde_json::to_vec(&allow).unwrap_or_default()).iter().map(|b| format!("{b:02x}")).collect();
    state
        .repository
        .put_platform_setting(
            "egress.allow",
            &allow,
            &caller.eppn,
            crate::repository::AppendDecisionParams {
                principal: caller.eppn.clone(),
                organization_code: "INST".to_string(),
                app_slug: None,
                decision_type: DecisionType::PolicyRevision,
                oscal_control_id: "SC-07".to_string(),
                rationale: format!("Platform setting 'egress.allow' updated by {}: {reason}", caller.eppn),
                payload: json!({ "key": "egress.allow", "value_sha256": digest, "reason": reason }),
            },
        )
        .map_err(storage)?;
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The list was saved, but this server could not reload: {e}")))?;
    Ok(json!({ "allow": allow }))
}

/// What the outbound check says of `host` and `port`, in words. Nothing is sent: a name is looked up only when the list
/// could allow it, exactly as a real connection does.
pub fn egress_test(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    egress_test_with(caller, body, state, &SystemResolver)
}

pub fn egress_test_with(caller: &AuthUser, body: &Value, state: &SharedState, resolver: &dyn Resolver) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let host = body.get("host").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if host.is_empty() || host.len() > 253 || host.contains(|c: char| c.is_whitespace() || c == '/' || c == '@') {
        return Err(ServiceError::bad_request("Give a host name or an address, with no scheme or path"));
    }
    let port = match body.get("port") {
        None | Some(Value::Null) => 443,
        Some(p) => p.as_u64().filter(|p| (1..=65535).contains(p)).ok_or_else(|| ServiceError::bad_request("A port is a number from 1 to 65535"))? as u16,
    };
    let allow = {
        let settings = state.settings.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
        Allow::from_settings(&settings)
    };
    let (allowed, address, explanation) = match resolve_and_check(&host, port, &allow, resolver) {
        Ok(addr) => (
            true,
            Some(addr.ip().to_string()),
            format!("Allowed. {host}:{port} is on the list. It resolves to {}, and a connection would go to that address and no other.", addr.ip()),
        ),
        Err(EgressError::NotAllowed(m)) => (false, None, format!("Refused. {m}. Add an entry for it to allow it.")),
        Err(EgressError::Blocked(m)) => (false, None, format!("Blocked. {m}. Only a network entry that names it on purpose can allow it.")),
        Err(e) => (false, None, format!("Could not check. {e}.")),
    };
    Ok(json!({ "host": host, "port": port, "allowed": allowed, "address": address, "explanation": explanation }))
}

/// A webhook's app and id.
type Key = (String, String);

/// The host of a webhook's address.
fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split(['/', ':', '?']).next()).unwrap_or_default().to_string()
}

/// Every webhook of every app: where it goes, what it listens for, whether it is on, when it last succeeded, and how
/// many deliveries in a row have failed. Never a secret's value, only its name.
pub fn webhooks(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let (states, last): (HashMap<Key, (i32, bool)>, HashMap<Key, String>) = state
        .repository
        .with_client(|c| {
            let states = c
                .query("SELECT app_slug, webhook_id, consecutive_failures, disabled_at IS NOT NULL FROM webhook_state", &[])?
                .iter()
                .map(|r| ((r.get::<_, String>(0), r.get::<_, String>(1)), (r.get::<_, i32>(2), r.get::<_, bool>(3))))
                .collect();
            let last = c
                .query("SELECT app_slug, webhook_id, to_char(max(delivered_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM webhook_deliveries WHERE state = 'delivered' GROUP BY 1, 2", &[])?
                .iter()
                .filter_map(|r| r.get::<_, Option<String>>(2).map(|t| ((r.get::<_, String>(0), r.get::<_, String>(1)), t)))
                .collect();
            Ok((states, last))
        })
        .map_err(storage)?;
    let mut manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    manifests.sort_by(|a, b| a.slug.cmp(&b.slug));
    let mut rows = Vec::new();
    for m in &manifests {
        for w in &m.integrations.webhooks {
            let key = (m.slug.clone(), w.id.clone());
            let (failures, off) = states.get(&key).copied().unwrap_or((0, false));
            let status = if off {
                "Disabled"
            } else if failures > 0 {
                "Failing"
            } else {
                "Active"
            };
            rows.push(json!({
                "app": m.slug, "app_title": m.title, "id": w.id, "name": w.name, "host": host_of(&w.url),
                "events": w.events, "state": status, "consecutive_failures": failures, "last_success": last.get(&key),
            }));
        }
    }
    Ok(json!({ "webhooks": rows }))
}

fn require_webhook(state: &SharedState, app: &str, id: &str) -> Result<(), ServiceError> {
    let eng = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?;
    let manifest = eng.resolve_by_slug(app).ok_or_else(|| ServiceError::NotFound(format!("App '{app}' not found")))?;
    if manifest.integrations.webhooks.iter().any(|w| w.id == id) {
        Ok(())
    } else {
        Err(ServiceError::NotFound(format!("The app has no webhook '{id}'")))
    }
}

/// One webhook's newest deliveries: state, tries, the last status and error. Never a body.
pub fn deliveries(caller: &AuthUser, app: &str, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    require_webhook(state, app, id)?;
    Ok(json!({ "deliveries": crate::webhooks::call_log(&state.repository, app, id).map_err(storage)? }))
}

/// Turns a webhook off or on. Off: nothing more is queued or sent, and what was queued ends `dead`, with the reason in
/// its error, so the call log still shows it. On: the failure count clears.
pub fn set_enabled(caller: &AuthUser, app: &str, id: &str, enabled: bool, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    require_webhook(state, app, id)?;
    let reason = reason_of(body);
    let action = if enabled { "enabled" } else { "disabled" };
    admin_write(state, caller, DecisionType::WebhookChanged, "SC-07", &reason, &json!({ "app": app, "webhook": id, "action": action }), || {
        if enabled {
            crate::webhooks::enable(&state.repository, app, id).map_err(storage)?;
        } else {
            crate::webhooks::disable(&state.repository, app, id, reason.trim()).map_err(storage)?;
        }
        Ok(json!({ "app": app, "webhook": id, "enabled": enabled }))
    })
}

/// What the Overview shows: webhooks that are failing or off, and deliveries that ended dead in the last day.
pub fn counts(state: &SharedState) -> Value {
    let (failing, disabled, dead) = state
        .repository
        .with_client(|c| {
            let r = c.query_one(
                "SELECT (SELECT COUNT(*) FROM webhook_state WHERE disabled_at IS NULL AND consecutive_failures > 0), \
                        (SELECT COUNT(*) FROM webhook_state WHERE disabled_at IS NOT NULL), \
                        (SELECT COUNT(*) FROM webhook_deliveries WHERE state = 'dead' AND finished_at > NOW() - INTERVAL '1 day')",
                &[],
            )?;
            Ok((r.get::<_, i64>(0), r.get::<_, i64>(1), r.get::<_, i64>(2)))
        })
        .unwrap_or((0, 0, 0));
    json!({ "webhooks_failing": failing, "webhooks_disabled": disabled, "deliveries_dead_last_day": dead })
}
