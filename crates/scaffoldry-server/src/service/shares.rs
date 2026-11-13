//! Views phase 6: shared links.
//!
//! A share is a link to one view. It is off until a Platform Admin turns a scope on in `sharing.allowed_scopes`. The token is
//! 128 random bits, shown once, and only its hash is stored. A shared view is read-only and never carries a sensitive or a
//! hidden field: the request reads as a `SharedLink`, and the field rules leave out every protected field for it.
//!
//! - `institution`: the viewer is signed in. Their own row rules apply, and the workspace's guards can still forbid.
//! - `public`: anyone with the link. There is no person behind the request, so a row rule that depends on a person shows no
//!   rows. A public link needs a workspace classified Public.

use crate::service::access::{authorize_app, AppAction};
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::views::{self, RunQuery};
use crate::service::{records, scope, ServiceError};
use crate::state::{AuthUser, RecordDecisionInput, SharedLink, SharedState};
use aes_gcm::aead::{rand_core::RngCore, OsRng};
use scaffoldry_core::access::{Operand, Pred, RowOp};
use scaffoldry_core::ledger::DecisionType;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub const SCOPES: [&str; 2] = ["institution", "public"];
const DEFAULT_DAYS: i64 = 30;
const MAX_DAYS: i64 = 90;
/// The most active shares one view holds.
pub const MAX_ACTIVE: i64 = 20;
/// Requests a minute for one token, and for one address on a public link.
pub const PER_TOKEN: i32 = 60;
pub const PER_ADDRESS: i32 = 20;
/// The most rows a download holds.
pub const DOWNLOAD_ROWS: usize = 10_000;

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

/// Whether a `sharing.allowed_scopes` value is a list of known scopes with none twice.
pub fn valid_scopes(value: &Value) -> bool {
    value.as_array().is_some_and(|a| {
        let names: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
        names.len() == a.len() && names.iter().all(|n| SCOPES.contains(n)) && names.iter().enumerate().all(|(i, n)| !names[..i].contains(n))
    })
}

fn allowed_scopes(state: &SharedState) -> Vec<String> {
    state
        .settings
        .read()
        .ok()
        .and_then(|s| s.get("sharing.allowed_scopes").cloned())
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// The classification of the workspace that holds an app.
fn classification_of(app_slug: &str, state: &SharedState) -> Option<String> {
    use scaffoldry_engine::HostRouter;
    let workspace = state.engine.read().ok()?.resolve_by_slug(app_slug)?.workspace_id.clone()?;
    state.workspaces.read().ok()?.get(&workspace).map(|w| w.data_classification.clone())
}

fn public_problem(app_slug: &str, state: &SharedState) -> Option<String> {
    match classification_of(app_slug, state).as_deref() {
        Some("Public") => None,
        Some(other) => Some(format!("A public link needs a workspace classified Public, and this one is classified {other}.")),
        None => Some("A public link needs a workspace classified Public, and this app has none.".to_string()),
    }
}

fn person_dependent(p: &Pred) -> bool {
    match p {
        Pred::True => false,
        Pred::Cmp { value, .. } => matches!(value, Operand::Person(_)),
        Pred::And(parts) | Pred::Or(parts) => parts.iter().any(person_dependent),
        Pred::Not(inner) => person_dependent(inner),
    }
}

fn hash_of(token: &str) -> String {
    crate::service::identity::hash_token(token)
}

fn new_token() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const SHARE_COLUMNS: &str = "id, app_slug, view_id, scope, allow_download, expires_at, created_by, created_at, revoked_at, last_used_at, use_count, \
    CASE WHEN revoked_at IS NOT NULL THEN 'Revoked' WHEN expires_at <= NOW() THEN 'Expired' ELSE 'Active' END";

fn share_json(r: &postgres::Row) -> Value {
    let at = |i: usize| r.get::<_, Option<chrono::DateTime<chrono::Utc>>>(i).map(|t| t.to_rfc3339());
    json!({
        "id": r.get::<_, Uuid>(0), "app": r.get::<_, String>(1), "view_id": r.get::<_, Uuid>(2), "scope": r.get::<_, String>(3),
        "allow_download": r.get::<_, bool>(4), "expires_at": at(5), "created_by": r.get::<_, String>(6), "created_at": at(7),
        "revoked_at": at(8), "last_used_at": at(9), "use_count": r.get::<_, i64>(10), "state": r.get::<_, String>(11),
    })
}

fn record(state: &SharedState, caller: &AuthUser, app_slug: &str, rationale: String, payload: Value) -> Result<(), ServiceError> {
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: Some(app_slug.to_string()),
            decision_type: DecisionType::ShareChanged,
            oscal_control_id: "AC-22".to_string(),
            rationale,
            payload: &payload,
        })
        .map(|_| ())
        .map_err(|e| ServiceError::Internal(format!("Failed to record audit ledger: {e}")))
}

