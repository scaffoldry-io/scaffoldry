//! App bundles (import-export phase 3).
//!
//! A bundle is one JSON document that holds an app's definition: the manifest, its non-personal saved views, its process
//! rules, the source of its custom pages keyed by hash, and optionally some sample records. It never holds a secret, a token,
//! a member, a comment, history or an attachment. A hash over the rest detects damage.
//!
//! Importing a bundle never makes a live app. It checks the hash and the shape, then makes a proposal with the same checks
//! every proposal has. The proposal gets a free slug if the bundle's is taken, and new ids where the bundle's collide.
//! A webhook in a bundle arrives off: it needs its own approval, which `integrations.md` builds.

use crate::repository_views::{materialize, StoredView};
use crate::service::proposals::{self, ProposeInput};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const BUNDLE_VERSION: u64 = 1;
/// The most sample records a bundle may hold.
pub const MAX_SAMPLE_RECORDS: usize = 5_000;
/// The parts a bundle may have. Anything else is refused.
const PARTS: [&str; 8] = ["bundle_version", "manifest", "views", "rules", "page_sources", "integrations", "sample_data", "sha256"];
/// Keys a bundle may not have at any depth outside its sample records. `secret_name` names a secret and is not one.
const BANNED_KEYS: [&str; 8] = ["secret", "token", "password", "members", "collaborators", "comments", "history", "attachments"];
/// What the manifest holds that belongs to the place the app lives, not to the app.
const PLACE_KEYS: [&str; 5] = ["workspace_id", "coordinates", "custom_domain", "custom_domain_verified", "sealed"];

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn bad(message: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(message.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash a bundle carries: SHA-256 of the document without its own `sha256`, as compact JSON with sorted keys.
pub fn digest_of(doc: &Value) -> String {
    let mut without = doc.clone();
    if let Some(o) = without.as_object_mut() {
        o.remove("sha256");
    }
    hex(&Sha256::digest(serde_json::to_string(&without).unwrap_or_default().as_bytes()))
}

/// The bundle of one app. `sample` adds up to 5,000 records, with the sensitive fields left out. Needs the app's owner or an
/// admin.
pub fn export(caller: &AuthUser, slug: &str, sample: bool, state: &SharedState) -> Result<Value, ServiceError> {
    let repo = &*state.repository;
    // The stored manifest, not the engine's: the engine's has every saved view merged in, a person's own among them.
    let manifest = repo.get_app_manifest(slug).map_err(internal)?.ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))?;
    let workspace = manifest.workspace_id.clone().unwrap_or_default();
    if !matches!(proposals::role_in(caller, &workspace, state).as_deref(), Some("owner" | "admin")) {
        return Err(ServiceError::forbidden("Forbidden: exporting an app needs the owner or admin role of its workspace"));
    }

    let mut manifest_json = serde_json::to_value(&manifest).map_err(internal)?;
    if let Some(o) = manifest_json.as_object_mut() {
        for key in PLACE_KEYS {
            o.remove(key);
        }
    }

    let views: Vec<Value> = repo
        .list_app_views(slug)
        .map_err(internal)?
        .into_iter()
        .filter(|v| v.ownership != "personal")
        .map(|v| json!({ "name": v.name, "kind": v.kind, "table_id": v.table_id, "ownership": v.ownership, "config": v.config }))
        .collect();

    let rules: Vec<Value> = state
        .automations
        .read()
        .map_err(internal)?
        .get(slug)
        .map(|list| {
            list.iter()
                .map(|r| {
                    let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                    if let Some(o) = v.as_object_mut() {
                        o.remove("app_slug");
                        o.remove("version");
                    }
                    v
                })
                .collect()
        })
        .unwrap_or_default();

    let mut page_sources = Map::new();
    for page in &manifest.pages {
        if let scaffoldry_engine::PageKind::Custom { source_sha256, .. } = &page.kind {
            if let Some(text) = repo.page_source(source_sha256).map_err(internal)? {
                page_sources.insert(source_sha256.to_lowercase(), Value::String(text));
            }
        }
    }

    let mut doc = json!({
        "bundle_version": BUNDLE_VERSION,
        "manifest": manifest_json,
        "views": views,
        "rules": rules,
        "page_sources": page_sources,
        "integrations": { "webhooks": [] },
    });
    if sample {
        doc["sample_data"] = sample_data(&manifest, state)?;
    }
    doc["sha256"] = json!(digest_of(&doc));
    Ok(doc)
}

