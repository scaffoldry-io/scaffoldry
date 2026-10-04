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
        cedar_policy_guard: Some("policy-ferpa-34cfr99".to_string()),
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
        "dr.curie@science.state.edu",
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
        "dr.curie@science.state.edu",
    );

    assert!(result2.trigger_matched);
    assert!(!result2.conditions_met);
    assert!(result2.actions_executed.is_empty());
}
