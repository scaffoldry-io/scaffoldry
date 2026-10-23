//! Custom pages: the document a host shows, and the one call that bounds what that document may do.
//!
//! `get_page` returns a document as a string inside JSON. It is never an HTTP response of its own, so a browser cannot be
//! sent to it. The document holds the page's source, the runtime it needs, and the policy of the frame. It holds no
//! record, no eppn, and no token.
//!
//! `page_call` is the limit. Every call a page makes comes here. It checks the page's grants, then calls the same tool
//! the data tools use, so the caller's own rights still apply. A grant never widens them.

use crate::service::history::Origin;
use crate::service::{tools, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::{AppPage, HostRouter, PageFormat, PageGrant, PageKind};
use serde_json::{json, Map, Value};

const SHELL: &str = include_str!("../ui/page-shell.html");
const REACT: &str = include_str!("../ui/vendor/react.js");
const REACT_DOM: &str = include_str!("../ui/vendor/react-dom.js");
const SUCRASE: &str = include_str!("../ui/vendor/sucrase.js");
const BRIDGE: &str = include_str!("../ui/bridge.js");
const LOADER: &str = include_str!("../ui/loader.js");

/// The most one person keeps in settings for one page.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_SETTING_KEYS: i64 = 256;
const MAX_KEY_CHARS: usize = 128;

/// The page from the manifest the app runs now, and its source.
struct Custom {
    page: AppPage,
    source_sha256: String,
    grants: Vec<PageGrant>,
    format: PageFormat,
}

fn live_page(slug: &str, page_id: &str, state: &SharedState) -> Result<AppPage, ServiceError> {
    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    let manifest = engine.resolve_by_slug(slug).ok_or_else(|| ServiceError::NotFound(format!("No app '{slug}'")))?;
    manifest
        .pages
        .iter()
        .find(|p| p.id == page_id)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("The app '{slug}' has no page '{page_id}'")))
}

fn live_custom(slug: &str, page_id: &str, state: &SharedState) -> Result<Custom, ServiceError> {
    let page = live_page(slug, page_id, state)?;
    match page.kind.clone() {
        PageKind::Custom { source_sha256, grants, format, .. } => Ok(Custom { page, source_sha256, grants, format }),
        PageKind::View { .. } => Err(ServiceError::BadRequest(format!("The page '{page_id}' shows a view. Only a custom page has a document"))),
    }
}

/// The pages of an app, with what each may do.
pub fn list_pages(slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    let manifest = engine.resolve_by_slug(slug).ok_or_else(|| ServiceError::NotFound(format!("No app '{slug}'")))?;
    let pages: Vec<Value> = manifest
        .pages
        .iter()
        .map(|p| match &p.kind {
            PageKind::View { view_id } => json!({ "id": p.id, "title": p.title, "kind": "view", "view_id": view_id }),
            PageKind::Custom { grants, format, dependencies, .. } => json!({
                "id": p.id, "title": p.title, "kind": "custom", "format": format_name(*format),
                "grants": grants, "dependencies": dependencies,
            }),
        })
        .collect();
    Ok(json!({ "pages": pages }))
}

fn format_name(f: PageFormat) -> &'static str {
    match f {
        PageFormat::Module => "module",
        PageFormat::Bundle => "bundle",
    }
}

