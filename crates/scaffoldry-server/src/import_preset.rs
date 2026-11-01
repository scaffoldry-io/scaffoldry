//! Import phase 6: the preset for files exported from spreadsheet-database tools.
//!
//! The preset is a list of decisions. Each one is about one column, says in a sentence what it will do, and can be turned
//! off. A decision that is off leaves its column to the ordinary import, which reads the cell as the file wrote it.
//! Nothing here writes a row or fetches anything. The import jobs ask the plan to convert a cell and write the answer
//! through the same path every other cell takes.
//!
//! Decisions, by the target field's type:
//! - any column the mapping did not name: matched to a field by name, ignoring spaces, case and punctuation (`name`)
//! - multi-select: a comma-separated cell becomes a list of options (`multi`)
//! - link: the cell names a record of the linked table, which the import resolves by that table's primary field (`link`)
//! - checkbox: `checked`, `yes`, `true`, `1` are yes; blank, `unchecked`, `no`, `false`, `0` are no (`checkbox`)
//! - currency: `$1,200.50` is 1200.5 and `($5.25)` is -5.25 (`currency`)
//! - attachment: `name (address)` pairs go in a report. The address is never fetched and nothing is attached (`attachment`)

use scaffoldry_engine::{AppTable, FieldSpec, FieldType};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The name a mapping gives to ask for this preset.
pub const PRESET: &str = "other_tool";
/// The most file addresses a report holds.
pub const MAX_REPORT: usize = 1_000;

#[derive(Debug, Clone)]
pub struct Decision {
    pub kind: &'static str,
    pub column: String,
    /// The field the column is read into.
    pub field: String,
    pub sentence: String,
    pub enabled: bool,
}

impl Decision {
    pub fn id(&self) -> String {
        format!("{}:{}", self.kind, self.column)
    }

    pub fn to_json(&self) -> Value {
        json!({ "id": self.id(), "kind": self.kind, "column": self.column, "field": self.field, "sentence": self.sentence, "enabled": self.enabled })
    }
}

/// What a cell becomes.
pub enum Converted {
    Value(Value),
    /// The cell writes nothing. An attachment column is a report, not a value.
    Skip,
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub decisions: Vec<Decision>,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// The preset a mapping asks for, and the decisions it turns off.
pub fn requested(mapping: &Value) -> Option<HashSet<String>> {
    if mapping.get("preset").and_then(Value::as_str) != Some(PRESET) {
        return None;
    }
    Some(mapping.get("preset_off").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default())
}

fn decision(kind: &'static str, column: &str, field: &str, sentence: String, off: &HashSet<String>) -> Decision {
    let mut d = Decision { kind, column: column.to_string(), field: field.to_string(), sentence, enabled: true };
    d.enabled = !off.contains(&d.id());
    d
}

/// Works out the decisions for a file. `mapped` holds the columns the mapping already names, with their fields.
pub fn plan(headers: &[String], mapped: &HashMap<String, String>, table: &AppTable, off: &HashSet<String>) -> Plan {
    let mut decisions = Vec::new();
    for column in headers {
        // 1. Which field does this column go to?
        let field: Option<&FieldSpec> = match mapped.get(column) {
            Some(name) => table.fields.iter().find(|f| &f.name == name),
            None => {
                let wanted = norm(column);
                let found = table.fields.iter().find(|f| norm(&f.name) == wanted || norm(&f.label) == wanted);
                let Some(f) = found else { continue };
                let d = decision(
                    "name",
                    column,
                    &f.name,
                    format!("Column \"{column}\" goes to the field \"{}\", matched by name. Spaces and capital letters do not count.", f.label),
                    off,
                );
                let enabled = d.enabled;
                decisions.push(d);
                if !enabled {
                    continue;
                }
                Some(f)
            }
        };
        let Some(f) = field else { continue };

        // 2. What does the field's type ask of the cell?
        let d = match &f.field_type {
            FieldType::MultiSelect => Some(decision(
                "multi",
                column,
                &f.name,
                format!("Column \"{column}\" holds several options in one cell, separated by commas. Each is read as one option of \"{}\".", f.label),
                off,
            )),
            FieldType::Link => Some(decision(
                "link",
                column,
                &f.name,
                format!(
                    "Column \"{column}\" holds linked records. Each name is looked up in the {} of the linked table, and a name that matches none, or more than one, is an error.",
                    f.target_display_field.as_deref().map(|d| format!("field \"{d}\"")).unwrap_or_else(|| "primary field".to_string())
                ),
                off,
            )),
            FieldType::Checkbox | FieldType::Boolean => Some(decision(
                "checkbox",
                column,
                &f.name,
                format!("Column \"{column}\" is a checkbox. \"checked\", \"yes\", \"true\" and \"1\" are yes. A blank cell, \"unchecked\", \"no\", \"false\" and \"0\" are no."),
                off,
            )),
            FieldType::Currency => Some(decision(
                "currency",
                column,
                &f.name,
                format!("Column \"{column}\" is money. Currency signs and thousands marks are removed, and an amount in parentheses is negative."),
                off,
            )),
            FieldType::Attachment { .. } => Some(decision(
                "attachment",
                column,
                &f.name,
                format!("Column \"{column}\" holds files. Their addresses are listed in a report and are not fetched, so no file is attached. Attach them by hand."),
                off,
            )),
            _ => None,
        };
        decisions.extend(d);
    }
    Plan { decisions }
}

impl Plan {
    /// The columns the plan matched to fields by name, for the columns the mapping left alone.
    pub fn name_matches(&self) -> BTreeMap<String, String> {
        self.decisions.iter().filter(|d| d.kind == "name" && d.enabled).map(|d| (d.column.clone(), d.field.clone())).collect()
    }

