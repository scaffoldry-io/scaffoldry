//! Golden decision matrix.
//!
//! The fixture `tests/fixtures/decision_matrix.json` was generated from the four original
//! `authorize_*` functions before the schema and entity-builder refactor. Every later
//! implementation must return the same decision for every row.
//!
//! Regenerate only on purpose: `GOLDEN_WRITE=1 cargo test -p scaffoldry-policy --test decision_matrix`.

use scaffoldry_core::standards::eduperson::EduPersonIdentity;
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
            engine.authorize_record_action(&identity, action, "golden-app", resource_department, *sensitive)
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

    let stored: Vec<Row> = serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("fixture exists"))
        .expect("fixture parses");
    let expected: BTreeMap<String, String> = stored.into_iter().map(|r| (r.key, r.decision)).collect();
    assert_eq!(expected.len(), rows.len(), "fixture row count must match the grid");

    let mut differing = Vec::new();
    for row in &rows {
        match expected.get(&row.key) {
            Some(d) if *d == row.decision => {}
            other => differing.push(format!("{} expected {:?} got {}", row.key, other, row.decision)),
        }
    }
    assert!(differing.is_empty(), "{} rows differ:\n{}", differing.len(), differing.join("\n"));
}
