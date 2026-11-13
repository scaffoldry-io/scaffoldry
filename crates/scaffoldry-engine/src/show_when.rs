//! Views phase 7: `show_when` on the fields of a form view.
//!
//! A form field may carry conditions over the answers to other fields of the same form. The field shows only when every
//! condition holds. The server runs this at submit and is authoritative. The browser has a second, equally small
//! evaluator for immediate feedback, and `tests/show_when_test.rs` generates the vectors that keep the two the same.
//!
//! Rules a vector pins down:
//! - An answer is empty when it is missing, null, a blank string, or an empty list. A hidden field answers as empty.
//! - A number or a boolean compares as its text. A list answer matches `Eq` and `In` when any item does.
//! - A condition sees only the fields before its own. A field that is later, or not on the form, is empty.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowOp {
    Eq,
    Ne,
    In,
    IsEmpty,
    IsNotEmpty,
}

/// One condition: `field` is a field of the form, and `value` is what `Eq`, `Ne` and `In` compare with. `In` takes a list, or
/// a comma list in a string. `IsEmpty` and `IsNotEmpty` take no value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub field: String,
    pub op: ShowOp,
    #[serde(default)]
    pub value: Value,
}

/// A field of the form, with the conditions that show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    #[serde(default)]
    pub show_when: Vec<Condition>,
}

/// A scalar as the text it compares as. `None` is empty.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

/// The texts of an answer: one for a scalar, one for each item of a list, none when empty.
fn texts(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().filter_map(text).collect(),
        other => text(other).into_iter().collect(),
    }
}

/// The values `In` compares with.
fn wanted(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => s.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect(),
        other => texts(other),
    }
}

fn holds(c: &Condition, answer: &Value) -> bool {
    let have = texts(answer);
    match c.op {
        ShowOp::IsEmpty => have.is_empty(),
        ShowOp::IsNotEmpty => !have.is_empty(),
        ShowOp::Eq => {
            let want = text(&c.value);
            want.is_some_and(|w| have.contains(&w))
        }
        ShowOp::Ne => {
            let want = text(&c.value);
            !want.is_some_and(|w| have.contains(&w))
        }
        ShowOp::In => {
            let list = wanted(&c.value);
            have.iter().any(|h| list.contains(h))
        }
    }
}

/// Whether each field of the form shows, in the form's order, for these answers.
pub fn visible_fields(fields: &[FormField], answers: &Map<String, Value>) -> Vec<bool> {
    // What each earlier field answers as: its answer when it shows, and nothing when it does not.
    let mut seen: Map<String, Value> = Map::new();
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        let shown = f.show_when.iter().all(|c| holds(c, seen.get(&c.field).unwrap_or(&Value::Null)));
        seen.insert(f.name.clone(), if shown { answers.get(&f.name).cloned().unwrap_or(Value::Null) } else { Value::Null });
        out.push(shown);
    }
    out
}
