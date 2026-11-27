//! Typed text in a tool result is marked untrusted (`docs/plans/mcp-guidance.md`, phase 6).
//!
//! A person typed it, so it may carry instructions meant for the agent that reads it. It moves out of its usual place and into
//! the `untrusted_text` member of the object that held it. That is the member `read_library_snapshot` already uses.
//!
//! The shape:
//! - A text with no other name, such as a page of a policy source, is the member itself: `"untrusted_text": "..."`.
//! - Text that had a name goes under it: `{ "title": "Notes", "untrusted_text": { "description": "..." } }`.
//! - Text in a record's `data` keeps its place in the path: `{ "id": "r1", "data": {}, "untrusted_text": { "data": { "title": "..." } } }`.
//!
//! The marking is applied to the MCP result, after the tool has run, so no tool can forget it and a REST route is not changed.
//! What counts as typed text is a list of keys, a few tool-specific paths, and the text fields of an app for record data. A new
//! tool that returns typed text under a new key fails `tests/mcp_guidance_phase6_test.rs` until the key is listed here.

use crate::state::SharedState;
use scaffoldry_engine::{FieldType, HostRouter};
use serde_json::{Map, Value};

/// The member that holds typed text.
pub const FIELD: &str = "untrusted_text";

/// Keys that hold text a person typed, wherever they appear in a result.
const TYPED_KEYS: &[&str] = &[
    "title", "description", "label", "summary", "reason", "rationale", "note", "comment", "question", "statement", "justification", "answers", "log",
    "task", "instructions", "prompt", "help", "applies_to", "context_passage", "passage", "anchor_name", "path_names", "decision_note", "project",
    "agreement_reference",
    // Sentences the server builds from titles, labels and values a person typed, and the values of a record that changed.
    "changes", "messages",
];

/// A name is typed text for a workspace, a table, a view or a person. For a field it is an identifier, so it is listed by path.
const NAME_PATHS: &[(&str, &str)] = &[
    ("list_workspaces", "items[].name"),
    ("list_workspaces", "items[].collaborators[].name"),
    ("get_workspace", "name"),
    ("get_workspace", "collaborators[].name"),
    ("update_workspace", "name"),
    ("update_workspace", "collaborators[].name"),
    ("describe_app", "tables[].name"),
    ("list_datasets", "items[].name"),
    ("list_datasets", "items[].app_title"),
    // Sample rows are made up, but a select shows the label a person typed.
    ("list_datasets", "items[].sample.rows"),
    ("list_apps", "items[].tables"),
    ("list_views", "views[].name"),
    ("save_view", "name"),
    // The document embeds the source a person wrote.
    ("get_page", "html"),
    ("admin_inventory", "rows[].name"),
    ("admin_inventory", "rows[].payload.name"),
    // A sentence that names the thing asked for, by its title.
    ("request_access", "object"),
    ("aggregate_records", "groups[].key"),
    ("aggregate_records", "groups[].key2"),
    ("aggregate_records", "groups[].group"),
    ("get_view_data", "stages[].label"),
    ("get_view_data", "stages[].name"),
];

/// Arrays whose elements are the bare data of a record. The app is the call's `app_slug`.
const RECORD_DATA_PATHS: &[(&str, &str)] = &[("get_view_data", "stages[].records[]")];

fn is_text(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        _ => false,
    }
}

/// Moves `key` of `carrier` under its `untrusted_text` member.
fn move_key(carrier: &mut Map<String, Value>, key: &str) {
    if key == FIELD || !carrier.get(key).is_some_and(is_text) {
        return;
    }
    // A tool may already have its own `untrusted_text` here that is not a map. It stays as it is, and so does the text.
    if carrier.get(FIELD).is_some_and(|f| !f.is_object()) {
        return;
    }
    if let Some(text) = carrier.remove(key) {
        if let Value::Object(slot) = carrier.entry(FIELD).or_insert_with(|| Value::Object(Map::new())) {
            slot.insert(key.to_string(), text);
        }
    }
}

/// Runs `f` on every object a path such as `items[].collaborators[]` reaches.
fn each_object(v: &mut Value, path: &[&str], f: &mut dyn FnMut(&mut Map<String, Value>)) {
    let Some((head, rest)) = path.split_first() else {
        if let Value::Object(m) = v {
            f(m);
        }
        return;
    };
    let (key, each) = match head.strip_suffix("[]") {
        Some(k) => (k, true),
        None => (*head, false),
    };
    let Some(child) = v.get_mut(key) else { return };
    match (each, child) {
        (true, Value::Array(items)) => items.iter_mut().for_each(|i| each_object(i, rest, f)),
        (false, child) => each_object(child, rest, f),
        _ => {}
    }
}

