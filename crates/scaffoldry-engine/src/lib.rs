//! Scaffoldry Dynamic Engine and Manifest Renderer (Layer 3)

pub mod automation;
pub use automation::*;

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_policy::{PolicyDecision, ScaffoldryPolicyEngine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Manifest validation error: {0}")]
    ValidationError(String),

    #[error("App not found: {0}")]
    NotFound(String),

    #[error("Authorization denied by policy: {0}")]
    AccessDenied(String),

    #[error("Policy evaluation error: {0}")]
    PolicyError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewType {
    Table,
    Form,
    Dashboard,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Text,
    Number,
    Date,
    Select,
    Boolean,
    Relation,
    // Rich Field Types
    Checkbox,
    MultiSelect,
    Currency,
    Percent,
    Rating,
    Email,
    Phone,
    Url,
    Autonumber,
    CreatedTime,
    LastModifiedTime,
    // Computed & Relational Fields
    Lookup,
    Count,
    Rollup,
    Formula,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub ferpa_sensitive: bool,
    #[serde(default)]
    pub linked_dataset_id: Option<String>,
    #[serde(default)]
    pub linked_field: Option<String>,
    #[serde(default)]
    pub target_table_id: Option<String>,
    #[serde(default)]
    pub target_display_field: Option<String>,
    #[serde(default)]
    pub formula_expression: Option<String>,
    #[serde(default)]
    pub rollup_function: Option<String>,
    #[serde(default)]
    pub select_options: Vec<String>,
    #[serde(default)]
    pub currency_symbol: Option<String>,
    #[serde(default)]
    pub precision: Option<u8>,
}

impl FieldSpec {
    pub fn simple(
        name: impl Into<String>,
        label: impl Into<String>,
        field_type: FieldType,
        required: bool,
        ferpa_sensitive: bool,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            field_type,
            required,
            ferpa_sensitive,
            linked_dataset_id: None,
            linked_field: None,
            target_table_id: None,
            target_display_field: None,
            formula_expression: None,
            rollup_function: None,
            select_options: Vec::new(),
            currency_symbol: None,
            precision: None,
        }
    }
}

pub fn evaluate_formula(expression: &str, record: &Value) -> Value {
    let expr = expression.trim();
    if expr.is_empty() {
        return Value::Null;
    }

    // Direct single reference e.g. "{budget}"
    if expr.starts_with('{') && expr.ends_with('}') && !expr[1..expr.len() - 1].contains('}') {
        let field = &expr[1..expr.len() - 1];
        return record.get(field).cloned().unwrap_or(Value::Null);
    }

    // Multiplication: {budget} * 0.15 or {a} * {b}
    if let Some((left, right)) = expr.split_once('*') {
        let v1 = parse_operand(left.trim(), record);
        let v2 = parse_operand(right.trim(), record);
        let n1 = v1.as_f64().or_else(|| v1.as_i64().map(|i| i as f64));
        let n2 = v2.as_f64().or_else(|| v2.as_i64().map(|i| i as f64));
        if let (Some(n1), Some(n2)) = (n1, n2) {
            return Value::from(n1 * n2);
        }
    }

    // Division: {budget} / 12
    if let Some((left, right)) = expr.split_once('/') {
        let v1 = parse_operand(left.trim(), record);
        let v2 = parse_operand(right.trim(), record);
        let n1 = v1.as_f64().or_else(|| v1.as_i64().map(|i| i as f64));
        let n2 = v2.as_f64().or_else(|| v2.as_i64().map(|i| i as f64));
        if let (Some(n1), Some(n2)) = (n1, n2) {
            if n2 != 0.0 {
                return Value::from(n1 / n2);
            }
        }
    }

    // Addition or string concatenation: {first} + " " + {last} or {a} + {b}
    if expr.contains('+') {
        let parts: Vec<&str> = expr.split('+').map(|s| s.trim()).collect();
        let mut all_numbers = true;
        let mut sum = 0.0;
        let mut concat_str = String::new();

        for part in &parts {
            let op_val = parse_operand(part, record);
            if let Some(n) = op_val.as_f64().or_else(|| op_val.as_i64().map(|i| i as f64)) {
                sum += n;
            } else {
                all_numbers = false;
            }

            if let Some(s) = op_val.as_str() {
                concat_str.push_str(s);
            } else if let Some(n) = op_val.as_f64() {
                concat_str.push_str(&n.to_string());
            } else if let Some(i) = op_val.as_i64() {
                concat_str.push_str(&i.to_string());
            }
        }

        if all_numbers && parts.len() > 1 {
            return Value::from(sum);
        } else {
            return Value::from(concat_str);
        }
    }

    // Subtraction: {budget} - {spent}
    if let Some((left, right)) = expr.split_once('-') {
        let v1 = parse_operand(left.trim(), record);
        let v2 = parse_operand(right.trim(), record);
        let n1 = v1.as_f64().or_else(|| v1.as_i64().map(|i| i as f64));
        let n2 = v2.as_f64().or_else(|| v2.as_i64().map(|i| i as f64));
        if let (Some(n1), Some(n2)) = (n1, n2) {
            return Value::from(n1 - n2);
        }
    }

    parse_operand(expr, record)
}