/// The view a share is for: shared by others, and not a dashboard.
fn shareable(caller: &AuthUser, app_slug: &str, view: Uuid, state: &SharedState) -> Result<crate::repository_views::StoredView, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Manage, state)?;
    let stored = views::existing(caller, app_slug, view, state)?;
    if stored.ownership == "personal" {
        return Err(bad("A personal view is its owner's. Make it a collaborative view before sharing it"));
    }
    if stored.kind == "dashboard" {
        return Err(bad("A dashboard cannot be shared by a link yet"));
    }
    if stored.table_id.is_none() {
        return Err(bad("This view is not over a table"));
    }
    Ok(stored)
}

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub scope: String,
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    #[serde(default)]
    pub allow_download: bool,
}

/// `POST /apps/{slug}/views/{id}/shares`. The token is in this answer and nowhere else, ever.
pub fn create(caller: &AuthUser, app_slug: &str, view: Uuid, body: CreateBody, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = shareable(caller, app_slug, view, state)?;
    if !SCOPES.contains(&body.scope.as_str()) {
        return Err(bad("scope is institution or public"));
    }
    let days = body.expires_in_days.unwrap_or(DEFAULT_DAYS);
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(bad(format!("A link lasts 1 to {MAX_DAYS} days")));
    }
    if !allowed_scopes(state).contains(&body.scope) {
        return Err(ServiceError::ShareNotAllowed(format!(
            "Shared links for {} are not turned on at this institution. A Platform Admin turns them on in the settings.",
            if body.scope == "public" { "anyone" } else { "the institution" }
        )));
    }
    let table = stored.table_id.clone().unwrap_or_default();
    if body.scope == "public" {
        if let Some(why) = public_problem(app_slug, state) {
            return Err(ServiceError::ShareNotAllowed(why));
        }
        if body.allow_download && !records::sensitive_fields_of(app_slug, &table, state).is_empty() {
            return Err(ServiceError::ShareNotAllowed("A public link cannot download a table that has a sensitive field.".to_string()));
        }
    }

    let (token, id) = (new_token(), Uuid::new_v4());
    let (slug, hash, scope_name, by) = (app_slug.to_string(), hash_of(&token), body.scope.clone(), caller.eppn.clone());
    let allow = body.allow_download;
    let row = state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock(hashtext($1::text))", &[&view.to_string()])?;
            let active: i64 = tx.query_one("SELECT count(*) FROM view_shares WHERE view_id = $1 AND revoked_at IS NULL AND expires_at > NOW()", &[&view])?.get(0);
            if active >= MAX_ACTIVE {
                return Ok(None);
            }
            let row = tx.query_one(
                &format!(
                    "INSERT INTO view_shares (id, app_slug, view_id, token_hash, scope, allow_download, expires_at, created_by) \
                     VALUES ($1, $2, $3, $4, $5, $6, NOW() + make_interval(days => $7::int), $8) RETURNING {SHARE_COLUMNS}"
                ),
                &[&id, &slug, &view, &hash, &scope_name, &allow, &(days as i32), &by],
            )?;
            tx.commit()?;
            Ok(Some(share_json(&row)))
        })
        .map_err(storage)?
        .ok_or_else(|| ServiceError::QuotaExceeded(format!("A view holds {MAX_ACTIVE} active links. Revoke one first.")))?;

    // The creator is told what a public link will show, now and not after someone opens it.
    let mut warnings: Vec<String> = Vec::new();
    if body.scope == "public" {
        let depends = scope::rules_of(app_slug, state).iter().any(|r| r.table_id == table && r.ops.contains(&RowOp::Read) && person_dependent(&r.visible_when));
        if depends {
            warnings.push("A row rule of this table depends on who is looking. A public link has no person, so it shows no rows.".to_string());
        }
    }
    record(
        state,
        caller,
        app_slug,
        format!("Shared the view '{}' by a {} link for {days} days", stored.name, body.scope),
        json!({ "view": view, "share": id, "scope": body.scope, "expires_in_days": days, "allow_download": body.allow_download }),
    )?;
    Ok(json!({ "share": row, "token": token, "link": format!("/s/{token}"), "warnings": warnings }))
}

