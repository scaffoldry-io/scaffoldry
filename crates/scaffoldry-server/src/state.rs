//! Server State and Storage for Datasets, Workspaces, and SCIM Identity

use chrono::Utc;
use scaffoldry_core::{
    ActionType, AutomationRule, ConditionOperator, DatasetField,
    DatasetRelationship, DecisionType, FieldPredicate, LedgerEntry, LedgerError,
    PublishedDataset, RelationshipType, TriggerEvent,
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
    pub kind: String, // 'agent' | 'impersonation' | 'scim' | 'setup' | 'viewas'
    pub eppn: String,
    pub label: String,
    pub original_admin: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The one app a `viewas` token reaches. Empty for every other kind.
    #[serde(default)]
    pub app_slug: Option<String>,
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
    pub admin_hold: bool,
    #[serde(default)]
    pub emails: Vec<Value>,
    #[serde(default)]
    pub roles: Vec<Value>,
    #[serde(
        default,
        rename = "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
    )]
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
    #[serde(default = "default_hierarchy_key")]
    pub hierarchy_key: String,
}

pub fn default_hierarchy_key() -> String {
    "academic".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyRecord {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub source: String,
    pub status: String,
    pub created_by: String,
    pub created_at: String,
    pub retired_at: Option<String>,
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
    /// Set when this row is a position holding. `None` for an affiliation or an admin appointment.
    #[serde(default)]
    pub position_key: Option<String>,
}

/// A kind of post an organization unit can have, such as a department chair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionType {
    pub key: String,
    pub name: String,
    pub description: String,
    pub org_types: Vec<String>,
    pub max_holders: i32,
    pub retired_at: Option<String>,
    pub created_by: String,
    pub created_at: String,
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
    pub role: String, // "owner" | "admin" | "editor" | "commenter" | "viewer"
    pub scoped_affiliation: String,
    #[serde(default)]
    pub department: String,
    #[serde(default)]
    pub added_at: String,
    /// The one app this membership reaches. Empty means the whole workspace.
    #[serde(default)]
    pub app_slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    pub id: String,
    pub app_slug: String,
    pub data: Value,
    pub ceds_mapping: Value,
    pub is_ferpa_sensitive: bool,
    pub created_at: String,
    /// The eppn of the principal who created the record. Absent on records written before 0022.
    #[serde(default)]
    pub created_by: Option<String>,
    /// Changes by one on every save. A save names the version it read.
    #[serde(default = "default_record_version")]
    pub version: i32,
}

fn default_record_version() -> i32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub eppn: String,
    pub name: String,
    pub role_title: String,
    pub affiliation: String,
    pub department: String,
    /// Set on a view-as request: the administrator looking. Rights are the intersection of both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_as_by: Option<Box<AuthUser>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub token: String,
    pub user: AuthUser,
    pub original_admin: Option<AuthUser>,
    pub created_at: String,
}

pub struct ServerState {
    /// The stored views of each app (`app_views`). The engine runs each app's manifest with these merged in.
    pub app_views: RwLock<HashMap<String, Vec<crate::repository_views::StoredView>>>,
    /// How long an exact row count may run, in milliseconds, before the answer becomes `at least`. Two seconds unless
    /// `SCAFFOLDRY_COUNT_TIMEOUT_MS` says otherwise. A test lowers it to make a count time out.
    pub count_timeout_ms: std::sync::atomic::AtomicU64,
    pub workspaces: RwLock<HashMap<String, WorkspaceRecord>>,
    pub collaborators: RwLock<HashMap<String, Vec<CollaboratorRecord>>>,
    pub guards: RwLock<HashMap<String, Vec<scaffoldry_core::WorkspaceGuardRecord>>>,
    pub guard_policy_cache: RwLock<HashMap<(String, i32), scaffoldry_policy::PolicySet>>,
    pub datasets: RwLock<HashMap<String, PublishedDataset>>,
    pub relationships: RwLock<HashMap<String, DatasetRelationship>>,
    pub automations: RwLock<HashMap<String, Vec<AutomationRule>>>,
    pub engine: RwLock<ManifestEngine>,
    pub policy_engine: ScaffoldryPolicyEngine,
    pub repository: Arc<crate::repository::PostgresRepository>,
    /// The version of each cached collection this instance holds, in the order of `cache::NAMES`.
    pub cache_seen: [std::sync::atomic::AtomicI64; crate::cache::NAMES.len()],
    pub organizations: RwLock<HashMap<uuid::Uuid, OrganizationNode>>,
    pub tree: RwLock<scaffoldry_core::tree::OrgTree>,
    pub position_types: RwLock<HashMap<String, PositionType>>,
    /// Sensitivity labels by `(app, table, field)`. They can only raise a field's sensitivity.
    /// Empty until the label table exists (admin console phase 4, later part).
    pub labels: RwLock<scaffoldry_engine::LabelSet>,
    pub settings: RwLock<HashMap<String, serde_json::Value>>,
    pub formula_rate_limits: RwLock<HashMap<String, (std::time::Instant, u32)>>,
}

