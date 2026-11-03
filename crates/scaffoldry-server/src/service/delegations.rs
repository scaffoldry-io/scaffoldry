//! Approvers phase 4: delegation.
//!
//! A person who holds a position names someone to decide for them while away: for one position or all they hold, for
//! a window of at most 365 days, with a reason. The delegate must be someone the platform knows and who is active, and
//! not the delegator. Resolution reads the time, so a delegation ends on `ends_at` without anyone acting, and the
//! resolver still never lets a submitter or a starter decide their own record. Every create and revoke is a
//! `DelegationChanged` ledger entry. NIST SP 800-53 AC-5 and AC-6: the standing-in is explicit, bounded and recorded.

use crate::service::admin::{admin_write, decode_cursor, encode_cursor, require_platform_admin};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use chrono::{DateTime, Duration, Utc};
use scaffoldry_core::approver::Delegation;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use uuid::Uuid;

/// The longest window a delegation may cover.
pub const MAX_DAYS: i64 = 365;
const PAGE: i64 = 50;
const COLUMNS: &str = "id, delegator_eppn, delegate_eppn, position_key, starts_at, ends_at, reason, created_at, revoked_at";

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// Where a delegation stands now.
fn state_of(starts: DateTime<Utc>, ends: DateTime<Utc>, revoked: Option<DateTime<Utc>>) -> &'static str {
    let now = Utc::now();
    if revoked.is_some() {
        "revoked"
    } else if now < starts {
        "scheduled"
    } else if now < ends {
        "active"
    } else {
        "ended"
    }
}

fn row_json(r: &postgres::Row) -> Value {
    let (starts, ends): (DateTime<Utc>, DateTime<Utc>) = (r.get(4), r.get(5));
    let created: DateTime<Utc> = r.get(7);
    let revoked: Option<DateTime<Utc>> = r.get(8);
    json!({
        "id": r.get::<_, Uuid>(0),
        "delegator": r.get::<_, String>(1),
        "delegate": r.get::<_, String>(2),
        "position_key": r.get::<_, Option<String>>(3),
        "starts_at": starts.to_rfc3339(),
        "ends_at": ends.to_rfc3339(),
        "reason": r.get::<_, String>(6),
        "created_at": created.to_rfc3339(),
        "revoked_at": revoked.map(|t| t.to_rfc3339()),
        "state": state_of(starts, ends, revoked),
    })
}

/// The delegations the resolver needs: those made by any of `delegators` that are not revoked and not yet over.
pub fn for_resolution(state: &SharedState, delegators: &[String]) -> Vec<Delegation> {
    if delegators.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = delegators.iter().map(|e| e.to_lowercase()).collect();
    state
        .repository
        .with_client(move |c| {
            Ok(c.query(
                "SELECT delegator_eppn, delegate_eppn, position_key, starts_at, ends_at FROM delegations \
                 WHERE lower(delegator_eppn) = ANY($1) AND revoked_at IS NULL AND ends_at > NOW()",
                &[&names],
            )?
            .iter()
            .map(|r| Delegation {
                delegator: r.get(0),
                delegate: r.get(1),
                position_key: r.get(2),
                starts_at: r.get::<_, DateTime<Utc>>(3).timestamp(),
                ends_at: r.get::<_, DateTime<Utc>>(4).timestamp(),
                revoked: false,
            })
            .collect())
        })
        .unwrap_or_default()
}

fn time(body: &Value, key: &str) -> Result<DateTime<Utc>, ServiceError> {
    body.get(key)
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| ServiceError::bad_request(format!("{key} must be a date and time, such as 2026-10-14T00:00:00Z")))
}

