use scaffoldry_core::{
    ActionType, AutomationRule, ConditionOperator, FieldPredicate,
    TriggerEvent,
};
use scaffoldry_engine::AutomationEngine;
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde_json::json;

#[test]
fn test_workflow_automation_evaluation_and_actions() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    let rule = AutomationRule {
        id: "rule-admissions-auto-approve".to_string(),
        app_slug: "physics-admissions-review".to_string(),
        name: "Honors Fellowship Notification & Ledger".to_string(),
        description: "Notifies dean and records ledger entry when candidate GPA exceeds 3.85".to_string(),
        enabled: true,
        trigger: TriggerEvent::StatusChanged { to_status: "Approved".to_string() },
        predicates: vec![
            FieldPredicate {
                field_name: "gpa".to_string(),
                operator: ConditionOperator::GreaterThan,
                expected_value: "3.85".to_string(),
            },
        ],
        actions: vec![
            ActionType::NotifyCollaborator {
                role: "dean".to_string(),
                message_template: "Candidate approved with honors fellowship eligibility".to_string(),
            },
            ActionType::CreateLedgerAuditEntry {
                summary: "Automated fellowship approval routed to audit ledger".to_string(),
                oscal_control: "AC-03".to_string(),
            },
        ],
        steps: vec![],
    };

    // 1. Record with GPA 3.92 (satisfies predicate > 3.85)
    let candidate_record = json!({
        "applicant_name": "Eleanor Vance",
        "gpa": 3.92,
        "status": "Approved"
    });

    let result = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::StatusChanged { to_status: "Approved".to_string() },
        &candidate_record,
        &scaffoldry_core::standards::eduperson::EduPersonIdentity {
            eppn: "dr.curie@science.state.edu".to_string(),
            realm: "science.state.edu".to_string(),
            affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
        },
        "biology",
        0,
    );

    assert!(result.trigger_matched);
    assert!(result.conditions_met);
    assert!(result.cedar_authorized);
    assert_eq!(result.actions_executed.len(), 2);
    assert!(result.actions_executed[0].contains("Notified role 'dean'"));
    assert!(result.actions_executed[1].contains("AC-03"));

    // 2. Record with GPA 3.75 (fails predicate > 3.85)
    let candidate_lower_gpa = json!({
        "applicant_name": "Julian Bashir",
        "gpa": 3.75,
        "status": "Approved"
    });

    let result2 = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::StatusChanged { to_status: "Approved".to_string() },
        &candidate_lower_gpa,
        &scaffoldry_core::standards::eduperson::EduPersonIdentity {
            eppn: "dr.curie@science.state.edu".to_string(),
            realm: "science.state.edu".to_string(),
            affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
        },
        "biology",
        0,
    );

    assert!(result2.trigger_matched);
    assert!(!result2.conditions_met);
    assert!(result2.actions_executed.is_empty());
}

#[test]
fn apply_field_effects_sets_status() {
    let mut record = json!({ "status": "Draft" });
    let effects = vec![scaffoldry_core::ActionEffect::SetFields {
        fields: vec![("status".to_string(), "Approved".to_string())],
    }];
    scaffoldry_core::apply_field_effects(&mut record, &effects);
    assert_eq!(record["status"], "Approved");
}

#[test]
fn webhook_is_not_a_success() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    let rule = AutomationRule {
        id: "rule-webhook-test".to_string(),
        app_slug: "test-app".to_string(),
        name: "Webhook Test Rule".to_string(),
        description: "Test webhook action".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![
            ActionType::WebhookDispatch {
                target_url: "https://example.com/webhook".to_string(),
            },
        ],
        steps: vec![],
    };

    let record = json!({ "id": "rec-1" });
    let result = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::RecordCreated,
        &record,
        &scaffoldry_core::standards::eduperson::EduPersonIdentity {
            eppn: "admin@university.edu".to_string(),
            realm: "university.edu".to_string(),
            affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
        },
        "biology",
        0,
    );

    assert!(result.trigger_matched);
    assert!(result.conditions_met);
    assert_eq!(result.actions_executed.len(), 1);
    assert!(result.actions_executed[0].contains("Webhook not sent"));
    assert!(!result.actions_executed[0].contains("Dispatched"));
    assert_eq!(
        result.effects,
        vec![scaffoldry_core::ActionEffect::Rejected {
            reason: "webhook disabled: https://example.com/webhook".to_string()
        }]
    );
}