/// `GET /apps/{slug}/views/{id}/shares`: the shares of a view, and what this person may choose when they make one.
pub fn list(caller: &AuthUser, app_slug: &str, view: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    shareable(caller, app_slug, view, state)?;
    let rows: Vec<Value> = state
        .repository
        .with_client(move |c| Ok(c.query(&format!("SELECT {SHARE_COLUMNS} FROM view_shares WHERE view_id = $1 ORDER BY created_at DESC"), &[&view])?.iter().map(share_json).collect()))
        .map_err(storage)?;
    let allowed = allowed_scopes(state);
    let public = json!({ "allowed": allowed.iter().any(|s| s == "public") && public_problem(app_slug, state).is_none(), "reason": if allowed.iter().any(|s| s == "public") { public_problem(app_slug, state) } else { Some("A Platform Admin has not turned on public links.".to_string()) } });
    Ok(json!({ "shares": rows, "options": { "scopes": allowed, "public": public, "default_days": DEFAULT_DAYS, "max_days": MAX_DAYS } }))
}

/// `DELETE /apps/{slug}/views/{id}/shares/{share}`.
pub fn revoke(caller: &AuthUser, app_slug: &str, view: Uuid, share: Uuid, state: &SharedState) -> Result<Value, ServiceError> {
    shareable(caller, app_slug, view, state)?;
    let slug = app_slug.to_string();
    let done = state
        .repository
        .with_client(move |c| Ok(c.execute("UPDATE view_shares SET revoked_at = NOW() WHERE id = $1 AND app_slug = $2 AND view_id = $3 AND revoked_at IS NULL", &[&share, &slug, &view])?))
        .map_err(storage)?;
    if done == 0 {
        return Err(ServiceError::NotFound("Link not found".to_string()));
    }
    record(state, caller, app_slug, "Revoked a shared link".to_string(), json!({ "view": view, "share": share }))?;
    Ok(json!({ "revoked": true, "id": share }))
}

/// `GET /admin/shares`: every share, newest first, with its use. A token is never in it.
pub fn admin_list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let rows: Vec<Value> = state
        .repository
        .with_client(|c| {
            let q = "SELECT s.id, s.app_slug, s.view_id, s.scope, s.allow_download, s.expires_at, s.created_by, s.created_at, s.revoked_at, s.last_used_at, s.use_count, \
                 CASE WHEN s.revoked_at IS NOT NULL THEN 'Revoked' WHEN s.expires_at <= NOW() THEN 'Expired' ELSE 'Active' END, v.name \
                 FROM view_shares s LEFT JOIN app_views v ON v.id = s.view_id ORDER BY s.created_at DESC LIMIT 500";
            Ok(c.query(q, &[])?
                .iter()
                .map(|r| {
                    let mut j = share_json(r);
                    j["view_name"] = json!(r.get::<_, Option<String>>(12));
                    j
                })
                .collect())
        })
        .map_err(storage)?;
    let rows = rows
        .into_iter()
        .map(|mut r| {
            let title = {
                use scaffoldry_engine::HostRouter;
                let slug = r["app"].as_str().unwrap_or("").to_string();
                state.engine.read().ok().and_then(|e| e.resolve_by_slug(&slug).map(|m| m.title.clone()))
            };
            r["app_title"] = json!(title);
            r
        })
        .collect::<Vec<_>>();
    Ok(json!({ "shares": rows }))
}

