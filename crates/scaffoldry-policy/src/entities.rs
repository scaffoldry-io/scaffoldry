//! One typed builder for every Cedar entity, and the single `authorize` entry point.
//!
//! The shapes here match `schema/scaffoldry.cedarschema`. Every attribute is required.

use crate::{AuthorizationResult, PolicyError, ScaffoldryPolicyEngine};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

/// Ancestor walks stop after this many steps, so a cycle in bad data cannot loop.
pub const MAX_UNIT_WALK: usize = 32;

#[derive(Debug, Clone)]
pub struct PrincipalCtx {
    pub eppn: String,
    pub name: String,
    pub scoped_affiliation: String,
    pub department: String,
    pub unit_ids: Vec<String>,
    /// The units where the person is a unit administrator. Not their descendants: the workspace carries its ancestors.
    pub is_platform_admin: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceCtx {
    pub workspace_id: String,
    pub department: String,
    pub visibility: String,
    pub data_classification: String,
    pub member_role: String,
    pub unit_id: String,
    /// The workspace's unit and every ancestor of it, so a unit administrator of any of them matches.
    pub unit_path_ids: Vec<String>,
    pub is_member: bool,
    /// Set when the question is about one app in the workspace: whether its fields hold protected data, and the
    /// categories of those fields. A plain workspace question leaves them false and empty.
    pub is_ferpa_sensitive: bool,
    pub categories: std::collections::BTreeSet<String>,
    /// The domain the workspace sits in, or the app when the question is about one, and every domain above it.
    pub domain_path: std::collections::BTreeSet<String>,
    /// The place of its sensitivity on the ladder: 0 Public, 1 Directory, 2 Internal, 3 Restricted.
    pub sensitivity_rank: i64,
}

#[derive(Debug, Clone)]
pub struct AppCtx {
    pub slug: String,
    pub department: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone)]
pub struct RecordCtx {
    pub app_slug: String,
    pub department: String,
    pub workspace_id: String,
    pub is_ferpa_sensitive: bool,
    /// The sensitivity categories of the fields this record holds. No default policy reads them.
    /// A workspace guard can name one.
    pub categories: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct SystemCtx {
    pub id: String,
    pub department: String,
    pub is_ferpa_sensitive: bool,
}

#[derive(Debug, Clone)]
pub enum Resource {
    Workspace(WorkspaceCtx),
    App(AppCtx),
    Record(RecordCtx),
    System(SystemCtx),
}

fn entity(type_name: &str, id: &str, attrs: Value) -> Value {
    json!({ "uid": { "type": type_name, "id": id }, "attrs": attrs, "parents": [] })
}

fn entity_with_parents(type_name: &str, id: &str, attrs: Value, parents: Vec<Value>) -> Value {
    json!({ "uid": { "type": type_name, "id": id }, "attrs": attrs, "parents": parents })
}

impl PrincipalCtx {
    pub fn effective_units(&self) -> Vec<String> {
        let mut units = self.unit_ids.clone();
        if !self.department.is_empty() && !units.contains(&self.department) {
            units.push(self.department.clone());
        }
        units
    }

    pub fn entity(&self) -> Value {
        let units_val: Vec<Value> = self
            .effective_units()
            .iter()
            .map(|u| json!({ "type": "Unit", "id": u }))
            .collect();
        entity(
            "User",
            &self.eppn,
            json!({
                "eppn": self.eppn,
                "name": self.name,
                "scoped_affiliation": self.scoped_affiliation,
                "department": self.department,
                "units": units_val,
                "unit_ids": self.unit_ids,
                "is_platform_admin": self.is_platform_admin,
            }),
        )
    }
}

impl WorkspaceCtx {
    pub fn effective_unit_id(&self) -> String {
        if !self.unit_id.is_empty() {
            self.unit_id.clone()
        } else if !self.department.is_empty() {
            self.department.clone()
        } else {
            "default".to_string()
        }
    }

    /// A non-member always has the empty string, whatever role text the caller holds.
    pub fn entity_member_role(&self) -> &str {
        if self.is_member {
            &self.member_role
        } else {
            ""
        }
    }

    pub fn entity(&self) -> Value {
        let uid = self.effective_unit_id();
        let parents = vec![json!({ "type": "Unit", "id": uid })];
        entity_with_parents(
            "Workspace",
            &self.workspace_id,
            json!({
                "workspace_id": self.workspace_id,
                "department": self.department,
                "visibility": self.visibility,
                "data_classification": self.data_classification,
                "member_role": self.entity_member_role(),
                "unit": { "type": "Unit", "id": uid },
                "unit_id": self.unit_id,
                "unit_path_ids": self.unit_path_ids,
                "is_member": self.is_member,
                "is_ferpa_sensitive": self.is_ferpa_sensitive,
                "categories": self.categories,
                "domain_path": self.domain_path,
                "sensitivity_rank": self.sensitivity_rank,
            }),
            parents,
        )
    }
}

impl AppCtx {
    pub fn effective_unit_id(&self) -> String {
        if !self.department.is_empty() {
            self.department.clone()
        } else {
            "default".to_string()
        }
    }

    pub fn entity(&self) -> Value {
        let uid = self.effective_unit_id();
        let parents = vec![json!({ "type": "Unit", "id": uid })];
        entity_with_parents(
            "App",
            &self.slug,
            json!({
                "slug": self.slug,
                "department": self.department,
                "workspace_id": self.workspace_id,
                "unit": { "type": "Unit", "id": uid },
            }),
            parents,
        )
    }
}

impl RecordCtx {
    pub fn effective_unit_id(&self) -> String {
        if !self.department.is_empty() {
            self.department.clone()
        } else {
            "default".to_string()
        }
    }

