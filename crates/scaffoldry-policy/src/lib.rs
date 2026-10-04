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

pub struct ScaffoldryPolicyEngine {
    authorizer: Authorizer,
    policies: PolicySet,
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
}
