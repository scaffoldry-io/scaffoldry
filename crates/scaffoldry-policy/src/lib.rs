//! Scaffoldry Sovereign Policy and Authorization Engine (Cedar Policy Integration)
//!
//! One Cedar schema (`schema/scaffoldry.cedarschema`) types every entity and every policy.
//! One builder (`entities`) makes the entities. One function (`entities::authorize`) decides.
//! The four `authorize_*` methods below are thin wrappers over it.

pub mod entities;

pub use cedar_policy::{Effect, Policy, PolicyId, PolicySet};

use cedar_policy::{
    Authorizer, Context, Decision, EntityId, EntityTypeName, EntityUid, Entities,
    Request, Schema, ValidationMode, Validator,
};
use entities::{authorize, PrincipalCtx, RecordCtx, Resource, SystemCtx, WorkspaceCtx};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::str::FromStr;
use thiserror::Error;

const SCHEMA_SRC: &str = include_str!("../schema/scaffoldry.cedarschema");

pub fn validate_raw_source(source: &str) -> Result<PolicySet, PolicyError> {
    if source.len() > 8192 {
        return Err(PolicyError::PolicyParseError(format!(
            "Raw Cedar source length {} exceeds maximum limit of 8192 bytes",
            source.len()
        )));
    }
    let policies: PolicySet = source
        .parse()
        .map_err(|e: cedar_policy::ParseErrors| PolicyError::PolicyParseError(e.to_string()))?;

    for p in policies.policies() {
        if p.effect() != Effect::Forbid {
            return Err(PolicyError::PolicyParseError(format!(
                "Raw Cedar policy '{}' is a permit; guards may only forbid",
                p.id()
            )));
        }
    }

    let (schema, _) = Schema::from_cedarschema_str(SCHEMA_SRC)
        .map_err(|e| PolicyError::PolicyParseError(format!("schema: {e}")))?;

    let result = Validator::new(schema).validate(&policies, ValidationMode::Strict);
    if !result.validation_passed() {
        let errors: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
        return Err(PolicyError::PolicyParseError(errors.join("; ")));
    }

    Ok(policies)
}

pub fn parse_guard_policy(
    id: &str,
    text: &str,
) -> Result<Policy, PolicyError> {
    let policy_id = PolicyId::new(id);
    let policy = Policy::parse(Some(policy_id), text)
        .map_err(|e: cedar_policy::ParseErrors| PolicyError::PolicyParseError(e.to_string()))?;
    if policy.effect() != Effect::Forbid {
        return Err(PolicyError::PolicyParseError(format!(
            "Guard policy '{}' must be forbid",
            id
        )));
    }
    Ok(policy)
}

#[derive(Error, Debug)]
pub enum PolicyError {
    #[error("Failed to parse Cedar policy set: {0}")]
    PolicyParseError(String),

    #[error("Failed to construct Cedar entity set: {0}")]
    EntityError(String),

    #[error("Failed to build authorization request: {0}")]
    RequestError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny,
}

/// A policy named in a decision, with the plain sentence an owner can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRef {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationResult {
    pub decision: PolicyDecision,
    pub reasons: Vec<String>,
    pub diagnostics: Vec<String>,
    /// For a denial, the first matching `forbid`. `None` for an allow and for an implicit deny.
    pub deciding_policy: Option<PolicyRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationRequest {
    pub principal: String,
    pub action: String,
    pub resource: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceActionInput<'a> {
    pub principal_eppn: &'a str,
    pub principal_affiliation: &'a str,
    pub principal_department: &'a str,
    pub action_name: &'a str,
    pub workspace_id: &'a str,
    pub workspace_department: &'a str,
    pub workspace_visibility: &'a str,
    pub is_member: bool,
    pub member_role: Option<&'a str>,
}

pub struct ScaffoldryPolicyEngine {
    authorizer: Authorizer,
    policies: PolicySet,
    schema: Schema,
    source: std::sync::Arc<str>,
}

impl Clone for ScaffoldryPolicyEngine {
    fn clone(&self) -> Self {
        Self {
            authorizer: Authorizer::new(),
            policies: self.policies.clone(),
            schema: self.schema.clone(),
            source: self.source.clone(),
        }
    }
}

fn entity_uid(type_name: &str, id: &str) -> Result<EntityUid, PolicyError> {
    let type_name = EntityTypeName::from_str(type_name)
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;
    Ok(EntityUid::from_type_name_and_id(type_name, EntityId::new(id)))
}

