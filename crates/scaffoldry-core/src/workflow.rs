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
    /// A person pressed a button field that names the rule (`field-types.md` phase 5).
    Manual,
    /// The date in `field`, moved by `offset_days` (negative is before), at `at` (`HH:MM`, UTC) or midnight. The hourly
    /// `scan_date_triggers` job starts each record once for each date.
    DateField {
        field: String,
        offset_days: i32,
        #[serde(default)]
        at: Option<String>,
    },
    /// A time each day, week or month, `at` (`HH:MM`) in `platform.timezone`. The `fire_schedules` job starts it.
    Schedule { every: ScheduleEvery, at: String },
    /// Another flow starts it with `StartFlow`, passing these inputs.
    Called {
        #[serde(default)]
        inputs: Vec<FlowInput>,
    },
    /// A flow ends with this status: `Completed` or `Failed`.
    FlowCompleted { flow_id: String, status: String },
    /// A decision step of `rule` in `app` is decided: `approve` or `reject`.
    ProcessDecided { app: String, rule: String, decision: String },
}

/// How often a scheduled flow runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduleEvery {
    Daily,
    /// On a weekday, 1 for Monday to 7 for Sunday.
    Weekly { day: u8 },
    /// On a day of the month, 1 to 28.
    Monthly { day: u8 },
}

/// One input a `Called` flow declares.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowInput {
    pub name: String,
    pub kind: InputKind,
}

/// The type of a flow input. Inputs are values and ids, never records.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InputKind {
    Text,
    Number,
    Boolean,
    Date,
    RecordId,
}

/// The most flows one chain may hold: a flow started at this depth fails.
pub const MAX_CHAIN_DEPTH: usize = 5;

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

/// One arm of a branch: its steps run when every condition holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Branch {
    #[serde(default)]
    pub when: Vec<FieldPredicate>,
    #[serde(default)]
    pub steps: Vec<ProcessStep>,
}

/// The most levels branches may nest, the most steps a rule may hold, and the most steps one branch may hold.
pub const MAX_BRANCH_DEPTH: usize = 2;
pub const MAX_RULE_STEPS: usize = 50;
pub const MAX_BRANCH_STEPS: usize = 20;

/// Every step, depth first, including the steps inside branches.
pub fn all_steps(steps: &[ProcessStep]) -> Vec<&ProcessStep> {
    let mut out = Vec::new();
    for s in steps {
        out.push(s);
        if let StepKind::Branch { branches, otherwise } = &s.kind {
            for b in branches {
                out.extend(all_steps(&b.steps));
            }
            out.extend(all_steps(otherwise));
        }
    }
    out
}

/// The step with this id, wherever it sits.
pub fn find_step<'a>(steps: &'a [ProcessStep], id: &str) -> Option<&'a ProcessStep> {
    all_steps(steps).into_iter().find(|s| s.id == id)
}

/// How many levels of branches nest in these steps. Steps with no branch are 0.
pub fn branch_depth(steps: &[ProcessStep]) -> usize {
    steps
        .iter()
        .map(|s| match &s.kind {
            StepKind::Branch { branches, otherwise } => {
                1 + branches.iter().map(|b| branch_depth(&b.steps)).chain(std::iter::once(branch_depth(otherwise))).max().unwrap_or(0)
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
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
    /// The steps of the first arm whose conditions hold run, or else `otherwise`. The choice is a process event.
    Branch {
        branches: Vec<Branch>,
        #[serde(default)]
        otherwise: Vec<ProcessStep>,
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
    /// Starts a new run of a `Called` flow with these inputs, and does not wait for it.
    StartFlow { flow_id: String, inputs: Vec<(String, Expr)> },
}

impl ActionType {
    /// True for the actions that find, create, update or link records: the steps of a flow.
    pub fn is_flow(&self) -> bool {
        matches!(self, Self::FindRecord { .. } | Self::CreateRecord { .. } | Self::UpdateRecord { .. } | Self::SetLink { .. } | Self::StartFlow { .. })
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
    /// Each branch the run passed, as `(step id, arm)`: the arm's number from 1, or `otherwise`.
    #[serde(default)]
    pub branches: Vec<(String, String)>,
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
        ActionType::StartFlow { flow_id, .. } => Some(format!("start the flow {flow_id}")),
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
    let mut out = Vec::new();
    out.push(match &rule.trigger {
        TriggerEvent::RecordCreated => "When a record is created in this app.".to_string(),
        TriggerEvent::RecordUpdated => "When a record is updated in this app.".to_string(),
        TriggerEvent::FieldChanged { field_name } => format!("When the field {field_name} changes."),
        TriggerEvent::StatusChanged { to_status } => format!("When the status changes to {to_status}."),
        TriggerEvent::Manual => "When a person presses its button.".to_string(),
        TriggerEvent::Schedule { every, at } => match every {
            ScheduleEvery::Daily => format!("Every day at {at}."),
            ScheduleEvery::Weekly { day } => format!("Every week on day {day} at {at}."),
            ScheduleEvery::Monthly { day } => format!("Every month on day {day} at {at}."),
        },
        TriggerEvent::Called { inputs } if inputs.is_empty() => "When another flow starts it.".to_string(),
        TriggerEvent::Called { inputs } => format!("When another flow starts it with {}.", inputs.iter().map(|i| format!("{} ({:?})", i.name, i.kind)).collect::<Vec<_>>().join(", ")),
        TriggerEvent::FlowCompleted { flow_id, status } => format!("When the flow {flow_id} ends {}.", status.to_lowercase()),
        TriggerEvent::ProcessDecided { rule, decision, .. } => format!("When a step of {rule} is decided: {decision}."),
        TriggerEvent::DateField { field, offset_days, .. } => match offset_days {
            0 => format!("On the date in {field}."),
            d if *d < 0 => format!("{} days before the date in {field}.", -d),
            d => format!("{d} days after the date in {field}."),
        },
    });
    for (i, p) in rule.predicates.iter().enumerate() {
        out.push(format!("{} {}.", if i == 0 { "If" } else { "And if" }, predicate_words(p)));
    }
    for action in rule.actions.iter().filter_map(action_words) {
        out.push(format!("Then {action}."));
    }
    describe_steps(&rule.steps, &mut out);
    out
}

fn describe_steps(steps: &[ProcessStep], out: &mut Vec<String>) {
    use crate::approver::{effective_approver, ApproverSpec};
    for step in steps {
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
            StepKind::Branch { branches, otherwise } => {
                if let Some(c) = lead {
                    out.push(format!("If {c}:"));
                }
                for (i, b) in branches.iter().enumerate() {
                    let cond = condition_words(&b.when).unwrap_or_else(|| "always".to_string());
                    out.push(format!("{} {cond}:", if i == 0 { "If" } else { "Otherwise, if" }));
                    describe_steps(&b.steps, out);
                }
                if !otherwise.is_empty() {
                    out.push("Otherwise:".to_string());
                    describe_steps(otherwise, out);
                }
            }
        }
    }
}