/// Up to 5,000 records, a table at a time, as plain field values. Sensitive fields and the server's own keys are left out.
fn sample_data(manifest: &scaffoldry_engine::AppManifest, state: &SharedState) -> Result<Value, ServiceError> {
    let sensitive = proposals::sensitive_fields(manifest, state);
    let mut left = MAX_SAMPLE_RECORDS;
    let mut out = Map::new();
    for table in &manifest.tables {
        let mut rows: Vec<Value> = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        while left > 0 {
            let page = state.repository.list_records_page(&manifest.slug, &table.id, left.min(500), cursor.clone()).map_err(internal)?;
            let Some(last) = page.last() else { break };
            cursor = Some((last.created_at.clone(), last.id.clone()));
            let full = page.len() == left.min(500);
            for record in page {
                let mut data = record.data.as_object().cloned().unwrap_or_default();
                data.retain(|k, _| !k.starts_with('_') && !sensitive.contains(&(table.id.clone(), k.clone())));
                rows.push(Value::Object(data));
                left -= 1;
            }
            if !full {
                break;
            }
        }
        if !rows.is_empty() {
            out.insert(table.id.clone(), Value::Array(rows));
        }
    }
    Ok(Value::Object(out))
}

/// The first key at any depth that a bundle may not have, outside its sample records and its own hash.
fn banned_key(v: &Value, path: &str) -> Option<String> {
    match v {
        Value::Object(o) => o.iter().find_map(|(k, child)| {
            let here = format!("{path}.{k}");
            if BANNED_KEYS.contains(&k.to_lowercase().as_str()) {
                Some(here)
            } else {
                banned_key(child, &here)
            }
        }),
        Value::Array(a) => a.iter().enumerate().find_map(|(i, child)| banned_key(child, &format!("{path}[{i}]"))),
        _ => None,
    }
}

