//! Test cases for a policy: a question in entity JSON and the answer it must get.
//!
//! The default cases are the assertions of `tests/cedar_authorization_test.rs`, kept as data so an administrator can run them
//! against a draft before it is switched on.

use crate::entities::{PrincipalCtx, RecordCtx, Resource, SystemCtx, WorkspaceCtx};
use crate::{PolicyDecision, ScaffoldryPolicyEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyCase {
    pub name: String,
    /// Cedar entity JSON: `{ "type", "id", "attrs", "parents" }`.
    pub principal: Value,
    pub action: String,
    pub resource: Value,
    /// `Allow` or `Deny`.
    pub expected: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseOutcome {
    pub name: String,
    pub expected: String,
    pub actual: String,
    pub pass: bool,
    /// Why the question could not be asked, such as an attribute of the wrong type. Empty when it ran.
    pub error: String,
}

fn word(d: PolicyDecision) -> &'static str {
    match d {
        PolicyDecision::Allow => "Allow",
        PolicyDecision::Deny => "Deny",
    }
}

/// Asks one case of one engine.
pub fn run_case(engine: &ScaffoldryPolicyEngine, case: &PolicyCase) -> CaseOutcome {
    let (actual, error) = match engine.is_allowed(&case.principal, &case.action, &case.resource) {
        Ok(r) => (word(r.decision).to_string(), String::new()),
        Err(e) => ("Error".to_string(), e.to_string()),
    };
    CaseOutcome { name: case.name.clone(), expected: case.expected.clone(), pass: actual == case.expected, actual, error }
}

fn as_json(entity: Value) -> Value {
    json!({ "type": entity["uid"]["type"], "id": entity["uid"]["id"], "attrs": entity["attrs"], "parents": entity["parents"] })
}

fn person(eppn: &str, affiliation: &str, department: &str) -> Value {
    as_json(PrincipalCtx {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        scoped_affiliation: affiliation.to_string(),
        department: department.to_string(),
        unit_ids: Vec::new(),
        is_platform_admin: false,
    }
    .entity())
}

fn record(department: &str, sensitive: bool) -> Value {
    as_json(Resource::Record(RecordCtx { app_slug: "bio-lab-inventory".into(), department: department.into(), workspace_id: String::new(), is_ferpa_sensitive: sensitive, categories: Default::default() }).entity())
}

fn system(id: &str) -> Value {
    as_json(Resource::System(SystemCtx { id: id.into(), department: "central_admin".into(), is_ferpa_sensitive: false }).entity())
}

fn workspace(id: &str, department: &str, visibility: &str, member: Option<&str>) -> Value {
    as_json(Resource::Workspace(WorkspaceCtx {
        workspace_id: id.into(),
        department: department.into(),
        visibility: visibility.into(),
        data_classification: String::new(),
        member_role: member.unwrap_or("").into(),
        unit_id: String::new(),
        unit_path_ids: Vec::new(),
        is_member: member.is_some(),
        is_ferpa_sensitive: false,
        categories: Default::default(),
        domain_path: Default::default(),
        sensitivity_rank: 2,
    })
    .entity())
}

/// The cases a new database starts with.
pub fn default_cases() -> Vec<PolicyCase> {
    let case = |name: &str, principal: Value, action: &str, resource: Value, expected: &str| PolicyCase { name: name.into(), principal, action: action.into(), resource, expected: expected.into() };
    let (curie, smith, officer, einstein) = (
        person("prof.curie@science.state.edu", "faculty", "biology"),
        person("student.smith@science.state.edu", "student", "biology"),
        person("compliance.officer@science.state.edu", "staff", "biology"),
        person("einstein@physics.state.edu", "student", "physics"),
    );
    let admin = person("jordan.lee@state.edu", "central_admin", "Central Enterprise IT");
    let sarah = person("sarah.connor@state.edu", "faculty", "Computer Science");
    vec![
        case("Faculty can write their department's records", curie.clone(), "write", record("biology", false), "Allow"),
        case("A student can read their department's records", smith.clone(), "read", record("biology", false), "Allow"),
        case("A student cannot export protected student records", smith, "export", record("biology", true), "Deny"),
        case("Staff can export protected student records", officer, "export", record("biology", true), "Allow"),
        case("A student cannot read another department's records", einstein.clone(), "read", record("biology", false), "Deny"),
        case("A central administrator can open the console", admin.clone(), "access_admin", system("institutional-console"), "Allow"),
        case("Faculty cannot open the console", sarah.clone(), "access_admin", system("institutional-console"), "Deny"),
        case("A central administrator can impersonate", admin.clone(), "impersonate", system("sarah.connor@state.edu"), "Allow"),
        case("Faculty cannot impersonate", sarah, "impersonate", system("marcus.vance@state.edu"), "Deny"),
        case("A member can open a restricted workspace", curie.clone(), "access_workspace", workspace("ws-bio-lab", "biology", "restricted", Some("owner")), "Allow"),
        case("A non-member cannot open a restricted workspace", einstein, "access_workspace", workspace("ws-bio-lab", "biology", "restricted", None), "Deny"),
        case("A department peer can open a departmental workspace", person("elena.rodriguez@state.edu", "compliance", "compliance"), "access_workspace", workspace("ws-campus-compliance", "compliance", "departmental", None), "Allow"),
        case("Another department cannot open a departmental workspace", curie.clone(), "access_workspace", workspace("ws-campus-compliance", "compliance", "departmental", None), "Deny"),
        case("A central administrator can open any workspace", admin, "access_workspace", workspace("ws-bio-lab", "biology", "restricted", None), "Allow"),
        case("An owner can manage the workspace", curie, "manage_workspace", workspace("ws-bio-lab", "biology", "restricted", Some("owner")), "Allow"),
        case("A viewer cannot manage the workspace", person("marcus.vance@state.edu", "staff", "biology"), "manage_workspace", workspace("ws-bio-lab", "biology", "restricted", Some("viewer")), "Deny"),
    ]
}