/// `POST /admin/shares/revoke`: every active link of one app, with a reason.
pub fn admin_revoke_app(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let app = body.get("app").and_then(Value::as_str).filter(|a| !a.is_empty()).ok_or_else(|| bad("Name the app"))?.to_string();
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("");
    let slug = app.clone();
    admin_write(state, caller, DecisionType::ShareChanged, "AC-22", reason, &json!({ "app": app, "action": "revoked_all" }), || {
        let n = state
            .repository
            .with_client(move |c| Ok(c.execute("UPDATE view_shares SET revoked_at = NOW() WHERE app_slug = $1 AND revoked_at IS NULL", &[&slug])?))
            .map_err(storage)?;
        Ok(json!({ "revoked": n, "app": app }))
    })
}

// ---------------------------------------------------------------------------------------------
// Opening a link
// ---------------------------------------------------------------------------------------------

/// A link that has been opened: who reads through it, and what it is.
pub struct Opened {
    pub reader: AuthUser,
    pub share: Uuid,
    pub app: String,
    pub view: Uuid,
    pub scope: String,
    pub allow_download: bool,
    pub expires_at: String,
}

/// Counts one request against a key in the current minute. The request that passes the limit is refused.
fn hit(state: &SharedState, key: String, limit: i32) -> Result<(), ServiceError> {
    let count: i32 = state
        .repository
        .with_client(move |c| {
            let n: i32 = c
                .query_one(
                    "INSERT INTO rate_limits (key, window_start, count) VALUES ($1, date_trunc('minute', NOW()), 1) \
                     ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1 RETURNING count",
                    &[&key],
                )?
                .get(0);
            // A key's first request of a minute also clears what is over an hour old.
            if n == 1 {
                c.execute("DELETE FROM rate_limits WHERE window_start < NOW() - INTERVAL '1 hour'", &[])?;
            }
            Ok(n)
        })
        .map_err(storage)?;
    if count > limit {
        return Err(ServiceError::RateLimited("Too many requests for this link. Wait a minute and try again.".to_string()));
    }
    Ok(())
}

/// Opens a token for one request: finds it, counts it, and decides who reads through it. A token that is wrong, revoked or
/// past its expiry is not found, whatever the reason.
pub fn open(state: &SharedState, headers: &axum::http::HeaderMap, token: &str) -> Result<Opened, ServiceError> {
    let not_found = || ServiceError::NotFound("This link does not exist or is no longer open".to_string());
    let hash = hash_of(token);
    let hashed = hash.clone();
    let found = state
        .repository
        .with_client(move |c| {
            Ok(c.query_opt(
                "SELECT id, app_slug, view_id, scope, allow_download, to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
                 FROM view_shares WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > NOW()",
                &[&hashed],
            )?
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1), r.get::<_, Uuid>(2), r.get::<_, String>(3), r.get::<_, bool>(4), r.get::<_, String>(5))))
        })
        .map_err(storage)?;
    let (share, app, view, scope_name, allow_download, expires_at) = found.ok_or_else(not_found)?;

    hit(state, format!("share:{hash}"), PER_TOKEN)?;
    let link = Box::new(SharedLink { share_id: share, app_slug: app.clone(), scope: scope_name.clone() });
    let reader = if scope_name == "public" {
        let address = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .unwrap_or("unknown")
            .to_string();
        hit(state, format!("addr:{address}"), PER_ADDRESS)?;
        AuthUser {
            eppn: String::new(),
            name: "Shared link".to_string(),
            role_title: String::new(),
            affiliation: "public".to_string(),
            department: String::new(),
            view_as_by: None,
            via_client: None,
            shared_link: Some(link),
        }
    } else {
        let mut user = crate::guard::session_user(state, headers).ok_or_else(|| ServiceError::Unauthorized("Sign in to open this link".to_string()))?;
        // A token limited to some apps or tables does not take the way round the limit that a link would be.
        if crate::guard::request_scope(state, headers, &user).is_some() {
            return Err(ServiceError::TokenScope("A limited token cannot open a shared link. Sign in as yourself.".to_string()));
        }
        user.shared_link = Some(link);
        // The person's own right to read the app, and any guard of its workspace, still decide.
        authorize_app(&user, &app, AppAction::Read, state)?;
        user
    };
    let counted = share;
    state
        .repository
        .with_client(move |c| Ok(c.execute("UPDATE view_shares SET use_count = use_count + 1, last_used_at = NOW() WHERE id = $1", &[&counted])?))
        .map_err(storage)?;
    Ok(Opened { reader, share, app, view, scope: scope_name, allow_download, expires_at })
}

