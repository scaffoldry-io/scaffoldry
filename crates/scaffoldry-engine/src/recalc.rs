//! What a changed definition recalculates, and how a value converts when a field changes type.

use crate::graph::{build_graph, NodeId};
use crate::index::index_value;
use crate::{AppManifest, FieldSpec, FieldType};
use serde_json::Value;
use std::collections::BTreeSet;

/// The stored computed fields of one table that need their values written again, in run order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableChange {
    pub table_id: String,
    pub fields: Vec<String>,
}

/// A field whose type changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeChange {
    pub table_id: String,
    pub field: String,
    pub from: FieldType,
    pub to: FieldType,
}

/// What a computed field is, as far as its stored value goes.
fn definition(f: &FieldSpec) -> (FieldType, Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    (f.field_type, f.formula_expression.as_deref(), f.computed_via.as_deref(), f.target_display_field.as_deref(), f.rollup_function.as_deref())
}

/// True when the field's value is stored in the record. A lookup is read, not stored, and so is a
/// formula over a lookup.
fn is_stored_computed(table_fields: &[FieldSpec], f: &FieldSpec) -> bool {
    match f.field_type {
        FieldType::Rollup | FieldType::Count => true,
        FieldType::Formula => {
            let refs = f.formula_expression.as_deref().and_then(crate::formula_field_refs).unwrap_or_default();
            !refs.iter().any(|r| table_fields.iter().any(|x| &x.name == r && x.field_type == FieldType::Lookup))
        }
        _ => false,
    }
}

/// The computed fields whose stored values the change from `old` to `new` makes wrong: those whose
/// definition changed or that are new, and everything downstream of them. `None` for `old` is a new
/// app, which has no records.
pub fn changed_calculations(old: Option<&AppManifest>, new: &AppManifest) -> Vec<TableChange> {
    let Some(old) = old else { return Vec::new() };
    let mut changed: Vec<NodeId> = Vec::new();
    for t in &new.tables {
        let before = old.tables.iter().find(|x| x.id == t.id);
        for f in t.fields.iter().filter(|f| is_stored_computed(&t.fields, f)) {
            let same = before.and_then(|b| b.fields.iter().find(|x| x.name == f.name)).is_some_and(|b| definition(b) == definition(f));
            if !same {
                changed.push(NodeId { table: t.id.clone(), field: f.name.clone() });
            }
        }
    }
    if changed.is_empty() {
        return Vec::new();
    }
    let graph = build_graph(new).ok();
    let mut nodes: BTreeSet<NodeId> = changed.iter().cloned().collect();
    if let Some(g) = &graph {
        nodes.extend(g.affected(&changed));
    }
    let mut out = Vec::new();
    for t in &new.tables {
        let order: Vec<String> = match &graph {
            Some(g) => g.order(&t.id),
            None => t.fields.iter().map(|f| f.name.clone()).collect(),
        };
        let fields: Vec<String> = order
            .into_iter()
            .filter(|name| nodes.contains(&NodeId { table: t.id.clone(), field: name.clone() }))
            .filter(|name| t.fields.iter().find(|f| &f.name == name).is_some_and(|f| is_stored_computed(&t.fields, f)))
            .collect();
        if !fields.is_empty() {
            out.push(TableChange { table_id: t.id.clone(), fields });
        }
    }
    out
}

/// The fields that exist in both manifests with a different type, where neither end is computed.
pub fn type_changes(old: &AppManifest, new: &AppManifest) -> Vec<TypeChange> {
    let mut out = Vec::new();
    for t in &new.tables {
        let Some(before) = old.tables.iter().find(|x| x.id == t.id) else { continue };
        for f in &t.fields {
            let Some(b) = before.fields.iter().find(|x| x.name == f.name) else { continue };
            if b.field_type != f.field_type && !b.field_type.is_computed() && !f.field_type.is_computed() {
                out.push(TypeChange { table_id: t.id.clone(), field: f.name.clone(), from: b.field_type, to: f.field_type });
            }
        }
    }
    out
}

fn text_like(t: FieldType) -> bool {
    matches!(t, FieldType::Text | FieldType::Select | FieldType::Email | FieldType::Phone | FieldType::Url | FieldType::LongText)
}

fn number_like(t: FieldType) -> bool {
    matches!(t, FieldType::Number | FieldType::Currency | FieldType::Percent | FieldType::Rating)
}

/// True when a field can change from one type to the other with a defined result for every value.
pub fn conversion_supported(from: FieldType, to: FieldType) -> bool {
    (text_like(from) || number_like(from)) && (text_like(to) || number_like(to) || (text_like(from) && to == FieldType::Date))
        || (from == FieldType::Date && text_like(to))
}

/// What a value becomes.
#[derive(Debug, Clone, PartialEq)]
pub enum Converted {
    /// It already fits, or it is blank. Leave it.
    Keep,
    /// It is written as this.
    To(Value),
    /// It does not fit the new type, so the field is left empty.
    Blank,
}

fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// The value of a field after its type changes from `from` to `to`.
pub fn convert_value(value: &Value, from: FieldType, to: FieldType) -> Converted {
    let blank = match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    };
    if blank || from == to {
        return Converted::Keep;
    }
    if number_like(to) {
        if value.is_number() {
            return Converted::Keep;
        }
        let spec = FieldSpec::simple("x", "x", to, false, false);
        return match index_value(&spec, &Value::String(value.as_str().unwrap_or_default().trim().to_string())).0 {
            Some(n) => Converted::To(number_value(n)),
            None => Converted::Blank,
        };
    }
    if text_like(to) {
        return match value {
            Value::String(_) => Converted::Keep,
            Value::Number(n) => Converted::To(Value::String(n.as_f64().map(|f| if f.fract() == 0.0 && f.abs() < 9e15 { (f as i64).to_string() } else { f.to_string() }).unwrap_or_else(|| n.to_string()))),
            other => Converted::To(Value::String(other.to_string())),
        };
    }
    if to == FieldType::Date {
        return match value.as_str().and_then(crate::parse_instant) {
            Some(_) => Converted::Keep,
            None => Converted::Blank,
        };
    }
    Converted::Keep
}
