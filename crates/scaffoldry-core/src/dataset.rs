//! Sovereign Dataset, Schema, and Relational Lattice Domain Models

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetField {
    pub name: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    pub ferpa_sensitive: bool,
    pub ceds_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedDataset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub department: String,
    pub organization: String,
    pub sensitivity_level: String,
    pub herm_capability_id: Option<String>,
    pub fields: Vec<DatasetField>,
    pub record_count: usize,
    pub published_at: String,
}
