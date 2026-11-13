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
    Ok(())
}

/// `POST /apps/{slug}/views/{id}/submit`: a filled-in form becomes a record.
pub fn submit(caller: &AuthUser, origin: &crate::service::history::Origin, app_slug: &str, id: Uuid, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let stored = views::existing(caller, app_slug, id, state)?;
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
    let record = records::create_record_skipping(caller, origin, app_slug, &Value::Object(data), &hidden, state)?;
    Ok(json!({
        "record": record,
        "success_message": stored.config.get("success_message").and_then(Value::as_str).unwrap_or("Thank you. Your answers were saved."),
    }))
}