impl ServerState {
    /// Makes this instance agree with the database. It reads every cache counter in one statement and reloads each
    /// collection whose counter differs from the one this instance holds. The counter stored is the one read before
    /// the reload, so a write that lands during the reload is seen by the next call. A reload that fails keeps the old
    /// counter and returns the error, and the next call tries again.
    pub fn refresh(&self) -> Result<(), crate::repository::RepositoryError> {
        use std::sync::atomic::Ordering::SeqCst;
        for (name, version) in self.repository.cache_versions()? {
            let Some(i) = crate::cache::index_of(&name) else {
                continue;
            };
            if self.cache_seen[i].load(SeqCst) == version {
                continue;
            }
            self.reload(&name)?;
            self.cache_seen[i].store(version, SeqCst);
        }
        Ok(())
    }

    /// Reads one stored app definition and puts it in the engine, with the stored views of its app merged in. One that
    /// no longer parses or validates is taken out of service by name and logged. The other apps are not touched, and
    /// the version is noted either way, so a damaged definition is not parsed again until it changes.
    fn load_app(&self, slug: &str) -> Result<(), crate::repository::RepositoryError> {
        let Some((version, value)) = self.repository.get_app_manifest_row(slug)? else { return Ok(()) };
        let own = self.app_views.read().unwrap_or_else(|p| p.into_inner()).get(slug).cloned().unwrap_or_default();
        let outcome = self
            .repository
            .parse_manifest(value)
            .map_err(|e| e.to_string())
            .and_then(|m| self.engine.write().unwrap_or_else(|p| p.into_inner()).register_manifest(crate::repository_views::merged(m, &own)).map_err(|e| e.to_string()));
        let mut engine = self.engine.write().unwrap_or_else(|p| p.into_inner());
        if let Err(reason) = outcome {
            eprintln!("Warning: the stored app '{slug}' no longer parses and is not served: {reason}");
            engine.mark_broken(slug, &reason);
        }
        engine.set_version(slug, version);
        Ok(())
    }

    /// Loads one cached collection again. A collection moves here from a boot-time copy to the mechanism in the phase
    /// that makes it cached. The ones not listed are still read at boot only.
    fn reload(&self, name: &str) -> Result<(), crate::repository::RepositoryError> {
        if name == "settings" {
            let stored = self.repository.get_platform_settings()?;
            let mut held = self.settings.write().unwrap_or_else(|p| p.into_inner());
            for (k, v) in stored {
                if k == "platform.timezone" {
                    if let Some(tz) = v.as_str() {
                        scaffoldry_engine::formula::set_institution_timezone(tz);
                    }
                }
                held.insert(k, v);
            }
        }
        match name {
            "organizations" => {
                let all: HashMap<uuid::Uuid, OrganizationNode> = self
                    .repository
                    .list_organizations()?
                    .into_iter()
                    .map(|o| (o.id, o))
                    .collect();
                let closure = self.repository.list_org_closure()?;
                let tree = scaffoldry_core::tree::OrgTree::new(closure);
                *self
                    .organizations
                    .write()
                    .unwrap_or_else(|p| p.into_inner()) = all;
                *self.tree.write().unwrap_or_else(|p| p.into_inner()) = tree;
            }
            "position_types" => {
                let all: HashMap<String, PositionType> = self
                    .repository
                    .list_position_types()?
                    .into_iter()
                    .map(|p| (p.key.clone(), p))
                    .collect();
                *self
                    .position_types
                    .write()
                    .unwrap_or_else(|p| p.into_inner()) = all;
            }
            "workspaces" => {
                // A workspace and its collaborators are one collection. Both are read before either is replaced.
                let mut workspaces = HashMap::new();
                let mut collaborators = HashMap::new();
                for ws in self.repository.list_workspaces()? {
                    collaborators.insert(ws.id.clone(), self.repository.get_collaborators(&ws.id)?);
                    workspaces.insert(ws.id.clone(), ws);
                }
                *self.workspaces.write().unwrap_or_else(|p| p.into_inner()) = workspaces;
                *self
                    .collaborators
                    .write()
                    .unwrap_or_else(|p| p.into_inner()) = collaborators;
            }
            "guards" => {
                let mut guards: HashMap<String, Vec<scaffoldry_core::WorkspaceGuardRecord>> =
                    HashMap::new();
                for g in self.repository.list_all_workspace_guards()? {
                    guards.entry(g.workspace_id.clone()).or_default().push(g);
                }
                *self.guards.write().unwrap_or_else(|p| p.into_inner()) = guards;
                // The compiled sets are keyed by workspace and guard version, so they are right without this. A
                // reload drops them anyway, so a set built from a guard that was rewritten does not outlive it.
                self.guard_policy_cache
                    .write()
                    .unwrap_or_else(|p| p.into_inner())
                    .clear();
            }
            "datasets" => {
                let all: HashMap<String, PublishedDataset> = self
                    .repository
                    .list_published_datasets()?
                    .into_iter()
                    .map(|d| (d.id.clone(), d))
                    .collect();
                *self.datasets.write().unwrap_or_else(|p| p.into_inner()) = all;
            }
            "relationships" => {
                let all: HashMap<String, DatasetRelationship> = self
                    .repository
                    .list_dataset_relationships()?
                    .into_iter()
                    .map(|r| (r.id.clone(), r))
                    .collect();
                *self
                    .relationships
                    .write()
                    .unwrap_or_else(|p| p.into_inner()) = all;
            }
            _ => {}
        }
        if name == "manifests" {
            // One statement lists the stored versions. Only a definition whose version differs from the one this engine
            // holds is read and parsed, so approving one app among hundreds reloads one.
            let held = self.engine.read().unwrap_or_else(|p| p.into_inner()).versions().clone();
            for (slug, version) in self.repository.app_manifest_versions()? {
                if held.get(&slug) != Some(&version) {
                    self.load_app(&slug)?;
                }
            }
        }
        if name == "rules" {
            let mut by_app: HashMap<String, Vec<AutomationRule>> = HashMap::new();
            for rule in self.repository.list_workflow_automations(None)? {
                by_app.entry(rule.app_slug.clone()).or_default().push(rule);
            }
            *self.automations.write().unwrap_or_else(|p| p.into_inner()) = by_app;
        }
        Ok(())
    }

