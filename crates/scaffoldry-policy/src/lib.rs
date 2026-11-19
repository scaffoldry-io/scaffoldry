//! Scaffoldry Sovereign Policy and Authorization Engine (Cedar Policy Integration)
//!
//! One Cedar schema (`schema/scaffoldry.cedarschema`) types every entity and every policy.
//! One builder (`entities`) makes the entities. One function (`entities::authorize`) decides.
//! The four `authorize_*` methods below are thin wrappers over it.

pub mod cases;
pub mod entities;

pub use cedar_policy::{Effect, Policy, PolicyId, PolicySet};

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityId, EntityTypeName, EntityUid, Request, Schema,
    ValidationMode, Validator,
};
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

pub fn parse_guard_policy(id: &str, text: &str) -> Result<Policy, PolicyError> {
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
    /// The address of the library rule bound to this policy, when one is. Filled by the server on a denial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

/// One policy of the running set, as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicySummary {
    /// The id Cedar gives the policy. Decisions and receipts name it.
    pub id: String,
    /// The `@id` annotation: a name that does not change when policies are reordered.
    pub key: String,
    pub effect: String,
    /// The `@oscal` annotation, such as `ac-3`. Empty when there is none.
    pub oscal: String,
    /// The `@source` annotation: the statute that applies. Empty when there is none.
    pub source: String,
    pub description: String,
    pub text: String,
}

/// One problem found in a policy source, with where it is. Lines and columns count from 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceProblem {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = source.get(..offset).unwrap_or(source);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    (line, column)
}

fn first_offset<D: miette::Diagnostic>(d: &D) -> Option<usize> {
    d.labels().and_then(|mut l| l.next()).map(|l| l.offset())
}

