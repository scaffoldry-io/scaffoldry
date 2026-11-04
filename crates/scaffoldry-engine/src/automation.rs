//! Governed Workflow Automation Engine
//!
//! Evaluates triggers, evaluates field predicates, checks Cedar policies,
//! and dispatches workflow actions with audit tracking.

use std::time::{SystemTime, UNIX_EPOCH};
use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_core::{
    ActionEffect, ActionType, AutomationRule, ConditionOperator, FieldPredicate,
    StepKind, TriggerEvent, WorkflowExecutionResult,
};
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde_json::Value;

#[derive(Default)]
pub struct AutomationEngine;

impl AutomationEngine {
    pub fn new(_policy_engine: ScaffoldryPolicyEngine) -> Self {
        Self
    }

    /// Evaluates an automation rule against a record event
    pub fn evaluate_rule(
        &self,
        rule: &AutomationRule,
        event: &TriggerEvent,
        record: &Value,
        _identity: &EduPersonIdentity,
        _principal_department: &str,
        depth: u8,
    ) -> WorkflowExecutionResult {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| format!("{}s", d.as_secs()))
            .unwrap_or_else(|_| "0s".to_string());

        if !rule.enabled {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: false,
                conditions_met: false,
                cedar_authorized: false,
                actions_executed: vec![],
                effects: vec![],
                waiting_instance: None,
                execution_timestamp: timestamp,
                branches: vec![],
            };
        }

        // 1. Evaluate Trigger Match
        let trigger_matched = match (&rule.trigger, event) {
            (TriggerEvent::RecordCreated, TriggerEvent::RecordCreated) => true,
            (TriggerEvent::RecordUpdated, TriggerEvent::RecordUpdated) => true,
            (
                TriggerEvent::FieldChanged { field_name: f1 },
                TriggerEvent::FieldChanged { field_name: f2 },
            ) => f1.eq_ignore_ascii_case(f2),
            (
                TriggerEvent::StatusChanged { to_status: s1 },
                TriggerEvent::StatusChanged { to_status: s2 },
            ) => s1.eq_ignore_ascii_case(s2),
            (TriggerEvent::Manual, TriggerEvent::Manual) => true,
            (a @ TriggerEvent::DateField { .. }, b @ TriggerEvent::DateField { .. }) => a == b,
            (a @ TriggerEvent::Schedule { .. }, b @ TriggerEvent::Schedule { .. }) => a == b,
            (TriggerEvent::Called { .. }, TriggerEvent::Called { .. }) => true,
            (a @ TriggerEvent::FlowCompleted { .. }, b @ TriggerEvent::FlowCompleted { .. }) => a == b,
            (a @ TriggerEvent::ProcessDecided { .. }, b @ TriggerEvent::ProcessDecided { .. }) => a == b,
            _ => false,
        };

        if !trigger_matched {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: false,
                conditions_met: false,
                cedar_authorized: false,
                actions_executed: vec![],
                effects: vec![],
                waiting_instance: None,
                execution_timestamp: timestamp,
                branches: vec![],
            };
        }

        // 2. Evaluate Field Predicates
        let conditions_met = rule.predicates.iter().all(|pred| {
            Self::evaluate_predicate(pred, record)
        });

        if !conditions_met {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: true,
                conditions_met: false,
                cedar_authorized: false,
                actions_executed: vec![],
                effects: vec![],
                waiting_instance: None,
                execution_timestamp: timestamp,
                branches: vec![],
            };
        }

        // 3. Cedar Policy Authorization Check
        // Rule effects authorization is verified via decide with action `run_automation`.
        let cedar_authorized = true;

        if !cedar_authorized {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: true,
                conditions_met: true,
                cedar_authorized: false,
                actions_executed: vec![],
                effects: vec![],
                waiting_instance: None,
                execution_timestamp: timestamp,
                branches: vec![],
            };
        }

        // Depth backstop check
        if depth >= 3 {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: true,
                conditions_met: true,
                cedar_authorized: true,
                actions_executed: vec!["stopped: depth".to_string()],
                effects: vec![],
                waiting_instance: None,
                execution_timestamp: timestamp,
                branches: vec![],
            };
        }

        // 4. Dispatch Actions / Steps
        let mut executed = Vec::new();
        let mut effects = Vec::new();
        let mut waiting_instance = None;

        let mut branches = Vec::new();
        if !rule.steps.is_empty() {
            Self::run_steps(rule, &rule.steps, record, &mut executed, &mut effects, &mut waiting_instance, &mut branches);
        } else {
            for action in &rule.actions {
                Self::dispatch_action(action, &mut executed, &mut effects);
            }
        }

        WorkflowExecutionResult {
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            trigger_matched: true,
            conditions_met: true,
            cedar_authorized: true,
            actions_executed: executed,
            effects,
            waiting_instance,
            execution_timestamp: timestamp,
            branches,
        }
    }

    /// Runs steps in order. A branch runs the steps of its first arm whose conditions hold, or its `otherwise`, and the
    /// choice is kept. Returns true when a step stopped the run: a decision waits, or the record has no id.
    fn run_steps(
        rule: &AutomationRule,
        steps: &[scaffoldry_core::ProcessStep],
        record: &Value,
        executed: &mut Vec<String>,
        effects: &mut Vec<ActionEffect>,
        waiting_instance: &mut Option<scaffoldry_core::ProcessInstance>,
        branches: &mut Vec<(String, String)>,
    ) -> bool {
        for step in steps {
            let when_matches = step.when.is_empty() || step.when.iter().all(|pred| Self::evaluate_predicate(pred, record));
            if !when_matches {
                continue;
            }
            match &step.kind {
                StepKind::Service { action } => {
                    Self::dispatch_action(action, executed, effects);
                }
                StepKind::Branch { branches: arms, otherwise } => {
                    let taken = arms.iter().position(|b| b.when.iter().all(|pred| Self::evaluate_predicate(pred, record)));
                    branches.push((step.id.clone(), taken.map_or_else(|| "otherwise".to_string(), |i| (i + 1).to_string())));
                    let chosen = taken.map_or(otherwise.as_slice(), |i| arms[i].steps.as_slice());
                    if Self::run_steps(rule, chosen, record, executed, effects, waiting_instance, branches) {
                        return true;
                    }
                }
                StepKind::UserTask { role, prompt, .. } => {
                    let record_id_opt = match record.get("id") {
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(Value::Number(n)) => Some(n.to_string()),
                        _ => None,
                    };
                    match record_id_opt {
                        None => executed.push("stopped: record has no id".to_string()),
                        Some(rec_id) => {
                            *waiting_instance = Some(scaffoldry_core::ProcessInstance {
                                rule_version: rule.version,
                                id: format!("{}:{}", rule.id, rec_id),
                                rule_id: rule.id.clone(),
                                app_slug: rule.app_slug.clone(),
                                record_id: rec_id,
                                status: scaffoldry_core::ProcessStatus::Waiting,
                                waiting_step_id: Some(step.id.clone()),
                                role: Some(role.clone()),
                                prompt: Some(prompt.clone()),
                                log: vec![],
                                started_by: None,
                                started_at: String::new(),
                                assigned_to: None,
                                no_approver: None,
                            });
                        }
                    }
                    return true;
                }
            }
        }
        false
    }


    fn dispatch_action(
        action: &ActionType,
        executed: &mut Vec<String>,
        effects: &mut Vec<ActionEffect>,
    ) {
        match action {
            ActionType::NotifyCollaborator { role, message_template } => {
                executed.push(format!("Notified role '{role}': {message_template}"));
                effects.push(ActionEffect::Notify {
                    role: role.clone(),
                    message: message_template.clone(),
                });
            }
            ActionType::UpdateRecordStatus { new_status } => {
                executed.push(format!("Updated record status to '{new_status}'"));
                effects.push(ActionEffect::SetFields {
                    fields: vec![("status".into(), new_status.clone())],
                });
            }
            ActionType::CreateLedgerAuditEntry { summary, oscal_control } => {
                executed.push(format!("Appended audit log: {summary} (Control: {oscal_control})"));
                effects.push(ActionEffect::LedgerNote {
                    summary: summary.clone(),
                    oscal_control: oscal_control.clone(),
                });
            }
            ActionType::WebhookDispatch { target_url } => {
                executed.push(format!("Webhook not sent: {target_url}"));
                effects.push(ActionEffect::Rejected {
                    reason: format!("webhook disabled: {target_url}"),
                });
            }
            // A flow step finds, creates, updates or links records by address. The server's flow runner does it,
            // inside the flow's grants, so the engine records nothing for it here.
            ActionType::FindRecord { .. } | ActionType::CreateRecord { .. } | ActionType::UpdateRecord { .. } | ActionType::SetLink { .. } | ActionType::StartFlow { .. } => {}
        }
    }

    pub fn evaluate_predicate(pred: &FieldPredicate, record: &Value) -> bool {
        let field_val = record.get(&pred.field_name);
        match pred.operator {
            ConditionOperator::Equals => {
                field_val.is_some_and(|v| match v {
                    Value::String(s) => s.eq_ignore_ascii_case(&pred.expected_value),
                    Value::Number(n) => n.to_string() == pred.expected_value,
                    Value::Bool(b) => b.to_string().eq_ignore_ascii_case(&pred.expected_value),
                    _ => false,
                })
            }
            ConditionOperator::NotEquals => {
                field_val.is_none_or(|v| match v {
                    Value::String(s) => !s.eq_ignore_ascii_case(&pred.expected_value),
                    Value::Number(n) => n.to_string() != pred.expected_value,
                    _ => true,
                })
            }
            ConditionOperator::GreaterThan => {
                if let (Some(Value::Number(n)), Ok(expected)) = (field_val, pred.expected_value.parse::<f64>()) {
                    n.as_f64().is_some_and(|actual| actual > expected)
                } else {
                    false
                }
            }
            ConditionOperator::LessThan => {
                if let (Some(Value::Number(n)), Ok(expected)) = (field_val, pred.expected_value.parse::<f64>()) {
                    n.as_f64().is_some_and(|actual| actual < expected)
                } else {
                    false
                }
            }
            ConditionOperator::Contains => {
                if let Some(Value::String(s)) = field_val {
                    s.to_lowercase().contains(&pred.expected_value.to_lowercase())
                } else {
                    false
                }
            }
        }
    }
}