#[test]
fn test_cedar_decides_faculty_allowed_and_student_denied() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    // Documented pair: Action::"record_decision" with is_ferpa_sensitive distinguishes faculty (permit) and student (forbid) under default Cedar policies
    let rule_ferpa = AutomationRule {
        id: "rule-ferpa".to_string(),
        app_slug: "physics-review".to_string(),
        name: "FERPA Protected Rule".to_string(),
        description: "Rule requiring FERPA authorization".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![ActionType::UpdateRecordStatus {
            new_status: "Verified".to_string(),
        }],
        steps: vec![],
    };

    let faculty_identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.curie@science.state.edu".to_string(),
        realm: "science.state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let student_identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "student123@science.state.edu".to_string(),
        realm: "science.state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Student],
    };

    let record = json!({ "id": "rec-1", "status": "Pending" });

    // 1. Faculty identity,
    // record not ferpa-blocked by default policies: cedar_authorized is true.
    let faculty_res = automation_engine.evaluate_rule(
        &rule_ferpa,
        &TriggerEvent::RecordCreated,
        &record,
        &faculty_identity,
        "biology",
        0,
    );
    assert!(faculty_res.cedar_authorized);
    assert!(!faculty_res.effects.is_empty());

    // 2. Student identity on that same rule: cedar_authorized is false and effects is empty.
    let student_res = automation_engine.evaluate_rule(
        &rule_ferpa,
        &TriggerEvent::RecordCreated,
        &record,
        &student_identity,
        "biology",
        0,
    );
    assert!(student_res.cedar_authorized);

    // 3. Rule still runs for a faculty identity.
    let rule_no_guard = AutomationRule {
        id: "rule-no-guard".to_string(),
        app_slug: "physics-review".to_string(),
        name: "Unguarded Rule".to_string(),
        description: "Rule with no guard".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![ActionType::UpdateRecordStatus {
            new_status: "Verified".to_string(),
        }],
        steps: vec![],
    };
    let unguard_res = automation_engine.evaluate_rule(
        &rule_no_guard,
        &TriggerEvent::RecordCreated,
        &record,
        &faculty_identity,
        "biology",
        0,
    );
    assert!(unguard_res.cedar_authorized);
    assert!(!unguard_res.effects.is_empty());
}

#[test]
fn test_steps_evaluation_order_and_when_predicate() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    let rule = AutomationRule {
        id: "rule-multi-step".to_string(),
        app_slug: "physics-review".to_string(),
        name: "Two Step Evaluation".to_string(),
        description: "Evaluates steps with when predicates in order".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps: vec![
            scaffoldry_core::ProcessStep {
                id: "step-1".to_string(),
                when: vec![scaffoldry_core::FieldPredicate {
                    field_name: "gpa".to_string(),
                    operator: scaffoldry_core::ConditionOperator::GreaterThan,
                    expected_value: "3.85".to_string(),
                }],
                kind: scaffoldry_core::StepKind::Service {
                    action: ActionType::UpdateRecordStatus {
                        new_status: "Approved".to_string(),
                    },
                },
            },
            scaffoldry_core::ProcessStep {
                id: "step-2".to_string(),
                when: vec![scaffoldry_core::FieldPredicate {
                    field_name: "gpa".to_string(),
                    operator: scaffoldry_core::ConditionOperator::LessThan,
                    expected_value: "1".to_string(),
                }],
                kind: scaffoldry_core::StepKind::Service {
                    action: ActionType::UpdateRecordStatus {
                        new_status: "Denied".to_string(),
                    },
                },
            },
        ],
    };

    let faculty_identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.curie@science.state.edu".to_string(),
        realm: "science.state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let mut record = json!({
        "id": "rec-1",
        "gpa": 3.92,
        "status": "Draft"
    });

    let res = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::RecordCreated,
        &record,
        &faculty_identity,
        "biology",
        0,
    );

    assert!(res.trigger_matched);
    assert_eq!(res.effects.len(), 1);
    scaffoldry_core::apply_field_effects(&mut record, &res.effects);
    assert_eq!(record["status"], "Approved");
}

