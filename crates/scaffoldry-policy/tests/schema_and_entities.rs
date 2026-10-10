//! Guards phase 1: one Cedar schema, one typed entity builder, plain descriptions,
//! and denials that name the policy.

use scaffoldry_policy::entities::{
    authorize, unit_ids_for, PrincipalCtx, RecordCtx, Resource, WorkspaceCtx,
};
use scaffoldry_policy::{PolicyDecision, ScaffoldryPolicyEngine, WorkspaceActionInput};
use std::collections::HashMap;

fn principal(affiliation: &str, department: &str) -> PrincipalCtx {
    PrincipalCtx {
        eppn: "someone@state.edu".into(),
        name: "Someone".into(),
        scoped_affiliation: affiliation.into(),
        department: department.into(),
        unit_ids: vec![],
        is_platform_admin: false,
    }
}

#[test]
fn default_policy_set_validates_against_the_schema() {
    ScaffoldryPolicyEngine::default_institutional_engine().expect("default policies must type-check");
}

#[test]
fn a_policy_that_references_a_missing_attribute_fails_validation() {
    let bad = r#"
        @description("Broken on purpose.")
        permit (principal, action == Action::"read", resource)
        when { resource.no_such_attribute == "x" };
    "#;
    let err = ScaffoldryPolicyEngine::new(bad).err().expect("must be rejected");
    assert!(err.to_string().contains("no_such_attribute"), "error must name the attribute: {err}");
}

#[test]
fn a_non_member_has_an_empty_member_role_and_policy_13_evaluates_without_error() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let ws = WorkspaceCtx {
        workspace_id: "ws-1".into(),
        department: "biology".into(),
        visibility: "institutional".into(),
        data_classification: String::new(),
        member_role: String::new(),
        unit_id: String::new(),
        is_member: false,
    };
    assert_eq!(ws.entity_member_role(), "");
    let result = authorize(&engine, &principal("faculty", "biology"), "manage_workspace", &Resource::Workspace(ws))
        .expect("evaluation must succeed");
    assert!(result.diagnostics.is_empty(), "no Cedar evaluation error: {:?}", result.diagnostics);
    assert_eq!(result.decision, PolicyDecision::Deny);
}

#[test]
fn unit_ids_hold_the_department_the_college_and_the_root() {
    // root <- college <- department. The person holds a role in the department only.
    let parents: HashMap<String, Option<String>> = HashMap::from([
        ("root".to_string(), None),
        ("college".to_string(), Some("root".to_string())),
        ("dept".to_string(), Some("college".to_string())),
        ("other".to_string(), Some("root".to_string())),
    ]);
    let mut ids = unit_ids_for(&["dept".to_string()], &parents);
    ids.sort();
    assert_eq!(ids, vec!["college".to_string(), "dept".to_string(), "root".to_string()]);
}

#[test]
fn unit_ids_stop_after_32_steps_even_with_a_cycle() {
    let parents: HashMap<String, Option<String>> = HashMap::from([
        ("a".to_string(), Some("b".to_string())),
        ("b".to_string(), Some("a".to_string())),
    ]);
    let ids = unit_ids_for(&["a".to_string()], &parents);
    assert!(ids.len() <= 2, "a cycle must not repeat or loop forever: {ids:?}");
}

#[test]
fn every_default_policy_has_a_non_empty_description() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let summaries = engine.policy_summaries();
    assert_eq!(summaries.len(), 24, "the default set holds 24 policies");
    for p in summaries {
        assert!(!p.description.trim().is_empty(), "policy {} has no @description", p.id);
    }
}

#[test]
fn a_denial_from_a_forbid_names_that_policy() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    // Students cannot export sensitive records: policy 2.
    let record = RecordCtx {
        app_slug: "bio-lab".into(),
        department: "biology".into(),
        workspace_id: String::new(),
        is_ferpa_sensitive: true,
    };
    let result = authorize(&engine, &principal("student", "biology"), "export", &Resource::Record(record))
        .expect("evaluation must succeed");
    assert_eq!(result.decision, PolicyDecision::Deny);
    let deciding = result.deciding_policy.expect("a forbid denial names its policy");
    assert!(!deciding.id.is_empty());
    assert!(deciding.description.to_lowercase().contains("sensitive"), "got: {}", deciding.description);
}

#[test]
fn an_implicit_deny_names_no_policy() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let record = RecordCtx {
        app_slug: "bio-lab".into(),
        department: "biology".into(),
        workspace_id: String::new(),
        is_ferpa_sensitive: false,
    };
    // Different department, no permit applies, no forbid applies.
    let result = authorize(&engine, &principal("faculty", "physics"), "read", &Resource::Record(record))
        .expect("evaluation must succeed");
    assert_eq!(result.decision, PolicyDecision::Deny);
    assert!(result.deciding_policy.is_none());
}

#[test]
fn the_legacy_workspace_wrapper_reports_the_deciding_policy_too() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let result = engine
        .authorize_workspace_action(&WorkspaceActionInput {
            principal_eppn: "e@physics.state.edu",
            principal_affiliation: "student",
            principal_department: "physics",
            action_name: "access_workspace",
            workspace_id: "ws-bio",
            workspace_department: "biology",
            workspace_visibility: "restricted",
            is_member: false,
            member_role: None,
        })
        .unwrap();
    assert_eq!(result.decision, PolicyDecision::Deny);
    assert!(result.deciding_policy.is_some(), "the restricted-workspace forbid decided this");
}
