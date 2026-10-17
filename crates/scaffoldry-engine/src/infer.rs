//! Type inference for imports.
//!
//! Examines values in a column to propose a field type:
//! - number
//! - date
//! - date-time
//! - boolean
//! - a select when the column has at most 20 distinct values over at least 20 rows
//! - an email
//! - a URL
//! - a collaborator when every value resolves to a person
//! - text otherwise

use crate::FieldType;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredType {
    Number,
    Date,
    DateTime,
    Boolean,
    Select,
    Email,
    Url,
    Collaborator,
    Text,
}

impl InferredType {
    pub fn to_field_type(&self) -> FieldType {
        match self {
            InferredType::Number => FieldType::Number,
            InferredType::Date => FieldType::Date,
            InferredType::DateTime => FieldType::DateTime,
            InferredType::Boolean => FieldType::Boolean,
            InferredType::Select => FieldType::Select,
            InferredType::Email => FieldType::Email,
            InferredType::Url => FieldType::Url,
            InferredType::Collaborator => FieldType::Collaborator { allow_multiple: false },
            InferredType::Text => FieldType::Text,
        }
    }
}

pub struct AnalyzeOptions<F = fn(&str) -> bool>
where
    F: Fn(&str) -> bool,
{
    pub is_person: Option<F>,
}

impl Default for AnalyzeOptions<fn(&str) -> bool> {
    fn default() -> Self {
        Self { is_person: None }
    }
}

pub fn is_boolean_str(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "true" | "false" | "yes" | "no" | "t" | "f" | "1" | "0")
}

pub fn is_number_str(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    s.parse::<f64>().is_ok()
}

pub fn is_date_str(s: &str) -> bool {
    let s = s.trim();
    if s.len() == 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-') {
        let parts: Vec<&str> = s.split("-").collect();
        if parts.len() == 3 {
            return parts[0].parse::<u32>().is_ok()
                && parts[1].parse::<u32>().is_ok()
                && parts[2].parse::<u32>().is_ok();
        }
    }
    false
}

pub fn is_datetime_str(s: &str) -> bool {
    let s = s.trim();
    if s.len() >= 19 && (s.contains("T") || s.contains("t") || s.contains(" ")) {
        if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
            return true;
        }
        if chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").is_ok() {
            return true;
        }
    }
    false
}

pub fn is_email_str(s: &str) -> bool {
    let s = s.trim();
    if let Some((user, domain)) = s.split_once("@") {
        !user.is_empty() && domain.contains(".") && !domain.starts_with(".") && !domain.ends_with(".")
    } else {
        false
    }
}

pub fn is_url_str(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn infer_column_type<F>(values: &[&str], options: &AnalyzeOptions<F>) -> InferredType
where
    F: Fn(&str) -> bool,
{
    let non_empty: Vec<&str> = values.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if non_empty.is_empty() {
        return InferredType::Text;
    }

    // Check collaborator first if person checker provided
    if let Some(ref is_person) = options.is_person {
        if non_empty.iter().all(|s| is_person(s)) {
            return InferredType::Collaborator;
        }
    }

    // Check boolean
    if non_empty.iter().all(|s| is_boolean_str(s)) {
        return InferredType::Boolean;
    }

    // Check number
    if non_empty.iter().all(|s| is_number_str(s)) {
        return InferredType::Number;
    }

    // Check date / datetime
    if non_empty.iter().all(|s| is_datetime_str(s)) {
        return InferredType::DateTime;
    }
    if non_empty.iter().all(|s| is_date_str(s)) {
        return InferredType::Date;
    }

    // Check select: at most 20 distinct values over at least 20 rows
    if non_empty.len() >= 20 {
        let distinct: HashSet<&str> = non_empty.iter().copied().collect();
        if distinct.len() <= 20 {
            return InferredType::Select;
        }
    }

    // Check email
    if non_empty.iter().all(|s| is_email_str(s)) {
        return InferredType::Email;
    }

    // Check url
    if non_empty.iter().all(|s| is_url_str(s)) {
        return InferredType::Url;
    }

    InferredType::Text
}
