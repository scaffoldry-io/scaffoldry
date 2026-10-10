//! Sensitivity categories and the settings that define them.
//!
//! A category is a named kind of sensitive data. The organization defines its categories, and
//! says which are protected. A detector belongs to a category by id. Both lists live in
//! settings. This module checks them. The published schema is
//! `governance/schema/sensitivity-settings.schema.json`, and these checks enforce it.

use crate::detect::{validate_shape, DetectorKind, DetectorSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const MAX_CATEGORIES: usize = 50;
pub const MAX_DETECTORS: usize = 100;
const MAX_ID_LEN: usize = 64;
const MAX_TEXT_LEN: usize = 128;

/// What a field is sensitive for: whether any of its categories is protected, and which it has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sensitivity {
    pub protected: bool,
    pub categories: BTreeSet<String>,
}

impl Sensitivity {
    /// The yes-or-no question every caller used to ask of the old flag.
    pub fn is_protected(&self) -> bool {
        self.protected
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// A free label for the statute or standard, shown to reviewers.
    #[serde(default)]
    pub source: String,
    /// A protected category is treated as the legacy flag always was.
    pub protected: bool,
    /// Detector ids.
    pub detectors: Vec<String>,
}

/// A category and its detectors, as shipped in a release and copied into settings when switched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub category: Category,
    pub detectors: Vec<DetectorSpec>,
}

/// Where a setting is wrong. `path` is relative to the setting, such as `[1].detectors[0]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    pub path: String,
    pub message: String,
}

impl SettingsError {
    fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }
}

fn id_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

const ID_RULE: &str = "must be 1 to 64 characters: lowercase letters, digits, '-' and '_'";

/// Checks `sensitivity.detectors`. Returns the parsed detectors.
pub fn validate_detectors(value: &Value) -> Result<Vec<DetectorSpec>, SettingsError> {
    let items = value.as_array().ok_or_else(|| SettingsError::at("", "must be a list"))?;
    if items.len() > MAX_DETECTORS {
        return Err(SettingsError::at("", format!("holds at most {MAX_DETECTORS} detectors")));
    }
    let mut out: Vec<DetectorSpec> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let d: DetectorSpec = serde_json::from_value(item.clone())
            .map_err(|e| SettingsError::at(format!("[{i}]"), e.to_string()))?;
        if !id_valid(&d.id) {
            return Err(SettingsError::at(format!("[{i}].id"), format!("id {ID_RULE}")));
        }
        if out.iter().any(|o| o.id == d.id) {
            return Err(SettingsError::at(format!("[{i}].id"), format!("detector id '{}' is used twice", d.id)));
        }
        match d.kind {
            DetectorKind::Shape => {
                let shape = d
                    .shape
                    .as_deref()
                    .ok_or_else(|| SettingsError::at(format!("[{i}].shape"), "a shape detector needs a shape"))?;
                validate_shape(shape).map_err(|m| SettingsError::at(format!("[{i}].shape"), m))?;
                if d.checksum.as_deref().is_some_and(|c| c != "luhn") {
                    return Err(SettingsError::at(format!("[{i}].checksum"), "the only checksum is 'luhn'"));
                }
            }
            _ => {
                if d.shape.is_some() {
                    return Err(SettingsError::at(format!("[{i}].shape"), "only a shape detector has a shape"));
                }
                if d.checksum.is_some() {
                    return Err(SettingsError::at(format!("[{i}].checksum"), "only a shape detector has a checksum"));
                }
            }
        }
        if d.allow_unseparated && d.kind != DetectorKind::UsSsn {
            return Err(SettingsError::at(
                format!("[{i}].allow_unseparated"),
                "only a us_ssn detector can allow unseparated digits",
            ));
        }
        out.push(d);
    }
    Ok(out)
}

