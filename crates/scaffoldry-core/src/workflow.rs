//! Governed Workflow Automation Domain Models
//!
//! Enforces event triggers, field predicates, and Cedar authorization guardrails
//! for declarative institutional workflow automations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TriggerEvent {
    RecordCreated,
    RecordUpdated,
    FieldChanged { field_name: String },
    StatusChanged { to_status: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldPredicate {
    pub field_name: String,
    pub operator: ConditionOperator,
    pub expected_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionEffect {
    SetFields { fields: Vec<(String, String)> },
    Notify { role: String, message: String },
    LedgerNote { summary: String, oscal_control: String },
    Rejected { reason: String },
}

pub fn apply_field_effects(record: &mut serde_json::Value, effects: &[ActionEffect]) {
    let Some(map) = record.as_object_mut() else {
        return;
    };

    for effect in effects {
        if let ActionEffect::SetFields { fields } = effect {
            for (key, val) in fields {
                if let Some(existing) = map.get(key) {
                    if existing.is_number() {
                        if let Ok(i) = val.parse::<i64>() {
                            map.insert(key.clone(), serde_json::Value::Number(i.into()));
                            continue;
                        } else if let Ok(f) = val.parse::<f64>() {
                            if let Some(num) = serde_json::Number::from_f64(f) {
                                map.insert(key.clone(), serde_json::Value::Number(num));
                                continue;
                            }
                        }
                    }
                }
                map.insert(key.clone(), serde_json::Value::String(val.clone()));
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessStep {
    pub id: String,
    #[serde(default)]
    pub when: Vec<FieldPredicate>, // empty = always; otherwise AND
    pub kind: StepKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepKind {
    Service { action: ActionType },
    UserTask {
        role: String,
        prompt: String,
        approve: Vec<ActionType>,
        reject: Vec<ActionType>,
        /// Who decides. When absent, the step uses `Role { name: role }`.
        #[serde(default)]
        approver: Option<crate::approver::ApproverSpec>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessStatus {
    Waiting,
    Completed,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessInstance {
    pub id: String,            // "{rule_id}:{record_id}"
    pub rule_id: String,
    pub app_slug: String,
    pub record_id: String,
    pub status: ProcessStatus, // Waiting, Completed, Rejected, Failed
    pub waiting_step_id: Option<String>,
    pub role: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub log: Vec<String>,
    /// The principal whose write started this process.
    #[serde(default)]
    pub started_by: Option<String>,
    /// RFC 3339.
    #[serde(default)]
    pub started_at: String,
    /// Set by an administrator reassigning the instance.
    #[serde(default)]
    pub assigned_to: Option<String>,
    /// Why nobody can decide the waiting step. Set by the engine.
    #[serde(default)]
    pub no_approver: Option<String>,
}

pub fn effects_retrigger(effects: &[ActionEffect]) -> Option<TriggerEvent> {
    if effects.iter().any(|e| matches!(e, ActionEffect::SetFields { .. })) {
        Some(TriggerEvent::RecordUpdated)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    NotifyCollaborator { role: String, message_template: String },
    UpdateRecordStatus { new_status: String },
    CreateLedgerAuditEntry { summary: String, oscal_control: String },
    WebhookDispatch { target_url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRule {
    pub id: String,
    #[serde(default)]
    pub app_slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub enabled: bool,
    pub trigger: TriggerEvent,
    #[serde(default)]
    pub predicates: Vec<FieldPredicate>,
    #[serde(default)]
    pub actions: Vec<ActionType>,
    #[serde(default)]
    pub steps: Vec<ProcessStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowExecutionResult {
    pub rule_id: String,
    pub rule_name: String,
    pub trigger_matched: bool,
    pub conditions_met: bool,
    pub cedar_authorized: bool,
    pub actions_executed: Vec<String>,
    #[serde(default)]
    pub effects: Vec<ActionEffect>,
    #[serde(default)]
    pub waiting_instance: Option<ProcessInstance>,
    pub execution_timestamp: String,
}
