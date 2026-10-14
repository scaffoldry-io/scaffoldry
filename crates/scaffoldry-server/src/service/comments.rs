//! Comments on records, and the mentions in them.
//!
//! A comment follows the record's access: a person who cannot read the record cannot read its
//! comments. A mention names a person with `@[address]`. Only people who can read the app are kept,
//! and the author is never told which were dropped, so a comment cannot probe who has access.

use crate::repository_comments::CommentRow;
use crate::service::access::{authorize_app, AppAction};
use crate::service::history::{write_ctx, Origin};
use crate::service::ServiceError;
use crate::state::{AuthUser, DatasetRecord, SharedState};
use serde_json::{json, Value};
use uuid::Uuid;

pub const MAX_BODY: usize = 4_000;
const MAX_MENTIONS: usize = 20;
/// An author may edit or remove their own comment for this long.
const AUTHOR_MINUTES: i64 = 15;
const PAGE: i64 = 500;

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    Ok(&*state.repository)
}

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// A person who may not read the app learns nothing about it, not even that it is there.
fn hide(err: ServiceError) -> ServiceError {
    match err {
        ServiceError::Forbidden { .. } => ServiceError::NotFound("Not found".to_string()),
        other => other,
    }
}

fn find(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Result<DatasetRecord, ServiceError> {
    state
        .find_record(app_slug, record_id)
        .map_err(internal)?
        .filter(|r| crate::service::scope::allows(caller, app_slug, r, scaffoldry_core::access::RowOp::Read, state))
        .ok_or_else(|| ServiceError::NotFound(format!("Record '{record_id}' not found")))
}

/// The addresses written as `@[address]` in a body, once each, in order.
pub fn mentions_in(body: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("@[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(']') else { break };
        let address = after[..end].trim();
        let plausible = address.len() <= 255 && address.contains('@') && !address.chars().any(|c| c.is_whitespace() || c == '[');
        if plausible && !found.iter().any(|a| a.eq_ignore_ascii_case(address)) {
            found.push(address.to_string());
        }
        rest = &after[end + 1..];
    }
    found.truncate(MAX_MENTIONS);
    found
}

/// The person as the app knows them, when they can read the app.
fn reader(address: &str, app_slug: &str, state: &SharedState) -> Option<AuthUser> {
    use scaffoldry_engine::HostRouter;
    let workspace = state.engine.read().ok()?.resolve_by_slug(app_slug)?.workspace_id.clone()?;
    let member = crate::service::members::people_records(&workspace, state).into_iter().find(|m| m.eppn.eq_ignore_ascii_case(address));
    let user = match member {
        Some(m) => AuthUser { eppn: m.eppn, name: m.name, role_title: m.role, affiliation: m.scoped_affiliation, department: m.department, view_as_by: None, },
        None => crate::service::identity::resolve_user(address, state)?,
    };
    authorize_app(&user, app_slug, AppAction::Read, state).ok().map(|_| user)
}

/// The mentions that stay: those of people who can read the app.
fn kept_mentions(body: &str, app_slug: &str, state: &SharedState) -> Vec<String> {
    mentions_in(body).iter().filter_map(|a| reader(a, app_slug, state)).map(|u| u.eppn).collect()
}

fn check_body(body: Option<&str>) -> Result<String, ServiceError> {
    let fail = |why: &str| ServiceError::Invalid {
        message: "The comment cannot be saved.".to_string(),
        fields: std::collections::BTreeMap::from([("body".to_string(), why.to_string())]),
    };
    let body = body.ok_or_else(|| fail("required"))?;
    if body.trim().is_empty() {
        return Err(fail("required"));
    }
    if body.chars().count() > MAX_BODY {
        return Err(fail("at most 4000 characters"));
    }
    Ok(body.to_string())
}

fn shown(row: &CommentRow, state: &SharedState) -> Value {
    let name = crate::service::identity::resolve_user(&row.author, state).map(|u| u.name).unwrap_or_else(|| row.author.clone());
    json!({
        "id": row.id,
        "record_id": row.record_id,
        "parent_id": row.parent_id,
        "author": row.author,
        "author_name": name,
        "body": row.body,
        "mentions": row.mentions,
        "deleted": row.deleted,
        "created_at": row.created_at,
        "edited_at": row.edited_at,
        "deleted_at": row.deleted_at,
    })
}

pub fn list_comments(caller: &AuthUser, app_slug: &str, record_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state).map_err(hide)?;
    find(caller, app_slug, record_id, state)?;
    let rows = repo(state)?.list_comments(app_slug, record_id, PAGE).map_err(internal)?;
    Ok(json!({ "comments": rows.iter().map(|r| shown(r, state)).collect::<Vec<_>>() }))
}