/// Checks `sensitivity.categories` against the detectors that exist. Returns the parsed categories.
pub fn validate_categories(value: &Value, detectors: &[DetectorSpec]) -> Result<Vec<Category>, SettingsError> {
    let items = value.as_array().ok_or_else(|| SettingsError::at("", "must be a list"))?;
    if items.len() > MAX_CATEGORIES {
        return Err(SettingsError::at("", format!("holds at most {MAX_CATEGORIES} categories")));
    }
    let mut out: Vec<Category> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let c: Category = serde_json::from_value(item.clone())
            .map_err(|e| SettingsError::at(format!("[{i}]"), e.to_string()))?;
        if !id_valid(&c.id) {
            return Err(SettingsError::at(format!("[{i}].id"), format!("id {ID_RULE}")));
        }
        if out.iter().any(|o| o.id == c.id) {
            return Err(SettingsError::at(format!("[{i}].id"), format!("category id '{}' is used twice", c.id)));
        }
        if c.name.trim().is_empty() || c.name.chars().count() > MAX_TEXT_LEN {
            return Err(SettingsError::at(format!("[{i}].name"), "a name is 1 to 128 characters"));
        }
        if c.source.chars().count() > MAX_TEXT_LEN {
            return Err(SettingsError::at(format!("[{i}].source"), "a source is at most 128 characters"));
        }
        for (j, d) in c.detectors.iter().enumerate() {
            if !detectors.iter().any(|x| &x.id == d) {
                return Err(SettingsError::at(
                    format!("[{i}].detectors[{j}]"),
                    format!("no detector has the id '{d}'"),
                ));
            }
        }
        out.push(c);
    }
    Ok(out)
}

/// Every detector a category names must still exist. Run when the detector list is saved.
pub fn check_references(categories: &[Category], detectors: &[DetectorSpec]) -> Result<(), SettingsError> {
    for (i, c) in categories.iter().enumerate() {
        for (j, d) in c.detectors.iter().enumerate() {
            if !detectors.iter().any(|x| &x.id == d) {
                return Err(SettingsError::at(
                    format!("[{i}].detectors[{j}]"),
                    format!("category '{}' names the detector '{d}', which would no longer exist", c.id),
                ));
            }
        }
    }
    Ok(())
}

/// Checks a preset file: `{ "category": {...}, "detectors": [...] }`.
pub fn validate_preset(value: &Value) -> Result<Preset, SettingsError> {
    let obj = value.as_object().ok_or_else(|| SettingsError::at("", "a preset is an object"))?;
    let detectors = validate_detectors(obj.get("detectors").unwrap_or(&Value::Null))
        .map_err(|e| SettingsError::at(format!("detectors{}", e.path), e.message))?;
    let category_json = obj.get("category").ok_or_else(|| SettingsError::at("category", "a preset has a category"))?;
    let mut categories = validate_categories(&Value::Array(vec![category_json.clone()]), &detectors)
        .map_err(|e| SettingsError::at(format!("category{}", e.path.trim_start_matches("[0]")), e.message))?;
    Ok(Preset { category: categories.remove(0), detectors })
}

/// The presets shipped in this release, as files under `governance/sensitivity-presets/`.
/// Switching one on copies it into settings. After that it is the organization's, and a new
/// release never changes it. A preset's id is the id of its category, so no id is written here.
const PRESET_FILES: [&str; 4] = [
    include_str!("../../../governance/sensitivity-presets/pii.json"),
    include_str!("../../../governance/sensitivity-presets/pci.json"),
    include_str!("../../../governance/sensitivity-presets/phi.json"),
    include_str!("../../../governance/sensitivity-presets/ferpa.json"),
];

/// The presets a new installation switches on.
pub const DEFAULT_PRESETS: [&str; 2] = ["pii", "pci"];

fn parse_preset(text: &str) -> Result<Preset, SettingsError> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => validate_preset(&v),
        Err(e) => Err(SettingsError::at("", format!("a preset file is not JSON: {e}"))),
    }
}

/// The ids of the shipped presets, in the order they ship.
pub fn preset_ids() -> Vec<String> {
    PRESET_FILES.iter().filter_map(|t| parse_preset(t).ok()).map(|p| p.category.id).collect()
}

/// One shipped preset, parsed and checked. `None` when no preset has that id.
pub fn preset(id: &str) -> Option<Result<Preset, SettingsError>> {
    PRESET_FILES.iter().map(|t| parse_preset(t)).find(|p| p.as_ref().is_ok_and(|p| p.category.id == id))
}

/// The categories and detectors of a new installation: the default presets, in order.
pub fn default_settings() -> (Vec<Category>, Vec<DetectorSpec>) {
    let mut categories = Vec::new();
    let mut detectors: Vec<DetectorSpec> = Vec::new();
    for id in DEFAULT_PRESETS {
        if let Some(Ok(p)) = preset(id) {
            categories.push(p.category);
            for d in p.detectors {
                if !detectors.iter().any(|x| x.id == d.id) {
                    detectors.push(d);
                }
            }
        }
    }
    (categories, detectors)
}
