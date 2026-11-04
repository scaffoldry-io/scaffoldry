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

/// An expression in the formula language. In a flow it may name `{trigger.field}`, `{step_id.field}`, `{run.id}` and
/// `{run.started_at}`.
pub type Expr = String;

/// The most records one `FindRecord` returns, and so the most one step fans out to.
pub const MAX_FIND: u8 = 100;

/// Which records a flow step acts on. A flow names records by address and never by a query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    /// The record that started the run.
    Trigger,
    /// The records the trigger record links to through one of its link fields.
    Linked { link_field: String },
    /// The record an earlier `CreateRecord` step of this run created.
    Created { step: String },
    /// The records an earlier `FindRecord` step of this run found.
    Found { step: String },
}

/// What a flow may touch in one table. Derived from the steps and stored with the flow version.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowGrant {
    pub app_slug: String,
    pub table_id: String,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub create: bool,
    #[serde(default)]
    pub link: Vec<String>,
}

/// An action. There is no variant that deletes a record, and none that queries a table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    NotifyCollaborator { role: String, message_template: String },
    UpdateRecordStatus { new_status: String },
    CreateLedgerAuditEntry { summary: String, oscal_control: String },
    WebhookDispatch { target_url: String },
    /// Matches one indexed field by exact value, case-insensitive. At most `limit` records, at most 100.
    FindRecord { id: String, app: String, table: String, field: String, equals: Expr, limit: u8 },
    CreateRecord { id: String, app: String, table: String, fields: Vec<(String, Expr)> },
    /// On a `Found` target, applies to each record found.
    UpdateRecord { target: Target, fields: Vec<(String, Expr)> },
    SetLink { target: Target, link_field: String, to: Target },
}

impl ActionType {
    /// True for the actions that find, create, update or link records: the steps of a flow.
    pub fn is_flow(&self) -> bool {
        matches!(self, Self::FindRecord { .. } | Self::CreateRecord { .. } | Self::UpdateRecord { .. } | Self::SetLink { .. })
    }
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
    /// Where the flow is owned and listed.
    #[serde(default)]
    pub home_workspace: Option<String>,
    /// What the flow may read and write, derived from its steps when a version is saved. The run checks every read and
    /// write against these.
    #[serde(default)]
    pub grants: Vec<FlowGrant>,
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
        ActionType::FindRecord { table, field, .. } => Some(format!("find records in {table} by {field}")),
        ActionType::CreateRecord { table, fields, .. } => Some(format!("create a record in {table} setting {}", names(fields))),
        ActionType::UpdateRecord { target, fields } => Some(format!("set {} on {}", names(fields), target_words(target))),
        ActionType::SetLink { target, link_field, to } => Some(format!("link {} to {} through {link_field}", target_words(target), target_words(to))),
    }
}

fn names(fields: &[(String, Expr)]) -> String {
    fields.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>().join(", ")
}

fn target_words(t: &Target) -> String {
    match t {
        Target::Trigger => "the record".to_string(),
        Target::Linked { link_field } => format!("the records linked through {link_field}"),
        Target::Created { step } => format!("the record step {step} created"),
        Target::Found { step } => format!("the records step {step} found"),
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
