//! Governed Workflow Automation Engine
//!
//! Evaluates triggers, evaluates field predicates, checks Cedar policies,
//! and dispatches workflow actions with audit tracking.

use std::time::{SystemTime, UNIX_EPOCH};
use scaffoldry_core::standards::eduperson::EduPersonIdentity;
use scaffoldry_core::{
    ActionEffect, ActionType, AutomationRule, ConditionOperator, FieldPredicate,
    TriggerEvent, WorkflowExecutionResult,
};
use scaffoldry_policy::ScaffoldryPolicyEngine;
use serde_json::Value;

pub struct AutomationEngine {
    policy_engine: ScaffoldryPolicyEngine,
}

impl AutomationEngine {
    pub fn new(policy_engine: ScaffoldryPolicyEngine) -> Self {
        Self { policy_engine }
    }

    pub fn policy_engine(&self) -> &ScaffoldryPolicyEngine {
        &self.policy_engine
    }

    /// Evaluates an automation rule against a record event
    pub fn evaluate_rule(
        &self,
        rule: &AutomationRule,
        event: &TriggerEvent,
        record: &Value,
        identity: &EduPersonIdentity,
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
                execution_timestamp: timestamp,
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
                execution_timestamp: timestamp,
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
                execution_timestamp: timestamp,
            };
        }

        // 3. Cedar Policy Authorization Check
        let cedar_authorized = if rule.cedar_policy_guard.is_some() {
            // Documented action name pair: "record_decision" with is_ferpa_sensitive distinguishes faculty (permit) and student (forbid) under default Cedar policies
            match self.policy_engine.authorize_record_action(
                identity,
                "record_decision",
                &rule.app_slug,
                "institutional",
                rule.cedar_policy_guard.as_deref().is_some_and(|p| p.to_lowercase().contains("ferpa")),
            ) {
                Ok(res) => res.decision == scaffoldry_policy::PolicyDecision::Allow,
                Err(_) => false,
            }
        } else {
            true
        };

        if !cedar_authorized {
            return WorkflowExecutionResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                trigger_matched: true,
                conditions_met: true,
                cedar_authorized: false,
                actions_executed: vec![],
                effects: vec![],
                execution_timestamp: timestamp,
            };
        }

        // 4. Dispatch Actions
        let mut executed = Vec::new();
        let mut effects = Vec::new();
        for action in &rule.actions {
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
            execution_timestamp: timestamp,
        }
    }

    fn evaluate_predicate(pred: &FieldPredicate, record: &Value) -> bool {
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
