//! Server State and Storage for Datasets, Workspaces, and SCIM Identity

use chrono::Utc;
use scaffoldry_core::{
    verify_ledger_chain, ActionType, AutomationRule, ConditionOperator, DatasetField,
    DatasetRelationship, DecisionType, FieldPredicate, LedgerEntry, LedgerError,
    NewLedgerEntryParams, ProcessInstance, PublishedDataset, RelationshipType, TriggerEvent,
    GENESIS_PREVIOUS_HASH,
};
use scaffoldry_engine::{AppManifest, ManifestEngine};
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub token_hash: String,
    pub id: uuid::Uuid,
    pub kind: String, // 'agent' | 'impersonation' | 'scim' | 'setup'
    pub eppn: String,
    pub label: String,
    pub original_admin: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    pub id: String,
    #[serde(alias = "userName")]
    pub user_name: String,
    #[serde(default)]
    pub name: Value,
    pub active: bool,
    #[serde(default)]
    pub emails: Vec<Value>,
    #[serde(default)]
    pub roles: Vec<Value>,
    #[serde(default, rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User")]
    pub enterprise_extension: Option<Value>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    pub id: String,
    pub display_name: String,
    pub members: Vec<Value>,
}

fn default_department() -> String {
    "general".to_string()
}
fn default_icon() -> String {
    "📁".to_string()
}
fn default_visibility() -> String {
    "restricted".to_string()
}
fn default_classification() -> String {
    "Internal".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub code: String,
    pub organization: String,
    #[serde(default = "default_department")]
    pub department: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub lead: String,
    #[serde(default = "default_visibility")]
    pub visibility: String, // "restricted" | "departmental" | "institutional"
    #[serde(default)]
    pub allowed_affiliations: Vec<String>,
    #[serde(default = "default_classification")]
    pub data_classification: String,
    #[serde(default)]
    pub cedar_policy_guard: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub organization_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationNode {
    pub id: uuid::Uuid,
    pub parent_id: Option<uuid::Uuid>,
    pub name: String,
    pub code: String,
    pub org_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRow {
    pub id: uuid::Uuid,
    pub person_id: uuid::Uuid,
    pub eppn: String,
    pub organization_id: uuid::Uuid,
    pub role_title: String,
    pub scoped_affiliation: String,
    pub is_primary: bool,
    #[serde(default = "default_role_source")]
    pub source: String,
}

fn default_role_source() -> String {
    "api".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorRecord {
    pub id: String,
    pub workspace_id: String,
    pub eppn: String,
    #[serde(default)]
    pub name: String,
    pub role: String, // "owner" | "admin" | "editor" | "viewer"
    pub scoped_affiliation: String,
    #[serde(default)]
    pub department: String,
    #[serde(default)]
    pub added_at: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub eppn: String,
    pub name: String,
    pub role_title: String,
    pub affiliation: String,
    pub department: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub token: String,
    pub user: AuthUser,
    pub original_admin: Option<AuthUser>,
    pub created_at: String,
}

pub struct ServerState {
    pub users: RwLock<HashMap<String, ScimUser>>,
    pub groups: RwLock<HashMap<String, ScimGroup>>,
    pub workspaces: RwLock<HashMap<String, WorkspaceRecord>>,
    pub collaborators: RwLock<HashMap<String, Vec<CollaboratorRecord>>>,
    pub records: RwLock<HashMap<String, Vec<DatasetRecord>>>,
    pub datasets: RwLock<HashMap<String, PublishedDataset>>,
    pub relationships: RwLock<HashMap<String, DatasetRelationship>>,
    pub automations: RwLock<HashMap<String, Vec<AutomationRule>>>,
    pub process_instances: RwLock<HashMap<String, ProcessInstance>>,
    pub ledger: RwLock<Vec<LedgerEntry>>,
    pub engine: RwLock<ManifestEngine>,
    pub policy_engine: ScaffoldryPolicyEngine,
    pub sessions: RwLock<HashMap<String, AuthSession>>,
    pub api_tokens: RwLock<HashMap<String, ApiToken>>,
    pub repository: Option<Arc<crate::repository::PostgresRepository>>,
    pub organizations: RwLock<HashMap<uuid::Uuid, OrganizationNode>>,
    pub roles: RwLock<Vec<RoleRow>>,
    pub settings: RwLock<HashMap<String, serde_json::Value>>,
}

impl ServerState {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut engine = ManifestEngine::new()
            .map_err(|e| format!("Failed to initialize ManifestEngine: {e}"))?;
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| format!("Failed to initialize PolicyEngine: {e}"))?;

        let mut users = HashMap::new();
        let mut groups = HashMap::new();
        let default_root_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let default_root_org = OrganizationNode {
            id: default_root_id,
            parent_id: None,
            name: "Institution".to_string(),
            code: "INST".to_string(),
            org_type: "Institution".to_string(),
        };
        let mut organizations = HashMap::new();
        organizations.insert(default_root_id, default_root_org);
        let mut roles: Vec<RoleRow> = Vec::new();
        let mut datasets = HashMap::new();
        let mut relationships = HashMap::new();
        let mut process_instances = HashMap::new();
        let mut automations: HashMap<String, Vec<AutomationRule>> = HashMap::new();

        let now = Utc::now().to_rfc3339();

        let entry_0 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "system@scaffoldry.internal".to_string(),
            organization_code: "INST".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional sovereign genesis ledger entry".to_string(),
            payload: &json!({"system": "scaffoldry", "genesis": true}),
        });

        let mut ledger = vec![entry_0.clone()];

        let mut sessions = HashMap::new();
        let mut workspaces = HashMap::new();
        let mut collaborators: HashMap<String, Vec<CollaboratorRecord>> = HashMap::new();
        let mut api_tokens = HashMap::new();
        let scim_hash = crate::service::identity::hash_token("test-scim-token");
        let default_scim_token = ApiToken {
            token_hash: scim_hash.clone(),
            id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            kind: "scim".to_string(),
            eppn: "jordan.lee@state.edu".to_string(),
            label: "SCIM Provisioning Credential".to_string(),
            original_admin: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(365),
            last_used_at: None,
            revoked_at: None,
        };
        api_tokens.insert(scim_hash, default_scim_token.clone());

        let mut settings_map = HashMap::new();
        settings_map.insert("cors.allowed_origins".to_string(), serde_json::json!([]));
        settings_map.insert("tokens.max_days".to_string(), serde_json::json!(90));
        settings_map.insert("tokens.agent_enabled".to_string(), serde_json::json!(true));
        settings_map.insert("process.stale_days".to_string(), serde_json::json!(14));
        settings_map.insert("mcp.disabled_tools".to_string(), serde_json::json!([]));
        settings_map.insert("pages.disabled".to_string(), serde_json::json!([]));

        let repository = match crate::repository::PostgresRepository::connect(None) {
            Ok(repo) => Some(Arc::new(repo)),
            Err(e) => {
                if let crate::repository::RepositoryError::TamperDetected(msg) = &e {
                    return Err(format!("Cryptographic tamper detected in ledger: {msg}").into());
                }
                return Err(format!("Failed to connect to PostgreSQL repository: {e}").into());
            }
        };

        if let Some(ref repo) = repository {
            if let Ok(db_settings) = repo.get_platform_settings() {
                for (k, v) in db_settings {
                    settings_map.insert(k, v);
                }
            }
        }


        if let Some(ref repo) = repository {
            // Always ensure root organization exists before workspaces
            for org in organizations.values() {
                let _ = repo.upsert_organization(org);
            }

            // Always ensure root organization exists
            for org in organizations.values() {
                let _ = repo.upsert_organization(org);
            }

            // Ensure genesis ledger entry exists in repo
            let _ = repo.append_ledger_entry(&entry_0, &json!({"system": "scaffoldry", "genesis": true}));
            // Load persisted manifests into engine
            if let Ok(persisted_manifests) = repo.list_app_manifests() {
                for m in persisted_manifests {
                    let _ = engine.register_manifest(m);
                }
            }

            // Load persisted datasets
            if let Ok(persisted_datasets) = repo.list_published_datasets() {
                for ds in persisted_datasets {
                    datasets.insert(ds.id.clone(), ds);
                }
            }

            // Load persisted relationships
            if let Ok(persisted_rels) = repo.list_dataset_relationships() {
                for rel in persisted_rels {
                    relationships.insert(rel.id.clone(), rel);
                }
            }

            // Load persisted automations
            if let Ok(persisted_automations) = repo.list_workflow_automations(None) {
                for r in persisted_automations {
                    let entry = automations.entry(r.app_slug.clone()).or_default();
                    if !entry.iter().any(|existing| existing.id == r.id) {
                        entry.push(r);
                    }
                }
            }

            // Load persisted process instances
            if let Ok(persisted_instances) = repo.list_process_instances(None, None) {
                for inst in persisted_instances {
                    process_instances.insert(inst.id.clone(), inst);
                }
            }

            for org in organizations.values() {
                let _ = repo.upsert_organization(org);
            }
            if let Ok(persisted_orgs) = repo.list_organizations() {
                for org in persisted_orgs {
                    organizations.insert(org.id, org);
                }
            }
            if let Ok(persisted_roles) = repo.list_roles() {
                roles = persisted_roles;
            }

            // Load persisted SCIM users and groups
            if let Ok(persisted_users) = repo.list_scim_users() {
                for u in persisted_users {
                    users.insert(u.id.clone(), u);
                }
            }
            if let Ok(persisted_groups) = repo.list_scim_groups() {
                for g in persisted_groups {
                    groups.insert(g.id.clone(), g);
                }
            }

            if let Ok(persisted_ws) = repo.list_workspaces() {
                for ws in persisted_ws {
                    if let Ok(collabs) = repo.get_collaborators(&ws.id) {
                        collaborators.insert(ws.id.clone(), collabs);
                    }
                    workspaces.insert(ws.id.clone(), ws);
                }
            }

            if let Ok(persisted_sessions) = repo.list_sessions() {
                if persisted_sessions.is_empty() {
                    for s in sessions.values() {
                        let _ = repo.upsert_session(s);
                    }
                } else {
                    sessions = persisted_sessions;
                }
            }
            let _ = repo.insert_api_token(&default_scim_token);

            if let Ok(persisted_ledger) = repo.get_ledger() {
                if !persisted_ledger.is_empty() {
                    ledger = persisted_ledger;
                }
            }
        }

        Ok(Self {
            users: RwLock::new(users),
            groups: RwLock::new(groups),
            workspaces: RwLock::new(workspaces),
            collaborators: RwLock::new(collaborators),
            records: RwLock::new(HashMap::new()),
            datasets: RwLock::new(datasets),
            relationships: RwLock::new(relationships),
            automations: RwLock::new(automations),
            process_instances: RwLock::new(process_instances),
            ledger: RwLock::new(ledger),
            engine: RwLock::new(engine),
            policy_engine,
            sessions: RwLock::new(sessions),
            api_tokens: RwLock::new(api_tokens),
            repository,
            organizations: RwLock::new(organizations),
            roles: RwLock::new(roles),
            settings: RwLock::new(settings_map),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecordDecisionInput<'a> {
    pub principal: String,
    pub organization_code: String,
    pub app_slug: Option<String>,
    pub decision_type: DecisionType,
    pub oscal_control_id: String,
    pub rationale: String,
    pub payload: &'a Value,
}

impl ServerState {
    pub fn persist_organization(&self, org: &OrganizationNode) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.organizations.write().unwrap().insert(org.id, org.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_organization(org)?;
        }
        Ok(())
    }

    pub fn delete_scim_roles(&self, eppn: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut guard = self.roles.write().unwrap();
        guard.retain(|r| !(r.eppn == eppn && r.source == "scim"));
        if let Some(ref repo) = self.repository {
            repo.delete_scim_roles_for_user(eppn)?;
        }
        Ok(())
    }

    pub fn persist_role(&self, role: &RoleRow) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut guard = self.roles.write().unwrap();
        if let Some(pos) = guard.iter().position(|r| r.id == role.id) {
            guard[pos] = role.clone();
        } else {
            guard.push(role.clone());
        }
        if let Some(ref repo) = self.repository {
            repo.upsert_role(role)?;
        }
        Ok(())
    }

    pub fn persist_workspace(&self, ws: WorkspaceRecord) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.workspaces.write().unwrap().insert(ws.id.clone(), ws.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_workspace(&ws)?;
        }
        Ok(())
    }

    pub fn get_workspace(&self, id: &str) -> Option<WorkspaceRecord> {
        if let Some(ref repo) = self.repository {
            if let Ok(Some(ws)) = repo.get_workspace(id) {
                return Some(ws);
            }
        }
        self.workspaces.read().unwrap().get(id).cloned()
    }

    pub fn persist_workspaces_batch(&self, batch: &[WorkspaceRecord]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut guard = self.workspaces.write().unwrap();
        for ws in batch {
            guard.insert(ws.id.clone(), ws.clone());
        }
        if let Some(ref repo) = self.repository {
            repo.persist_workspaces_batch(batch)?;
        }
        Ok(())
    }

    pub fn count_workspaces(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref repo) = self.repository {
            return Ok(repo.count_workspaces()?);
        }
        Ok(self.workspaces.read().unwrap().len())
    }

    pub fn persist_app_manifest(&self, m: AppManifest) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.engine.write().map_err(|_| "engine lock poisoned")?.register_manifest(m.clone())?;
        if let Some(ref repo) = self.repository {
            repo.upsert_app_manifest(&m)?;
        }
        Ok(())
    }

    pub fn get_app_manifest(&self, slug: &str) -> Option<AppManifest> {
        if let Some(ref repo) = self.repository {
            if let Ok(Some(m)) = repo.get_app_manifest(slug) {
                return Some(m);
            }
        }
        use scaffoldry_engine::HostRouter;
        self.engine.read().ok()?.resolve_by_slug(slug).cloned()
    }

    pub fn persist_dataset(&self, ds: PublishedDataset) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.datasets.write().map_err(|_| "datasets lock poisoned")?.insert(ds.id.clone(), ds.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_published_dataset(&ds)?;
        }
        Ok(())
    }

    pub fn get_dataset(&self, id: &str) -> Option<PublishedDataset> {
        self.datasets.read().ok()?.get(id).cloned()
    }

    pub fn persist_relationship(&self, rel: DatasetRelationship) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.relationships.write().map_err(|_| "relationships lock poisoned")?.insert(rel.id.clone(), rel.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_dataset_relationship(&rel)?;
        }
        Ok(())
    }

    pub fn persist_automation(&self, rule: AutomationRule) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.automations.write().map_err(|_| "automations lock poisoned")?.entry(rule.app_slug.clone()).or_default().push(rule.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_workflow_automation(&rule)?;
        }
        Ok(())
    }

    pub fn persist_process_instance(&self, inst: ProcessInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.process_instances.write().map_err(|_| "process_instances lock poisoned")?.insert(inst.id.clone(), inst.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_process_instance(&inst)?;
        }
        Ok(())
    }

    pub fn persist_scim_user(&self, user: ScimUser) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.users.write().map_err(|_| "users lock poisoned")?.insert(user.id.clone(), user.clone());
        if let Some(ref repo) = self.repository {
            repo.upsert_scim_user(&user)?;
        }
        Ok(())
    }

    pub fn get_scim_user(&self, id: &str) -> Option<ScimUser> {
        self.users.read().ok()?.get(id).cloned()
    }

    pub fn append_ledger_entry(
        &self,
        input: RecordDecisionInput<'_>,
    ) -> Result<LedgerEntry, LedgerError> {
        if let Some(ref repo) = self.repository {
            let entry = repo.append_ledger_decision(crate::repository::AppendDecisionParams {
                principal: input.principal.clone(),
                organization_code: input.organization_code.clone(),
                app_slug: input.app_slug.clone(),
                decision_type: input.decision_type.clone(),
                oscal_control_id: input.oscal_control_id.clone(),
                rationale: input.rationale.clone(),
                payload: input.payload.clone(),
            }).map_err(|e| LedgerError::RepositoryError(e.to_string()))?;
            self.ledger.write().unwrap().push(entry.clone());
            return Ok(entry);
        }

        let mut ledger = self.ledger.write().unwrap();
        let sequence = ledger.len() as u64;
        let previous_hash = ledger
            .last()
            .map(|l| l.entry_hash.clone())
            .unwrap_or_else(|| GENESIS_PREVIOUS_HASH.to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let entry = LedgerEntry::new(NewLedgerEntryParams {
            sequence,
            timestamp_iso: now,
            previous_hash,
            principal: input.principal,
            organization_code: input.organization_code,
            app_slug: input.app_slug,
            decision_type: input.decision_type,
            oscal_control_id: input.oscal_control_id,
            rationale: input.rationale,
            payload: input.payload,
        });
        ledger.push(entry.clone());
        Ok(entry)
    }

    pub fn verify_ledger(&self) -> Result<bool, LedgerError> {
        if let Some(ref repo) = self.repository {
            match repo.verify_and_initialize_ledger(false) {
                Ok(()) => return Ok(true),
                Err(_) => return Ok(false),
            }
        }
        let ledger = self.ledger.read().unwrap();
        verify_ledger_chain(&ledger)
    }

    pub fn export_oscal_component_definition(&self) -> Value {
        let entries = if let Some(ref repo) = self.repository {
            repo.get_ledger().unwrap_or_else(|_| self.ledger.read().unwrap().clone())
        } else {
            self.ledger.read().unwrap().clone()
        };
        let now = chrono::Utc::now().to_rfc3339();

        let implemented_requirements: Vec<Value> = entries
            .iter()
            .map(|entry| {
                json!({
                    "uuid": uuid::Uuid::new_v4().to_string(),
                    "control-id": entry.oscal_control_id.to_lowercase(),
                    "description": entry.rationale.clone(),
                    "props": [
                        {
                            "name": "ledger-sequence",
                            "value": entry.sequence.to_string()
                        },
                        {
                            "name": "ledger-principal",
                            "value": entry.principal.clone()
                        },
                        {
                            "name": "ledger-org-code",
                            "value": entry.organization_code.clone()
                        },
                        {
                            "name": "ledger-entry-hash",
                            "value": entry.entry_hash.clone()
                        },
                        {
                            "name": "ledger-previous-hash",
                            "value": entry.previous_hash.clone()
                        }
                    ]
                })
            })
            .collect();

        json!({
            "component-definition": {
                "uuid": uuid::Uuid::new_v4().to_string(),
                "metadata": {
                    "title": "Scaffoldry Sovereign Application Platform Component Definition",
                    "last-modified": now,
                    "version": "1.0.0",
                    "oscal-version": "1.1.2"
                },
                "components": [
                    {
                        "uuid": uuid::Uuid::new_v4().to_string(),
                        "type": "software",
                        "title": "Scaffoldry Sovereign Application Platform",
                        "description": "Governed collaborative workspace, tabular engine, and Cedar authorization lattice",
                        "control-implementations": [
                            {
                                "uuid": uuid::Uuid::new_v4().to_string(),
                                "source": "https://doi.org/10.6028/NIST.SP.800-53r5",
                                "description": "Automated institutional control implementation and cryptographic verification ledger",
                                "implemented-requirements": implemented_requirements
                            }
                        ]
                    }
                ]
            }
        })
    }

    pub fn persist_api_token(&self, token: &ApiToken) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref repo) = self.repository {
            repo.insert_api_token(token)?;
        }
        if let Ok(mut tokens) = self.api_tokens.write() {
            tokens.insert(token.token_hash.clone(), token.clone());
        }
        Ok(())
    }

    pub fn get_api_token(&self, hash: &str) -> Option<ApiToken> {
        if let Some(ref repo) = self.repository {
            if let Ok(Some(token)) = repo.get_api_token(hash) {
                return Some(token);
            }
        }
        self.api_tokens.read().ok().and_then(|t| t.get(hash).cloned())
    }

    pub fn update_token_last_used(&self, hash: &str, now: chrono::DateTime<chrono::Utc>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref repo) = self.repository {
            repo.update_token_last_used(hash, now)?;
        }
        if let Ok(mut tokens) = self.api_tokens.write() {
            if let Some(t) = tokens.get_mut(hash) {
                t.last_used_at = Some(now);
            }
        }
        Ok(())
    }

    pub fn revoke_api_token(&self, id: uuid::Uuid, caller_eppn: &str, is_platform_admin: bool) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let revoked = if let Some(ref repo) = self.repository {
            repo.revoke_api_token(id, caller_eppn, is_platform_admin)?
        } else {
            let mut ok = false;
            if let Ok(mut tokens) = self.api_tokens.write() {
                for token in tokens.values_mut() {
                    if token.id == id && (is_platform_admin || token.eppn == caller_eppn) {
                        token.revoked_at = Some(chrono::Utc::now());
                        ok = true;
                        break;
                    }
                }
            }
            ok
        };
        Ok(revoked)
    }

    pub fn revoke_api_token_by_hash(&self, hash: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let revoked = if let Some(ref repo) = self.repository {
            repo.revoke_api_token_by_hash(hash)?
        } else {
            let mut ok = false;
            if let Ok(mut tokens) = self.api_tokens.write() {
                if let Some(t) = tokens.get_mut(hash) {
                    t.revoked_at = Some(chrono::Utc::now());
                    ok = true;
                }
            }
            ok
        };
        Ok(revoked)
    }

    pub fn in_memory() -> Self {
        let engine = ManifestEngine::new().expect("ManifestEngine in memory");
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine().expect("PolicyEngine in memory");
        let users = HashMap::new();
        let groups = HashMap::new();
        let default_root_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let default_root_org = OrganizationNode {
            id: default_root_id,
            parent_id: None,
            name: "Institution".to_string(),
            code: "INST".to_string(),
            org_type: "Institution".to_string(),
        };
        let mut organizations = HashMap::new();
        organizations.insert(default_root_id, default_root_org);
        let roles: Vec<RoleRow> = Vec::new();
        let datasets = HashMap::new();
        let relationships = HashMap::new();
        let process_instances = HashMap::new();
        let automations: HashMap<String, Vec<AutomationRule>> = HashMap::new();

        let now = Utc::now().to_rfc3339();
        let entry_0 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "system@scaffoldry.internal".to_string(),
            organization_code: "INST".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional sovereign genesis ledger entry".to_string(),
            payload: &json!({"system": "scaffoldry", "genesis": true}),
        });
        let ledger = vec![entry_0];
        let sessions = HashMap::new();
        let workspaces = HashMap::new();
        let collaborators: HashMap<String, Vec<CollaboratorRecord>> = HashMap::new();

        let mut api_tokens = HashMap::new();
        let scim_hash = crate::service::identity::hash_token("test-scim-token");
        let default_scim_token = ApiToken {
            token_hash: scim_hash.clone(),
            id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            kind: "scim".to_string(),
            eppn: "jordan.lee@state.edu".to_string(),
            label: "SCIM Provisioning Credential".to_string(),
            original_admin: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(365),
            last_used_at: None,
            revoked_at: None,
        };
        api_tokens.insert(scim_hash, default_scim_token);

        let mut settings_map = HashMap::new();
        settings_map.insert("cors.allowed_origins".to_string(), serde_json::json!([]));
        settings_map.insert("tokens.max_days".to_string(), serde_json::json!(90));
        settings_map.insert("tokens.agent_enabled".to_string(), serde_json::json!(true));
        settings_map.insert("process.stale_days".to_string(), serde_json::json!(14));
        settings_map.insert("mcp.disabled_tools".to_string(), serde_json::json!([]));
        settings_map.insert("pages.disabled".to_string(), serde_json::json!([]));

        Self {
            users: RwLock::new(users),
            groups: RwLock::new(groups),
            workspaces: RwLock::new(workspaces),
            collaborators: RwLock::new(collaborators),
            records: RwLock::new(HashMap::new()),
            datasets: RwLock::new(datasets),
            relationships: RwLock::new(relationships),
            automations: RwLock::new(automations),
            process_instances: RwLock::new(process_instances),
            ledger: RwLock::new(ledger),
            engine: RwLock::new(engine),
            policy_engine,
            sessions: RwLock::new(sessions),
            api_tokens: RwLock::new(api_tokens),
            repository: None,
            organizations: RwLock::new(organizations),
            roles: RwLock::new(roles),
            settings: RwLock::new(settings_map),
        }
    }

    pub fn seed_demo(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = chrono::Utc::now().to_rfc3339();
        let default_root_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let mut datasets = HashMap::new();
        let mut relationships = HashMap::new();
        // 1. Faculty Roster & PI Directory
        datasets.insert(
            "faculty".to_string(),
            PublishedDataset {
                id: "faculty".to_string(),
                name: "Faculty & Principal Investigator Directory".to_string(),
                description: "Institutional faculty roster, appointments, and research affiliations".to_string(),
                department: "Academic Affairs".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Directory".to_string(),
                herm_capability_id: Some("HR-01-ROSTER".to_string()),
                fields: vec![
                    DatasetField { name: "eppn".to_string(), label: "Identity (ePPN)".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000033".to_string()) },
                    DatasetField { name: "full_name".to_string(), label: "Full Name".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000115".to_string()) },
                    DatasetField { name: "title".to_string(), label: "Academic Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "department".to_string(), label: "Department".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "research_specialty".to_string(), label: "Research Specialty".to_string(), field_type: "Text".to_string(), required: false, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 240,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "eppn": "dr.smith@university.edu", "full_name": "Dr. Sarah Smith", "title": "Professor of Physics", "department": "Physics", "research_specialty": "Quantum Lattice Systems" }),
                    json!({ "eppn": "dr.alan@university.edu", "full_name": "Dr. Alan Turing", "title": "Chair of Computing", "department": "Computer Science", "research_specialty": "Automata & Cryptography" }),
                ],
            },
        );

        // 2. Academic Programs & Degrees
        datasets.insert(
            "programs".to_string(),
            PublishedDataset {
                id: "programs".to_string(),
                name: "Academic Programs & Degrees".to_string(),
                description: "Accredited degree programs, CIP taxonomy, and departmental authority".to_string(),
                department: "Provost & Academic Council".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Public".to_string(),
                herm_capability_id: Some("ACA-02-CURRICULUM".to_string()),
                fields: vec![
                    DatasetField { name: "code".to_string(), label: "Program Code".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000067".to_string()) },
                    DatasetField { name: "degree_name".to_string(), label: "Degree Name".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "department".to_string(), label: "Governing Department".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 58,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "code": "PHYS-PHD", "degree_name": "Doctor of Philosophy in Physics", "department": "Physics" }),
                    json!({ "code": "CS-BS", "degree_name": "Bachelor of Science in Computer Science", "department": "Computer Science" }),
                ],
            },
        );

        // 3. University Course Catalog
        datasets.insert(
            "courses".to_string(),
            PublishedDataset {
                id: "courses".to_string(),
                name: "University Course Catalog".to_string(),
                description: "Active course roster, schedule units, and designated faculty instructors".to_string(),
                department: "Office of the Registrar".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Directory".to_string(),
                herm_capability_id: Some("REG-01-CATALOG".to_string()),
                fields: vec![
                    DatasetField { name: "course_code".to_string(), label: "Course Code".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000062".to_string()) },
                    DatasetField { name: "course_title".to_string(), label: "Course Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: Some("000064".to_string()) },
                    DatasetField { name: "credits".to_string(), label: "Credit Units".to_string(), field_type: "Number".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "instructor_eppn".to_string(), label: "Lead Instructor".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "program_code".to_string(), label: "Program Plan".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                ],
                record_count: 840,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "course_code": "PHYS-401", "course_title": "Quantum Mechanics I", "credits": 4, "instructor_eppn": "dr.smith@university.edu", "program_code": "PHYS-PHD" }),
                    json!({ "course_code": "CS-302", "course_title": "Theory of Computation", "credits": 3, "instructor_eppn": "dr.alan@university.edu", "program_code": "CS-BS" }),
                ],
            },
        );

        // 4. Sponsored Research Grants
        datasets.insert(
            "grants".to_string(),
            PublishedDataset {
                id: "grants".to_string(),
                name: "Sponsored Research Projects & Grants".to_string(),
                description: "Federal and private grant awards with FERPA-sensitive student stipends".to_string(),
                department: "Office of Sponsored Research".to_string(),
                organization: "University".to_string(),
                sensitivity_level: "Restricted / FERPA".to_string(),
                herm_capability_id: Some("RES-01-GRANTS".to_string()),
                fields: vec![
                    DatasetField { name: "award_number".to_string(), label: "Award Identifier".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "project_title".to_string(), label: "Project Title".to_string(), field_type: "Text".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "pi_eppn".to_string(), label: "Principal Investigator".to_string(), field_type: "Relation".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "amount".to_string(), label: "Total Award Amount".to_string(), field_type: "Number".to_string(), required: true, ferpa_sensitive: false, ceds_code: None },
                    DatasetField { name: "is_ferpa_restricted".to_string(), label: "Contains Student Assistant Data".to_string(), field_type: "Boolean".to_string(), required: false, ferpa_sensitive: true, ceds_code: None },
                ],
                record_count: 115,
                published_at: now.clone(),
                sample_data: vec![
                    json!({ "award_number": "NSF-PHY-2026-01", "project_title": "Quantum Lattice Topological Phases", "pi_eppn": "dr.smith@university.edu", "amount": 750000, "is_ferpa_restricted": true }),
                ],
            },
        );

        // Seed Relationships
        relationships.insert(
            "rel_course_instructor".to_string(),
            DatasetRelationship {
                id: "rel_course_instructor".to_string(),
                name: "Course Instructor Lookup".to_string(),
                source_dataset_id: "courses".to_string(),
                target_dataset_id: "faculty".to_string(),
                source_field: "instructor_eppn".to_string(),
                target_field: "eppn".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "full_name".to_string(),
            },
        );

        relationships.insert(
            "rel_course_program".to_string(),
            DatasetRelationship {
                id: "rel_course_program".to_string(),
                name: "Course Degree Plan".to_string(),
                source_dataset_id: "courses".to_string(),
                target_dataset_id: "programs".to_string(),
                source_field: "program_code".to_string(),
                target_field: "code".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "degree_name".to_string(),
            },
        );

        relationships.insert(
            "rel_grant_pi".to_string(),
            DatasetRelationship {
                id: "rel_grant_pi".to_string(),
                name: "Grant Principal Investigator".to_string(),
                source_dataset_id: "grants".to_string(),
                target_dataset_id: "faculty".to_string(),
                source_field: "pi_eppn".to_string(),
                target_field: "eppn".to_string(),
                relationship_type: RelationshipType::OneToMany,
                display_field: "full_name".to_string(),
            },
        );

        let demo_app = AppManifest {
            slug: "physics-admissions-review".to_string(),
            title: "Physics Admissions Review".to_string(),
            description: "Graduate admissions review and fellowship nomination portal for Physics.".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            department: "physics".to_string(),
            workspace_id: Some("ws-physics-optics".to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            tables: vec![],
            relationships: vec![],
            views: vec![],
            ceds_mappings: HashMap::new(),
        };
        let _ = self.engine.write().unwrap().register_manifest(demo_app.clone());

        let mut automations = HashMap::new();
        automations.insert(
            "physics-admissions-review".to_string(),
            vec![AutomationRule {
                id: "rule-admissions-auto-approve".to_string(),
                app_slug: "physics-admissions-review".to_string(),
                name: "Honors Fellowship Notification".to_string(),
                description: "Notifies dean and records ledger entry when candidate GPA exceeds 3.85".to_string(),
                enabled: true,
                trigger: TriggerEvent::StatusChanged { to_status: "Approved".to_string() },
                cedar_policy_guard: Some("policy-ferpa-34cfr99".to_string()),
                predicates: vec![FieldPredicate {
                    field_name: "gpa".to_string(),
                    operator: ConditionOperator::GreaterThan,
                    expected_value: "3.85".to_string(),
                }],
                actions: vec![
                    ActionType::NotifyCollaborator {
                        role: "dean".to_string(),
                        message_template: "Candidate approved for fellowship funding".to_string(),
                    },
                    ActionType::CreateLedgerAuditEntry {
                        summary: "Automated fellowship approval recorded".to_string(),
                        oscal_control: "AC-03".to_string(),
                    },
                ],
                steps: vec![],
            }],
        );



        let entry_0_hash = self.ledger.read().unwrap().first().map(|e| e.entry_hash.clone()).unwrap_or_else(|| GENESIS_PREVIOUS_HASH.to_string());
        let entry_0 = LedgerEntry {
            sequence: 0,
            timestamp_iso: now.clone(),
            previous_hash: GENESIS_PREVIOUS_HASH.to_string(),
            principal: "system@scaffoldry.internal".to_string(),
            organization_code: "INST".to_string(),
            app_slug: None,
            decision_type: DecisionType::PolicyRevision,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional sovereign genesis ledger entry".to_string(),
            payload_hash: String::new(),
            entry_hash: entry_0_hash,
        };
        let entry_1 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 1,
            timestamp_iso: now.clone(),
            previous_hash: entry_0.entry_hash.clone(),
            principal: "prof.curie@science.state.edu".to_string(),
            organization_code: "DIV-SCIENCES".to_string(),
            app_slug: Some("biology-lab-inventory".to_string()),
            decision_type: DecisionType::VanityDnsBound,
            oscal_control_id: "SC-07".to_string(),
            rationale: "Vanity DNS alias bound with sovereign gateway validation".to_string(),
            payload: &json!({"domain": "inventory.biology.state.edu", "verified": true}),
        });

        let entry_2 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 2,
            timestamp_iso: now.clone(),
            previous_hash: entry_1.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("physics-admissions-review".to_string()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: "FERPA compliance and GPA threshold auto-admit rule approved".to_string(),
            payload: &json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}),
        });

        let entry_3 = LedgerEntry::new(NewLedgerEntryParams {
            sequence: 3,
            timestamp_iso: now.clone(),
            previous_hash: entry_2.entry_hash.clone(),
            principal: "dr.watson@science.state.edu".to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            app_slug: Some("compliance-ferpa-requests".to_string()),
            decision_type: DecisionType::StatutoryAttestation,
            oscal_control_id: "AU-02".to_string(),
            rationale: "Institutional statutory compliance attestation for 34 CFR Part 99".to_string(),
            payload: &json!({"framework": "FERPA", "standard": "34 CFR Part 99"}),
        });



        let mut workspaces = HashMap::new();
        let mut collaborators: HashMap<String, Vec<CollaboratorRecord>> = HashMap::new();
        // 1. Biology Research Laboratory (Restricted)
        let ws_bio = WorkspaceRecord {
            id: "ws-bio-lab".to_string(),
            name: "Biology Research Laboratory".to_string(),
            code: "BIO".to_string(),
            organization: "College of Sciences".to_string(),
            department: "biology".to_string(),
            description: "Collaborative research protocols, instrumentation registers, and specimen data manifests.".to_string(),
            icon: "🔬".to_string(),
            lead: "Dr. Marie Curie".to_string(),
            visibility: "restricted".to_string(),
            allowed_affiliations: vec!["faculty".to_string(), "staff".to_string(), "student".to_string()],
            data_classification: "Restricted".to_string(),
            cedar_policy_guard: Some(r#"forbid (principal, action == Action::"access_workspace", resource) when { resource.visibility == "restricted" && resource.is_member == false };"#.to_string()),
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-bio-lab".to_string(), ws_bio);
        collaborators.insert(
            "ws-bio-lab".to_string(),
            vec![
                CollaboratorRecord {
                    id: "collab-bio-1".to_string(),
                    workspace_id: "ws-bio-lab".to_string(),
                    eppn: "prof.curie@science.state.edu".to_string(),
                    name: "Dr. Marie Curie".to_string(),
                    role: "owner".to_string(),
                    scoped_affiliation: "faculty".to_string(),
                    department: "biology".to_string(),
                    added_at: now.clone(),
                },
                CollaboratorRecord {
                    id: "collab-bio-2".to_string(),
                    workspace_id: "ws-bio-lab".to_string(),
                    eppn: "student.smith@science.state.edu".to_string(),
                    name: "Alex Smith".to_string(),
                    role: "editor".to_string(),
                    scoped_affiliation: "student".to_string(),
                    department: "biology".to_string(),
                    added_at: now.clone(),
                },
                CollaboratorRecord {
                    id: "collab-bio-3".to_string(),
                    workspace_id: "ws-bio-lab".to_string(),
                    eppn: "marcus.vance@state.edu".to_string(),
                    name: "Marcus Vance".to_string(),
                    role: "viewer".to_string(),
                    scoped_affiliation: "staff".to_string(),
                    department: "Office of Sponsored Programs".to_string(),
                    added_at: now.clone(),
                },
            ],
        );

        // 2. Physics & Quantum Optics (Restricted)
        let ws_phys = WorkspaceRecord {
            id: "ws-physics-optics".to_string(),
            name: "Physics & Quantum Optics".to_string(),
            code: "PHYS".to_string(),
            organization: "College of Sciences".to_string(),
            department: "physics".to_string(),
            description: "High-energy laser logs, quantum optics sensor arrays, and space utilization manifests.".to_string(),
            icon: "⚡".to_string(),
            lead: "Albert Einstein".to_string(),
            visibility: "restricted".to_string(),
            allowed_affiliations: vec!["faculty".to_string(), "student".to_string()],
            data_classification: "Internal".to_string(),
            cedar_policy_guard: None,
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-physics-optics".to_string(), ws_phys);
        collaborators.insert(
            "ws-physics-optics".to_string(),
            vec![
                CollaboratorRecord {
                    id: "collab-phys-1".to_string(),
                    workspace_id: "ws-physics-optics".to_string(),
                    eppn: "einstein@physics.state.edu".to_string(),
                    name: "Albert Einstein".to_string(),
                    role: "owner".to_string(),
                    scoped_affiliation: "student".to_string(),
                    department: "physics".to_string(),
                    added_at: now.clone(),
                },
            ],
        );

        // 3. Campus Compliance & Privacy (Departmental)
        let ws_comp = WorkspaceRecord {
            id: "ws-campus-compliance".to_string(),
            name: "Campus Compliance & Privacy".to_string(),
            code: "COMPLIANCE".to_string(),
            organization: "Office of the General Counsel".to_string(),
            department: "compliance".to_string(),
            description: "Institutional FERPA disclosure registers, export control logs, and statutory audit records.".to_string(),
            icon: "🛡️".to_string(),
            lead: "Dr. Arthur Watson".to_string(),
            visibility: "departmental".to_string(),
            allowed_affiliations: vec!["staff".to_string(), "compliance".to_string()],
            data_classification: "FERPA Sensitive".to_string(),
            cedar_policy_guard: None,
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-campus-compliance".to_string(), ws_comp);
        collaborators.insert(
            "ws-campus-compliance".to_string(),
            vec![
                CollaboratorRecord {
                    id: "collab-comp-1".to_string(),
                    workspace_id: "ws-campus-compliance".to_string(),
                    eppn: "dr.watson@science.state.edu".to_string(),
                    name: "Dr. Arthur Watson".to_string(),
                    role: "owner".to_string(),
                    scoped_affiliation: "staff".to_string(),
                    department: "compliance".to_string(),
                    added_at: now.clone(),
                },
                CollaboratorRecord {
                    id: "collab-comp-2".to_string(),
                    workspace_id: "ws-campus-compliance".to_string(),
                    eppn: "elena.rodriguez@state.edu".to_string(),
                    name: "Elena Rodriguez".to_string(),
                    role: "admin".to_string(),
                    scoped_affiliation: "compliance".to_string(),
                    department: "Institutional Review Board".to_string(),
                    added_at: now.clone(),
                },
            ],
        );

        // 4. Computer Science & Systems Lab (Restricted)
        let ws_cs = WorkspaceRecord {
            id: "ws-cs-research".to_string(),
            name: "Computer Science & Systems Lab".to_string(),
            code: "CS".to_string(),
            organization: "College of Engineering".to_string(),
            department: "Computer Science".to_string(),
            description: "Distributed systems, sovereign agent computing, and verifiable lattice architectures.".to_string(),
            icon: "💻".to_string(),
            lead: "Dr. Sarah Connor".to_string(),
            visibility: "restricted".to_string(),
            allowed_affiliations: vec!["faculty".to_string(), "staff".to_string()],
            data_classification: "Internal".to_string(),
            cedar_policy_guard: None,
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-cs-research".to_string(), ws_cs);
        collaborators.insert(
            "ws-cs-research".to_string(),
            vec![
                CollaboratorRecord {
                    id: "collab-cs-1".to_string(),
                    workspace_id: "ws-cs-research".to_string(),
                    eppn: "sarah.connor@state.edu".to_string(),
                    name: "Dr. Sarah Connor".to_string(),
                    role: "owner".to_string(),
                    scoped_affiliation: "faculty".to_string(),
                    department: "Computer Science".to_string(),
                    added_at: now.clone(),
                },
            ],
        );



        if let Some(ref repo) = self.repository {
            for ws in workspaces.values() {
                let _ = repo.upsert_workspace(ws);
            }
            for list in collaborators.values() {
                for c in list {
                    let _ = repo.upsert_collaborator(c);
                }
            }
            for ds in datasets.values() {
                let _ = repo.upsert_published_dataset(ds);
            }
            for rel in relationships.values() {
                let _ = repo.upsert_dataset_relationship(rel);
            }
            let _ = repo.upsert_app_manifest(&demo_app);
            for rules in automations.values() {
                for r in rules {
                    let _ = repo.upsert_workflow_automation(r);
                }
            }
            let _ = repo.append_ledger_entry(&entry_1, &json!({"domain": "inventory.biology.state.edu", "verified": true}));
            let _ = repo.append_ledger_entry(&entry_2, &json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"}));
            let _ = repo.append_ledger_entry(&entry_3, &json!({"framework": "FERPA", "standard": "34 CFR Part 99"}));
        }

        for (k, v) in workspaces {
            self.workspaces.write().unwrap().insert(k, v);
        }
        for (k, v) in collaborators {
            self.collaborators.write().unwrap().insert(k, v);
        }
        for (k, v) in datasets {
            self.datasets.write().unwrap().insert(k, v);
        }
        for (k, v) in relationships {
            self.relationships.write().unwrap().insert(k, v);
        }
        for (k, v) in automations {
            self.automations.write().unwrap().insert(k, v);
        }
        let _ = self.engine.write().unwrap().register_manifest(demo_app);
        self.ledger.write().unwrap().extend(vec![entry_1, entry_2, entry_3]);

        Ok(())
    }
    pub fn list_api_tokens(&self, eppn: &str) -> Vec<ApiToken> {
        if let Some(ref repo) = self.repository {
            if let Ok(list) = repo.list_api_tokens_for_eppn(eppn) {
                return list;
            }
        }
        self.api_tokens.read().map(|tokens| {
            tokens.values().filter(|t| t.eppn == eppn && t.kind == "agent" && t.revoked_at.is_none()).cloned().collect()
        }).unwrap_or_default()
    }
}

pub type SharedState = Arc<ServerState>;

pub fn lock_err() -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({"error": "Failed to acquire lock: state lock poisoned or unavailable"})),
    )
}