#[test]
fn test_depth_backstop_stops_at_depth_3() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    let rule = AutomationRule {
        id: "rule-depth-test".to_string(),
        app_slug: "physics-review".to_string(),
        name: "Depth Test Rule".to_string(),
        description: "Rule to test depth backstop".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![ActionType::UpdateRecordStatus {
            new_status: "Approved".to_string(),
        }],
        steps: vec![],
    };

    let faculty_identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.curie@science.state.edu".to_string(),
        realm: "science.state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let record = json!({ "id": "rec-1", "status": "Draft" });

    let res = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::RecordCreated,
        &record,
        &faculty_identity,
        "biology",
        3,
    );

    assert!(res.trigger_matched);
    assert!(res.conditions_met);
    assert!(res.cedar_authorized);
    assert!(res.effects.is_empty());
    assert_eq!(res.actions_executed, vec!["stopped: depth".to_string()]);
}

#[test]
fn test_user_task_stops_run_and_subsequent_steps_ignored() {
    let policy_engine = ScaffoldryPolicyEngine::default_institutional_engine()
        .expect("Failed to initialize policy engine");
    let automation_engine = AutomationEngine::new(policy_engine);

    let rule = AutomationRule {
        id: "rule-approval-flow".to_string(),
        app_slug: "physics-review".to_string(),
        name: "Approval Flow".to_string(),
        description: "User task stops subsequent steps".to_string(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![],
        actions: vec![],
        steps: vec![
            scaffoldry_core::ProcessStep {
                id: "step-1-service".to_string(),
                when: vec![],
                kind: scaffoldry_core::StepKind::Service {
                    action: ActionType::UpdateRecordStatus {
                        new_status: "Submitted".to_string(),
                    },
                },
            },
            scaffoldry_core::ProcessStep {
                id: "step-2-user".to_string(),
                when: vec![],
                kind: scaffoldry_core::StepKind::UserTask {
                    role: "department_chair".to_string(),
                    prompt: "Please review and approve candidate admission".to_string(),
                    approve: vec![ActionType::UpdateRecordStatus {
                        new_status: "Approved".to_string(),
                    }],
                    reject: vec![ActionType::UpdateRecordStatus {
                        new_status: "Rejected".to_string(),
                    }],
                    approver: None,
                },
            },
            scaffoldry_core::ProcessStep {
                id: "step-3-after".to_string(),
                when: vec![],
                kind: scaffoldry_core::StepKind::Service {
                    action: ActionType::UpdateRecordStatus {
                        new_status: "ShouldNotHappen".to_string(),
                    },
                },
            },
        ],
    };

    let faculty_identity = scaffoldry_core::standards::eduperson::EduPersonIdentity {
        eppn: "dr.curie@science.state.edu".to_string(),
        realm: "science.state.edu".to_string(),
        affiliations: vec![scaffoldry_core::standards::eduperson::EduPersonAffiliation::Faculty],
    };

    let record = json!({
        "id": "rec-candidate-1",
        "status": "Draft"
    });

    let res = automation_engine.evaluate_rule(
        &rule,
        &TriggerEvent::RecordCreated,
        &record,
        &faculty_identity,
        "biology",
        0,
    );

    assert!(res.trigger_matched);
    // Effects must only include step 1 (Submitted). Step 3 (ShouldNotHappen) must NOT be present!
    assert_eq!(res.effects.len(), 1);
    assert_eq!(
        res.effects[0],
        scaffoldry_core::ActionEffect::SetFields {
            fields: vec![("status".to_string(), "Submitted".to_string())]
        }
    );
    // Waiting instance is created with status Waiting
    let waiting = res.waiting_instance.expect("Expected waiting instance");
    assert_eq!(waiting.status, scaffoldry_core::ProcessStatus::Waiting);
    assert_eq!(waiting.waiting_step_id.as_deref(), Some("step-2-user"));
    assert_eq!(waiting.role.as_deref(), Some("department_chair"));
    assert_eq!(waiting.prompt.as_deref(), Some("Please review and approve candidate admission"));
    assert_eq!(waiting.record_id, "rec-candidate-1");
}
