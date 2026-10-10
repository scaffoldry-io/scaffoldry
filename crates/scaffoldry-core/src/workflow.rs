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
    /// The version of the rule this instance started under. 0 means read the live rule.
    #[serde(default)]
    pub rule_version: u32,
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
    /// Assigned by the server when a version is saved. 0 until then.
    #[serde(default)]
    pub version: u32,
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

fn operator_words(op: &ConditionOperator) -> &'static str {
    match op {
        ConditionOperator::Equals => "is",
        ConditionOperator::NotEquals => "is not",
        ConditionOperator::GreaterThan => "is greater than",
        ConditionOperator::LessThan => "is less than",
        ConditionOperator::Contains => "contains",
    }
}

fn predicate_words(p: &FieldPredicate) -> String {
    format!("{} {} {}", p.field_name, operator_words(&p.operator), p.expected_value)
}

/// The condition of a step as a lead-in, or nothing when it always runs.
fn condition_words(when: &[FieldPredicate]) -> Option<String> {
    if when.is_empty() {
        None
    } else {
        Some(when.iter().map(predicate_words).collect::<Vec<_>>().join(" and "))
    }
}

/// An action as words, or `None` for one a rule may not hold. A webhook is refused, so it has no sentence.
fn action_words(action: &ActionType) -> Option<String> {
    match action {
        ActionType::UpdateRecordStatus { new_status } => Some(format!("set status to {new_status}")),
        ActionType::NotifyCollaborator { role, message_template } => Some(format!("notify {role}: {message_template}")),
        ActionType::CreateLedgerAuditEntry { summary, .. } => Some(format!("record in the audit ledger: {summary}")),
        ActionType::WebhookDispatch { .. } => None,
    }
}

fn actions_words(actions: &[ActionType]) -> String {
    let words: Vec<String> = actions.iter().filter_map(action_words).collect();
    if words.is_empty() {
        "do nothing".to_string()
    } else {
        words.join(", then ")
    }
}

fn approver_words(spec: &crate::approver::ApproverSpec) -> String {
    use crate::approver::{ApproverSpec, Origin};
    match spec {
        ApproverSpec::Position { key, from, .. } => {
            let unit = match from {
                Origin::Submitter => "the submitter's unit",
                Origin::Workspace => "the workspace's unit",
            };
            format!("the {key} of {unit}")
        }
        ApproverSpec::Person { eppn } => eppn.clone(),
        ApproverSpec::Role { name } => format!("anyone with the role {name}"),
    }
}

/// A rule in plain sentences, so a reviewer approves what it does and not its JSON.
pub fn describe_rule(rule: &AutomationRule) -> Vec<String> {
    use crate::approver::{effective_approver, ApproverSpec};
    let mut out = Vec::new();
    out.push(match &rule.trigger {
        TriggerEvent::RecordCreated => "When a record is created in this app.".to_string(),
        TriggerEvent::RecordUpdated => "When a record is updated in this app.".to_string(),
        TriggerEvent::FieldChanged { field_name } => format!("When the field {field_name} changes."),
        TriggerEvent::StatusChanged { to_status } => format!("When the status changes to {to_status}."),
    });
    for (i, p) in rule.predicates.iter().enumerate() {
        out.push(format!("{} {}.", if i == 0 { "If" } else { "And if" }, predicate_words(p)));
    }
    for action in rule.actions.iter().filter_map(action_words) {
        out.push(format!("Then {action}."));
    }
    for step in &rule.steps {
        let lead = condition_words(&step.when);
        match &step.kind {
            StepKind::Service { action } => {
                let Some(words) = action_words(action) else { continue };
                out.push(match lead {
                    Some(c) => format!("If {c}, then {words}."),
                    None => format!("Then {words}."),
                });
            }
            StepKind::UserTask { role, approve, reject, approver, .. } => {
                let spec = effective_approver(role, approver);
                let mut sentence = format!("Then wait for a decision by {}.", approver_words(&spec));
                if let ApproverSpec::Position { walk_up: true, .. } = &spec {
                    sentence.push_str(" If that position is vacant, ask the next unit up.");
                }
                if !approve.is_empty() {
                    sentence.push_str(&format!(" If approved: {}.", actions_words(approve)));
                }
                if !reject.is_empty() {
                    sentence.push_str(&format!(" If rejected: {}.", actions_words(reject)));
                }
                if let Some(c) = lead {
                    out.push(format!("If {c}:"));
                }
                out.push(sentence);
            }
        }
    }
    out
}