/// A slug nobody holds: the one asked for, else that with `-2`, `-3`, and so on.
fn free_slug(base: &str, state: &SharedState) -> Result<String, ServiceError> {
    let in_use = |s: &str| state.repository.slug_in_use(s).map_err(internal);
    if !in_use(base)? {
        return Ok(base.to_string());
    }
    for n in 2.. {
        let suffix = format!("-{n}");
        let stem: String = base.chars().take(64usize.saturating_sub(suffix.len())).collect();
        let candidate = format!("{stem}{suffix}");
        if !in_use(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!("a free slug is found")
}

/// Gives a rule a new id where another app already holds its id, since a rule's id is unique across apps.
fn rewrite_rule_ids(rules: &mut [Value], slug: &str, state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let held: std::collections::HashSet<String> =
        state.automations.read().map_err(internal)?.values().flat_map(|list| list.iter().map(|r| r.id.clone())).collect();
    let mut taken: std::collections::HashSet<String> = held.clone();
    let mut rewritten = Vec::new();
    for rule in rules.iter_mut() {
        let Some(id) = rule["id"].as_str().map(str::to_string) else { continue };
        if !held.contains(&id) {
            taken.insert(id);
            continue;
        }
        let mut candidate = format!("{id}-{slug}");
        let mut n = 2;
        while taken.contains(&candidate) {
            candidate = format!("{id}-{slug}-{n}");
            n += 1;
        }
        taken.insert(candidate.clone());
        rewritten.push(json!({ "kind": "rule", "from": id, "to": candidate }));
        rule["id"] = json!(candidate);
    }
    Ok(rewritten)
}

/// The saved views of a bundle as manifest views with ids of their own. The proposal carries them, and approving it stores them
/// like any other view, so none can collide with a view of the app the bundle came from.
fn views_into_manifest(views: &[Value], manifest: &mut Value, rewritten: &mut Vec<Value>) -> Result<(), ServiceError> {
    for v in views {
        let name = v["name"].as_str().ok_or_else(|| bad("a view in the bundle has no name"))?.to_string();
        let stored = StoredView {
            id: Uuid::new_v4(),
            app_slug: String::new(),
            table_id: v["table_id"].as_str().map(str::to_string),
            name: name.clone(),
            kind: v["kind"].as_str().unwrap_or("grid").to_string(),
            ownership: "collaborative".to_string(),
            owner: None,
            config: v["config"].clone(),
            version: 1,
            created_by: String::new(),
            created_at: String::new(),
            updated_by: String::new(),
            updated_at: String::new(),
        };
        let view = materialize(&stored).ok_or_else(|| bad(format!("the view '{name}' in the bundle is not a view")))?;
        if !manifest["views"].is_array() {
            manifest["views"] = json!([]);
        }
        manifest["views"].as_array_mut().expect("an array").push(serde_json::to_value(view).map_err(internal)?);
        rewritten.push(json!({ "kind": "view", "name": name, "to": stored.id.to_string() }));
    }
    Ok(())
}

/// Checks a bundle's hash and shape. Nothing is stored. Returns the parts the import uses.
fn verify(bundle: &Value) -> Result<(), ServiceError> {
    let doc = bundle.as_object().ok_or_else(|| bad("The bundle is not a JSON object"))?;
    let carried = doc.get("sha256").and_then(Value::as_str).ok_or_else(|| bad("The bundle has no sha256, so damage to it cannot be detected"))?;
    if !carried.eq_ignore_ascii_case(&digest_of(bundle)) {
        return Err(bad("The bundle is damaged: its sha256 does not match what it holds. Export it again."));
    }
    if doc.get("bundle_version").and_then(Value::as_u64) != Some(BUNDLE_VERSION) {
        return Err(bad(format!("This server reads bundle version {BUNDLE_VERSION}")));
    }
    if let Some(extra) = doc.keys().find(|k| !PARTS.contains(&k.as_str())) {
        return Err(bad(format!("The bundle has a part this server does not know: '{extra}'")));
    }
    if !doc.get("manifest").is_some_and(Value::is_object) {
        return Err(bad("The bundle has no manifest"));
    }
    for (part, kind) in [("views", "array"), ("rules", "array")] {
        if let Some(v) = doc.get(part) {
            if (kind == "array" && !v.is_array()) || (kind == "object" && !v.is_object()) {
                return Err(bad(format!("The bundle's {part} must be an {kind}")));
            }
        }
    }
    if !doc.get("page_sources").is_none_or(|v| v.as_object().is_some_and(|o| o.values().all(Value::is_string))) {
        return Err(bad("The bundle's page_sources must map a hash to the page's source text"));
    }
    let mut without_samples = bundle.clone();
    if let Some(o) = without_samples.as_object_mut() {
        o.remove("sample_data");
        o.remove("sha256");
    }
    if let Some(at) = banned_key(&without_samples, "bundle") {
        let word = at.rsplit('.').next().unwrap_or("").to_lowercase();
        return Err(bad(format!("A bundle never holds a {word}: found it at {at}")));
    }
    if let Some(samples) = doc.get("sample_data") {
        let tables = samples.as_object().ok_or_else(|| bad("The bundle's sample_data must map a table to its records"))?;
        let total: usize = tables.values().map(|rows| rows.as_array().map_or(0, Vec::len)).sum();
        if total > MAX_SAMPLE_RECORDS {
            return Err(bad(format!("The bundle holds {total} sample records. A bundle holds at most 5,000")));
        }
        if tables.values().any(|rows| !rows.as_array().is_some_and(|a| a.iter().all(Value::is_object))) {
            return Err(bad("Each table in sample_data must be a list of records"));
        }
    }
    Ok(())
}

/// The sources of the bundle's custom pages. A bundle holds everything its pages run, and each source hashes to the key it is
/// filed under. A server that already stores a page with that hash would otherwise take the stored text and never look at
/// the bundle's.
fn checked_page_sources(bundle: &Value, manifest: &Value) -> Result<Vec<String>, ServiceError> {
    let held = bundle["page_sources"].as_object().cloned().unwrap_or_default();
    for (key, text) in &held {
        let actual = hex(&Sha256::digest(text.as_str().unwrap_or("").as_bytes()));
        if !actual.eq_ignore_ascii_case(key) {
            return Err(bad(format!("page_source: the source filed under {key} hashes to {actual}. The bundle's page has been changed")));
        }
    }
    for page in manifest["pages"].as_array().into_iter().flatten() {
        if let Some(hash) = page["kind"]["Custom"]["source_sha256"].as_str() {
            if !held.keys().any(|k| k.eq_ignore_ascii_case(hash)) {
                return Err(bad(format!("page_source: the bundle does not hold the source of page '{}' ({hash})", page["id"].as_str().unwrap_or(""))));
            }
        }
    }
    Ok(held.values().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// The webhooks of a bundle as they arrive: off, with the name of the secret they need and none of its value.
fn arriving_webhooks(bundle: &Value) -> Result<Vec<Value>, ServiceError> {
    let Some(hooks) = bundle["integrations"]["webhooks"].as_array() else { return Ok(vec![]) };
    hooks
        .iter()
        .map(|h| {
            let name = h["name"].as_str().ok_or_else(|| bad("a webhook in the bundle has no name"))?;
            let url = h["url"].as_str().filter(|u| u.starts_with("https://")).ok_or_else(|| bad(format!("the webhook '{name}' needs an https url")))?;
            Ok(json!({ "name": name, "url": url, "events": h["events"], "enabled": false, "secret_name": h["secret_name"] }))
        })
        .collect()
}

/// Makes a proposal from a bundle. The workspace's owner, admin or editor may. Nothing becomes live.
pub fn import(caller: &AuthUser, workspace_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let bundle = body.get("bundle").ok_or_else(|| bad("The request needs a 'bundle'"))?;
    verify(bundle)?;
    let webhooks = arriving_webhooks(bundle)?;

    let mut manifest = bundle["manifest"].clone();
    let original = manifest["slug"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| bad("The bundle's manifest has no slug"))?.to_string();
    let wanted = body.get("slug").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or(&original).to_string();
    let slug = free_slug(&wanted, state)?;
    manifest["slug"] = json!(slug);
    manifest["workspace_id"] = json!(workspace_id);

    let mut rules: Vec<Value> = bundle["rules"].as_array().cloned().unwrap_or_default();
    let mut rewritten = rewrite_rule_ids(&mut rules, &slug, state)?;
    let views: Vec<Value> = bundle["views"].as_array().cloned().unwrap_or_default();
    views_into_manifest(&views, &mut manifest, &mut rewritten)?;

    let page_sources = checked_page_sources(bundle, &manifest)?;

    let mut notes: Vec<String> = Vec::new();
    for h in &webhooks {
        let secret = h["secret_name"].as_str().map(|s| format!(" and the secret '{s}' supplied by name")).unwrap_or_default();
        notes.push(format!(
            "The webhook '{}' to {} came with the bundle and is off. It needs its own approval and its host on the outbound allowlist{secret}.",
            h["name"].as_str().unwrap_or(""),
            h["url"].as_str().unwrap_or("")
        ));
    }
    let sample_records: usize = bundle["sample_data"].as_object().map(|t| t.values().map(|r| r.as_array().map_or(0, Vec::len)).sum()).unwrap_or(0);
    if sample_records > 0 {
        notes.push(format!("{sample_records} sample records came with the bundle. This import does not load them."));
    }

    let title = manifest["title"].as_str().unwrap_or(&slug).to_string();
    let summary = body.get("summary").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("Import the bundle '{title}'"));
    let mut proposal = proposals::propose(
        caller,
        ProposeInput { workspace_id: workspace_id.to_string(), app_slug: None, manifest: Some(manifest), rules, summary, page_sources, notes },
        state,
    )?;
    proposal["bundle"] = json!({
        "original_slug": original,
        "suggested_slug": if slug != wanted || slug != original { json!(slug) } else { Value::Null },
        "rewritten_ids": rewritten,
        "webhooks": webhooks,
        "sample_records": sample_records,
    });
    Ok(proposal)
}
