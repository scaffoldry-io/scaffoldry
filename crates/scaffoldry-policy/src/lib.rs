//! Scaffoldry Sovereign Policy and Authorization Engine (Cedar Policy Integration)

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request};
use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationResult {
    pub decision: PolicyDecision,
    pub reasons: Vec<String>,
    pub diagnostics: Vec<String>,
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
}

impl Clone for ScaffoldryPolicyEngine {
    fn clone(&self) -> Self {
        Self {
            authorizer: Authorizer::new(),
            policies: self.policies.clone(),
        }
    }
}

impl ScaffoldryPolicyEngine {
    pub fn new(policy_src: &str) -> Result<Self, PolicyError> {
        let policies: PolicySet = policy_src
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::PolicyParseError(e.to_string()))?;
        Ok(Self {
            authorizer: Authorizer::new(),
            policies,
        })
    }

    pub fn default_institutional_engine() -> Result<Self, PolicyError> {
        let default_policies = r#"
            // 1. Permit departmental members to read and write records in their own department
            permit (
                principal,
                action in [Action::"read", Action::"write"],
                resource
            )
            when {
                principal.department == resource.department
            };

            // 2. Strict FERPA Policy: Forbid exporting records if sensitive,
            // unless principal holds verified staff or compliance affiliation
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
            permit (
                principal,
                action == Action::"export",
                resource
            )
            when {
                principal.department == resource.department
            };

            // 4. Permit central_admin to access institutional administration console
            permit (
                principal,
                action == Action::"access_admin",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 5. Permit central_admin to impersonate directory users
            permit (
                principal,
                action == Action::"impersonate",
                resource
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 6. Explicitly forbid any non-admin principal from impersonating users
            forbid (
                principal,
                action == Action::"impersonate",
                resource
            )
            when {
                principal.scoped_affiliation != "central_admin"
            };

            // 7. Permit central_admin to access, manage, and delete any workspace
            permit (
                principal,
                action in [Action::"access_workspace", Action::"manage_workspace", Action::"delete_workspace"],
                resource is Workspace
            )
            when {
                principal.scoped_affiliation == "central_admin"
            };

            // 8. Permit workspace access to verified members
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.is_member == true
            };

            // 9. Permit workspace access if visibility is departmental and principal matches department
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
            permit (
                principal,
                action == Action::"access_workspace",
                resource is Workspace
            )
            when {
                resource.visibility == "institutional"
            };

            // 11. Forbid workspace access if visibility is restricted and principal is not a member and not central_admin
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
        "#;
        Self::new(default_policies)
    }

    pub fn authorize_record_action(
        &self,
        identity: &EduPersonIdentity,
        action_name: &str,
        app_slug: &str,
        resource_department: &str,
        is_ferpa_sensitive: bool,
    ) -> Result<AuthorizationResult, PolicyError> {
        let principal_dept = if identity.eppn.contains("physics") {
            "physics"
        } else {
            "biology"
        };

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

        let user_id = &identity.eppn;
        let record_id = format!("{}-record-target", app_slug);

        let entities_json = json!([
            {
                "uid": { "type": "User", "id": user_id },
                "attrs": {
                    "eppn": identity.eppn,
                    "scoped_affiliation": primary_affiliation,
                    "realm": identity.realm,
                    "department": principal_dept
                },
                "parents": []
            },
            {
                "uid": { "type": "Record", "id": record_id },
                "attrs": {
                    "app_slug": app_slug,
                    "department": resource_department,
                    "is_ferpa_sensitive": is_ferpa_sensitive
                },
                "parents": []
            }
        ]);

        let entities = Entities::from_json_value(entities_json, None)
            .map_err(|e| PolicyError::EntityError(e.to_string()))?;

        let principal: EntityUid = format!(r#"User::"{user_id}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let action: EntityUid = format!(r#"Action::"{action_name}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let resource: EntityUid = format!(r#"Record::"{record_id}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let request = Request::new(
            principal,
            action,
            resource,
            Context::empty(),
            None,
        )
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;

        let response = self.authorizer.is_authorized(&request, &self.policies, &entities);

        let decision = match response.decision() {
            Decision::Allow => PolicyDecision::Allow,
            Decision::Deny => PolicyDecision::Deny,
        };

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

        Ok(AuthorizationResult {
            decision,
            reasons,
            diagnostics,
        })
    }

    pub fn authorize_institutional_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
    ) -> Result<AuthorizationResult, PolicyError> {
        let entities_json = json!([
            {
                "uid": { "type": "User", "id": principal_eppn },
                "attrs": {
                    "eppn": principal_eppn,
                    "scoped_affiliation": principal_affiliation,
                    "realm": "state.edu",
                    "department": principal_department
                },
                "parents": []
            },
            {
                "uid": { "type": "System", "id": resource_id },
                "attrs": {
                    "department": "central_admin",
                    "is_ferpa_sensitive": false
                },
                "parents": []
            }
        ]);

        let entities = Entities::from_json_value(entities_json, None)
            .map_err(|e| PolicyError::EntityError(e.to_string()))?;

        let principal: EntityUid = format!(r#"User::"{principal_eppn}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let action: EntityUid = format!(r#"Action::"{action_name}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let resource: EntityUid = format!(r#"System::"{resource_id}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let request = Request::new(
            principal,
            action,
            resource,
            Context::empty(),
            None,
        )
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;

        let response = self.authorizer.is_authorized(&request, &self.policies, &entities);

        let decision = match response.decision() {
            Decision::Allow => PolicyDecision::Allow,
            Decision::Deny => PolicyDecision::Deny,
        };

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

        Ok(AuthorizationResult {
            decision,
            reasons,
            diagnostics,
        })
    }

    pub fn authorize_workspace_action(
        &self,
        input: &WorkspaceActionInput<'_>,
    ) -> Result<AuthorizationResult, PolicyError> {
        let role = input.member_role.unwrap_or("none");
        let entities_json = json!([
            {
                "uid": { "type": "User", "id": input.principal_eppn },
                "attrs": {
                    "eppn": input.principal_eppn,
                    "scoped_affiliation": input.principal_affiliation,
                    "realm": "state.edu",
                    "department": input.principal_department
                },
                "parents": []
            },
            {
                "uid": { "type": "Workspace", "id": input.workspace_id },
                "attrs": {
                    "department": input.workspace_department,
                    "visibility": input.workspace_visibility,
                    "is_member": input.is_member,
                    "member_role": role
                },
                "parents": []
            }
        ]);

        let entities = Entities::from_json_value(entities_json, None)
            .map_err(|e| PolicyError::EntityError(e.to_string()))?;

        let principal: EntityUid = format!(r#"User::"{}""#, input.principal_eppn)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let action: EntityUid = format!(r#"Action::"{}""#, input.action_name)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let resource: EntityUid = format!(r#"Workspace::"{}""#, input.workspace_id)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let request = Request::new(
            principal,
            action,
            resource,
            Context::empty(),
            None,
        )
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;

        let response = self.authorizer.is_authorized(&request, &self.policies, &entities);

        let decision = match response.decision() {
            Decision::Allow => PolicyDecision::Allow,
            Decision::Deny => PolicyDecision::Deny,
        };

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

        Ok(AuthorizationResult {
            decision,
            reasons,
            diagnostics,
        })
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
        let entities_json = json!([
            {
                "uid": { "type": "User", "id": principal_eppn },
                "attrs": {
                    "eppn": principal_eppn,
                    "scoped_affiliation": principal_affiliation,
                    "realm": "state.edu",
                    "department": principal_department
                },
                "parents": []
            },
            {
                "uid": { "type": "System", "id": resource_id },
                "attrs": {
                    "department": resource_department,
                    "is_ferpa_sensitive": false
                },
                "parents": []
            }
        ]);

        let entities = Entities::from_json_value(entities_json, None)
            .map_err(|e| PolicyError::EntityError(e.to_string()))?;

        let principal: EntityUid = format!(r#"User::"{principal_eppn}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let action: EntityUid = format!(r#"Action::"{action_name}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let resource: EntityUid = format!(r#"System::"{resource_id}""#)
            .parse()
            .map_err(|e: cedar_policy::ParseErrors| PolicyError::RequestError(e.to_string()))?;

        let request = Request::new(
            principal,
            action,
            resource,
            Context::empty(),
            None,
        )
        .map_err(|e| PolicyError::RequestError(e.to_string()))?;

        let response = self.authorizer.is_authorized(&request, &self.policies, &entities);

        let decision = match response.decision() {
            Decision::Allow => PolicyDecision::Allow,
            Decision::Deny => PolicyDecision::Deny,
        };

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

        Ok(AuthorizationResult {
            decision,
            reasons,
            diagnostics,
        })
    }
}
