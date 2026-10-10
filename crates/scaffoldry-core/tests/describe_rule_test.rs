//! `describe_rule` turns a rule into plain sentences a department chair can check.

use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::workflow::{
    describe_rule, ActionType, AutomationRule, ConditionOperator, FieldPredicate, ProcessStep, StepKind, TriggerEvent,
};

fn rule() -> AutomationRule {
    AutomationRule {
        id: "r1".into(),
        app_slug: "admissions".into(),
        name: "Fast track".into(),
        description: String::new(),
        enabled: true,
        trigger: TriggerEvent::RecordCreated,
        predicates: vec![FieldPredicate { field_name: "gpa".into(), operator: ConditionOperator::GreaterThan, expected_value: "3.85".into() }],
        actions: vec![],
        steps: vec![
            ProcessStep {
                id: "s1".into(),
                when: vec![],
                kind: StepKind::Service { action: ActionType::UpdateRecordStatus { new_status: "Approved".into() } },
            },
            ProcessStep {
                id: "s2".into(),
                when: vec![],
                kind: StepKind::UserTask {
                    role: "reviewer".into(),
                    prompt: "Check it".into(),
                    approve: vec![ActionType::UpdateRecordStatus { new_status: "Admitted".into() }],
                    reject: vec![ActionType::UpdateRecordStatus { new_status: "Denied".into() }],
                    approver: Some(ApproverSpec::Position { key: "chair".into(), from: Origin::Submitter, walk_up: true }),
                },
            },
        ],
        version: 0,
    }
}

#[test]
fn a_condition_a_service_step_and_a_user_task_read_as_plain_sentences() {
    assert_eq!(
        describe_rule(&rule()),
        vec![
            "When a record is created in this app.",
            "If gpa is greater than 3.85.",
            "Then set status to Approved.",
            "Then wait for a decision by the chair of the submitter's unit. If that position is vacant, ask the next unit up. If approved: set status to Admitted. If rejected: set status to Denied.",
        ]
    );
}

#[test]
fn a_webhook_step_has_no_sentence() {
    let mut r = rule();
    r.steps.push(ProcessStep {
        id: "hook".into(),
        when: vec![],
        kind: StepKind::Service { action: ActionType::WebhookDispatch { target_url: "https://example.org".into() } },
    });
    let sentences = describe_rule(&r);
    assert!(sentences.iter().all(|s| !s.contains("example.org")), "{sentences:?}");
    assert_eq!(sentences.len(), 4);
}

#[test]
fn a_step_condition_and_other_approvers_are_in_words() {
    let mut r = rule();
    r.predicates = vec![
        FieldPredicate { field_name: "level".into(), operator: ConditionOperator::Equals, expected_value: "grad".into() },
        FieldPredicate { field_name: "country".into(), operator: ConditionOperator::NotEquals, expected_value: "US".into() },
    ];
    r.steps[0].when = vec![FieldPredicate { field_name: "name".into(), operator: ConditionOperator::Contains, expected_value: "Lee".into() }];
    r.steps[1].kind = StepKind::UserTask {
        role: "registrar".into(),
        prompt: String::new(),
        approve: vec![],
        reject: vec![],
        approver: Some(ApproverSpec::Person { eppn: "dean@example.edu".into() }),
    };
    let s = describe_rule(&r);
    assert_eq!(s[1], "If level is grad.");
    assert_eq!(s[2], "And if country is not US.");
    assert_eq!(s[3], "If name contains Lee, then set status to Approved.");
    assert_eq!(s[4], "Then wait for a decision by dean@example.edu.");

    r.steps[1].kind = StepKind::UserTask { role: "registrar".into(), prompt: String::new(), approve: vec![], reject: vec![], approver: None };
    assert_eq!(describe_rule(&r)[4], "Then wait for a decision by anyone with the role registrar.");
}
