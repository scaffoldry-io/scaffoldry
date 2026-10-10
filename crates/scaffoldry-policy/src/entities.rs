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
    pub is_member: bool,
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

impl PrincipalCtx {
    pub fn entity(&self) -> Value {
        entity(
            "User",
            &self.eppn,
            json!({
                "eppn": self.eppn,
                "name": self.name,
                "scoped_affiliation": self.scoped_affiliation,
                "department": self.department,
                "unit_ids": self.unit_ids,
                "is_platform_admin": self.is_platform_admin,
            }),
        )
    }
}

impl WorkspaceCtx {
    /// A non-member always has the empty string, whatever role text the caller holds.
    pub fn entity_member_role(&self) -> &str {
        if self.is_member {
            &self.member_role
        } else {
            ""
        }
    }

    pub fn entity(&self) -> Value {
        entity(
            "Workspace",
            &self.workspace_id,
            json!({
                "workspace_id": self.workspace_id,
                "department": self.department,
                "visibility": self.visibility,
                "data_classification": self.data_classification,
                "member_role": self.entity_member_role(),
                "unit_id": self.unit_id,
                "is_member": self.is_member,
            }),
        )
    }
}

impl AppCtx {
    pub fn entity(&self) -> Value {
        entity(
            "App",
            &self.slug,
            json!({
                "slug": self.slug,
                "department": self.department,
                "workspace_id": self.workspace_id,
            }),
        )
    }
}

impl RecordCtx {
    pub fn id(&self) -> String {
        format!("{}-record-target", self.app_slug)
    }

    pub fn entity(&self) -> Value {
        entity(
            "Record",
            &self.id(),
            json!({
                "app_slug": self.app_slug,
                "department": self.department,
                "workspace_id": self.workspace_id,
                "is_ferpa_sensitive": self.is_ferpa_sensitive,
            }),
        )
    }
}

impl SystemCtx {
    pub fn entity(&self) -> Value {
        entity(
            "System",
            &self.id,
            json!({
                "department": self.department,
                "is_ferpa_sensitive": self.is_ferpa_sensitive,
            }),
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


/// Builds the entities for one request and evaluates it against an explicit policy set.
pub fn authorize_with_policy_set(
    engine: &ScaffoldryPolicyEngine,
    policies: &cedar_policy::PolicySet,
    principal: &PrincipalCtx,
    action: &str,
    resource: &Resource,
) -> Result<AuthorizationResult, PolicyError> {
    engine.evaluate_with_policy_set(
        policies,
        vec![principal.entity(), resource.entity()],
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
    engine.evaluate(
        vec![principal.entity(), resource.entity()],
        ("User", &principal.eppn),
        action,
        (resource.type_name(), &resource.id()),
    )
}

/// Every unit where the person holds a role, plus all ancestors of each.
///
/// `parents` maps a unit id to its parent id (`None` for the root). Each walk stops after
/// `MAX_UNIT_WALK` steps. The result is sorted and has no duplicates.
pub fn unit_ids_for(role_unit_ids: &[String], parents: &HashMap<String, Option<String>>) -> Vec<String> {
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