    /// A server on the database `DATABASE_URL` names, or the one a test named for its thread.
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::connect(&crate::testing::database_url())
    }

    /// A server on this database. It migrates the database and reads everything it holds. When any of that fails
    /// the server does not start.
    pub fn connect(database_url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut engine = ManifestEngine::new()
            .map_err(|e| format!("Failed to initialize ManifestEngine: {e}"))?;
        let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
            .map_err(|e| format!("Failed to initialize PolicyEngine: {e}"))?;

        let mut app_views_cache: HashMap<String, Vec<crate::repository_views::StoredView>> =
            HashMap::new();
        let default_root_id =
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let default_root_org = OrganizationNode {
            id: default_root_id,
            parent_id: None,
            name: "Institution".to_string(),
            code: "INST".to_string(),
            org_type: "Institution".to_string(),
            hierarchy_key: "academic".to_string(),
        };
        let mut organizations = HashMap::new();
        organizations.insert(default_root_id, default_root_org);
        let mut position_types: HashMap<String, PositionType> = HashMap::new();
        let mut datasets = HashMap::new();
        let mut relationships = HashMap::new();
        let mut automations: HashMap<String, Vec<AutomationRule>> = HashMap::new();
        let mut guards: HashMap<String, Vec<scaffoldry_core::WorkspaceGuardRecord>> =
            HashMap::new();

        let mut workspaces = HashMap::new();
        let mut collaborators: HashMap<String, Vec<CollaboratorRecord>> = HashMap::new();
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
            app_slug: None,
        };

        let mut settings_map = HashMap::new();
        settings_map.insert("cors.allowed_origins".to_string(), serde_json::json!([]));
        settings_map.insert("tokens.max_days".to_string(), serde_json::json!(90));
        settings_map.insert("tokens.agent_enabled".to_string(), serde_json::json!(true));
        settings_map.insert("process.stale_days".to_string(), serde_json::json!(14));
        settings_map.insert("mcp.disabled_tools".to_string(), serde_json::json!([]));
        settings_map.insert("pages.disabled".to_string(), serde_json::json!([]));
        settings_map.insert("platform.timezone".to_string(), serde_json::json!("UTC"));
        add_sensitivity_defaults(&mut settings_map);

        let repository = match crate::repository::PostgresRepository::connect(Some(database_url)) {
            Ok(repo) => Arc::new(repo),
            Err(e) => {
                if let crate::repository::RepositoryError::TamperDetected(msg) = &e {
                    return Err(format!("Cryptographic tamper detected in ledger: {msg}").into());
                }
                return Err(format!("Failed to connect to PostgreSQL repository: {e}").into());
            }
        };
        let repo = &*repository;
        // A read that fails stops the server. It does not start with a collection empty and say nothing.
        let read = |what: &str,
                    e: crate::repository::RepositoryError|
         -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to read {what} at start: {e}").into()
        };

        // The counters come first. A write that lands while the rest loads is then seen by the first refresh.
        let versions = repo
            .cache_versions()
            .map_err(|e| read("the cache versions", e))?;

        for (k, v) in repo
            .get_platform_settings()
            .map_err(|e| read("the settings", e))?
        {
            if k == "platform.timezone" {
                if let Some(tz) = v.as_str() {
                    scaffoldry_engine::formula::set_institution_timezone(tz);
                }
            }
            settings_map.insert(k, v);
        }

        // The root organization exists before anything that hangs from it.
        for org in organizations.values() {
            repo.upsert_organization(org)
                .map_err(|e| read("the root organization", e))?;
        }

        // Views live in their own table. A manifest stored with views still in it has them moved, once, and
        // the migration is one ledger entry with the count. What the engine runs is the manifest with the
        // stored views merged back in, so nothing that reads a manifest's views changes.
        for v in repo
            .list_all_app_views()
            .map_err(|e| read("the views", e))?
        {
            app_views_cache
                .entry(v.app_slug.clone())
                .or_default()
                .push(v);
        }
        for (stored_slug, version, value) in repo.list_app_manifest_rows().map_err(|e| read("the app manifests", e))? {
            // A definition that does not parse is that app's problem. The server starts and serves the others.
            let mut m = match repo.parse_manifest(value) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Warning: the stored app '{stored_slug}' no longer parses and is not served: {e}");
                    engine.mark_broken(&stored_slug, &e.to_string());
                    engine.set_version(&stored_slug, version);
                    continue;
                }
            };
            if !m.views.is_empty() {
                if let Some(done) = repo
                    .migrate_manifest_views(&m.slug)
                    .map_err(|e| read("the view migration", e))?
                {
                    let count = done.views.len();
                    repo
                        .append_ledger_decision(crate::repository::AppendDecisionParams {
                            principal: "system@scaffoldry.local".to_string(),
                            organization_code: m.organization_code.clone(),
                            app_slug: Some(m.slug.clone()),
                            decision_type: scaffoldry_core::ledger::DecisionType::ViewsMigrated,
                            oscal_control_id: "CM-03".to_string(),
                            rationale: format!("Moved {count} views of {} out of the manifest into their own table", m.slug),
                            payload: serde_json::json!({ "app_slug": m.slug, "views": count }),
                        })
                        .map_err(|e| read("the ledger", e))?;
                    app_views_cache.entry(m.slug.clone()).or_default().extend(done.views);
                    m = done.manifest;
                }
            }
            let views = app_views_cache.get(&m.slug).cloned().unwrap_or_default();
            let slug = m.slug.clone();
            match engine.register_manifest(crate::repository_views::merged(m, &views)) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Warning: the stored app '{slug}' no longer validates and is not served: {e}");
                    engine.mark_broken(&slug, &e.to_string());
                }
            }
            engine.set_version(&stored_slug, version);
        }

        for ds in repo
            .list_published_datasets()
            .map_err(|e| read("the datasets", e))?
        {
            datasets.insert(ds.id.clone(), ds);
        }
        for rel in repo
            .list_dataset_relationships()
            .map_err(|e| read("the relationships", e))?
        {
            relationships.insert(rel.id.clone(), rel);
        }
        for r in repo
            .list_workflow_automations(None)
            .map_err(|e| read("the automations", e))?
        {
            let entry = automations.entry(r.app_slug.clone()).or_default();
            if !entry.iter().any(|existing| existing.id == r.id) {
                entry.push(r);
            }
        }

        for org in repo
            .list_organizations()
            .map_err(|e| read("the organizations", e))?
        {
            organizations.insert(org.id, org);
        }
        let closure = repo
            .list_org_closure()
            .map_err(|e| read("the org closure", e))?;
        let tree = scaffoldry_core::tree::OrgTree::new(closure);
        for pt in repo
            .list_position_types()
            .map_err(|e| read("the position types", e))?
        {
            position_types.insert(pt.key.clone(), pt);
        }

        for g in repo
            .list_all_workspace_guards()
            .map_err(|e| read("the workspace guards", e))?
        {
            guards.entry(g.workspace_id.clone()).or_default().push(g);
        }

        let archived_count = repo
            .count_legacy_workspace_guards()
            .map_err(|e| read("the archived guards", e))?;
        if archived_count > 0 {
            let already_recorded = repo
                .ledger_has_rationale(scaffoldry_core::ledger::DecisionType::PolicyRevision, "Archived legacy workspace guards")
                .map_err(|e| read("the ledger", e))?;
            if !already_recorded {
                let payload = serde_json::json!({
                    "archived_rows": archived_count,
                    "table": "workspace_guards_legacy"
                });
                repo.append_ledger_decision(crate::repository::AppendDecisionParams {
                    principal: "system@scaffoldry.local".to_string(),
                    organization_code: "INST".to_string(),
                    app_slug: None,
                    decision_type: scaffoldry_core::ledger::DecisionType::PolicyRevision,
                    oscal_control_id: "AC-03".to_string(),
                    rationale: format!("Archived legacy workspace guards ({} rows)", archived_count),
                    payload,
                })
                .map_err(|e| read("the ledger", e))?;
            }
        }
        for ws in repo
            .list_workspaces()
            .map_err(|e| read("the workspaces", e))?
        {
            let collabs = repo
                .get_collaborators(&ws.id)
                .map_err(|e| read("the collaborators", e))?;
            collaborators.insert(ws.id.clone(), collabs);
            workspaces.insert(ws.id.clone(), ws);
        }

        repo.insert_api_token(&default_scim_token)
            .map_err(|e| read("the provisioning token", e))?;

        // The ledger stays in the database. A database with none gets the genesis entry.
        if repo.ledger_summary().map_err(|e| read("the ledger", e))?.0 == 0 {
            repo.append_ledger_decision(crate::repository::AppendDecisionParams {
                principal: "system@scaffoldry.internal".to_string(),
                organization_code: "INST".to_string(),
                app_slug: None,
                decision_type: DecisionType::PolicyRevision,
                oscal_control_id: "AU-02".to_string(),
                rationale: "Institutional sovereign genesis ledger entry".to_string(),
                payload: json!({"system": "scaffoldry", "genesis": true}),
            })
            .map_err(|e| read("the ledger", e))?;
        }

        let cache_seen: [std::sync::atomic::AtomicI64; crate::cache::NAMES.len()] =
            std::array::from_fn(|i| {
                let version = versions
                    .iter()
                    .find(|(n, _)| n == crate::cache::NAMES[i])
                    .map(|(_, v)| *v)
                    .unwrap_or(-1);
                std::sync::atomic::AtomicI64::new(version)
            });

        Ok(Self {
            app_views: RwLock::new(app_views_cache),
            count_timeout_ms: std::sync::atomic::AtomicU64::new(
                std::env::var("SCAFFOLDRY_COUNT_TIMEOUT_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2_000),
            ),
            workspaces: RwLock::new(workspaces),
            collaborators: RwLock::new(collaborators),
            guards: RwLock::new(guards),
            guard_policy_cache: RwLock::new(HashMap::new()),
            datasets: RwLock::new(datasets),
            relationships: RwLock::new(relationships),
            automations: RwLock::new(automations),
            engine: RwLock::new(engine),
            policy_engine,
            repository,
            cache_seen,
            organizations: RwLock::new(organizations),
            tree: RwLock::new(tree),
            position_types: RwLock::new(position_types),
            labels: RwLock::new(scaffoldry_engine::LabelSet::new()),
            settings: RwLock::new(settings_map),
            formula_rate_limits: RwLock::new(HashMap::new()),
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
    /// Stores a unit, then makes this instance read it back. The write commits first and bumps `organizations`, so
    /// memory never holds a unit the database refused and another instance reads it on its next request.
    pub fn persist_organization(
        &self,
        org: &OrganizationNode,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_organization(org)?;
        self.refresh()?;
        Ok(())
    }

    pub fn delete_scim_roles(
        &self,
        eppn: &str,
        keep_admin_rows: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository
            .delete_scim_roles_for_user(eppn, keep_admin_rows)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // RECORDS. PostgreSQL holds the rows. Nothing is loaded at boot.
    // -------------------------------------------------------------------------

    pub fn list_app_records(&self, app_slug: &str) -> Result<Vec<DatasetRecord>, String> {
        {
            let repo = &self.repository;
            repo.list_records(app_slug).map_err(|e| e.to_string())
        }
    }

    /// One page of a table in `(created_at, id)` order, after `cursor` when given.
    pub fn list_records_page(
        &self,
        app_slug: &str,
        table_id: &str,
        limit: usize,
        cursor: Option<(String, String)>,
    ) -> Result<Vec<DatasetRecord>, String> {
        {
            let repo = &self.repository;
            repo.list_records_page(app_slug, table_id, limit, cursor)
                .map_err(|e| e.to_string())
        }
    }

    /// One page that filters or sorts.
    pub fn query_records(
        &self,
        app_slug: &str,
        query: &crate::repository::RecordQuery,
    ) -> Result<Vec<DatasetRecord>, String> {
        {
            let repo = &self.repository;
            repo.query_records(app_slug, query)
                .map_err(|e| e.to_string())
        }
    }

    /// Aggregate over record_index.
    pub fn aggregate_records(
        &self,
        plan: &crate::repository_index::AggregatePlan,
    ) -> Result<crate::repository_index::AggregateResultRaw, String> {
        {
            let repo = &self.repository;
            repo.aggregate_records(plan).map_err(|e| e.to_string())
        }
    }

    /// The percent done of the build holding back one of `fields`, or `None` when none is building.
    pub fn index_building_percent(
        &self,
        app_slug: &str,
        table_id: &str,
        fields: &[String],
    ) -> Result<Option<i64>, String> {
        {
            let repo = &self.repository;
            repo.index_building_percent(app_slug, table_id, fields)
                .map_err(|e| e.to_string())
        }
    }

    /// One page through the pivot index.
    pub fn indexed_page(
        &self,
        query: &crate::repository::IndexedQuery,
    ) -> Result<Vec<DatasetRecord>, String> {
        {
            let repo = &self.repository;
            repo.indexed_page(query).map_err(|e| e.to_string())
        }
    }

    /// How many records point at `record_id` through any of `fields`.
    pub fn count_links_to(
        &self,
        app_slug: &str,
        record_id: &str,
        fields: &[String],
    ) -> Result<i64, String> {
        {
            let repo = &self.repository;
            repo.count_links_to(app_slug, record_id, fields)
                .map_err(|e| e.to_string())
        }
    }

    /// Which of `ids` are records of this table of this app.
    pub fn existing_record_ids(
        &self,
        app_slug: &str,
        table_id: &str,
        ids: &[String],
    ) -> Result<Vec<String>, String> {
        {
            let repo = &self.repository;
            repo.existing_record_ids(app_slug, table_id, ids)
                .map_err(|e| e.to_string())
        }
    }

    /// True when a table holds more than `limit` rows.
    pub fn table_has_more_than(
        &self,
        app_slug: &str,
        table_id: &str,
        limit: i64,
    ) -> Result<bool, String> {
        {
            let repo = &self.repository;
            repo.table_has_more_than(app_slug, table_id, limit)
                .map_err(|e| e.to_string())
        }
    }

    /// Deletes a record only if it is still at `expected_version`.
    pub fn remove_record_checked(
        &self,
        app_slug: &str,
        id: &str,
        expected_version: i32,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<crate::repository::RecordWrite, String> {
        {
            let repo = &self.repository;
            repo.delete_record_checked(app_slug, id, expected_version, ctx)
                .map_err(|e| e.to_string())
        }
    }

    pub fn count_app_records(&self, app_slug: &str) -> Result<usize, String> {
        Ok(self.list_app_records(app_slug)?.len())
    }

    pub fn find_record(&self, app_slug: &str, id: &str) -> Result<Option<DatasetRecord>, String> {
        {
            let repo = &self.repository;
            repo.get_record(app_slug, id).map_err(|e| e.to_string())
        }
    }

    /// A new record, at version 1.
    pub fn insert_record(
        &self,
        rec: &DatasetRecord,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<(), crate::repository::RepositoryError> {
        self.repository.insert_record(rec, ctx)
    }

    /// Inserts the record, or replaces the stored one and bumps its version. For fixtures.
    pub fn put_record(
        &self,
        rec: &DatasetRecord,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<(), String> {
        {
            let repo = &self.repository;
            repo.upsert_record(rec, ctx).map_err(|e| e.to_string())
        }
    }

    /// Saves new data when the stored version is the one the caller read.
    pub fn save_record_checked(
        &self,
        app_slug: &str,
        id: &str,
        data: &Value,
        expected_version: i32,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<
        (crate::repository::RecordWrite, Option<DatasetRecord>),
        crate::repository::RepositoryError,
    > {
        use crate::repository::RecordWrite;
        let outcome =
            self.repository
                .update_record_checked(app_slug, id, data, expected_version, ctx)?;
        let saved = if matches!(outcome, RecordWrite::Saved(_)) {
            self.repository.get_record(app_slug, id)?
        } else {
            None
        };
        Ok((outcome, saved))
    }

    /// A write by the system, such as an automation. It always applies and still bumps the version.
    pub fn overwrite_record_data(
        &self,
        app_slug: &str,
        id: &str,
        data: &Value,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<bool, String> {
        {
            let repo = &self.repository;
            repo.overwrite_record_data(app_slug, id, data, ctx)
                .map(|v| v.is_some())
                .map_err(|e| e.to_string())
        }
    }

    pub fn remove_record(
        &self,
        app_slug: &str,
        id: &str,
        ctx: &crate::repository::WriteCtx,
    ) -> Result<bool, String> {
        {
            let repo = &self.repository;
            repo.delete_record(app_slug, id, ctx)
                .map_err(|e| e.to_string())
        }
    }

    pub fn persist_position_type(
        &self,
        pt: &PositionType,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_position_type(pt)?;
        self.refresh()?;
        Ok(())
    }

    pub fn delete_role(
        &self,
        id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.delete_role(id)?;
        Ok(())
    }

    pub fn persist_role(
        &self,
        role: &RoleRow,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_role(role)?;
        Ok(())
    }

    /// The appointments and holdings of one person.
    pub fn roles_of(&self, eppn: &str) -> Vec<RoleRow> {
        self.repository.roles_of_person(eppn).unwrap_or_default()
    }

    /// The ids of the groups a person belongs to. A group lists a member by id or by user name.
    pub fn group_ids_of(&self, eppn: &str) -> Vec<String> {
        let mut values = vec![eppn.to_string()];
        values.extend(self.user_by_name(eppn).map(|u| u.id));
        self.repository
            .group_ids_with_member(&values)
            .unwrap_or_default()
    }

    /// The user with this id, read now.
    pub fn user_by_id(&self, id: &str) -> Option<ScimUser> {
        self.repository.user_by_id(id).ok().flatten()
    }

    /// The user with this name, ignoring case.
    pub fn user_by_name(&self, name: &str) -> Option<ScimUser> {
        self.repository.user_by_name(name).ok().flatten()
    }

    pub fn persist_workspace(
        &self,
        ws: WorkspaceRecord,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_workspace(&ws)?;
        self.refresh()?;
        Ok(())
    }

    pub fn get_workspace(&self, id: &str) -> Option<WorkspaceRecord> {
        {
            let repo = &self.repository;
            if let Ok(Some(ws)) = repo.get_workspace(id) {
                return Some(ws);
            }
        }
        self.workspaces.read().unwrap().get(id).cloned()
    }

    pub fn persist_workspaces_batch(
        &self,
        batch: &[WorkspaceRecord],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.persist_workspaces_batch(batch)?;
        self.refresh()?;
        Ok(())
    }

    pub fn count_workspaces(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.repository.count_workspaces()?)
    }

    pub fn persist_app_manifest(&self, m: AppManifest) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_app_manifest(&m)?;
        self.refresh()?;
        Ok(())
    }

    pub fn get_app_manifest(&self, slug: &str) -> Option<AppManifest> {
        {
            let repo = &self.repository;
            if let Ok(Some(m)) = repo.get_app_manifest(slug) {
                return Some(m);
            }
        }
        use scaffoldry_engine::HostRouter;
        self.engine.read().ok()?.resolve_by_slug(slug).cloned()
    }

    pub fn persist_dataset(
        &self,
        ds: PublishedDataset,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_published_dataset(&ds)?;
        self.refresh()?;
        Ok(())
    }

    pub fn get_dataset(&self, id: &str) -> Option<PublishedDataset> {
        self.datasets.read().ok()?.get(id).cloned()
    }

    pub fn persist_relationship(
        &self,
        rel: DatasetRelationship,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_dataset_relationship(&rel)?;
        self.refresh()?;
        Ok(())
    }

    pub fn persist_automation(&self, rule: AutomationRule) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_workflow_automation(&rule)?;
        self.refresh()?;
        Ok(())
    }

    pub fn persist_scim_user(
        &self,
        user: ScimUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.upsert_scim_user(&user)?;
        Ok(())
    }

    pub fn get_scim_user(&self, id: &str) -> Option<ScimUser> {
        self.user_by_id(id)
    }

    pub fn platform_timezone(&self) -> String {
        self.settings
            .read()
            .ok()
            .and_then(|s| {
                s.get("platform.timezone")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "UTC".to_string())
    }

    pub fn append_ledger_entry(
        &self,
        input: RecordDecisionInput<'_>,
    ) -> Result<LedgerEntry, LedgerError> {
        let entry = self.repository.append_ledger_decision(crate::repository::AppendDecisionParams {
            principal: input.principal.clone(),
            organization_code: input.organization_code.clone(),
            app_slug: input.app_slug.clone(),
            decision_type: input.decision_type,
            oscal_control_id: input.oscal_control_id.clone(),
            rationale: input.rationale.clone(),
            payload: input.payload.clone(),
        }).map_err(|e| LedgerError::RepositoryError(e.to_string()))?;
        Ok(entry)
    }

    pub fn verify_ledger(&self) -> Result<bool, LedgerError> {
        match self.repository.verify_and_initialize_ledger(false) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub fn export_oscal_component_definition(&self) -> Value {
        // The export names every entry, oldest first. It reads the chain a page at a time.
        let mut entries: Vec<LedgerEntry> = Vec::new();
        let mut before = None;
        while let Ok(page) = self.repository.ledger_page(before, crate::repository::LEDGER_BATCH) {
            let Some(last) = page.last() else { break };
            before = Some(last.sequence);
            entries.extend(page);
        }
        entries.reverse();
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

    pub fn persist_api_token(
        &self,
        token: &ApiToken,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.insert_api_token(token)?;
        Ok(())
    }

    pub fn get_api_token(&self, hash: &str) -> Option<ApiToken> {
        self.repository.get_api_token(hash).ok().flatten()
    }

    pub fn update_token_last_used(
        &self,
        hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.repository.update_token_last_used(hash, now)?;
        Ok(())
    }

    pub fn revoke_api_token(
        &self,
        id: uuid::Uuid,
        caller_eppn: &str,
        is_platform_admin: bool,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .repository
            .revoke_api_token(id, caller_eppn, is_platform_admin)?)
    }

    pub fn revoke_api_token_by_hash(
        &self,
        hash: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.repository.revoke_api_token_by_hash(hash)?)
    }

    pub fn check_formula_rate_limit(&self, user_key: &str) -> bool {
        let mut limits = match self.formula_rate_limits.write() {
            Ok(g) => g,
            Err(_) => return true,
        };
        let now = std::time::Instant::now();
        if let Some((start_time, count)) = limits.get_mut(user_key) {
            if now.duration_since(*start_time).as_secs() >= 1 {
                *start_time = now;
                *count = 1;
                true
            } else if *count < 20 {
                *count += 1;
                true
            } else {
                false
            }
        } else {
            limits.insert(user_key.to_string(), (now, 1));
            true
        }
    }

    pub fn seed_demo(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = chrono::Utc::now().to_rfc3339();
        let default_root_id =
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

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
            description: "Graduate admissions review and fellowship nomination portal for Physics."
                .to_string(),
            organization_code: "DIV-COMPLIANCE".to_string(),
            department: "physics".to_string(),
            workspace_id: Some("ws-physics-optics".to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            tables: vec![],
            relationships: vec![],
            views: vec![],
            pages: vec![],
            access: Default::default(),
            ceds_mappings: HashMap::new(),
        };

        let mut automations = HashMap::new();
        automations.insert(
            "physics-admissions-review".to_string(),
            vec![AutomationRule {
                version: 0,
                id: "rule-admissions-auto-approve".to_string(),
                app_slug: "physics-admissions-review".to_string(),
                name: "Honors Fellowship Notification".to_string(),
                description:
                    "Notifies dean and records ledger entry when candidate GPA exceeds 3.85"
                        .to_string(),
                enabled: true,
                trigger: TriggerEvent::StatusChanged {
                    to_status: "Approved".to_string(),
                },
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



        // The demo ledger entries, written once, when the ledger holds nothing but its genesis entry.
        let demo_entries = [
            ("prof.curie@science.state.edu", "DIV-SCIENCES", "biology-lab-inventory", DecisionType::VanityDnsBound, "SC-07", "Vanity DNS alias bound with sovereign gateway validation", json!({"domain": "inventory.biology.state.edu", "verified": true})),
            ("dr.watson@science.state.edu", "DIV-COMPLIANCE", "physics-admissions-review", DecisionType::WorkflowRuleApproved, "AC-03", "FERPA compliance and GPA threshold auto-admit rule approved", json!({"rule": "auto-physics-honors-admit", "policy": "policy-ferpa-34cfr99"})),
            ("dr.watson@science.state.edu", "DIV-COMPLIANCE", "compliance-ferpa-requests", DecisionType::StatutoryAttestation, "AU-02", "Institutional statutory compliance attestation for 34 CFR Part 99", json!({"framework": "FERPA", "standard": "34 CFR Part 99"})),
        ];

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
                    app_slug: String::new(),
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
                    app_slug: String::new(),
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
                    app_slug: String::new(),
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
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-physics-optics".to_string(), ws_phys);
        collaborators.insert(
            "ws-physics-optics".to_string(),
            vec![CollaboratorRecord {
                id: "collab-phys-1".to_string(),
                workspace_id: "ws-physics-optics".to_string(),
                eppn: "einstein@physics.state.edu".to_string(),
                name: "Albert Einstein".to_string(),
                role: "owner".to_string(),
                scoped_affiliation: "student".to_string(),
                department: "physics".to_string(),
                added_at: now.clone(),
                app_slug: String::new(),
            }],
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
                    app_slug: String::new(),
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
                    app_slug: String::new(),
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
            created_at: now.clone(),
            organization_id: Some(default_root_id),
        };
        workspaces.insert("ws-cs-research".to_string(), ws_cs);
        collaborators.insert(
            "ws-cs-research".to_string(),
            vec![CollaboratorRecord {
                id: "collab-cs-1".to_string(),
                workspace_id: "ws-cs-research".to_string(),
                eppn: "sarah.connor@state.edu".to_string(),
                name: "Dr. Sarah Connor".to_string(),
                role: "owner".to_string(),
                scoped_affiliation: "faculty".to_string(),
                department: "Computer Science".to_string(),
                added_at: now.clone(),
                app_slug: String::new(),
            }],
        );

        // The demo set is compiled in. Each row is written only when its id is absent, and then it
        // is the institution's data. A later boot never writes it over an edit.
        let stored_workspaces: std::collections::HashSet<String> = {
            let repo = &self.repository;
            repo.list_workspaces()
                .unwrap_or_default()
                .into_iter()
                .map(|w| w.id)
                .collect()
        };
        let stored_datasets: std::collections::HashSet<String> = {
            let repo = &self.repository;
            repo.list_published_datasets()
                .unwrap_or_default()
                .into_iter()
                .map(|d| d.id)
                .collect()
        };
        let stored_relationships: std::collections::HashSet<String> = {
            let repo = &self.repository;
            repo.list_dataset_relationships()
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        let stored_rules: std::collections::HashSet<String> = {
            let repo = &self.repository;
            repo.list_workflow_automations(None)
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        let manifest_stored = {
            let repo = &self.repository;
            repo.get_app_manifest(&demo_app.slug)
                .ok()
                .flatten()
                .is_some()
        };

        // A workspace that is stored stays as stored, collaborators included.
        workspaces.retain(|id, _| !stored_workspaces.contains(id));
        collaborators.retain(|id, _| workspaces.contains_key(id));
        datasets.retain(|id, _| !stored_datasets.contains(id));
        relationships.retain(|id, _| !stored_relationships.contains(id));
        for rules in automations.values_mut() {
            rules.retain(|r| !stored_rules.contains(&r.id));
        }
        automations.retain(|_, rules| !rules.is_empty());

        {
            let repo = &self.repository;
            for ws in workspaces.values() {
                repo.upsert_workspace(ws)?;
            }
            for list in collaborators.values() {
                for c in list {
                    repo.upsert_collaborator(c)?;
                }
            }
            for ds in datasets.values() {
                repo.upsert_published_dataset(ds)?;
            }
            for rel in relationships.values() {
                repo.upsert_dataset_relationship(rel)?;
            }
            if !manifest_stored {
                repo.upsert_app_manifest(&demo_app)?;
            }
            for rules in automations.values() {
                for r in rules {
                    repo.upsert_workflow_automation(r)?;
                }
            }
        }

        if self.repository.ledger_summary()?.0 <= 1 {
            for (principal, org, slug, decision_type, control, rationale, payload) in demo_entries {
                let _ = self.repository.append_ledger_decision(crate::repository::AppendDecisionParams {
                    principal: principal.to_string(),
                    organization_code: org.to_string(),
                    app_slug: Some(slug.to_string()),
                    decision_type,
                    oscal_control_id: control.to_string(),
                    rationale: rationale.to_string(),
                    payload,
                });
            }
        }

        // The demo rows are stored. This instance reads them back, as it reads any other write.
        self.refresh()?;

        Ok(())
    }
    /// Every token the user holds, of every kind and state, newest first.
    pub fn list_all_api_tokens(&self, eppn: &str) -> Vec<ApiToken> {
        self.repository
            .list_all_api_tokens_for_eppn(eppn)
            .unwrap_or_default()
    }

    pub fn list_api_tokens(&self, eppn: &str) -> Vec<ApiToken> {
        self.repository
            .list_api_tokens_for_eppn(eppn)
            .unwrap_or_default()
    }
}

/// A new installation has the `pii` and `pci` presets on. A stored setting replaces these, so a
/// later release never changes what an organization has.
fn add_sensitivity_defaults(settings: &mut HashMap<String, serde_json::Value>) {
    let (categories, detectors) = scaffoldry_engine::sensitivity::default_settings();
    settings.insert(
        "sensitivity.categories".to_string(),
        serde_json::json!(categories),
    );
    settings.insert(
        "sensitivity.detectors".to_string(),
        serde_json::json!(detectors),
    );
}

impl ServerState {
    /// The organization's sensitivity categories, as settings hold them now.
    pub fn sensitivity_categories(&self) -> Vec<scaffoldry_engine::sensitivity::Category> {
        self.settings
            .read()
            .ok()
            .and_then(|s| s.get("sensitivity.categories").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

pub type SharedState = Arc<ServerState>;

pub fn lock_err() -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    crate::service::ServiceError::internal(
        "Failed to acquire lock: state lock poisoned or unavailable",
    )
    .into_pair()
}