    fn on(&self, kind: &str, column: &str) -> bool {
        self.decisions.iter().any(|d| d.kind == kind && d.column == column && d.enabled)
    }

    pub fn to_json(&self) -> Value {
        Value::Array(self.decisions.iter().map(Decision::to_json).collect())
    }

    /// What a cell becomes under the decisions that are on. `None` leaves the cell to the ordinary import.
    pub fn convert(&self, column: &str, field: &FieldSpec, cell: &str) -> Option<Converted> {
        let cell = cell.trim();
        match &field.field_type {
            FieldType::MultiSelect if self.on("multi", column) => {
                if cell.is_empty() {
                    return None;
                }
                let mut picked = Vec::new();
                for part in cell.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    if !field.select_options.iter().any(|o| o.label == part) {
                        return Some(Converted::Error(format!("\"{part}\" is not an option of \"{}\"", field.label)));
                    }
                    if !picked.iter().any(|p: &Value| p == part) {
                        picked.push(Value::String(part.to_string()));
                    }
                }
                Some(Converted::Value(Value::Array(picked)))
            }
            FieldType::Checkbox | FieldType::Boolean if self.on("checkbox", column) => Some(match cell.to_lowercase().as_str() {
                "checked" | "true" | "yes" | "y" | "1" | "x" | "✓" | "✔" => Converted::Value(Value::Bool(true)),
                "" | "unchecked" | "false" | "no" | "n" | "0" => Converted::Value(Value::Bool(false)),
                _ => Converted::Error(format!("\"{cell}\" is not a checkbox value. Use checked or blank")),
            }),
            FieldType::Currency if self.on("currency", column) => {
                if cell.is_empty() {
                    return None;
                }
                Some(match parse_currency(cell) {
                    Some(n) => Converted::Value(json!(n)),
                    None => Converted::Error(format!("\"{cell}\" is not an amount of money")),
                })
            }
            FieldType::Attachment { .. } if self.on("attachment", column) => Some(Converted::Skip),
            _ => None,
        }
    }

    /// The file addresses an attachment cell holds, when the attachment decision is on.
    pub fn attachments(&self, column: &str, cell: &str) -> Vec<(String, String)> {
        if self.on("attachment", column) {
            addresses(cell)
        } else {
            Vec::new()
        }
    }
}

/// `$1,200.50` is 1200.5. `($5.25)` and `-$5.25` are -5.25. Anything else with a letter in it is not an amount.
fn parse_currency(cell: &str) -> Option<f64> {
    let mut text = cell.trim().to_string();
    let mut negative = false;
    if text.starts_with('(') && text.ends_with(')') {
        negative = true;
        text = text[1..text.len() - 1].to_string();
    }
    let cleaned: String = text.chars().filter(|c| !matches!(c, '$' | '€' | '£' | '¥' | ',' | ' ' | '\u{a0}')).collect();
    let cleaned = match cleaned.strip_prefix('-') {
        Some(rest) => {
            negative = !negative;
            rest.to_string()
        }
        None => cleaned,
    };
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit() || c == '.') || cleaned.matches('.').count() > 1 {
        return None;
    }
    let n: f64 = cleaned.parse().ok()?;
    Some(if negative { -n } else { n })
}

/// `plan.pdf (https://host/a), logo.png (https://host/b)` as pairs of name and address. A bare address is its own name.
fn addresses(cell: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = cell.trim();
    while !rest.is_empty() {
        rest = rest.trim_start_matches([',', ' ', '\n']);
        if let (Some(open), true) = (rest.find(" ("), !rest.starts_with("http")) {
            let name = rest[..open].trim().to_string();
            let after = &rest[open + 2..];
            let Some(close) = after.find(')') else { break };
            out.push((name, after[..close].trim().to_string()));
            rest = &after[close + 1..];
        } else {
            let end = rest.find([',', ' ', '\n']).unwrap_or(rest.len());
            let url = rest[..end].to_string();
            if url.starts_with("http") {
                out.push((url.rsplit('/').next().unwrap_or(&url).to_string(), url));
            }
            rest = &rest[end..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_read_the_way_a_person_reads_it() {
        assert_eq!(parse_currency("$1,200.50"), Some(1200.5));
        assert_eq!(parse_currency("($5.25)"), Some(-5.25));
        assert_eq!(parse_currency("-$5.25"), Some(-5.25));
        assert_eq!(parse_currency("€ 30"), Some(30.0));
        assert_eq!(parse_currency("12abc"), None);
        assert_eq!(parse_currency("1.2.3"), None);
        assert_eq!(parse_currency("$"), None);
    }

    #[test]
    fn attachment_cells_give_names_and_addresses() {
        let got = addresses("plan.pdf (http://h/a/plan.pdf), logo.png (http://h/b/logo.png)");
        assert_eq!(got, vec![("plan.pdf".to_string(), "http://h/a/plan.pdf".to_string()), ("logo.png".to_string(), "http://h/b/logo.png".to_string())]);
        assert_eq!(addresses("http://h/x/file.csv"), vec![("file.csv".to_string(), "http://h/x/file.csv".to_string())]);
        assert!(addresses("").is_empty());
    }
}