/// Parses a policy source and type-checks it against the schema. Returns every problem with its line and column.
/// An empty list means the source is good.
pub fn check_source(source: &str) -> Vec<SourceProblem> {
    let mut out = Vec::new();
    let at = |m: String, off: Option<usize>| {
        let (line, column) = line_column(source, off.unwrap_or(0));
        SourceProblem { message: m, line, column }
    };
    let policies: PolicySet = match source.parse() {
        Ok(p) => p,
        Err(e) => {
            let e: cedar_policy::ParseErrors = e;
            for one in e.iter() {
                out.push(at(one.to_string(), first_offset(one)));
            }
            return out;
        }
    };
    let schema = match Schema::from_cedarschema_str(SCHEMA_SRC) {
        Ok((s, _)) => s,
        Err(e) => return vec![at(format!("schema: {e}"), None)],
    };
    let result = Validator::new(schema).validate(&policies, ValidationMode::Strict);
    for e in result.validation_errors() {
        out.push(at(e.to_string(), first_offset(e)));
    }
    out
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

/// The type and id of an entity JSON value. Both `{ "type", "id" }` and `{ "uid": { "type", "id" } }` are accepted.
fn entity_ref(v: &serde_json::Value) -> Result<(String, String), PolicyError> {
    let uid = v.get("uid").unwrap_or(v);
    let get = |k: &str| uid.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    match (get("type"), get("id")) {
        (Some(t), Some(i)) => Ok((t, i)),
        _ => Err(PolicyError::EntityError("An entity needs a type and an id".to_string())),
    }
}

/// The record Cedar reads: `uid`, `attrs` and `parents`.
fn entity_record(v: &serde_json::Value) -> Result<serde_json::Value, PolicyError> {
    let (t, i) = entity_ref(v)?;
    Ok(serde_json::json!({
        "uid": { "type": t, "id": i },
        "attrs": v.get("attrs").cloned().unwrap_or_else(|| serde_json::json!({})),
        "parents": v.get("parents").cloned().unwrap_or_else(|| serde_json::json!([])),
    }))
}

fn entity_uid(type_name: &str, id: &str) -> Result<EntityUid, PolicyError> {
    let type_name = EntityTypeName::from_str(type_name)
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;
    Ok(EntityUid::from_type_name_and_id(
        type_name,
        EntityId::new(id),
    ))
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
        let entities =
            Entities::from_json_value(serde_json::Value::Array(entity_values), Some(&self.schema))
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
        let reasons = response
            .diagnostics()
            .reason()
            .map(|r| r.to_string())
            .collect();
        let diagnostics = response
            .diagnostics()
            .errors()
            .map(|e| e.to_string())
            .collect();

        let deciding_policy = if decision == PolicyDecision::Deny {
            policies
                .policies()
                .find(|p| reason_ids.contains(p.id()))
                .map(|p| PolicyRef {
                    id: p.id().to_string(),
                    description: p.annotation("description").unwrap_or("").to_string(),
                    rule: None,
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
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Each policy with its effect, OSCAL control, statute and text, read from its annotations.
    pub fn policy_summaries(&self) -> Vec<PolicySummary> {
        self.policies
            .policies()
            .map(|p| PolicySummary {
                id: p.id().to_string(),
                key: p.annotation("id").unwrap_or("").to_string(),
                effect: if p.effect() == Effect::Permit { "permit" } else { "forbid" }.to_string(),
                oscal: p.annotation("oscal").unwrap_or("").to_string(),
                source: p.annotation("source").unwrap_or("").to_string(),
                description: p.annotation("description").unwrap_or("").to_string(),
                text: p.to_string(),
            })
            .collect()
    }

    /// Asks one question from entity JSON, the shape a stored test case holds: `{ "type", "id", "attrs", "parents" }`.
    /// The entity list holds only the two entities named.
    pub fn is_allowed(
        &self,
        principal: &serde_json::Value,
        action: &str,
        resource: &serde_json::Value,
    ) -> Result<AuthorizationResult, PolicyError> {
        self.is_allowed_with(principal, action, resource, Vec::new())
    }

    /// `is_allowed`, with more entities, such as the units a request's entities sit in.
    pub fn is_allowed_with(
        &self,
        principal: &serde_json::Value,
        action: &str,
        resource: &serde_json::Value,
        extra: Vec<serde_json::Value>,
    ) -> Result<AuthorizationResult, PolicyError> {
        let (p_type, p_id) = entity_ref(principal)?;
        let (r_type, r_id) = entity_ref(resource)?;
        let mut entities = vec![entity_record(principal)?, entity_record(resource)?];
        entities.extend(extra);
        self.evaluate(entities, (&p_type, &p_id), action, (&r_type, &r_id))
    }

    pub fn default_institutional_engine() -> Result<Self, PolicyError> {
        let default_policies = r#"
            // 1. Permit departmental members to read and write records in their own department
            @id("dept-read-write")
            @oscal("ac-3")
            @description("Members of a department can read and write that department's records.")
            permit (
                principal,
                action in [Action::"read", Action::"write", Action::"run_automation"],
                resource
            )
            when {
                resource.unit in principal.units ||
                principal.scoped_affiliation == "central_admin"
            };

            // 2. Strict FERPA Policy: Forbid exporting records if sensitive,
            // unless principal holds verified staff or compliance affiliation
            @id("ferpa-export-restriction")
            @oscal("ac-3")
            @source("FERPA, 20 U.S.C. 1232g and 34 CFR Part 99")
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
            @id("dept-export")
            @oscal("ac-3")
            @description("People can export records that belong to their own department.")
            permit (
                principal,
                action == Action::"export",
                resource
            )
            when {
                resource.unit in principal.units
            };

            // 4. Permit central_admin to access institutional administration console
            @id("admin-console-access")
            @oscal("ac-6")
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
            @id("admin-impersonate")
            @oscal("ac-6")
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
            @id("non-admin-no-impersonate")
            @oscal("ac-6")
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
            @id("admin-workspace-all")
            @oscal("ac-3")
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
            @id("member-open-workspace")
            @oscal("ac-3")
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
            @id("departmental-workspace-open")
            @oscal("ac-3")
            @description("People can open the departmental workspaces of their own department.")
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.visibility == "departmental" &&
                resource.unit in principal.units
            };

            // 10. Permit workspace access if visibility is institutional
            @id("institutional-workspace-open")
            @oscal("ac-3")
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
            @id("restricted-workspace-closed")
            @oscal("ac-3")
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
            @id("owner-admin-manage-workspace")
            @oscal("ac-3")
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
            @id("non-owner-no-manage-workspace")
            @oscal("ac-3")
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
            @id("staff-create-update-publish-app")
            @oscal("ac-3")
            @description("Faculty and staff can create, update and publish apps in their department.")
            permit (
                principal,
                action in [Action::"create_app", Action::"update_app", Action::"publish_app"],
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                ((principal.scoped_affiliation == "faculty" || principal.scoped_affiliation == "staff") && resource.unit in principal.units)
            };

            // 15. Forbid non-faculty/staff/admin (e.g. students, affiliates) from updating or publishing apps
            @id("student-no-app-change")
            @oscal("ac-3")
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
            @id("staff-publish-dataset")
            @oscal("ac-3")
            @description("Faculty and staff can publish datasets from their department.")
            permit (
                principal,
                action == Action::"publish_dataset",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                ((principal.scoped_affiliation == "faculty" || principal.scoped_affiliation == "staff") && resource.unit in principal.units)
            };

            // 17. Forbid students and affiliates from publishing datasets
            @id("student-no-publish-dataset")
            @oscal("ac-3")
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
            @id("record-decision")
            @oscal("au-2")
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
            @id("student-no-record-decision")
            @oscal("au-2")
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
            @id("approve-step")
            @oscal("ac-5")
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
            @id("student-no-approve-step")
            @oscal("ac-5")
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
            @id("read-app")
            @oscal("ac-3")
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
            @id("write-record")
            @oscal("ac-3")
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

            // 25. Permit workspace commenters, editors, admins and owners to comment on records
            @id("comment-record")
            @oscal("ac-3")
            @description("Workspace commenters, editors, admins and owners can comment on records.")
            permit (
                principal,
                action == Action::"comment",
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin" ||
                (resource.is_member == true && (
                    resource.member_role == "owner" ||
                    resource.member_role == "admin" ||
                    resource.member_role == "editor" ||
                    resource.member_role == "commenter"
                ))
            };

            // 24. Permit workspace owner and admin to manage app
            @id("manage-app")
            @oscal("ac-3")
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
        self.evaluate_with_policy_set(
            &self.policies,
            entity_values,
            principal,
            action_name,
            resource,
        )
    }
}
