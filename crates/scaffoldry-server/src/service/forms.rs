//! Views phase 7: form views.
//!
//! A form view is a subset of a table's fields, in an order, with a label, help text and `show_when` conditions on each, and
//! a message to show after a submit. The conditions are run here at submit, and this is authoritative: the browser runs a
//! second copy only for immediate feedback. A submit goes through the ordinary create path, so every rule of the table,
//! the person's rights and the app's guards still apply.

use crate::service::{records, views, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_engine::show_when::{visible_fields, Condition, FormField, ShowOp};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The most fields a form holds.
pub const MAX_FIELDS: usize = 100;
const MAX_TEXT: usize = 500;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

/// The fields of a form config, with their conditions.
fn form_fields(config: &Value) -> Result<Vec<FormField>, ServiceError> {
    let list = config.get("form_fields").and_then(Value::as_array).ok_or_else(|| bad("A form needs a list of fields"))?;
    list.iter()
        .map(|f| {
            let name = f.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or_else(|| bad("Every field of a form has a name"))?.to_string();
            let show_when: Vec<Condition> = match f.get("show_when") {
                None | Some(Value::Null) => Vec::new(),
                Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad(format!("The conditions of '{name}' are not valid. Use eq, ne, in, is_empty or is_not_empty")))?,
            };
            Ok(FormField { name, show_when })
        })
        .collect()
}

/// Checks a form's config when it is saved. `table_fields` are the names of the fields of the table it is over.
pub fn validate(config: &Value, table_fields: &[String]) -> Result<(), ServiceError> {
    let fields = form_fields(config)?;
    if fields.is_empty() || fields.len() > MAX_FIELDS {
        return Err(bad(format!("A form holds 1 to {MAX_FIELDS} fields")));
    }
    let mut before: BTreeSet<&str> = BTreeSet::new();
    for f in &fields {
        if !table_fields.contains(&f.name) {
            return Err(bad(format!("'{}' is not a field of this table", f.name)));
        }
        if !before.insert(f.name.as_str()) {
            return Err(bad(format!("The field '{}' is on the form twice", f.name)));
        }
        for c in &f.show_when {
            // A condition sees the answers above it. A later field, or this one, is empty when the form is filled in.
            if !before.contains(c.field.as_str()) || c.field == f.name {
                return Err(bad(format!("A condition of '{}' names '{}', which is not a field above it on the form", f.name, c.field)));
            }
            if matches!(c.op, ShowOp::Eq | ShowOp::Ne | ShowOp::In) && c.value.is_null() {
                return Err(bad(format!("A condition of '{}' needs a value to compare with", f.name)));
            }
        }
    }
    for (i, f) in config["form_fields"].as_array().into_iter().flatten().enumerate() {
        for key in ["label", "help"] {
            if f.get(key).and_then(Value::as_str).is_some_and(|t| t.chars().count() > MAX_TEXT) {
                return Err(bad(format!("The {key} of field {} is over {MAX_TEXT} characters", i + 1)));
            }
        }
    }
    if config.get("success_message").and_then(Value::as_str).is_some_and(|t| t.chars().count() > MAX_TEXT) {
        return Err(bad(format!("The success message is over {MAX_TEXT} characters")));
    }
    validate_guest_keys(config, table_fields)
}

/// What a form's audience is. A form that does not say is for members.
pub fn audience(config: &Value) -> &str {
    config.get("audience").and_then(Value::as_str).unwrap_or("Members")
}

fn whole(config: &Value, key: &str, low: i64, high: i64) -> Result<(), ServiceError> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(v) if v.as_i64().is_some_and(|n| (low..=high).contains(&n)) => Ok(()),
        Some(_) => Err(bad(format!("{key} is a whole number from {low} to {high}"))),
    }
}

/// The keys of a guest form. They are read for any form, so a members form cannot carry a setting that means something else.
fn validate_guest_keys(config: &Value, table_fields: &[String]) -> Result<(), ServiceError> {
    if !matches!(audience(config), "Members" | "Guests") {
        return Err(bad("audience is Members or Guests"));
    }
    whole(config, "max_per_day", 1, 100)?;
    whole(config, "max_total", 1, 1_000_000)?;
    if let Some(read_only) = config.get("read_only_fields") {
        let names = read_only.as_array().filter(|a| a.len() <= MAX_FIELDS).and_then(|a| a.iter().map(Value::as_str).collect::<Option<Vec<_>>>()).ok_or_else(|| bad("read_only_fields is a list of field names"))?;
        if let Some(n) = names.iter().find(|n| !table_fields.iter().any(|f| f == *n)) {
            return Err(bad(format!("'{n}' in read_only_fields is not a field of this table")));
        }
    }
    if let Some(lock) = config.get("locks_when").filter(|l| !l.is_null()) {
        let field = lock.get("field").and_then(Value::as_str).filter(|f| table_fields.iter().any(|t| t == f));
        let values = lock.get("in").and_then(Value::as_array).filter(|a| !a.is_empty() && a.len() <= 50);
        if field.is_none() || values.is_none() {
            return Err(bad("locks_when names a field of this table and a list of values: { \"field\": \"status\", \"in\": [\"decided\"] }"));
        }
    }
    for f in config["form_fields"].as_array().into_iter().flatten() {
        if f.get("own_data").is_some_and(|v| !v.is_boolean()) {
            return Err(bad("own_data is true or false"));
        }
    }
    Ok(())
}

