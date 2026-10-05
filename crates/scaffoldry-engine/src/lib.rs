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
        }
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