pub fn post_comment(caller: &AuthUser, origin: &Origin, app_slug: &str, record_id: &str, payload: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Comment, state)?;
    let record = find(caller, app_slug, record_id, state)?;
    let body = check_body(payload.get("body").and_then(Value::as_str))?;
    let repo = repo(state)?;
    let parent_id = match payload.get("parent_id").and_then(Value::as_str) {
        None => None,
        Some(raw) => {
            let id = Uuid::parse_str(raw).map_err(|_| ServiceError::BadRequest("parent_id is not a comment id".to_string()))?;
            let parent = repo.get_comment(app_slug, id).map_err(internal)?.filter(|p| p.record_id == record_id);
            match parent {
                None => return Err(ServiceError::BadRequest("parent_id is not a comment on this record".to_string())),
                Some(p) if p.parent_id.is_some() => return Err(ServiceError::BadRequest("A reply cannot be replied to".to_string())),
                Some(_) => Some(id),
            }
        }
    };
    let table = record.data.get("_table_id").and_then(Value::as_str).unwrap_or("").to_string();
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &record.data, state);
    let mentions = kept_mentions(&body, app_slug, state);
    let new = crate::repository_comments::NewComment { table_id: table, record_id: record_id.to_string(), parent_id, body, mentions };
    let row = repo.insert_comment(app_slug, new, &ctx).map_err(internal)?;
    Ok(shown(&row, state))
}

/// The comment, if the caller may change it: its author within 15 minutes, or `remove` by an owner or admin.
fn changeable(caller: &AuthUser, app_slug: &str, id: &str, admin_may: bool, state: &SharedState) -> Result<CommentRow, ServiceError> {
    let id = Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Comment not found".to_string()))?;
    let row = repo(state)?
        .get_comment(app_slug, id)
        .map_err(internal)?
        .filter(|c| !c.deleted)
        .ok_or_else(|| ServiceError::NotFound("Comment not found".to_string()))?;
    let recent = chrono::DateTime::parse_from_rfc3339(&row.created_at)
        .map(|t| chrono::Utc::now().signed_duration_since(t) <= chrono::Duration::minutes(AUTHOR_MINUTES))
        .unwrap_or(false);
    let own = row.author.eq_ignore_ascii_case(&caller.eppn);
    if own && recent {
        return Ok(row);
    }
    if admin_may && authorize_app(caller, app_slug, AppAction::Manage, state).is_ok() {
        return Ok(row);
    }
    Err(ServiceError::forbidden(if own {
        "You can change your own comment for 15 minutes."
    } else {
        "Only the author can change a comment."
    }))
}

pub fn edit_comment(caller: &AuthUser, origin: &Origin, app_slug: &str, id: &str, payload: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Comment, state)?;
    let row = changeable(caller, app_slug, id, false, state)?;
    let body = check_body(payload.get("body").and_then(Value::as_str))?;
    let mentions = kept_mentions(&body, app_slug, state);
    let record = find(caller, app_slug, &row.record_id, state)?;
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &record.data, state);
    let saved = repo(state)?
        .edit_comment(app_slug, row.id, &body, &mentions, &ctx)
        .map_err(internal)?
        .ok_or_else(|| ServiceError::NotFound("Comment not found".to_string()))?;
    Ok(shown(&saved, state))
}

pub fn remove_comment(caller: &AuthUser, origin: &Origin, app_slug: &str, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, app_slug, AppAction::Read, state).map_err(hide)?;
    let row = changeable(caller, app_slug, id, true, state)?;
    let record = find(caller, app_slug, &row.record_id, state)?;
    let ctx = write_ctx(&caller.eppn, origin, app_slug, &record.data, state);
    let saved = repo(state)?
        .remove_comment(app_slug, row.id, &ctx)
        .map_err(internal)?
        .ok_or_else(|| ServiceError::NotFound("Comment not found".to_string()))?;
    Ok(shown(&saved, state))
}

/// The people a mention can name: those who can read the app.
pub fn mentionable(caller: &AuthUser, app_slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    use scaffoldry_engine::HostRouter;
    authorize_app(caller, app_slug, AppAction::Read, state).map_err(hide)?;
    let workspace = state
        .engine
        .read()
        .map_err(internal)?
        .resolve_by_slug(app_slug)
        .and_then(|m| m.workspace_id.clone())
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let members = crate::service::members::people_records(&workspace, state);
    let mut people: Vec<Value> = members
        .iter()
        .filter(|m| reader(&m.eppn, app_slug, state).is_some())
        .map(|m| json!({ "eppn": m.eppn, "name": m.name }))
        .collect();
    people.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({ "people": people }))
}