/// The part of a guest form that decides what a guest is shown and may fill. A change to it, once the form is open to
/// guests, is a new proposal.
pub fn exposure_shape(config: &Value) -> Value {
    let fields: Vec<Value> = config["form_fields"].as_array().into_iter().flatten().map(|f| json!({ "name": f.get("name"), "own_data": f.get("own_data").and_then(Value::as_bool).unwrap_or(false) })).collect();
    json!({ "form_fields": fields, "read_only_fields": config.get("read_only_fields").cloned().unwrap_or_else(|| json!([])) })
}

/// The `guest_exposure` check of a form that is about to be open to guests. A listed link fails validation, because a guest
/// must never be a way into another table or a dataset. A sensitive field a guest is asked for fails the check unless the form
/// marks it as something the person gives about themself (`own_data`), and sensitive fields the form leaves out are a warning.
pub fn check_exposure(config: &Value, fields: &[scaffoldry_engine::FieldSpec]) -> Result<Value, ServiceError> {
    use scaffoldry_engine::FieldType;
    let listed: Vec<(&str, bool)> = config["form_fields"].as_array().into_iter().flatten().filter_map(|f| Some((f.get("name")?.as_str()?, f.get("own_data").and_then(Value::as_bool).unwrap_or(false)))).collect();
    let spec = |n: &str| fields.iter().find(|f| f.name == n);
    let shown: Vec<&str> = listed.iter().map(|(n, _)| *n).chain(config.get("read_only_fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)).collect();
    if let Some(link) = shown.iter().find(|n| spec(n).is_some_and(|f| matches!(f.field_type, FieldType::Link | FieldType::Lookup))) {
        return Err(bad(format!("'{link}' is a link. A form for guests cannot list a link to another table or to a dataset")));
    }
    let mut fails = Vec::new();
    for (name, own) in &listed {
        if spec(name).is_some_and(|f| f.ferpa_sensitive) && !own {
            fails.push(format!("'{name}' is sensitive. Mark it as something the person gives about themself, or take it off the form."));
        }
    }
    for name in config.get("read_only_fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if spec(name).is_some_and(|f| f.ferpa_sensitive) {
            fails.push(format!("'{name}' is sensitive and would be shown to guests. A sensitive field cannot be shown read-only."));
        }
    }
    let unlisted: Vec<String> = fields.iter().filter(|f| f.ferpa_sensitive && !shown.contains(&f.name.as_str())).map(|f| f.name.clone()).collect();
    let (outcome, messages) = if !fails.is_empty() {
        ("fail", fails)
    } else if !unlisted.is_empty() {
        ("warn", vec![format!("This table also holds sensitive fields the form does not list: {}. Guests do not see them.", unlisted.join(", "))])
    } else {
        ("pass", Vec::new())
    };
    Ok(json!({ "name": "guest_exposure", "outcome": outcome, "messages": messages }))
}

/// The sentence a proposal to open a form to guests puts in the change list.
pub fn exposure_sentence(table: &str) -> String {
    format!("This form lets people outside the institution create records in {table}. They can see only their own.")
}

/// `POST /apps/{slug}/views/{id}/submit`: a filled-in form becomes a record.
pub fn submit(caller: &AuthUser, origin: &crate::service::history::Origin, app_slug: &str, id: Uuid, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = views::existing(caller, app_slug, id, state)?;
    submit_stored(caller, origin, app_slug, &stored, body, &serde_json::Map::new(), state)
}

/// A submit of a form already read. `system` holds fields the server sets itself, such as who a guest is. The person's answers
/// can never carry them, because only the form's own fields are taken.
pub fn submit_stored(caller: &AuthUser, origin: &crate::service::history::Origin, app_slug: &str, stored: &crate::repository_views::StoredView, body: &Value, system: &serde_json::Map<String, Value>, state: &SharedState) -> Result<Value, ServiceError> {
    if stored.kind != "form" {
        return Err(bad("This view is not a form"));
    }
    let table = stored.table_id.clone().ok_or_else(|| bad("This form is not over a table"))?;
    let fields = form_fields(&stored.config)?;
    let answers = body.get("data").and_then(Value::as_object).ok_or_else(|| bad("Send the answers as `data`"))?;

    // The form is a subset of the table. An answer for a field it does not have is not taken.
    if let Some(extra) = answers.keys().find(|k| !fields.iter().any(|f| &f.name == *k)) {
        return Err(bad(format!("'{extra}' is not a field of this form")));
    }
    let shown = visible_fields(&fields, answers);
    // An answer for a field the conditions hid is a client that skipped a condition. It is refused, not dropped, so the
    // person is not told their answer was kept.
    for (f, shown) in fields.iter().zip(&shown) {
        let given = answers.get(&f.name).is_some_and(|v| !matches!(v, Value::Null) && v.as_str().is_none_or(|s| !s.trim().is_empty()) && v.as_array().is_none_or(|a| !a.is_empty()));
        if !shown && given {
            return Err(bad(format!("'{}' is not shown for these answers, so it cannot be answered", f.name)));
        }
    }
    let hidden: Vec<String> = fields.iter().zip(&shown).filter(|(_, s)| !**s).map(|(f, _)| f.name.clone()).collect();

    let mut data = serde_json::Map::new();
    data.insert("_table_id".to_string(), json!(table));
    for (k, v) in answers {
        if !hidden.contains(k) {
            data.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in system {
        data.insert(k.clone(), v.clone());
    }
    let record = records::create_record_skipping(caller, origin, app_slug, &Value::Object(data), &hidden, state)?;
    Ok(json!({
        "record": record,
        "success_message": stored.config.get("success_message").and_then(Value::as_str).unwrap_or("Thank you. Your answers were saved."),
    }))
}
