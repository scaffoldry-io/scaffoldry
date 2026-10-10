//! Golden decision matrix.
//!
//! The fixture `tests/fixtures/decision_matrix.json` was generated from the four original
//! `authorize_*` functions before the schema and entity-builder refactor. Every later
//! implementation must return the same decision for every row.
//!
//! Regenerate only on purpose: `GOLDEN_WRITE=1 cargo test -p scaffoldry-policy --test decision_matrix`.

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_policy::entities::{authorize, PrincipalCtx, RecordCtx, Resource, SystemCtx, WorkspaceCtx};
use scaffoldry_policy::{PolicyDecision, ScaffoldryPolicyEngine, WorkspaceActionInput};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/decision_matrix.json");

#[derive(Serialize, Deserialize)]
struct Row {
    key: String,
    decision: String,
}

enum Case {
    Record {
        realm: &'static str,
        affiliation: &'static str,
        action: &'static str,
        resource_department: &'static str,
        sensitive: bool,
    },
    Institutional {
        affiliation: &'static str,
        department: &'static str,
        action: &'static str,
    },
    Departmental {
        affiliation: &'static str,
        department: &'static str,
        action: &'static str,
    },
    Workspace {
        affiliation: &'static str,
        department: &'static str,
        visibility: &'static str,
        member_role: Option<&'static str>,
        action: &'static str,
    },
}

impl Case {
    fn key(&self) -> String {
        match self {
            Case::Record { realm, affiliation, action, resource_department, sensitive } => {
                format!("record|realm={realm}|aff={affiliation}|action={action}|res_dept={resource_department}|sensitive={sensitive}")
            }
            Case::Institutional { affiliation, department, action } => {
                format!("institutional|aff={affiliation}|dept={department}|action={action}")
            }
            Case::Departmental { affiliation, department, action } => {
                format!("departmental|aff={affiliation}|dept={department}|action={action}")
            }
            Case::Workspace { affiliation, department, visibility, member_role, action } => {
                format!(
                    "workspace|aff={affiliation}|dept={department}|vis={visibility}|role={}|action={action}",
                    member_role.unwrap_or("non-member")
                )
            }
        }
    }
}

fn cases() -> Vec<Case> {
    let mut all = Vec::new();

    // Record resources. The realm drives the department the old code guessed.
    let record_affiliations = ["faculty", "student", "staff", "employee", "member", "affiliate", "alum"];
    // "biology" matches the resource, "physics" does not, "science" is the old guess that maps to biology.
    let realms = ["biology", "physics", "science"];
    for realm in realms {
        for affiliation in record_affiliations {
            for action in ["read", "write", "export", "approve", "record_decision", "publish_dataset"] {
                for sensitive in [false, true] {
                    all.push(Case::Record {
                        realm,
                        affiliation,
                        action,
                        resource_department: "biology",
                        sensitive,
                    });
                }
            }
        }
    }

    let people = ["faculty", "student", "staff", "compliance", "central_admin", "member"];

    for affiliation in people {
        for department in ["biology", "Central Enterprise IT"] {
            for action in ["access_admin", "impersonate", "record_decision", "publish_dataset", "approve"] {
                all.push(Case::Institutional { affiliation, department, action });
            }
        }
    }

    for affiliation in people {
        for department in ["biology", "physics"] {
            for action in [
                "create_app",
                "update_app",
                "publish_app",
                "publish_dataset",
                "record_decision",
                "approve",
                "export",
            ] {
                all.push(Case::Departmental { affiliation, department, action });
            }
        }
    }

    for affiliation in people {
        for department in ["biology", "physics"] {
            for visibility in ["restricted", "departmental", "institutional"] {
                for member_role in [None, Some("owner"), Some("admin"), Some("editor"), Some("viewer")] {
                    for action in [
                        "access_workspace",
                        "manage_workspace",
                        "delete_workspace",
                        "read_app",
                        "manage_app",
                        "write_record",
                        "approve",
                        "export",
                    ] {
                        all.push(Case::Workspace { affiliation, department, visibility, member_role, action });
                    }
                }
            }
        }
    }

    all
}

/// The department the old code derived from the realm. The grid now passes it in as the
/// signed-in user's stored department, so the fixture rows stay valid.
fn stored_department(realm: &str) -> &str {
    match realm {
        "science" => "biology",
        other => other,
    }
}

fn principal(eppn: &str, affiliation: &str, department: &str) -> PrincipalCtx {
    PrincipalCtx {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        scoped_affiliation: affiliation.to_string(),
        department: department.to_string(),
        unit_ids: vec![],
        is_platform_admin: false,
    }
}