/// The names of the fields of `slug` whose values a person types.
fn text_fields(state: &SharedState, slug: &str, table: Option<&str>) -> Vec<String> {
    let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
    let Some(manifest) = engine.resolve_by_slug(slug) else { return Vec::new() };
    manifest
        .tables
        .iter()
        .filter(|t| table.is_none_or(|id| t.id == id))
        .flat_map(|t| t.fields.iter())
        .filter(|f| matches!(f.field_type, FieldType::Text | FieldType::LongText | FieldType::Email | FieldType::Phone | FieldType::Url | FieldType::Formula | FieldType::Lookup | FieldType::Rollup))
        .map(|f| f.name.clone())
        .collect()
}

/// The text fields of the record `data` belongs to.
fn record_text_names(state: &SharedState, slug: &str, data: &Value) -> Vec<String> {
    text_fields(state, slug, data.get("_table_id").and_then(Value::as_str))
}

fn walk(v: &mut Value, state: &SharedState, call_slug: Option<&str>) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(|i| walk(i, state, call_slug)),
        Value::Object(m) => {
            // A record: `app_slug` and its `data`. The text keeps the path `data`. A row a page reads has no `app_slug`.
            // It has `version` beside `data`, and the call's `app_slug` names its app.
            let slug = m.get("app_slug").and_then(Value::as_str).or_else(|| if m.contains_key("version") { call_slug } else { None });
            let record = slug.zip(m.get("data").filter(|d| d.is_object())).map(|(slug, d)| record_text_names(state, slug, d));
            if let Some(names) = record {
                if let Some(Value::Object(data)) = m.get_mut("data") {
                    let mut moved: Vec<(String, Value)> = Vec::new();
                    for n in &names {
                        if data.get(n).is_some_and(is_text) {
                            if let Some(t) = data.remove(n) {
                                moved.push((n.clone(), t));
                            }
                        }
                    }
                    if !moved.is_empty() {
                        let slot = m.entry(FIELD).or_insert_with(|| Value::Object(Map::new()));
                        if let Value::Object(slot) = slot {
                            let inner = slot.entry("data").or_insert_with(|| Value::Object(Map::new()));
                            if let Value::Object(inner) = inner {
                                inner.extend(moved);
                            }
                        }
                    }
                }
            }
            for key in TYPED_KEYS {
                move_key(m, key);
            }
            for (k, child) in m.iter_mut() {
                if k != FIELD {
                    walk(child, state, call_slug);
                }
            }
        }
        _ => {}
    }
}

/// Marks the typed text in `result`, the answer of `tool`. `args` are the call's arguments.
pub fn mark(tool: &str, args: &Value, state: &SharedState, result: &mut Value) {
    for (_, path) in NAME_PATHS.iter().filter(|(t, _)| *t == tool) {
        let segments: Vec<&str> = path.split('.').collect();
        let (last, parents) = segments.split_last().map_or(("", &[][..]), |(l, p)| (*l, p));
        each_object(result, parents, &mut |m| move_key(m, last));
    }
    if let Some(slug) = args.get("app_slug").and_then(Value::as_str) {
        for (_, path) in RECORD_DATA_PATHS.iter().filter(|(t, _)| *t == tool) {
            let segments: Vec<&str> = path.split('.').collect();
            each_object(result, &segments, &mut |data| {
                let names = record_text_names(state, slug, &Value::Object(data.clone()));
                for name in names {
                    move_key(data, &name);
                }
            });
        }
    }
    walk(result, state, args.get("app_slug").and_then(Value::as_str));
}

/// Puts every typed text back where it was, for code that renders it as text and is not a model. A screen inside the agent's
/// host does this. The server's own screens carry a copy of this function.
pub fn restore(v: &mut Value) {
    match v {
        Value::Array(items) => items.iter_mut().for_each(restore),
        Value::Object(m) => {
            if let Some(Value::Object(text)) = m.remove(FIELD) {
                for (k, t) in text {
                    match (m.get_mut(&k), t) {
                        (Some(Value::Object(into)), Value::Object(more)) => into.extend(more),
                        (_, t) => {
                            m.insert(k, t);
                        }
                    }
                }
            }
            m.values_mut().for_each(restore);
        }
        _ => {}
    }
}