/// `POST /me/delegations`.
pub fn create(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let delegate = body.get("delegate").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| ServiceError::bad_request("Name the person who will decide for you"))?.to_string();
    let position = body.get("position_key").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if reason.is_empty() || reason.len() > 500 {
        return Err(ServiceError::bad_request("A delegation needs a reason of 1 to 500 characters"));
    }
    let (starts, ends) = (time(body, "starts_at")?, time(body, "ends_at")?);
    if ends <= starts {
        return Err(ServiceError::bad_request("A delegation must end after it starts"));
    }
    if ends - starts > Duration::days(MAX_DAYS) {
        return Err(ServiceError::bad_request(format!("A delegation may last at most {MAX_DAYS} days")));
    }
    if delegate.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::bad_request("You cannot delegate to yourself"));
    }
    // The caller must hold what they hand on: that position, or any position at all.
    let held = state.repository.roles_of_people(std::slice::from_ref(&caller.eppn)).map_err(storage)?;
    let holds = match &position {
        Some(key) => held.iter().any(|r| r.position_key.as_deref() == Some(key.as_str())),
        None => held.iter().any(|r| r.position_key.is_some()),
    };
    if !holds {
        return Err(ServiceError::bad_request(match &position {
            Some(key) => format!("You do not hold the position {key}, so you cannot delegate it"),
            None => "You hold no position, so there is nothing to delegate".to_string(),
        }));
    }
    // The delegate must be someone the platform knows, and active.
    let found = state.repository.users_by_names(std::slice::from_ref(&delegate)).map_err(storage)?;
    let person = found.first().ok_or_else(|| ServiceError::bad_request(format!("No person is known as {delegate}")))?;
    if !person.active || person.admin_hold {
        return Err(ServiceError::bad_request(format!("{delegate} is not active, so they cannot decide for you")));
    }
    let delegate = person.user_name.clone();
    let id = Uuid::new_v4();
    let payload = json!({ "action": "created", "id": id, "delegator": caller.eppn, "delegate": delegate, "position_key": position, "starts_at": starts.to_rfc3339(), "ends_at": ends.to_rfc3339() });
    let row = admin_write(state, caller, DecisionType::DelegationChanged, "AC-05", &reason, &payload, || {
        let (delegator, delegate, position, reason) = (caller.eppn.clone(), delegate.clone(), position.clone(), reason.clone());
        state
            .repository
            .with_client(move |c| {
                let r = c.query_one(
                    &format!("INSERT INTO delegations (id, delegator_eppn, delegate_eppn, position_key, starts_at, ends_at, reason) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING {COLUMNS}"),
                    &[&id, &delegator, &delegate, &position, &starts, &ends, &reason],
                )?;
                Ok(row_json(&r))
            })
            .map_err(storage)
    })?;
    Ok(row)
}

fn revoke_row(state: &SharedState, id: Uuid) -> Result<Value, ServiceError> {
    state
        .repository
        .with_client(move |c| Ok(c.query_opt(&format!("UPDATE delegations SET revoked_at = COALESCE(revoked_at, NOW()) WHERE id = $1 RETURNING {COLUMNS}"), &[&id])?.map(|r| row_json(&r))))
        .map_err(storage)?
        .ok_or_else(|| ServiceError::NotFound("No such delegation".to_string()))
}

fn find(state: &SharedState, id: Uuid) -> Result<Option<Value>, ServiceError> {
    state.repository.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM delegations WHERE id = $1"), &[&id])?.map(|r| row_json(&r)))).map_err(storage)
}

fn parse_id(id: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("No such delegation".to_string()))
}

/// `DELETE /me/delegations/{id}`: the delegator revokes their own. A Platform Admin may revoke any.
pub fn revoke_mine(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(id)?;
    let row = find(state, id)?.ok_or_else(|| ServiceError::NotFound("No such delegation".to_string()))?;
    let mine = row["delegator"].as_str().is_some_and(|d| d.eq_ignore_ascii_case(&caller.eppn));
    if !mine && require_platform_admin(caller, state).is_err() {
        // Someone else's delegation is not shown to exist.
        return Err(ServiceError::NotFound("No such delegation".to_string()));
    }
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).filter(|r| !r.is_empty()).unwrap_or("Revoked by the person who made it").to_string();
    let payload = json!({ "action": "revoked", "id": id, "delegator": row["delegator"], "delegate": row["delegate"] });
    admin_write(state, caller, DecisionType::DelegationChanged, "AC-05", &reason, &payload, || revoke_row(state, id))
}

/// `DELETE /admin/delegations/{id}`: a Platform Admin, with a reason.
pub fn revoke_admin(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let id = parse_id(id)?;
    let row = find(state, id)?.ok_or_else(|| ServiceError::NotFound("No such delegation".to_string()))?;
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("");
    let payload = json!({ "action": "revoked", "id": id, "delegator": row["delegator"], "delegate": row["delegate"], "by_admin": true });
    admin_write(state, caller, DecisionType::DelegationChanged, "AC-05", reason, &payload, || revoke_row(state, id))
}