/// The same grid, straight through the one `authorize` function.
fn evaluate_direct(engine: &ScaffoldryPolicyEngine, case: &Case) -> String {
    let result = match case {
        Case::Record { realm, affiliation, action, resource_department, sensitive } => authorize(
            engine,
            &principal(&format!("someone@{realm}.state.edu"), affiliation, stored_department(realm)),
            action,
            &Resource::Record(RecordCtx {
                app_slug: "golden-app".into(),
                department: resource_department.to_string(),
                workspace_id: String::new(),
                is_ferpa_sensitive: *sensitive,
                categories: Default::default(),
            }),
        ),
        Case::Institutional { affiliation, department, action } => authorize(
            engine,
            &principal("someone@state.edu", affiliation, department),
            action,
            &Resource::System(SystemCtx {
                id: "institutional-console".into(),
                department: "central_admin".into(),
                is_ferpa_sensitive: false,
            }),
        ),
        Case::Departmental { affiliation, department, action } => authorize(
            engine,
            &principal("someone@state.edu", affiliation, department),
            action,
            &Resource::System(SystemCtx {
                id: "dept-resource".into(),
                department: "biology".into(),
                is_ferpa_sensitive: false,
            }),
        ),
        Case::Workspace { affiliation, department, visibility, member_role, action } => authorize(
            engine,
            &principal("someone@state.edu", affiliation, department),
            action,
            &Resource::Workspace(WorkspaceCtx {
                workspace_id: "ws-golden".into(),
                department: "biology".into(),
                visibility: visibility.to_string(),
                data_classification: String::new(),
                member_role: member_role.unwrap_or("").to_string(),
                unit_id: String::new(),
                is_member: member_role.is_some(),
            }),
        ),
    };
    decision_name(result.expect("evaluation must succeed").decision)
}

fn decision_name(d: PolicyDecision) -> String {
    match d {
        PolicyDecision::Allow => "allow".to_string(),
        PolicyDecision::Deny => "deny".to_string(),
    }
}

fn evaluate(engine: &ScaffoldryPolicyEngine, case: &Case) -> String {
    let result = match case {
        Case::Record { realm, affiliation, action, resource_department, sensitive } => {
            let eppn = format!("someone@{realm}.state.edu");
            let scoped = format!("{affiliation}@{realm}.state.edu");
            let identity = EduPersonIdentity::parse(&eppn, vec![scoped.as_str()]).expect("identity parses");
            engine.authorize_record_action(
                &identity,
                stored_department(realm),
                action,
                "golden-app",
                resource_department,
                *sensitive,
            )
        }
        Case::Institutional { affiliation, department, action } => engine.authorize_institutional_action(
            "someone@state.edu",
            affiliation,
            department,
            action,
            "institutional-console",
        ),
        Case::Departmental { affiliation, department, action } => engine.authorize_departmental_action(
            "someone@state.edu",
            affiliation,
            department,
            action,
            "dept-resource",
            "biology",
        ),
        Case::Workspace { affiliation, department, visibility, member_role, action } => {
            engine.authorize_workspace_action(&WorkspaceActionInput {
                principal_eppn: "someone@state.edu",
                principal_affiliation: affiliation,
                principal_department: department,
                action_name: action,
                workspace_id: "ws-golden",
                workspace_department: "biology",
                workspace_visibility: visibility,
                is_member: member_role.is_some(),
                member_role: *member_role,
            })
        }
    };
    decision_name(result.expect("evaluation must succeed").decision)
}

fn expected_rows() -> BTreeMap<String, String> {
    let stored: Vec<Row> = serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("fixture exists"))
        .expect("fixture parses");
    stored.into_iter().map(|r| (r.key, r.decision)).collect()
}

fn assert_matches_fixture(rows: &[Row]) {
    let expected = expected_rows();
    assert_eq!(expected.len(), rows.len(), "fixture row count must match the grid");

    let mut differing = Vec::new();
    for row in rows {
        match expected.get(&row.key) {
            Some(d) if *d == row.decision => {}
            other => differing.push(format!("{} expected {:?} got {}", row.key, other, row.decision)),
        }
    }
    assert!(differing.is_empty(), "{} rows differ:\n{}", differing.len(), differing.join("\n"));
}

#[test]
fn legacy_functions_match_the_golden_matrix() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().expect("engine builds");
    let rows: Vec<Row> = cases()
        .iter()
        .map(|c| Row { key: c.key(), decision: evaluate(&engine, c) })
        .collect();

    if std::env::var("GOLDEN_WRITE").is_ok() {
        let json = serde_json::to_string_pretty(&rows).expect("rows serialize");
        std::fs::create_dir_all(std::path::Path::new(FIXTURE).parent().unwrap()).unwrap();
        std::fs::write(FIXTURE, json + "\n").expect("fixture written");
        return;
    }

    assert_matches_fixture(&rows);
}

#[test]
fn the_new_authorize_function_matches_the_golden_matrix() {
    let engine = ScaffoldryPolicyEngine::default_institutional_engine().expect("engine builds");
    let rows: Vec<Row> = cases()
        .iter()
        .map(|c| Row { key: c.key(), decision: evaluate_direct(&engine, c) })
        .collect();
    assert_matches_fixture(&rows);
}
