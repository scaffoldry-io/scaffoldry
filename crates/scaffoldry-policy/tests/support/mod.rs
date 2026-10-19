//! Four question shapes the policy tests ask, built on the one public entry point `entities::authorize`.
//!
//! They used to be methods of the engine. Nothing in the server may call a shortcut that skips workspace guards, so
//! they live here, where only a test can reach them.
#![allow(dead_code)]

use scaffoldry_core::standards::eduperson::{EduPersonAffiliation, EduPersonIdentity};
use scaffoldry_policy::entities::{authorize, PrincipalCtx, RecordCtx, Resource, SystemCtx, WorkspaceCtx};
use scaffoldry_policy::{AuthorizationResult, PolicyError, ScaffoldryPolicyEngine};

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

/// A person with no units, whatever the caller says about their affiliation and department.
pub fn plain_principal(eppn: &str, affiliation: &str, department: &str) -> PrincipalCtx {
    PrincipalCtx {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        scoped_affiliation: affiliation.to_string(),
        department: department.to_string(),
        unit_ids: Vec::new(),
        admin_unit_ids: Vec::new(),
        is_platform_admin: false,
    }
}

pub trait Asks {
    fn authorize_record_action(
        &self,
        identity: &EduPersonIdentity,
        principal_department: &str,
        action_name: &str,
        app_slug: &str,
        resource_department: &str,
        is_ferpa_sensitive: bool,
    ) -> Result<AuthorizationResult, PolicyError>;

    fn authorize_institutional_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
    ) -> Result<AuthorizationResult, PolicyError>;

    fn authorize_workspace_action(&self, input: &WorkspaceActionInput<'_>) -> Result<AuthorizationResult, PolicyError>;

    fn authorize_departmental_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
        resource_department: &str,
    ) -> Result<AuthorizationResult, PolicyError>;
}

impl Asks for ScaffoldryPolicyEngine {
    fn authorize_record_action(
        &self,
        identity: &EduPersonIdentity,
        principal_department: &str,
        action_name: &str,
        app_slug: &str,
        resource_department: &str,
        is_ferpa_sensitive: bool,
    ) -> Result<AuthorizationResult, PolicyError> {
        let primary = match identity.affiliations.first() {
            Some(EduPersonAffiliation::Faculty) => "faculty",
            Some(EduPersonAffiliation::Student) => "student",
            Some(EduPersonAffiliation::Staff) => "staff",
            Some(EduPersonAffiliation::Employee) => "employee",
            Some(EduPersonAffiliation::Member) => "member",
            Some(EduPersonAffiliation::Affiliate) => "affiliate",
            Some(EduPersonAffiliation::Alum) => "alum",
            None => "member",
        };
        let resource = Resource::Record(RecordCtx {
            app_slug: app_slug.to_string(),
            department: resource_department.to_string(),
            workspace_id: String::new(),
            is_ferpa_sensitive,
            categories: Default::default(),
        });
        authorize(self, &plain_principal(&identity.eppn, primary, principal_department), action_name, &resource)
    }

    fn authorize_institutional_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
    ) -> Result<AuthorizationResult, PolicyError> {
        let resource = Resource::System(SystemCtx { id: resource_id.to_string(), department: "central_admin".to_string(), is_ferpa_sensitive: false });
        authorize(self, &plain_principal(principal_eppn, principal_affiliation, principal_department), action_name, &resource)
    }

    fn authorize_workspace_action(&self, input: &WorkspaceActionInput<'_>) -> Result<AuthorizationResult, PolicyError> {
        let resource = Resource::Workspace(WorkspaceCtx {
            workspace_id: input.workspace_id.to_string(),
            department: input.workspace_department.to_string(),
            visibility: input.workspace_visibility.to_string(),
            data_classification: String::new(),
            member_role: input.member_role.unwrap_or("").to_string(),
            unit_id: String::new(),
            unit_path_ids: Vec::new(),
            is_member: input.is_member,
            is_ferpa_sensitive: false,
            categories: Default::default(),
        });
        authorize(self, &plain_principal(input.principal_eppn, input.principal_affiliation, input.principal_department), input.action_name, &resource)
    }

    fn authorize_departmental_action(
        &self,
        principal_eppn: &str,
        principal_affiliation: &str,
        principal_department: &str,
        action_name: &str,
        resource_id: &str,
        resource_department: &str,
    ) -> Result<AuthorizationResult, PolicyError> {
        let resource = Resource::System(SystemCtx { id: resource_id.to_string(), department: resource_department.to_string(), is_ferpa_sensitive: false });
        authorize(self, &plain_principal(principal_eppn, principal_affiliation, principal_department), action_name, &resource)
    }
}
