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
pub enum ActionType {
    NotifyCollaborator { role: String, message_template: String },
    UpdateRecordStatus { new_status: String },
    CreateLedgerAuditEntry { summary: String, oscal_control: String },
    WebhookDispatch { target_url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRule {
    pub id: String,
    pub app_slug: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: TriggerEvent,
    pub cedar_policy_guard: Option<String>,
    pub predicates: Vec<FieldPredicate>,
    pub actions: Vec<ActionType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowExecutionResult {
    pub rule_id: String,
    pub rule_name: String,
    pub trigger_matched: bool,
    pub conditions_met: bool,
    pub cedar_authorized: bool,
    pub actions_executed: Vec<String>,
    pub execution_timestamp: String,
}