fn parse_operand(op: &str, record: &Value) -> Value {
    let op = op.trim();
    if op.starts_with('{') && op.ends_with('}') {
        let field = &op[1..op.len() - 1];
        record.get(field).cloned().unwrap_or(Value::Null)
    } else if (op.starts_with('"') && op.ends_with('"')) || (op.starts_with('\'') && op.ends_with('\'')) {
        Value::from(&op[1..op.len() - 1])
    } else if let Ok(n) = op.parse::<f64>() {
        Value::from(n)
    } else {
        record.get(op).cloned().unwrap_or(Value::Null)
    }
}

pub fn compute_field_value(
    field: &FieldSpec,
    record: &Value,
    linked_records: &[Value],
) -> Value {
    match field.field_type {
        FieldType::Formula => {
            if let Some(expr) = &field.formula_expression {
                evaluate_formula(expr, record)
            } else {
                Value::Null
            }
        }
        FieldType::Lookup => {
            if let Some(target_col) = &field.target_display_field {
                let values: Vec<Value> = linked_records
                    .iter()
                    .filter_map(|r| r.get(target_col).cloned())
                    .collect();
                if values.len() == 1 {
                    values[0].clone()
                } else {
                    Value::Array(values)
                }
            } else {
                Value::Null
            }
        }
        FieldType::Count => Value::from(linked_records.len()),
        FieldType::Rollup => {
            let target_col = field.target_display_field.as_deref().unwrap_or("amount");
            let numbers: Vec<f64> = linked_records
                .iter()
                .filter_map(|r| {
                    r.get(target_col)
                        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                })
                .collect();
            let func = field.rollup_function.as_deref().unwrap_or("sum");
            match func.to_lowercase().as_str() {
                "sum" => Value::from(numbers.iter().sum::<f64>()),
                "avg" => {
                    if numbers.is_empty() {
                        Value::from(0.0)
                    } else {
                        Value::from(numbers.iter().sum::<f64>() / numbers.len() as f64)
                    }
                }
                "min" => {
                    let min = numbers.iter().cloned().fold(f64::INFINITY, f64::min);
                    if min.is_infinite() {
                        Value::Null
                    } else {
                        Value::from(min)
                    }
                }
                "max" => {
                    let max = numbers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    if max.is_infinite() {
                        Value::Null
                    } else {
                        Value::from(max)
                    }
                }
                "count" => Value::from(numbers.len()),
                _ => Value::Null,
            }
        }
        _ => record.get(&field.name).cloned().unwrap_or(Value::Null),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRelationship {
    pub id: String,
    pub name: String,
    pub source_table_id: String,
    pub target_table_id: String,
    pub source_field: String,
    pub target_field: String,
    pub relationship_type: String,
    pub display_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppTable {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub primary_field: Option<String>,
    #[serde(default)]
    pub sample_records: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppView {
    pub id: String,
    pub title: String,
    pub view_type: ViewType,
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub organization_code: String,
    pub department: String,
    #[serde(default)]
    pub herm_capability_id: Option<String>,
    #[serde(default)]
    pub custom_domain: Option<String>,
    #[serde(default)]
    pub custom_domain_verified: bool,
    #[serde(default)]
    pub tables: Vec<AppTable>,
    #[serde(default)]
    pub relationships: Vec<TableRelationship>,
    pub views: Vec<AppView>,
    #[serde(default)]
    pub ceds_mappings: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmittedRecord {
    pub id: Uuid,
    pub app_slug: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
}

pub trait HostRouter {
    fn resolve_by_host(&self, host: &str) -> Option<&AppManifest>;
    fn resolve_by_slug(&self, slug: &str) -> Option<&AppManifest>;
}

pub struct ManifestEngine {
    manifests_by_slug: HashMap<String, AppManifest>,
    manifests_by_domain: HashMap<String, String>,
    policy_engine: ScaffoldryPolicyEngine,
}

impl ManifestEngine {
    pub fn new() -> Result<Self, EngineError> {
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| EngineError::PolicyError(e.to_string()))?;
        Ok(Self {
            manifests_by_slug: HashMap::new(),
            manifests_by_domain: HashMap::new(),
            policy_engine,
        })
    }

    pub fn register_manifest(&mut self, manifest: AppManifest) -> Result<(), EngineError> {
        if manifest.slug.trim().is_empty() {
            return Err(EngineError::ValidationError("App slug cannot be empty".to_string()));
        }

        if let Some(domain) = &manifest.custom_domain {
            if manifest.custom_domain_verified {
                self.manifests_by_domain.insert(domain.clone(), manifest.slug.clone());
            }
        }

        self.manifests_by_slug.insert(manifest.slug.clone(), manifest);
        Ok(())
    }

    pub fn submit_record(
        &self,
        caller: &EduPersonIdentity,
        app_slug: &str,
        payload: &Value,
    ) -> Result<SubmittedRecord, EngineError> {
        let manifest = self
            .manifests_by_slug
            .get(app_slug)
            .ok_or_else(|| EngineError::NotFound(format!("App '{}' not found", app_slug)))?;

        let auth_result = self
            .policy_engine
            .authorize_record_action(
                caller,
                "write",
                app_slug,
                &manifest.department,
                false,
            )
            .map_err(|e| EngineError::PolicyError(e.to_string()))?;

        if auth_result.decision != PolicyDecision::Allow {
            return Err(EngineError::AccessDenied(format!(
                "Caller '{}' denied write permission on app '{}'",
                caller.eppn, app_slug
            )));
        }

        let mut is_ferpa_sensitive = false;
        let mut applied_ceds = HashMap::new();

        for view in &manifest.views {
            for field in &view.fields {
                let val = payload.get(&field.name);
                if field.required && (val.is_none() || val.unwrap().is_null()) {
                    return Err(EngineError::ValidationError(format!(
                        "Required field '{}' is missing in payload",
                        field.name
                    )));
                }

                if val.is_some() && field.ferpa_sensitive {
                    is_ferpa_sensitive = true;
                }

                if let Some(ceds_code) = manifest.ceds_mappings.get(&field.name) {
                    applied_ceds.insert(field.name.clone(), ceds_code.clone());
                }
            }
        }

        Ok(SubmittedRecord {
            id: Uuid::new_v4(),
            app_slug: app_slug.to_string(),
            data: payload.clone(),
            ceds_mapping: serde_json::to_value(applied_ceds).unwrap_or_default(),
            is_ferpa_sensitive,
        })
    }
}

impl HostRouter for ManifestEngine {
    fn resolve_by_host(&self, host: &str) -> Option<&AppManifest> {
        let clean_host = host.split(':').next().unwrap_or(host);
        let slug = self.manifests_by_domain.get(clean_host)?;
        self.manifests_by_slug.get(slug)
    }

    fn resolve_by_slug(&self, slug: &str) -> Option<&AppManifest> {
        self.manifests_by_slug.get(slug)
    }
}
