//! Sovereign Dataset, Schema, and Relational Lattice Domain Models

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationshipType {
    OneToOne,
    OneToMany,
    ManyToMany,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRelationship {
    pub id: String,
    pub name: String,
    pub source_dataset_id: String,
    pub target_dataset_id: String,
    pub source_field: String,
    pub target_field: String,
    pub relationship_type: RelationshipType,
    pub display_field: String,
}

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
    #[serde(default)]
    pub sample_data: Vec<Value>,
}
