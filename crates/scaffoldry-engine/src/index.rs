//! What a table indexes, and what value each indexed field stores.
//!
//! `record_index` is a pivot table of typed value columns, keyed by field. It is derived from the
//! records, written in the same transaction as the record, and can be rebuilt from them at any time.
//! Two functions here decide its content, so the write path and the rebuild path cannot disagree.

use crate::{AppManifest, FieldSpec, FieldType};
use serde_json::Value;
use std::collections::BTreeSet;

/// The most characters of a text value that are indexed.
pub const MAX_TEXT: usize = 128;

/// The most fields one table may index.
pub const MAX_INDEXED_FIELDS: usize = 20;

/// `(num, txt, ts)`. At most one is set. All three are `None` for a blank value.
pub type IndexCols = (Option<f64>, Option<String>, Option<String>);

/// Which column a field's values go in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexColumn {
    Num,
    Txt,
    Ts,
}

pub fn index_column(t: FieldType) -> IndexColumn {
    use FieldType::*;
    match t {
        Number | Currency | Percent | Rating | Autonumber | Count | Rollup | Boolean | Checkbox => IndexColumn::Num,
        Date | CreatedTime | LastModifiedTime => IndexColumn::Ts,
        // A formula's column follows its result, so this is only the default for a sort on it.
        // A Link is never indexed here. `record_links` holds it.
        Text | Select | Email | Phone | Url | Relation | MultiSelect | Lookup | Formula | Link => IndexColumn::Txt,
    }
}

/// The fields of a table that are indexed: marked `indexed`, used by a view of the table to sort or
/// filter, or joined on by a relationship.
pub fn indexed_fields(m: &AppManifest, table_id: &str) -> BTreeSet<String> {
    let Some(table) = m.tables.iter().find(|t| t.id == table_id) else { return BTreeSet::new() };
    let mut out: BTreeSet<String> = table.fields.iter().filter(|f| f.indexed).map(|f| f.name.clone()).collect();
    for view in m.views.iter().filter(|v| v.table_id.as_deref() == Some(table_id)) {
        out.extend(view.sort_rules.iter().map(|r| r.field_name.clone()));
        if let Some(filter) = &view.filters {
            out.extend(filter.clauses.iter().map(|c| c.field_name.clone()));
        }
    }
    for r in &m.relationships {
        if r.source_table_id == table_id {
            out.insert(r.source_field.clone());
        }
        if r.target_table_id == table_id {
            out.insert(r.target_field.clone());
        }
    }
    // A rollup reads its target field from the index, so the field of the table it rolls up is indexed.
    for owner in &m.tables {
        for f in owner.fields.iter().filter(|f| f.field_type == FieldType::Rollup) {
            let target = f
                .computed_via
                .as_deref()
                .and_then(|via| owner.fields.iter().find(|x| x.name == via))
                .and_then(|via| via.link_target_table.as_deref());
            if target == Some(table_id) {
                if let Some(tf) = &f.target_display_field {
                    out.insert(tf.clone());
                }
            }
        }
    }
    // A name that is not a field of the table is not indexed. `validate_manifest` reports it.
    out.retain(|name| table.fields.iter().any(|f| &f.name == name));
    out
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// A number written as text, in plain decimal form only.
fn parse_decimal(s: &str) -> Option<f64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };
    let ok = !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()) && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()));
    if ok {
        s.parse::<f64>().ok()
    } else {
        None
    }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) => 29,
        _ => 28,
    }
}

/// `YYYY-MM-DD`, or RFC 3339 with a `Z` or `+HH:MM` offset. Returns a form PostgreSQL reads as a
/// `timestamptz`, or `None` for anything else.
pub fn parse_instant(s: &str) -> Option<String> {
    let num = |t: &str| -> Option<u32> { if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) { t.parse().ok() } else { None } };
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (num(&s[0..4])?, num(&s[5..7])?, num(&s[8..10])?);
    if !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) {
        return None;
    }
    if b.len() == 10 {
        return Some(format!("{s}T00:00:00+00:00"));
    }
    if b[10] != b'T' || b.len() < 20 || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (h, mi, sec) = (num(&s[11..13])?, num(&s[14..16])?, num(&s[17..19])?);
    if h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let n = frac.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &frac[n..];
    }
    let head = &s[..s.len() - rest.len()];
    match rest {
        "Z" => Some(format!("{head}+00:00")),
        _ if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) && rest.as_bytes()[3] == b':' => {
            let (oh, om) = (num(&rest[1..3])?, num(&rest[4..6])?);
            if oh > 23 || om > 59 {
                None
            } else {
                Some(s.to_string())
            }
        }
        _ => None,
    }
}

fn text_of(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.to_lowercase(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    Some(s.chars().take(MAX_TEXT).collect())
}

fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_decimal(s),
        _ => None,
    }
}

/// The columns for one scalar value. A value that does not fit the field's column is blank.
pub fn index_value(field: &FieldSpec, value: &Value) -> IndexCols {
    if is_blank(value) {
        return (None, None, None);
    }
    let column = match field.field_type {
        // A formula stores a number as a number and anything else as text.
        FieldType::Formula => {
            if number_of(value).is_some_and(|_| value.is_number()) {
                IndexColumn::Num
            } else {
                IndexColumn::Txt
            }
        }
        t => index_column(t),
    };
    match column {
        IndexColumn::Num => {
            let n = if matches!(field.field_type, FieldType::Boolean | FieldType::Checkbox) {
                match value {
                    Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                    Value::String(s) if s == "true" => Some(1.0),
                    Value::String(s) if s == "false" => Some(0.0),
                    _ => None,
                }
            } else {
                number_of(value)
            };
            (n, None, None)
        }
        IndexColumn::Ts => (None, None, value.as_str().and_then(parse_instant)),
        IndexColumn::Txt => (None, text_of(value), None),
    }
}

/// Every row a record writes for one field: one per value of a multi-valued field, numbered in
/// order, and one blank row when there is no value.
pub fn index_rows(field: &FieldSpec, value: &Value) -> Vec<IndexCols> {
    match value {
        Value::Array(items) if !items.is_empty() => items.iter().map(|v| index_value(field, v)).collect(),
        other => vec![index_value(field, other)],
    }
}