/// Replaces each marker in one pass over the template. What a marker is replaced with is never scanned again, so a page
/// that contains the text of a marker is not filled twice.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + values.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut rest = template;
    while let Some(i) = rest.find("@@") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match values.iter().find(|(m, _)| tail.starts_with(m)) {
            Some((m, v)) => {
                out.push_str(v);
                rest = &tail[m.len()..];
            }
            None => {
                out.push_str("@@");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// JSON that is safe inside a script element: no left angle bracket can end the element or open a comment.
fn script_json(v: &Value) -> String {
    v.to_string().replace('<', "\\u003c")
}

/// The document for a custom page.
pub fn render_page(slug: &str, page_id: &str, state: &SharedState) -> Result<String, ServiceError> {
    let custom = live_custom(slug, page_id, state)?;
    let source = state
        .repository
        .page_source(&custom.source_sha256)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .ok_or_else(|| ServiceError::Internal(format!("The source {} of page '{page_id}' is not stored", custom.source_sha256)))?;
    let meta = json!({
        "app": { "slug": slug, "page_id": page_id, "title": custom.page.title },
        "grants": custom.grants,
        "format": format_name(custom.format),
    });
    let meta = script_json(&meta);
    let source = script_json(&Value::String(source));
    Ok(fill(
        SHELL,
        &[
            ("@@META@@", &meta),
            ("@@SOURCE@@", &source),
            ("@@REACT@@", REACT),
            ("@@REACT_DOM@@", REACT_DOM),
            ("@@SUCRASE@@", SUCRASE),
            ("@@BRIDGE@@", BRIDGE),
            ("@@LOADER@@", LOADER),
        ],
    ))
}

pub fn get_page(slug: &str, page_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    Ok(json!({ "html": render_page(slug, page_id, state)? }))
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn need<'a>(args: &'a Value, key: &str, op: &str) -> Result<&'a str, ServiceError> {
    text(args, key).ok_or_else(|| ServiceError::BadRequest(format!("The op '{op}' needs the argument '{key}'")))
}

/// Every call a page makes. The checks run in this order and stop at the first refusal.
pub fn page_call(caller: &AuthUser, origin: &Origin, args: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let slug = need(&args, "app_slug", "page_call")?.to_string();
    let page_id = need(&args, "page_id", "page_call")?.to_string();
    let op = need(&args, "op", "page_call")?.to_string();

    // 1. The page, from the live manifest. It must be a custom page.
    let custom = live_custom(&slug, &page_id, state)?;

    if op == "settings_get" || op == "settings_set" {
        return settings(caller, &slug, &page_id, &op, &args, state);
    }
    if !["list", "get", "create", "update", "remove"].contains(&op.as_str()) {
        return Err(ServiceError::BadRequest(format!(
            "Unknown op '{op}'. Use list, get, create, update, remove, settings_get, or settings_set"
        )));
    }

    // 2. The grant for the table. A table with no grant is invisible to the page.
    let table_id = need(&args, "table_id", &op)?.to_string();
    let grant = custom
        .grants
        .iter()
        .find(|g| g.table_id == table_id)
        .ok_or_else(|| ServiceError::forbidden(format!("The page '{page_id}' has no grant for the table '{table_id}'")))?;

    // 3. What the grant allows.
    match op.as_str() {
        "create" if !grant.create => return Err(ServiceError::forbidden(format!("The page '{page_id}' may not create records in '{table_id}'"))),
        "remove" if !grant.delete => return Err(ServiceError::forbidden(format!("The page '{page_id}' may not delete records in '{table_id}'"))),
        _ => {}
    }
    if op == "create" || op == "update" {
        let data = args.get("data").and_then(Value::as_object).ok_or_else(|| ServiceError::BadRequest(format!("The op '{op}' needs the argument 'data'")))?;
        let refused: Vec<&str> = data.keys().filter(|k| !grant.write.contains(k)).map(String::as_str).collect();
        if !refused.is_empty() {
            let allowed = if grant.write.is_empty() { "nothing".to_string() } else { grant.write.join(", ") };
            return Err(ServiceError::forbidden(format!(
                "The page '{page_id}' may not set {} in '{table_id}'. It may set: {allowed}",
                refused.join(", ")
            )));
        }
    }
    if op == "list" {
        let mut named = Vec::new();
        if let Some(f) = args.get("filter") {
            if let Ok(f) = serde_json::from_value::<scaffoldry_engine::CompoundFilter>(f.clone()) {
                fields_of(&f, &mut named);
            }
        }
        if let Some(name) = args.get("sort").and_then(|s| s.get("field_name")).and_then(Value::as_str) {
            named.push(name.to_string());
        }
        if let Some(hidden) = named.iter().find(|n| !grant.read.contains(n)) {
            return Err(ServiceError::forbidden(format!("The page '{page_id}' may not filter or sort by '{hidden}', because it may not read it")));
        }
    }

    // 4. The same tool the data tools use. The caller's own rights are checked there.
    let result = match op.as_str() {
        "list" => {
            let mut forwarded = Map::new();
            forwarded.insert("app_slug".into(), json!(slug));
            forwarded.insert("table_id".into(), json!(table_id));
            for key in ["filter", "sort", "limit", "cursor"] {
                if let Some(v) = args.get(key).filter(|v| !v.is_null()) {
                    forwarded.insert(key.into(), v.clone());
                }
            }
            let listed = tools::call(caller, origin, "list_records", Value::Object(forwarded), state)?;
            let rows: Vec<Value> = listed["records"].as_array().map(|rs| rs.iter().map(|r| cut(r, &grant.read)).collect()).unwrap_or_default();
            return Ok(json!({ "records": rows, "next_cursor": listed["next_cursor"] }));
        }
        "create" => {
            let mut data = args["data"].as_object().cloned().unwrap_or_default();
            data.insert("_table_id".into(), json!(table_id));
            tools::call(caller, origin, "create_record", json!({ "app_slug": slug, "data": data }), state)?
        }
        _ => {
            let record_id = need(&args, "record_id", &op)?.to_string();
            // A grant names a table. A record of another table is not reachable by its id.
            let current = tools::call(caller, origin, "get_record", json!({ "app_slug": slug, "record_id": record_id }), state)?;
            if current["data"]["_table_id"].as_str() != Some(table_id.as_str()) {
                return Err(ServiceError::NotFound(format!("Record '{record_id}' not found")));
            }
            match op.as_str() {
                "get" => current,
                "update" => {
                    let version = args.get("version").and_then(Value::as_i64).ok_or_else(|| ServiceError::BadRequest("The op 'update' needs the argument 'version'".into()))?;
                    tools::call(caller, origin, "update_record", json!({ "app_slug": slug, "record_id": record_id, "version": version, "data": args["data"] }), state)?
                }
                _ => {
                    let version = args.get("version").and_then(Value::as_i64).ok_or_else(|| ServiceError::BadRequest("The op 'remove' needs the argument 'version'".into()))?;
                    return tools::call(caller, origin, "delete_record", json!({ "app_slug": slug, "record_id": record_id, "version": version }), state);
                }
            }
        }
    };
    // 5. Each field the page may not read is removed from what it gets back.
    Ok(cut(&result, &grant.read))
}

/// A record as a page may see it: its id, its version, and the fields it may read. Nothing else of the row leaves.
fn cut(record: &Value, read: &[String]) -> Value {
    let data = record.get("data").and_then(Value::as_object);
    let kept: Map<String, Value> = read.iter().filter_map(|f| data.and_then(|d| d.get(f)).map(|v| (f.clone(), v.clone()))).collect();
    json!({ "id": record["id"], "version": record["version"], "data": kept })
}

fn fields_of(f: &scaffoldry_engine::CompoundFilter, out: &mut Vec<String>) {
    out.extend(f.clauses.iter().map(|c| c.field_name.clone()));
    for g in &f.groups {
        fields_of(g, out);
    }
}

/// A setting is the person's own, kept per page. It is never shown to anyone else.
fn settings(caller: &AuthUser, slug: &str, page_id: &str, op: &str, args: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let key = need(args, "key", op)?.to_string();
    if key.chars().count() > MAX_KEY_CHARS {
        return Err(ServiceError::BadRequest(format!("A setting key is at most {MAX_KEY_CHARS} characters")));
    }
    let (slug, page_id, eppn) = (slug.to_string(), page_id.to_string(), caller.eppn.to_lowercase());
    let internal = |e: crate::repository::RepositoryError| ServiceError::Internal(e.to_string());
    if op == "settings_get" {
        let value: Option<Value> = state
            .repository
            .with_client(move |c| {
                let row = c.query_opt("SELECT value FROM page_settings WHERE app_slug = $1 AND page_id = $2 AND eppn = $3 AND key = $4", &[&slug, &page_id, &eppn, &key])?;
                Ok(row.map(|r| r.get::<_, Value>(0)))
            })
            .map_err(internal)?;
        return Ok(json!({ "value": value.unwrap_or(Value::Null) }));
    }
    let value = args.get("value").cloned().ok_or_else(|| ServiceError::BadRequest("The op 'settings_set' needs the argument 'value'".into()))?;
    let size = value.to_string().len();
    if size > MAX_SETTINGS_BYTES {
        return Err(ServiceError::BadRequest(format!("A value of {size} bytes is over the 64 KB a person may keep for a page")));
    }
    let outcome = state
        .repository
        .with_client(move |c| {
            let mut tx = c.transaction()?;
            // One person's writes to one page take turns, so two at once cannot together pass the limit.
            tx.execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&format!("page_settings:{slug}:{page_id}:{eppn}")])?;
            let row = tx.query_one(
                "SELECT COALESCE(SUM(octet_length(value::text)), 0)::BIGINT, COUNT(*)::BIGINT FROM page_settings \
                 WHERE app_slug = $1 AND page_id = $2 AND eppn = $3 AND key <> $4",
                &[&slug, &page_id, &eppn, &key],
            )?;
            let (others, keys): (i64, i64) = (row.get(0), row.get(1));
            if others as usize + size > MAX_SETTINGS_BYTES {
                return Ok(Some(format!("A person may keep 64 KB of settings for a page. {others} bytes are in use under other keys")));
            }
            if keys >= MAX_SETTING_KEYS {
                return Ok(Some(format!("A person may keep {MAX_SETTING_KEYS} setting keys for a page")));
            }
            tx.execute(
                "INSERT INTO page_settings (app_slug, page_id, eppn, key, value) VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (app_slug, page_id, eppn, key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&slug, &page_id, &eppn, &key, &value],
            )?;
            tx.commit()?;
            Ok(None)
        })
        .map_err(internal)?;
    match outcome {
        Some(problem) => Err(ServiceError::BadRequest(problem)),
        None => Ok(json!({ "saved": true })),
    }
}