impl ScaffoldryPolicyEngine {
    /// Parses the policies and type-checks them against the schema. A policy that does not
    /// type-check is an error.
    pub fn new(policy_src: &str) -> Result<Self, PolicyError> {
        let policies: PolicySet = policy_src
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::PolicyParseError(e.to_string()))?;
        let (schema, _warnings) = Schema::from_cedarschema_str(SCHEMA_SRC)
            .map_err(|e| PolicyError::PolicyParseError(format!("schema: {e}")))?;

        let result = Validator::new(schema.clone()).validate(&policies, ValidationMode::Strict);
        if !result.validation_passed() {
            let errors: Vec<String> = result.validation_errors().map(|e| e.to_string()).collect();
            return Err(PolicyError::PolicyParseError(errors.join("; ")));
        }

        Ok(Self {
            authorizer: Authorizer::new(),
            policies,
            schema,
            source: policy_src.into(),
        })
    }

    /// The Cedar text this engine was built from, exactly as given.
    pub fn policy_source(&self) -> &str {
        &self.source
    }

    /// The id and plain description of every policy in the set, in set order.
    pub fn policies(&self) -> &PolicySet {
        &self.policies
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn evaluate_with_policy_set(
        &self,
        policies: &PolicySet,
        entity_values: Vec<serde_json::Value>,
        principal: (&str, &str),
        action_name: &str,
        resource: (&str, &str),
    ) -> Result<AuthorizationResult, PolicyError> {
        let entities = Entities::from_json_value(serde_json::Value::Array(entity_values), Some(&self.schema))
            .map_err(|e| PolicyError::EntityError(e.to_string()))?;

        let request = Request::new(
            entity_uid(principal.0, principal.1)?,
            entity_uid("Action", action_name)?,
            entity_uid(resource.0, resource.1)?,
            Context::empty(),
            None,
        )
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;

        let response = self.authorizer.is_authorized(&request, policies, &entities);

        let decision = match response.decision() {
            Decision::Allow => PolicyDecision::Allow,
            Decision::Deny => PolicyDecision::Deny,
        };

        let reason_ids: HashSet<&PolicyId> = response.diagnostics().reason().collect();
        let reasons = response.diagnostics().reason().map(|r| r.to_string()).collect();
        let diagnostics = response.diagnostics().errors().map(|e| e.to_string()).collect();

        let deciding_policy = if decision == PolicyDecision::Deny {
            policies
                .policies()
                .find(|p| reason_ids.contains(p.id()))
                .map(|p| PolicyRef {
                    id: p.id().to_string(),
                    description: p.annotation("description").unwrap_or("").to_string(),
                })
        } else {
            None
        };

        Ok(AuthorizationResult {
            decision,
            reasons,
            diagnostics,
            deciding_policy,
        })
    }
    pub fn policy_summaries(&self) -> Vec<PolicyRef> {
        self.policies
            .policies()
            .map(|p| PolicyRef {
                id: p.id().to_string(),
                description: p.annotation("description").unwrap_or("").to_string(),
            })
            .collect()
    }

    pub fn default_institutional_engine() -> Result<Self, PolicyError> {
        let default_policies = r#"
            // 1. Permit departmental members to read and write records in their own department
            @description("Members of a department can read and write that department's records.")
            permit (
                principal,
                action in [Action::"read", Action::"write", Action::"run_automation"],
                resource
            )
            when {
                principal.department == resource.department ||
                principal.scoped_affiliation == "central_admin"
            };

            // 2. Strict FERPA Policy: Forbid exporting records if sensitive,
            // unless principal holds verified staff or compliance affiliation
            @description("Only staff, compliance and central administrators can export sensitive student records.")
            forbid (
                principal,
                action == Action::"export",
                resource
            )
            when {
                resource.is_ferpa_sensitive &&
                principal.scoped_affiliation != "staff" &&
                principal.scoped_affiliation != "compliance" &&
                principal.scoped_affiliation != "central_admin"
            };

            // 3. Permit authorized departmental staff and faculty to export non-restricted records
            @description("People can export records that belong to their own department.")
            permit (
                principal,
                action == Action::"export",
                resource
            )
            when {
                principal.department == resource.department
            };

            // 4. Permit central_admin to access institutional administration console
            @description("Only central administrators can open the administration console.")
            permit (
                principal,
                action == Action::"access_admin",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 5. Permit central_admin to impersonate directory users
            @description("Central administrators can impersonate users.")
            permit (
                principal,
                action == Action::"impersonate",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 6. Explicitly forbid any non-admin principal from impersonating users
            @description("Nobody except a central administrator can impersonate a user.")
            forbid (
                principal,
                action == Action::"impersonate",
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin"
            };

            // 7. Permit central_admin to access, manage, and delete any workspace
            @description("Central administrators can use, manage and delete any workspace.")
            permit (
                principal,
                action in [Action::"access_workspace", Action::"manage_workspace", Action::"delete_workspace"],
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 8. Permit workspace access to verified members
            @description("Members can open their workspace.")
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.is_member == true
            };

            // 9. Permit workspace access if visibility is departmental and principal matches department
            @description("People can open the departmental workspaces of their own department.")
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.visibility == "departmental" &&
                principal.department == resource.department
            };

            // 10. Permit workspace access if visibility is institutional
            @description("Everyone can open institutional workspaces.")
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.visibility == "institutional"
            };

            // 11. Forbid workspace access if visibility is restricted and principal is not a member and not central_admin
            @description("Only members and central administrators can open restricted workspaces.")
            forbid (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.visibility == "restricted" &&
                resource.is_member == false &&
                principal.scoped_affiliation != "central_admin"
            };

            // 12. Permit workspace management to owners and admins
            @description("Workspace owners and admins can manage and delete their workspace.")
            permit (
                principal,
                action in [Action::"manage_workspace", Action::"delete_workspace"],
                resource is Workspace
            )
            when {
                resource.member_role == "owner" ||
                resource.member_role == "admin"
            };

            // 13. Forbid workspace management to non-owner/admin members unless central_admin
            @description("Only workspace owners, admins and central administrators can manage or delete a workspace.")
            forbid (
                principal,
                action in [Action::"manage_workspace", Action::"delete_workspace"],
                resource is Workspace
            )
            when {
                resource.member_role != "owner" &&
                resource.member_role != "admin" &&
                principal.scoped_affiliation != "central_admin"
            };

            // 14. Permit faculty and staff to create, update, and publish apps in their department, and central_admin everywhere
            @description("Faculty and staff can create, update and publish apps in their department.")
            permit (
                principal,
                action in [Action::"create_app", Action::"update_app", Action::"publish_app"],
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                ((principal.scoped_affiliation == "faculty" || principal.scoped_affiliation == "staff") && principal.department == resource.department)
            };

            // 15. Forbid non-faculty/staff/admin (e.g. students, affiliates) from updating or publishing apps
            @description("Students and affiliates cannot create, update or publish apps.")
            forbid (
                principal,
                action in [Action::"create_app", Action::"update_app", Action::"publish_app"],
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin" &&
                principal.scoped_affiliation != "faculty" &&
                principal.scoped_affiliation != "staff"
            };

            // 16. Permit faculty and staff to publish datasets in their department, and central_admin everywhere
            @description("Faculty and staff can publish datasets from their department.")
            permit (
                principal,
                action == Action::"publish_dataset",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                ((principal.scoped_affiliation == "faculty" || principal.scoped_affiliation == "staff") && principal.department == resource.department)
            };

            // 17. Forbid students and affiliates from publishing datasets
            @description("Students and affiliates cannot publish datasets.")
            forbid (
                principal,
                action == Action::"publish_dataset",
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin" &&
                principal.scoped_affiliation != "faculty" &&
                principal.scoped_affiliation != "staff"
            };

            // 18. Permit central_admin and compliance staff to record decisions in the governance ledger
            @description("Faculty, staff, compliance and central administrators can record decisions in the ledger.")
            permit (
                principal,
                action == Action::"record_decision",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                principal.scoped_affiliation == "compliance" ||
                principal.scoped_affiliation == "faculty" ||
                principal.scoped_affiliation == "staff"
            };

            // 19. Forbid students and affiliates from recording decisions in the governance ledger
            @description("Students and affiliates cannot record decisions in the ledger.")
            forbid (
                principal,
                action == Action::"record_decision",
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin" &&
                principal.scoped_affiliation != "compliance" &&
                principal.scoped_affiliation != "faculty" &&
                principal.scoped_affiliation != "staff"
            };

            // 20. Permit faculty, staff, and central_admin to approve workflow decisions
            @description("Faculty, staff, compliance and central administrators can approve workflow steps.")
            permit (
                principal,
                action == Action::"approve",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                principal.scoped_affiliation == "compliance" ||
                principal.scoped_affiliation == "faculty" ||
                principal.scoped_affiliation == "staff"
            };

            // 21. Forbid students and affiliates from approving workflow decisions
            @description("Students and affiliates cannot approve workflow steps.")
            forbid (
                principal,
                action == Action::"approve",
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin" &&
                principal.scoped_affiliation != "compliance" &&
                principal.scoped_affiliation != "faculty" &&
                principal.scoped_affiliation != "staff"
            };

            // 22. Permit workspace collaborators and central_admin to read app
            @description("Workspace members and central administrators can read the apps in a workspace.")
            permit (
                principal,
                action == Action::"read_app",
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                resource.is_member == true
            };

            // 23. Permit workspace owner, admin, and editor to write records
            @description("Workspace owners, admins and editors can change records.")
            permit (
                principal,
                action == Action::"write_record",
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                (resource.is_member == true && (
                    resource.member_role == "owner" ||
                    resource.member_role == "admin" ||
                    resource.member_role == "editor"
                ))
            };

            // 24. Permit workspace owner and admin to manage app
            @description("Workspace owners and admins can manage apps.")
            permit (
                principal,
                action == Action::"manage_app",
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                (resource.is_member == true && (
                    resource.member_role == "owner" ||
                    resource.member_role == "admin"
                ))
            };
        "#;
        Self::new(default_policies)
    }

    /// Evaluates one request. Called only by `entities::authorize`.
    pub(crate) fn evaluate(
        &self,
        entity_values: Vec<serde_json::Value>,
        principal: (&str, &str),
        action_name: &str,
        resource: (&str, &str),
    ) -> Result<AuthorizationResult, PolicyError> {
        self.evaluate_with_policy_set(&self.policies, entity_values, principal, action_name, resource)
    }

    pub fn authorize_record_action(
        &self,
        identity: &EduPersonIdentity,
        principal_department: &str,
        action_name: &str,
        app_slug: &str,
        resource_department: &str,
        is_ferpa_sensitive: bool,
    ) -> Result<AuthorizationResult, PolicyError> {
        let primary_affiliation = match identity.affiliations.first() {
            Some(EduPersonAffiliation::Faculty) => "faculty",
            Some(EduPersonAffiliation::Student) => "student",
            Some(EduPersonAffiliation::Staff) => "staff",
            Some(EduPersonAffiliation::Employee) => "employee",
            Some(EduPersonAffiliation::Member) => "member",
            Some(EduPersonAffiliation::Affiliate) => "affiliate",
            Some(EduPersonAffiliation::Alum) => "alum",
            None => "member",
        };

        let principal = plain_principal(&identity.eppn, primary_affiliation, principal_department);
        let resource = Resource::Record(RecordCtx {
            app_slug: app_slug.to_string(),
            department: resource_department.to_string(),
            workspace_id: String::new(),
            is_ferpa_sensitive,
        });
        authorize(self, &principal, action_name, &resource)
    }

    pub fn authorize_institutional_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
    ) -> Result<AuthorizationResult, PolicyError> {
        let principal = plain_principal(principal_eppn, principal_affiliation, principal_department);
        let resource = Resource::System(SystemCtx {
            id: resource_id.to_string(),
            department: "central_admin".to_string(),
            is_ferpa_sensitive: false,
        });
        authorize(self, &principal, action_name, &resource)
    }

    pub fn authorize_workspace_action(
        &self,
        input: &WorkspaceActionInput<'_>,
    ) -> Result<AuthorizationResult, PolicyError> {
        let principal = plain_principal(
            input.principal_eppn,
            input.principal_affiliation,
            input.principal_department,
        );
        let resource = Resource::Workspace(WorkspaceCtx {
            workspace_id: input.workspace_id.to_string(),
            department: input.workspace_department.to_string(),
            visibility: input.workspace_visibility.to_string(),
            data_classification: String::new(),
            member_role: input.member_role.unwrap_or("").to_string(),
            unit_id: String::new(),
            is_member: input.is_member,
        });
        authorize(self, &principal, input.action_name, &resource)
    }

    pub fn authorize_departmental_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
        resource_department: &str,
    ) -> Result<AuthorizationResult, PolicyError> {
        let principal = plain_principal(principal_eppn, principal_affiliation, principal_department);
        let resource = Resource::System(SystemCtx {
            id: resource_id.to_string(),
            department: resource_department.to_string(),
            is_ferpa_sensitive: false,
        });
        authorize(self, &principal, action_name, &resource)
    }
}

/// A principal built from the fields the legacy callers hold. Unit membership is empty here.
fn plain_principal(eppn: &str, affiliation: &str, department: &str) -> PrincipalCtx {
    PrincipalCtx {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        scoped_affiliation: affiliation.to_string(),
        department: department.to_string(),
        unit_ids: Vec::new(),
        is_platform_admin: false,
    }
}
