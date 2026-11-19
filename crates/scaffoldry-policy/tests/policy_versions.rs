//! Admin console phase 5: the policy is a stored, checkable thing. These tests cover the engine side.

mod support;

use scaffoldry_policy::entities::{authorize, Resource, WorkspaceCtx};
use scaffoldry_policy::{check_source, PolicyDecision, ScaffoldryPolicyEngine};
use serde_json::json;

#[test]
fn every_default_policy_has_an_id_an_effect_and_an_oscal_control() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let all = engine.policy_summaries();
    assert_eq!(all.len(), 25);
    for p in &all {
        assert!(!p.id.is_empty());
        assert!(p.effect == "permit" || p.effect == "forbid", "{}: {}", p.id, p.effect);
        assert!(!p.oscal.is_empty(), "{} has no @oscal annotation", p.id);
        assert!(!p.key.is_empty(), "{} has no @id annotation", p.id);
        assert!(p.text.contains("principal"), "{} has no text", p.id);
    }
    let ferpa = all.iter().find(|p| p.effect == "forbid" && p.description.contains("export sensitive")).unwrap();
    assert!(ferpa.source.contains("34 CFR"), "the FERPA rule names its statute");
    assert_eq!(engine.source(), engine.policy_source());
}

#[test]
fn is_allowed_agrees_with_authorize() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let who = support::plain_principal("prof@x.edu", "faculty", "biology");
    let ws = WorkspaceCtx {
        workspace_id: "ws-1".into(),
        department: "biology".into(),
        visibility: "restricted".into(),
        data_classification: String::new(),
        member_role: "owner".into(),
        unit_id: String::new(),
        unit_path_ids: Vec::new(),
        is_member: true,
        is_ferpa_sensitive: false,
        categories: Default::default(),
        domain_path: Default::default(),
        sensitivity_rank: 2,
    };
    let via_authorize = authorize(&engine, &who, "manage_workspace", &Resource::Workspace(ws.clone())).unwrap();
    // The same question as entity JSON, the shape a stored case holds.
    let principal = json!({ "type": "User", "id": "prof@x.edu", "attrs": who.entity()["attrs"], "parents": [] });
    let resource = json!({ "type": "Workspace", "id": "ws-1", "attrs": ws.entity()["attrs"], "parents": [] });
    let direct = engine.is_allowed(&principal, "manage_workspace", &resource).unwrap();
    assert_eq!(via_authorize.decision, PolicyDecision::Allow);
    assert_eq!(direct.decision, via_authorize.decision);
    assert_eq!(direct.reasons, via_authorize.reasons);
}

#[test]
fn a_syntax_error_names_its_line_and_column() {
    let src = "permit (principal, action, resource);\n\nforbid (principal action, resource);\n";
    let problems = check_source(src);
    assert!(!problems.is_empty());
    assert_eq!(problems[0].line, 3);
    assert!(problems[0].column > 1);
}

#[test]
fn a_type_error_names_its_line() {
    let src = "permit (principal, action == Action::\"read\", resource);\n\n@id(\"bad\")\npermit (principal, action == Action::\"read\", resource)\nwhen { principal.no_such_attribute == 1 };\n";
    let problems = check_source(src);
    assert!(!problems.is_empty(), "the schema does not know that attribute");
    assert!(problems.iter().any(|p| p.line >= 3), "{problems:?}");
}

#[test]
fn a_good_source_has_no_problems() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    assert!(check_source(engine.source()).is_empty());
}

#[test]
fn the_default_cases_all_pass_against_the_default_policy() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().unwrap();
    let cases = scaffoldry_policy::cases::default_cases();
    assert_eq!(cases.len(), 16, "one case for each assertion of cedar_authorization_test");
    for c in &cases {
        let out = scaffoldry_policy::cases::run_case(&engine, c);
        assert!(out.pass, "{}: expected {}, got {} {}", c.name, out.expected, out.actual, out.error);
    }
}