/// `GET /shared/{token}`: what the link is, with nothing of the view's own settings (they name fields).
pub fn describe(state: &SharedState, opened: &Opened) -> Result<Value, ServiceError> {
    let stored = views::existing(&opened.reader, &opened.app, opened.view, state)?;
    let title = {
        use scaffoldry_engine::HostRouter;
        state.engine.read().ok().and_then(|e| e.resolve_by_slug(&opened.app).map(|m| m.title.clone()))
    };
    Ok(json!({ "name": stored.name, "kind": stored.kind, "app_title": title, "scope": opened.scope, "allow_download": opened.allow_download, "expires_at": opened.expires_at }))
}

/// `GET /shared/{token}/data`: the view's rows or groups, read as the link.
pub fn data(state: &SharedState, opened: &Opened, origin: &crate::service::history::Origin, query: RunQuery) -> Result<Value, ServiceError> {
    views::run(&opened.reader, origin, &opened.app, opened.view, query, state)
}

/// `GET /shared/{token}/download`: the view's rows as CSV, when the link was made to allow it.
pub fn download(state: &SharedState, opened: &Opened, origin: &crate::service::history::Origin) -> Result<String, ServiceError> {
    if !opened.allow_download {
        return Err(ServiceError::ShareNotAllowed("This link was not made to allow downloads.".to_string()));
    }
    let stored = views::existing(&opened.reader, &opened.app, opened.view, state)?;
    let table = stored.table_id.clone().unwrap_or_default();
    if opened.scope == "public" && !records::sensitive_fields_of(&opened.app, &table, state).is_empty() {
        return Err(ServiceError::ShareNotAllowed("A public link cannot download a table that has a sensitive field.".to_string()));
    }
    let mut rows: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = views::run(&opened.reader, origin, &opened.app, opened.view, RunQuery { cursor: cursor.clone(), limit: Some(1000), ..Default::default() }, state)?;
        let Some(batch) = page.get("records").and_then(Value::as_array) else {
            return Err(bad("This view has no rows to download"));
        };
        rows.extend(batch.iter().map(|r| r.get("data").cloned().unwrap_or(Value::Null)));
        cursor = page.get("next_cursor").and_then(Value::as_str).map(str::to_string);
        if cursor.is_none() || rows.len() >= DOWNLOAD_ROWS {
            break;
        }
    }
    rows.truncate(DOWNLOAD_ROWS);
    let mut headers: Vec<String> = Vec::new();
    for r in &rows {
        for k in r.as_object().into_iter().flat_map(|o| o.keys()) {
            if !k.starts_with('_') && !headers.contains(k) {
                headers.push(k.clone());
            }
        }
    }
    let names: Vec<&str> = headers.iter().map(String::as_str).collect();
    Ok(scaffoldry_engine::export_records_to_csv(&names, &rows))
}
