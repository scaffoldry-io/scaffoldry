//! Scaffoldry Dynamic Engine and Manifest Renderer (Layer 3)

pub mod access;
pub mod duration;
pub use duration::*;
pub mod filter_tree;
pub use filter_tree::{check_filter, clauses_of, instant_of, is_flat, list_of, range_of, relative_range, resolve_relative_dates, valid_zone, MAX_FILTER_CLAUSES, MAX_FILTER_DEPTH};
pub mod automation;
pub mod detect;
pub mod graph;
pub mod index;
pub mod manifest;
pub mod recalc;
pub mod sensitivity;
pub mod formula;
pub mod infer;
pub use formula::*;
pub use scaffoldry_core::access::AccessManifest;
pub use manifest::detect_access_problems;

pub use automation::*;
pub use index::{index_column, index_rows, index_value, indexed_fields, parse_instant, IndexColumn, IndexCols, MAX_INDEXED_FIELDS};
pub use manifest::{is_numeric, manifest_warnings, validate_manifest, ROLLUP_FUNCTIONS};

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_policy::ScaffoldryPolicyEngine;
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
    Grid,
    Kanban,
    Calendar,
    Gallery,
    Form,
    Dashboard,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    GreaterThan,
    LessThan,
    IsEmpty,
    IsNotEmpty,
    /// The cell is one of a list: a JSON array or a comma list.
    In,
    /// The cell is from `low` to `high`, both inside: `10..20`, `2026-01-01..2026-01-31`, or a relative date
    /// such as `last_7_days` on a date field.
    Between,
    /// A date before the value.
    Before,
    /// A date after the value.
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterConjunction {
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterClause {
    pub id: String,
    pub field_name: String,
    pub operator: FilterOperator,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundFilter {
    pub conjunction: FilterConjunction,
    pub clauses: Vec<FilterClause>,
    /// Nested groups, joined with the clauses by the same conjunction. A group nests two levels below the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CompoundFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortRule {
    pub id: String,
    pub field_name: String,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowDensity {
    Compact,
    Medium,
    Tall,
    ExtraTall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// A list of record ids, or one id, of records in another table. `record_links` indexes it.
    Link,
    /// Collaborator stores addresses.
    Collaborator {
        allow_multiple: bool,
    },
    /// Set at creation to the actor and never changes.
    CreatedBy,
    /// Set on every user write.
    LastModifiedBy,
    /// Plain text up to 100,000 characters, rendered with safe Markdown subset.
    LongText,
    /// Duration stored as whole seconds in a number.
    Duration {
        format: DurationFormat,
    },
    /// DateTime stored as a UTC instant.
    DateTime,
    /// Files kept outside the record. The value is a list of `{ref_id, name}`.
    Attachment {
        allow_multiple: bool,
    },
    /// A unit of the hierarchy the field's `hierarchy` names. The value is a unit id, validated against that
    /// hierarchy and indexed as text. A table may use it to place its rows (`AppTable::row_place`).
    Unit,
}

impl FieldType {
    /// True when the value follows from other values, so nobody writes it.
    pub fn is_computed(self) -> bool {
        use FieldType::*;
        matches!(self, Lookup | Count | Rollup | Formula | Autonumber | CreatedTime | LastModifiedTime | CreatedBy | LastModifiedBy)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldConstraints {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOn {
    Create,
    Change { fields: Vec<String> },
}

impl serde::Serialize for FieldType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            FieldType::Text => serializer.serialize_str("Text"),
            FieldType::Number => serializer.serialize_str("Number"),
            FieldType::Date => serializer.serialize_str("Date"),
            FieldType::Select => serializer.serialize_str("Select"),
            FieldType::Boolean => serializer.serialize_str("Boolean"),
            FieldType::Relation => serializer.serialize_str("Relation"),
            FieldType::Checkbox => serializer.serialize_str("Checkbox"),
            FieldType::MultiSelect => serializer.serialize_str("MultiSelect"),
            FieldType::Currency => serializer.serialize_str("Currency"),
            FieldType::Percent => serializer.serialize_str("Percent"),
            FieldType::Rating => serializer.serialize_str("Rating"),
            FieldType::Email => serializer.serialize_str("Email"),
            FieldType::Phone => serializer.serialize_str("Phone"),
            FieldType::Url => serializer.serialize_str("Url"),
            FieldType::Autonumber => serializer.serialize_str("Autonumber"),
            FieldType::CreatedTime => serializer.serialize_str("CreatedTime"),
            FieldType::LastModifiedTime => serializer.serialize_str("LastModifiedTime"),
            FieldType::Lookup => serializer.serialize_str("Lookup"),
            FieldType::Count => serializer.serialize_str("Count"),
            FieldType::Rollup => serializer.serialize_str("Rollup"),
            FieldType::Formula => serializer.serialize_str("Formula"),
            FieldType::Link => serializer.serialize_str("Link"),
            FieldType::Collaborator { .. } => serializer.serialize_str("Collaborator"),
            FieldType::CreatedBy => serializer.serialize_str("CreatedBy"),
            FieldType::LastModifiedBy => serializer.serialize_str("LastModifiedBy"),
            FieldType::LongText => serializer.serialize_str("LongText"),
            FieldType::Duration { .. } => serializer.serialize_str("Duration"),
            FieldType::DateTime => serializer.serialize_str("DateTime"),
            FieldType::Attachment { .. } => serializer.serialize_str("Attachment"),
            FieldType::Unit => serializer.serialize_str("Unit"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for FieldType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        match val {
            serde_json::Value::String(s) => match s.as_str() {
                "Text" => Ok(FieldType::Text),
                "Number" => Ok(FieldType::Number),
                "Date" => Ok(FieldType::Date),
                "Select" => Ok(FieldType::Select),
                "Boolean" => Ok(FieldType::Boolean),
                "Relation" => Ok(FieldType::Relation),
                "Checkbox" => Ok(FieldType::Checkbox),
                "MultiSelect" => Ok(FieldType::MultiSelect),
                "Currency" => Ok(FieldType::Currency),
                "Percent" => Ok(FieldType::Percent),
                "Rating" => Ok(FieldType::Rating),
                "Email" => Ok(FieldType::Email),
                "Phone" => Ok(FieldType::Phone),
                "Url" => Ok(FieldType::Url),
                "Autonumber" => Ok(FieldType::Autonumber),
                "CreatedTime" => Ok(FieldType::CreatedTime),
                "LastModifiedTime" => Ok(FieldType::LastModifiedTime),
                "Lookup" => Ok(FieldType::Lookup),
                "Count" => Ok(FieldType::Count),
                "Rollup" => Ok(FieldType::Rollup),
                "Formula" => Ok(FieldType::Formula),
                "Link" => Ok(FieldType::Link),
                "Collaborator" => Ok(FieldType::Collaborator { allow_multiple: false }),
                "CreatedBy" => Ok(FieldType::CreatedBy),
                "LastModifiedBy" => Ok(FieldType::LastModifiedBy),
                "LongText" => Ok(FieldType::LongText),
                "Duration" => Ok(FieldType::Duration { format: DurationFormat::HMm }),
                "DateTime" => Ok(FieldType::DateTime),
                "Attachment" => Ok(FieldType::Attachment { allow_multiple: true }),
                "Unit" => Ok(FieldType::Unit),
                other => Err(serde::de::Error::custom(format!("unknown field type: {other}"))),
            },
            serde_json::Value::Object(map) => {
                if let Some(dur) = map.get("Duration") {
                    let format = match dur {
                        serde_json::Value::String(s) => DurationFormat::parse(s).unwrap_or_default(),
                        serde_json::Value::Object(sub) => sub.get("format").and_then(serde_json::Value::as_str).and_then(DurationFormat::parse).unwrap_or_default(),
                        _ => DurationFormat::HMm,
                    };
                    Ok(FieldType::Duration { format })
                } else if map.contains_key("DateTime") {
                    Ok(FieldType::DateTime)
                } else if let Some(collab) = map.get("Collaborator") {
                    let allow_multiple = match collab {
                        serde_json::Value::Bool(b) => *b,
                        serde_json::Value::Object(sub) => sub.get("allow_multiple").and_then(serde_json::Value::as_bool).unwrap_or(false),
                        _ => false,
                    };
                    Ok(FieldType::Collaborator { allow_multiple })
                } else if let Some(att) = map.get("Attachment") {
                    let allow_multiple = match att {
                        serde_json::Value::Bool(b) => *b,
                        serde_json::Value::Object(sub) => sub.get("allow_multiple").and_then(serde_json::Value::as_bool).unwrap_or(true),
                        _ => true,
                    };
                    Ok(FieldType::Attachment { allow_multiple })
                } else if map.contains_key("CreatedBy") {
                    Ok(FieldType::CreatedBy)
                } else if map.contains_key("LastModifiedBy") {
                    Ok(FieldType::LastModifiedBy)
                } else {
                    Err(serde::de::Error::custom("unknown field type object"))
                }
            }
            _ => Err(serde::de::Error::custom("field type must be string or object")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectOptionColor {
    #[default]
    Neutral,
    Ok,
    Warn,
    Bad,
    Info,
    Accent1,
    Accent2,
    Accent3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub color: SelectOptionColor,
    #[serde(default)]
    pub order: i32,
}

pub fn deserialize_select_options<'de, D>(deserializer: D) -> Result<Vec<SelectOption>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Array(items) => {
            let mut result = Vec::new();
            for (idx, item) in items.into_iter().enumerate() {
                if let serde_json::Value::String(s) = item {
                    let clean: String = s
                        .chars()
                        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
                        .collect();
                    let trimmed = clean.trim_matches('_');
                    let slug = if trimmed.is_empty() { format!("opt_{idx}") } else { format!("opt_{trimmed}") };
                    result.push(SelectOption {
                        id: slug,
                        label: s,
                        color: SelectOptionColor::Neutral,
                        order: idx as i32,
                    });
                } else {
                    let opt: SelectOption = serde_json::from_value(item).map_err(serde::de::Error::custom)?;
                    result.push(opt);
                }
            }
            Ok(result)
        }
        _ => Ok(Vec::new()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KanbanStage {
    pub id: String,
    pub label: String,
    pub color: SelectOptionColor,
    pub records: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KanbanViewData {
    pub column_field: String,
    pub stages: Vec<KanbanStage>,
}

pub fn group_by_select(
    manifest: &AppManifest,
    view: &AppView,
    records: &[serde_json::Value],
) -> Option<KanbanViewData> {
    let col_name = view.kanban_column_field.as_deref().or(view.group_by_field.as_deref())?;
    let table_id = view.table_id.as_deref();
    let table = manifest.tables.iter().find(|t| table_id.is_none_or(|tid| t.id == tid || t.slug == tid))?;
    let field = table.fields.iter().find(|f| f.name == col_name)?;
    if !matches!(field.field_type, FieldType::Select | FieldType::MultiSelect) {
        return None;
    }

    let mut stages: Vec<KanbanStage> = field
        .select_options
        .iter()
        .map(|opt| KanbanStage {
            id: opt.id.clone(),
            label: opt.label.clone(),
            color: opt.color,
            records: Vec::new(),
        })
        .collect();

    for r in records {
        let val = r.get(col_name).or_else(|| r.get("data").and_then(|d| d.get(col_name)));
        if let Some(v) = val {
            if let Some(id_str) = v.as_str() {
                if let Some(stage) = stages.iter_mut().find(|s| s.id == id_str) {
                    stage.records.push(r.clone());
                }
            } else if let Some(arr) = v.as_array() {
                for item in arr {
                    if let Some(id_str) = item.as_str() {
                        if let Some(stage) = stages.iter_mut().find(|s| s.id == id_str) {
                            stage.records.push(r.clone());
                        }
                    }
                }
            }
        }
    }

    Some(KanbanViewData {
        column_field: col_name.to_string(),
        stages,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
    pub ferpa_sensitive: bool,
    /// The sensitivity categories this field carries, by id. The legacy flag above is the
    /// category named by `LEGACY_FLAG_CATEGORY`.
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub linked_dataset_id: Option<String>,
    #[serde(default)]
    pub linked_field: Option<String>,
    #[serde(default)]
    pub target_table_id: Option<String>,
    #[serde(default)]
    pub target_display_field: Option<String>,
    #[serde(default)]
    pub display_label_override: Option<String>,
    #[serde(default)]
    pub cardinality: Option<String>,
    #[serde(default)]
    pub allow_multiple: Option<bool>,
    #[serde(default)]
    pub link_filter: Option<serde_json::Value>,
    #[serde(default)]
    pub formula_expression: Option<String>,
    #[serde(default)]
    pub rollup_function: Option<String>,
    #[serde(default, deserialize_with = "deserialize_select_options")]
    pub select_options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_format: Option<String>,
    #[serde(default)]
    pub currency_symbol: Option<String>,
    #[serde(default)]
    pub precision: Option<u8>,
    /// Sorting and filtering on this field use the pivot index. A table indexes at most 20 fields.
    #[serde(default)]
    pub indexed: bool,
    /// A `Link` field: the table it points at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target_table: Option<String>,
    /// A `Link` field into a dataset app. Dataset apps are read-only (`connections.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target_app: Option<String>,
    #[serde(default, skip_serializing_if = "LinkCardinality::is_default")]
    pub link_cardinality: LinkCardinality,
    /// The field on the target table that shows the same links from the other side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_related_field: Option<String>,
    #[serde(default, skip_serializing_if = "LinkOnDelete::is_default")]
    pub link_on_delete: LinkOnDelete,
    /// A `Lookup`, `Rollup`, or `Count`: the `Link` field of this table it goes through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computed_via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_on: Option<SetOn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<FieldConstraints>,
    #[serde(default)]
    pub notify_on_assign: bool,
    /// The field's own sensitivity, a level on the ladder. It may be higher than its table's and never lower. A field
    /// keeps its `categories` as well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
    /// A `Unit` field: the key of the hierarchy whose units it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<String>,
}

/// How many records a link field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LinkCardinality {
    One,
    #[default]
    Many,
}

impl LinkCardinality {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What deleting a linked record does. `Unlink` removes the link and remembers it. `Restrict`
/// refuses the delete while links exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LinkOnDelete {
    #[default]
    Unlink,
    Restrict,
}

impl LinkOnDelete {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// An overlay on one field that can only add protection. It is how a compliance officer raises a
/// field's sensitivity at once, without a structural change to the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataLabel {
    pub ferpa_sensitive: bool,
    pub note: String,
    pub set_by: String,
}

/// Labels by `(app_slug, table_id, field)`. The table id is `""` for an untabled view. The set
/// holds exceptions only, so it stays small. It is empty until the label table exists.
pub type LabelSet = HashMap<(String, String, String), DataLabel>;

/// The category a legacy `ferpa_sensitive: true` stands for. This is the one place the code
/// names it. Everything else reads categories from settings.
pub const LEGACY_FLAG_CATEGORY: &str = "ferpa";

/// What a field is sensitive for. The legacy flag or a raising label puts the field in the legacy
/// category and protects it. Otherwise a field is protected when any category it carries is marked
/// protected in settings. A label of `false` never lowers anything. Every read of a field's
/// sensitivity calls this.
pub fn effective_ferpa_sensitive(
    field: &FieldSpec,
    label: Option<&DataLabel>,
    categories: &[sensitivity::Category],
) -> sensitivity::Sensitivity {
    let legacy = field.ferpa_sensitive || label.is_some_and(|l| l.ferpa_sensitive);
    let mut carried: std::collections::BTreeSet<String> = field.categories.iter().cloned().collect();
    if legacy {
        carried.insert(LEGACY_FLAG_CATEGORY.to_string());
    }
    let protected = legacy || carried.iter().any(|id| categories.iter().any(|c| &c.id == id && c.protected));
    sensitivity::Sensitivity { protected, categories: carried }
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
            categories: Vec::new(),
            linked_dataset_id: None,
            linked_field: None,
            target_table_id: None,
            target_display_field: None,
            display_label_override: None,
            cardinality: None,
            allow_multiple: None,
            link_filter: None,
            formula_expression: None,
            rollup_function: None,
            select_options: Vec::new(),
            duration_format: if let FieldType::Duration { format } = field_type { Some(format.as_str().to_string()) } else { None },
            currency_symbol: None,
            precision: None,
            indexed: false,
            link_target_table: None,
            link_target_app: None,
            link_cardinality: LinkCardinality::Many,
            link_related_field: None,
            link_on_delete: LinkOnDelete::Unlink,
            computed_via: None,
            default: None,
            set_on: None,
            constraints: None,
            notify_on_assign: false,
            sensitivity: None,
            hierarchy: None,
        }
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
    /// Where the table sits, narrower than its app's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<scaffoldry_core::coordinates::Coordinates>,
    /// For each hierarchy, the field that places a row in it: a `Unit` field of that hierarchy, or a `Collaborator`
    /// field for a hierarchy of people. Changing it is a manifest change.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub row_place: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppView {
    pub id: String,
    #[serde(default)]
    pub table_id: Option<String>,
    pub title: String,
    pub view_type: ViewType,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub filters: Option<CompoundFilter>,
    #[serde(default)]
    pub sort_rules: Vec<SortRule>,
    #[serde(default)]
    pub group_by_field: Option<String>,
    #[serde(default)]
    pub row_density: Option<RowDensity>,
    #[serde(default)]
    pub kanban_column_field: Option<String>,
    #[serde(default)]
    pub calendar_date_field: Option<String>,
    #[serde(default)]
    pub column_order: Vec<String>,
    #[serde(default)]
    pub column_widths: Vec<(String, u32)>,
    #[serde(default)]
    pub hidden_columns: Vec<String>,
    #[serde(default)]
    pub frozen_through: Option<String>,
    /// Each field's footer choice: `(field, function)`. The functions are in `FOOTER_FUNCTIONS`.
    #[serde(default)]
    pub column_summary: Vec<(String, String)>,
    /// Up to three fields to group by, outermost first. Computed by the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_by: Vec<String>,
    /// Up to ten rules that tone a row or a field when a filter matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub color_rules: Vec<ColorRule>,
    /// A summary view's measures: one column for each, computed over each group of `group_by`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aggregations: Vec<Aggregation>,
}

/// What a summary view computes for each group. `count` needs no field. Every other function needs one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(rename = "fn")]
    pub function: String,
}

/// The functions a summary view may use. They are the functions the aggregate route can group.
pub const SUMMARY_FUNCTIONS: [&str; 6] = ["count", "sum", "avg", "min", "max", "count_distinct"];
pub const MAX_SUMMARY_GROUPS: usize = 2;
pub const MAX_AGGREGATIONS: usize = 8;

impl Aggregation {
    /// The column heading: `count`, `sum of amount`, `distinct status`.
    pub fn label(&self) -> String {
        let field = self.field.as_deref().unwrap_or("");
        match self.function.as_str() {
            "count" => "count".to_string(),
            "count_distinct" => format!("distinct {field}"),
            "avg" => format!("average of {field}"),
            "min" => format!("smallest {field}"),
            "max" => format!("largest {field}"),
            other => format!("{other} of {field}"),
        }
    }
}

/// The footer choices a field may have.
pub const FOOTER_FUNCTIONS: [&str; 8] = ["count", "filled", "empty", "unique", "sum", "avg", "min", "max"];
pub const MAX_GROUP_LEVELS: usize = 3;
pub const MAX_COLOR_RULES: usize = 10;

/// The fixed set of tones. A tone shows as a color, an icon and the rule's label. Never as color alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorTarget {
    Row,
    Field,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorRule {
    pub id: String,
    pub label: String,
    pub when: CompoundFilter,
    pub tone: Tone,
    pub target: ColorTarget,
    /// The field a `field` rule colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

/// A rule that matched a row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppliedTone {
    pub id: String,
    pub label: String,
    pub tone: Tone,
    pub target: ColorTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

/// The rules that match a record, in the order the view lists them.
pub fn tones_of(record: &Value, rules: &[ColorRule]) -> Vec<AppliedTone> {
    rules
        .iter()
        .filter(|r| matches_filter(record, &r.when))
        .map(|r| AppliedTone { id: r.id.clone(), label: r.label.clone(), tone: r.tone, target: r.target, field: r.field.clone() })
        .collect()
}

impl AppView {
    pub fn table(
        id: impl Into<String>,
        title: impl Into<String>,
        view_type: ViewType,
        fields: Vec<FieldSpec>,
    ) -> Self {
        Self {
            id: id.into(),
            table_id: None,
            title: title.into(),
            view_type,
            fields,
            filters: None,
            sort_rules: Vec::new(),
            group_by_field: None,
            row_density: None,
            kanban_column_field: None,
            calendar_date_field: None,
            column_order: Vec::new(),
            column_widths: Vec::new(),
            hidden_columns: Vec::new(),
            frozen_through: None,
            column_summary: Vec::new(),
            group_by: Vec::new(),
            color_rules: Vec::new(),
            aggregations: Vec::new(),
        }
    }
}

pub fn matches_filter(record: &Value, filter: &CompoundFilter) -> bool {
    if filter.clauses.is_empty() && filter.groups.is_empty() {
        return true;
    }
    let clauses = filter.clauses.iter().map(|c| matches_clause(record, c));
    let groups = filter.groups.iter().map(|g| matches_filter(record, g));
    match filter.conjunction {
        FilterConjunction::And => clauses.chain(groups).all(|m| m),
        FilterConjunction::Or => clauses.chain(groups).any(|m| m),
    }
}

fn cell_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

pub fn matches_clause(record: &Value, clause: &FilterClause) -> bool {
    let cell_val = record.get(&clause.field_name);
    if clause.operator != FilterOperator::IsEmpty
        && clause.operator != FilterOperator::IsNotEmpty
        && clause.value.trim().is_empty()
    {
        return true;
    }
    match clause.operator {
        FilterOperator::IsEmpty => {
            cell_val.is_none()
                || cell_val == Some(&Value::Null)
                || cell_val == Some(&Value::String(String::new()))
        }
        FilterOperator::IsNotEmpty => {
            cell_val.is_some()
                && cell_val != Some(&Value::Null)
                && cell_val != Some(&Value::String(String::new()))
        }
        FilterOperator::Equals => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            s.eq_ignore_ascii_case(&clause.value)
        }
        FilterOperator::NotEquals => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            !s.eq_ignore_ascii_case(&clause.value)
        }
        FilterOperator::Contains => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            s.to_lowercase().contains(&clause.value.to_lowercase())
        }
        FilterOperator::NotContains => {
            let s = cell_val
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            !s.to_lowercase().contains(&clause.value.to_lowercase())
        }
        FilterOperator::GreaterThan => {
            let n1 = cell_val.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
            let n2 = clause.value.parse::<f64>().ok();
            match (n1, n2) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        FilterOperator::LessThan => {
            let n1 = cell_val.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)));
            let n2 = clause.value.parse::<f64>().ok();
            match (n1, n2) {
                (Some(a), Some(b)) => a < b,
                _ => false,
            }
        }
        FilterOperator::In => {
            let wanted: Vec<String> = list_of(&clause.value).into_iter().map(|v| v.to_lowercase()).collect();
            match cell_val {
                Some(Value::Array(items)) => items.iter().any(|i| wanted.contains(&cell_text(Some(i)).to_lowercase())),
                other => wanted.contains(&cell_text(other).to_lowercase()),
            }
        }
        FilterOperator::Between => match range_of(&clause.value) {
            None => false,
            Some((low, high)) => {
                if let (Some(n), Ok(a), Ok(b)) = (cell_val.and_then(Value::as_f64), low.parse::<f64>(), high.parse::<f64>()) {
                    return a <= n && n <= b;
                }
                match (cell_val.and_then(Value::as_str).and_then(instant_of), instant_of(low), instant_of(high)) {
                    (Some(t), Some(a), Some(b)) => a <= t && t <= b,
                    _ => false,
                }
            }
        },
        FilterOperator::Before => match (cell_val.and_then(Value::as_str).and_then(instant_of), instant_of(&clause.value)) {
            (Some(t), Some(limit)) => t < limit,
            _ => false,
        },
        FilterOperator::After => match (cell_val.and_then(Value::as_str).and_then(instant_of), instant_of(&clause.value)) {
            (Some(t), Some(limit)) => t > limit,
            _ => false,
        },
    }
}

pub fn sort_records(records: &mut [Value], rules: &[SortRule]) {
    records.sort_by(|a, b| {
        for rule in rules {
            let va = a.get(&rule.field_name);
            let vb = b.get(&rule.field_name);
            let ord = compare_values(va, vb);
            let directed = match rule.direction {
                SortDirection::Asc => ord,
                SortDirection::Desc => ord.reverse(),
            };
            if !directed.is_eq() {
                return directed;
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(va), Some(vb)) => {
            if let (Some(na), Some(nb)) = (va.as_f64(), vb.as_f64()) {
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
            } else if let (Some(sa), Some(sb)) = (va.as_str(), vb.as_str()) {
                sa.cmp(sb)
            } else {
                std::cmp::Ordering::Equal
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub organization_code: String,
    pub department: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
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
    #[serde(default)]
    pub pages: Vec<AppPage>,
    #[serde(default)]
    pub access: AccessManifest,
    /// Where the app sits, narrower than its workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<scaffoldry_core::coordinates::Coordinates>,
}

/// A screen of an app: a built-in view, or a custom page an agent wrote.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppPage {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PageKind {
    View { view_id: String },
    /// The source is stored apart from the manifest and named by its hash.
    Custom {
        source_sha256: String,
        grants: Vec<PageGrant>,
        /// A module is one JSX component, compiled when the page opens. A bundle is a built script that runs as written.
        #[serde(default, skip_serializing_if = "PageFormat::is_module")]
        format: PageFormat,
        /// The libraries a bundle carries. A reviewer sees this list, and a check refuses a license that is not approved.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        dependencies: Vec<PageDependency>,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PageFormat {
    #[default]
    Module,
    Bundle,
}

impl PageFormat {
    pub fn is_module(&self) -> bool {
        *self == PageFormat::Module
    }
}

/// One library inside a bundle, as the bundle's author declares it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageDependency {
    pub name: String,
    pub version: String,
    pub license: String,
}

/// What a custom page may do to one table. A page has nothing beyond its grants. There is no
/// wildcard, and a table with no grant is invisible to the page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageGrant {
    pub table_id: String,
    /// Field names the page may receive.
    #[serde(default)]
    pub read: Vec<String>,
    /// Field names the page may set.
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub create: bool,
    #[serde(default)]
    pub delete: bool,
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

/// Joins a live app's slug to a proposal id. An app under such a slug is a preview of a proposal, not an app: it is in
/// no list and it holds no record. Its reads are the live app's.
pub const PREVIEW_MARK: &str = "~preview~";

/// The live app a slug stands for. A slug that is not a preview is its own.
pub fn stored_slug(slug: &str) -> &str {
    slug.split_once(PREVIEW_MARK).map_or(slug, |(live, _)| live)
}

pub struct ManifestEngine {
    manifests_by_slug: HashMap<String, AppManifest>,
    manifests_by_domain: HashMap<String, String>,
    /// The stored version each definition was read at. A server compares these with the stored versions to find the
    /// definitions that changed, so one approval reloads one app.
    versions: HashMap<String, i32>,
    /// Apps whose stored definition did not parse or validate, with the reason. Such an app is not served.
    broken: HashMap<String, String>,
}

impl ManifestEngine {
    pub fn new() -> Result<Self, EngineError> {
        // Fail construction early if the built in policy set does not load.
        ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| EngineError::PolicyError(e.to_string()))?;
        Ok(Self {
            manifests_by_slug: HashMap::new(),
            manifests_by_domain: HashMap::new(),
            versions: HashMap::new(),
            broken: HashMap::new(),
        })
    }

    /// Notes the stored version a definition was read at.
    pub fn set_version(&mut self, slug: &str, version: i32) {
        self.versions.insert(slug.to_string(), version);
    }

    /// The stored version of every definition this engine holds or has given up on.
    pub fn versions(&self) -> &HashMap<String, i32> {
        &self.versions
    }

    /// Stops serving an app whose stored definition cannot be read. Other apps are not touched.
    pub fn mark_broken(&mut self, slug: &str, reason: &str) {
        if let Some(old) = self.manifests_by_slug.remove(slug) {
            if let Some(domain) = old.custom_domain {
                self.manifests_by_domain.remove(&domain);
            }
        }
        self.broken.insert(slug.to_string(), reason.to_string());
    }

    /// Why an app is not served, when its stored definition could not be read.
    pub fn broken_reason(&self, slug: &str) -> Option<&str> {
        self.broken.get(slug).map(String::as_str)
    }

    pub fn manifests_count(&self) -> usize {
        self.manifests_by_slug.keys().filter(|s| !s.contains(PREVIEW_MARK)).count()
    }

    /// Every app. A preview of a proposal is not one.
    pub fn list_manifests(&self) -> Vec<AppManifest> {
        self.manifests_by_slug.values().filter(|m| !m.slug.contains(PREVIEW_MARK)).cloned().collect()
    }

    /// Drops a preview. Nothing else is removed this way.
    pub fn remove_preview(&mut self, slug: &str) {
        if slug.contains(PREVIEW_MARK) {
            self.manifests_by_slug.remove(slug);
        }
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

        self.broken.remove(&manifest.slug);
        self.manifests_by_slug.insert(manifest.slug.clone(), manifest);
        Ok(())
    }

    pub fn submit_record(
        &self,
        caller: &EduPersonIdentity,
        app_slug: &str,
        payload: &Value,
    ) -> Result<SubmittedRecord, EngineError> {
        self.submit_record_labelled(caller, app_slug, payload, &LabelSet::new(), &[])
    }

    /// Submits a record, deciding sensitivity with the given labels.
    pub fn submit_record_labelled(
        &self,
        caller: &EduPersonIdentity,
        app_slug: &str,
        payload: &Value,
        labels: &LabelSet,
        categories: &[sensitivity::Category],
    ) -> Result<SubmittedRecord, EngineError> {
        let manifest = self
            .manifests_by_slug
            .get(app_slug)
            .ok_or_else(|| EngineError::NotFound(format!("App '{}' not found", app_slug)))?;

        let _ = caller;

        let mut is_ferpa_sensitive = false;
        let mut applied_ceds = HashMap::new();

        let record_table_id = payload.get("_table_id").and_then(|v| v.as_str());

        for view in &manifest.views {
            if let (Some(rec_tid), Some(view_tid)) = (record_table_id, view.table_id.as_deref()) {
                if rec_tid != view_tid {
                    continue;
                }
            }
            for field in &view.fields {
                let val = payload.get(&field.name);
                if field.required && val.is_none_or(|v| v.is_null()) {
                    return Err(EngineError::ValidationError(format!(
                        "Required field '{}' is missing in payload",
                        field.name
                    )));
                }

                let key = (
                    app_slug.to_string(),
                    view.table_id.clone().unwrap_or_default(),
                    field.name.clone(),
                );
                if val.is_some() && effective_ferpa_sensitive(field, labels.get(&key), categories).is_protected() {
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

/// Formats a single cell value for RFC 4180 CSV export.
pub fn escape_csv_cell(cell: &str) -> String {
    if cell.contains(',') || cell.contains('"') || cell.contains('\n') || cell.contains('\r') {
        let escaped = cell.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        cell.to_string()
    }
}

/// Generates an RFC 4180 CSV string from a slice of header names and JSON records.
pub fn export_records_to_csv(headers: &[&str], records: &[Value]) -> String {
    let mut out = String::new();

    let header_line: Vec<String> = headers.iter().map(|h| escape_csv_cell(h)).collect();
    out.push_str(&header_line.join(","));
    out.push('\n');

    for record in records {
        let row: Vec<String> = headers
            .iter()
            .map(|&h| match record.get(h) {
                Some(Value::String(s)) => escape_csv_cell(s),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Array(arr)) => {
                    let items: Vec<String> = arr
                        .iter()
                        .filter_map(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        })
                        .collect();
                    escape_csv_cell(&items.join(", "))
                }
                Some(Value::Null) | None => String::new(),
                Some(other) => escape_csv_cell(&other.to_string()),
            })
            .collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }

    out
}

/// Parses an RFC 4180 CSV text into a vector of JSON objects.
pub fn parse_csv_to_records(csv_text: &str) -> Result<Vec<Value>, String> {
    let trimmed = csv_text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = trimmed.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(ch);
            }
        } else {
            match ch {
                '"' => {
                    in_quotes = true;
                }
                ',' => {
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                    rows.push(std::mem::take(&mut current_row));
                }
                '\n' => {
                    current_row.push(current_field.trim().to_string());
                    current_field.clear();
                    rows.push(std::mem::take(&mut current_row));
                }
                _ => {
                    current_field.push(ch);
                }
            }
        }
    }

    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field.trim().to_string());
        rows.push(current_row);
    }

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let headers = rows[0].clone();
    let mut records = Vec::new();

    for row in rows.into_iter().skip(1) {
        if row.is_empty() || (row.len() == 1 && row[0].is_empty()) {
            continue;
        }
        let mut map = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            if header.is_empty() {
                continue;
            }
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            if let Ok(num) = cell.parse::<f64>() {
                map.insert(header.clone(), serde_json::json!(num));
            } else if cell.eq_ignore_ascii_case("true") {
                map.insert(header.clone(), serde_json::json!(true));
            } else if cell.eq_ignore_ascii_case("false") {
                map.insert(header.clone(), serde_json::json!(false));
            } else {
                map.insert(header.clone(), serde_json::json!(cell));
            }
        }
        records.push(Value::Object(map));
    }

    Ok(records)
}

/// Duplicates a record, giving it a new ID and appending "(Copy)" to the primary field.
pub fn duplicate_record(record: &Value, primary_field: &str, new_id: &str) -> Value {
    let mut cloned = record.clone();
    if let Value::Object(ref mut map) = cloned {
        map.insert("id".to_string(), Value::String(new_id.to_string()));
        if let Some(Value::String(val)) = map.get(primary_field) {
            map.insert(primary_field.to_string(), Value::String(format!("{} (Copy)", val)));
        }
    }
    cloned
}

/// Batch removes records whose "id" matches any id in `ids_to_delete`.
pub fn batch_delete_records(records: &mut Vec<Value>, ids_to_delete: &[&str]) {
    records.retain(|r| {
        if let Some(id_val) = r.get("id").and_then(|v| v.as_str()) {
            !ids_to_delete.contains(&id_val)
        } else {
            true
        }
    });
}

