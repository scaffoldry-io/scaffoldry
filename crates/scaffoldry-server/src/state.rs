//! Server State and Storage for Datasets, Workspaces, and SCIM Identity

use scaffoldry_engine::ManifestEngine;
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    pub id: String,
    pub user_name: String,
    pub name: Value,
    pub active: bool,
    pub emails: Vec<Value>,
    pub roles: Vec<Value>,
    #[serde(rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User")]
    pub enterprise_extension: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    pub id: String,
    pub display_name: String,
    pub members: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub code: String,
    pub organization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorRecord {
    pub id: String,
    pub workspace_id: String,
    pub eppn: String,
    pub role: String,
    pub scoped_affiliation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    pub id: String,
    pub app_slug: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
    pub created_at: String,
}

pub struct ServerState {
    pub users: RwLock<HashMap<String, ScimUser>>,
    pub groups: RwLock<HashMap<String, ScimGroup>>,
    pub workspaces: RwLock<HashMap<String, WorkspaceRecord>>,
    pub collaborators: RwLock<HashMap<String, Vec<CollaboratorRecord>>>,
    pub records: RwLock<HashMap<String, Vec<DatasetRecord>>>,
    pub engine: RwLock<ManifestEngine>,
    pub policy_engine: ScaffoldryPolicyEngine,
}

impl ServerState {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let engine = ManifestEngine::new()
            .map_err(|e| format!("Failed to initialize ManifestEngine: {e}"))?;
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| format!("Failed to initialize PolicyEngine: {e}"))?;

        Ok(Self {
            users: RwLock::new(HashMap::new()),
            groups: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            collaborators: RwLock::new(HashMap::new()),
            records: RwLock::new(HashMap::new()),
            engine: RwLock::new(engine),
            policy_engine,
        })
    }
}

pub type SharedState = Arc<ServerState>;