    pub fn id(&self) -> String {
        format!("{}-record-target", self.app_slug)
    }

    pub fn entity(&self) -> Value {
        let uid = self.effective_unit_id();
        let parents = vec![json!({ "type": "Unit", "id": uid })];
        entity_with_parents(
            "Record",
            &self.id(),
            json!({
                "app_slug": self.app_slug,
                "department": self.department,
                "workspace_id": self.workspace_id,
                "unit": { "type": "Unit", "id": uid },
                "is_ferpa_sensitive": self.is_ferpa_sensitive,
                "categories": self.categories,
            }),
            parents,
        )
    }
}

impl SystemCtx {
    pub fn effective_unit_id(&self) -> String {
        if !self.department.is_empty() {
            self.department.clone()
        } else {
            "central_admin".to_string()
        }
    }

    pub fn entity(&self) -> Value {
        let uid = self.effective_unit_id();
        let parents = vec![json!({ "type": "Unit", "id": uid })];
        entity_with_parents(
            "System",
            &self.id,
            json!({
                "department": self.department,
                "unit": { "type": "Unit", "id": uid },
                "is_ferpa_sensitive": self.is_ferpa_sensitive,
            }),
            parents,
        )
    }
}

impl Resource {
    pub fn type_name(&self) -> &'static str {
        match self {
            Resource::Workspace(_) => "Workspace",
            Resource::App(_) => "App",
            Resource::Record(_) => "Record",
            Resource::System(_) => "System",
        }
    }

    pub fn id(&self) -> String {
        match self {
            Resource::Workspace(w) => w.workspace_id.clone(),
            Resource::App(a) => a.slug.clone(),
            Resource::Record(r) => r.id(),
            Resource::System(s) => s.id.clone(),
        }
    }

    pub fn entity(&self) -> Value {
        match self {
            Resource::Workspace(w) => w.entity(),
            Resource::App(a) => a.entity(),
            Resource::Record(r) => r.entity(),
            Resource::System(s) => s.entity(),
        }
    }
}

fn collect_unit_entities(principal: &PrincipalCtx, resource: &Resource) -> Vec<Value> {
    let mut unit_parents: HashMap<String, Vec<String>> = HashMap::new();

    for u in principal.effective_units() {
        unit_parents.entry(u).or_default();
    }

    match resource {
        Resource::Workspace(w) => {
            let wid = w.effective_unit_id();
            unit_parents.entry(wid).or_default();
            let path = &w.unit_path_ids;
            for i in 0..path.len() {
                unit_parents.entry(path[i].clone()).or_default();
                if i + 1 < path.len() {
                    let parent = path[i + 1].clone();
                    let entry = unit_parents.entry(path[i].clone()).or_default();
                    if !entry.contains(&parent) {
                        entry.push(parent);
                    }
                }
            }
        }
        Resource::App(a) => {
            unit_parents.entry(a.effective_unit_id()).or_default();
        }
        Resource::Record(r) => {
            unit_parents.entry(r.effective_unit_id()).or_default();
        }
        Resource::System(s) => {
            unit_parents.entry(s.effective_unit_id()).or_default();
        }
    }

    let mut entities = Vec::new();
    for (unit_id, parents) in unit_parents {
        let parent_uids: Vec<Value> = parents
            .iter()
            .map(|p| json!({ "type": "Unit", "id": p }))
            .collect();
        entities.push(json!({
            "uid": { "type": "Unit", "id": unit_id },
            "attrs": { "hierarchy": "academic" },
            "parents": parent_uids,
        }));
    }
    entities
}

/// Builds the entities for one request and evaluates it against an explicit policy set.
pub fn authorize_with_policy_set(
    engine: &ScaffoldryPolicyEngine,
    policies: &cedar_policy::PolicySet,
    principal: &PrincipalCtx,
    action: &str,
    resource: &Resource,
) -> Result<AuthorizationResult, PolicyError> {
    let mut entities = vec![principal.entity(), resource.entity()];
    entities.extend(collect_unit_entities(principal, resource));
    engine.evaluate_with_policy_set(
        policies,
        entities,
        ("User", &principal.eppn),
        action,
        (resource.type_name(), &resource.id()),
    )
}

/// Builds the entities for one request and evaluates it. Every decision goes through here.
pub fn authorize(
    engine: &ScaffoldryPolicyEngine,
    principal: &PrincipalCtx,
    action: &str,
    resource: &Resource,
) -> Result<AuthorizationResult, PolicyError> {
    let mut entities = vec![principal.entity(), resource.entity()];
    entities.extend(collect_unit_entities(principal, resource));
    engine.evaluate(
        entities,
        ("User", &principal.eppn),
        action,
        (resource.type_name(), &resource.id()),
    )
}

/// Every unit where the person holds a role, plus all ancestors of each.
///
/// `parents` maps a unit id to its parent id (`None` for the root). Each walk stops after
/// `MAX_UNIT_WALK` steps. The result is sorted and has no duplicates.
pub fn unit_ids_for(
    role_unit_ids: &[String],
    parents: &HashMap<String, Option<String>>,
) -> Vec<String> {
    let mut found = BTreeSet::new();
    for start in role_unit_ids {
        let mut current = Some(start.clone());
        let mut steps = 0;
        while let Some(id) = current {
            if steps > MAX_UNIT_WALK {
                break;
            }
            steps += 1;
            if !found.insert(id.clone()) {
                // Already walked from here. Its ancestors are already in the set.
                break;
            }
            current = parents.get(&id).cloned().flatten();
        }
    }
    found.into_iter().collect()
}