/// `GET /me/delegations`: what the caller made and what they received, newest first, with the positions they hold.
pub fn mine(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let me = caller.eppn.to_lowercase();
    let (made, received) = state
        .repository
        .with_client(move |c| {
            let q = |col: &str| format!("SELECT {COLUMNS} FROM delegations WHERE lower({col}) = $1 ORDER BY created_at DESC LIMIT 200");
            let made: Vec<Value> = c.query(&q("delegator_eppn"), &[&me])?.iter().map(row_json).collect();
            let received: Vec<Value> = c.query(&q("delegate_eppn"), &[&me])?.iter().map(row_json).collect();
            Ok((made, received))
        })
        .map_err(storage)?;
    // The other person in each, named in words.
    let mut others: Vec<String> = made.iter().map(|d| d["delegate"].as_str().unwrap_or("").to_string()).collect();
    others.extend(received.iter().map(|d| d["delegator"].as_str().unwrap_or("").to_string()));
    others.sort();
    others.dedup();
    let users = state.repository.users_by_names(&others).map_err(storage)?;
    let name = |e: &str| users.iter().find(|u| u.user_name.eq_ignore_ascii_case(e)).map(crate::service::people::display_name).unwrap_or_else(|| e.to_string());
    let named = |list: Vec<Value>, key: &str, field: &str| -> Vec<Value> {
        list.into_iter()
            .map(|mut d| {
                let n = name(d[key].as_str().unwrap_or(""));
                d[field] = json!(n);
                d
            })
            .collect()
    };
    let made = named(made, "delegate", "delegate_name");
    let received = named(received, "delegator", "delegator_name");
    let held = state.repository.roles_of_people(std::slice::from_ref(&caller.eppn)).map_err(storage)?;
    let mut positions: Vec<String> = held.into_iter().filter_map(|r| r.position_key).collect();
    positions.sort();
    positions.dedup();
    Ok(json!({ "made": made, "received": received, "positions": positions }))
}

/// `GET /me/delegations/people?q=`: people the caller may name as a delegate, by the start of their name or address.
/// At least two letters and at most ten people, active and not on hold.
pub fn people(q: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let q = q.trim().to_lowercase();
    if q.chars().count() < 2 {
        return Err(ServiceError::bad_request("Type at least two letters of a name or address"));
    }
    let filter = crate::repository_people::UserFilter { prefix: Some(q), active: Some(true), hold: Some(false), ..Default::default() };
    let found = state.repository.search_users(&filter, 0, 10).map_err(storage)?;
    let people: Vec<Value> =
        found.iter().take(10).map(|u| json!({ "eppn": u.user_name, "name": crate::service::people::display_name(u) })).collect();
    Ok(json!({ "people": people }))
}

/// `GET /admin/delegations`: every delegation, filtered by `delegator`, `delegate` and `state`, 50 at a time.
pub fn list_all(caller: &AuthUser, query: &std::collections::HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let offset = query.get("cursor").map(|c| decode_cursor(c)).transpose()?.unwrap_or(0) as i64;
    let delegator = query.get("delegator").map(|s| s.to_lowercase());
    let delegate = query.get("delegate").map(|s| s.to_lowercase());
    let wanted = query.get("state").cloned();
    if wanted.as_deref().is_some_and(|s| !["active", "scheduled", "ended", "revoked"].contains(&s)) {
        return Err(ServiceError::bad_request("state is one of active, scheduled, ended or revoked"));
    }
    let rows = state
        .repository
        .with_client(move |c| {
            let rows = c.query(
                &format!(
                    "SELECT {COLUMNS} FROM delegations \
                     WHERE ($1::text IS NULL OR lower(delegator_eppn) = $1) AND ($2::text IS NULL OR lower(delegate_eppn) = $2) \
                       AND ($3::text IS NULL OR $3 = CASE WHEN revoked_at IS NOT NULL THEN 'revoked' WHEN NOW() < starts_at THEN 'scheduled' WHEN NOW() < ends_at THEN 'active' ELSE 'ended' END) \
                     ORDER BY created_at DESC, id OFFSET $4 LIMIT $5"
                ),
                &[&delegator, &delegate, &wanted, &offset, &(PAGE + 1)],
            )?;
            Ok(rows.iter().map(row_json).collect::<Vec<Value>>())
        })
        .map_err(storage)?;
    let more = rows.len() as i64 > PAGE;
    let page: Vec<Value> = rows.into_iter().take(PAGE as usize).collect();
    let next = more.then(|| encode_cursor((offset + PAGE) as usize));
    Ok(json!({ "delegations": page, "next_cursor": next }))
}
