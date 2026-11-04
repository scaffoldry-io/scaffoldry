//! Process definitions: what a rule must satisfy to be saved, and saving a version of one.
//!
//! A rule is governed like a manifest. It is proposed, checked, and approved. `save_rule` is the
//! one function that writes a rule version, and approval and the administrator's enable and
//! disable are the only callers.

use crate::service::access::{authorize_app, AppAction};
use crate::service::history::Origin;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::{effective_approver, validate_approver};
use scaffoldry_core::workflow::{ActionType, FieldPredicate, StepKind, TriggerEvent};
use scaffoldry_core::standards::eduperson::EduPersonAffiliation;
use scaffoldry_core::workflow::ActionEffect;
use scaffoldry_core::{AutomationRule, DecisionType, ProcessInstance, ProcessStatus};
use serde_json::{json, Value};
use std::str::FromStr;
use scaffoldry_engine::{AppManifest, HostRouter};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

/// The names of every field of an app, over its tables and views.
pub fn field_names(m: &AppManifest) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in &m.tables {
        out.extend(t.fields.iter().map(|f| f.name.clone()));
    }
    for v in &m.views {
        out.extend(v.fields.iter().map(|f| f.name.clone()));
    }
    out
}

fn computed_names(m: &AppManifest) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in &m.tables {
        out.extend(t.fields.iter().filter(|f| f.field_type.is_computed()).map(|f| f.name.clone()));
    }
    out
}

/// Every action a rule holds, wherever it sits.
pub fn all_actions(rule: &AutomationRule) -> Vec<&ActionType> {
    let mut out: Vec<&ActionType> = rule.actions.iter().collect();
    for step in scaffoldry_core::workflow::all_steps(&rule.steps) {
        match &step.kind {
            StepKind::Service { action } => out.push(action),
            StepKind::UserTask { approve, reject, .. } => {
                out.extend(approve.iter());
                out.extend(reject.iter());
            }
            StepKind::Branch { .. } => {}
        }
    }
    out
}

/// The fields a rule sets. The only action that sets a field is a status update.
pub fn fields_set(rule: &AutomationRule) -> BTreeSet<String> {
    all_actions(rule)
        .into_iter()
        .filter(|a| matches!(a, ActionType::UpdateRecordStatus { .. }))
        .map(|_| "status".to_string())
        .collect()
}

/// The fields a rule reads in a condition.
pub fn fields_read(rule: &AutomationRule) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = rule.predicates.iter().map(|p| p.field_name.clone()).collect();
    for step in scaffoldry_core::workflow::all_steps(&rule.steps) {
        out.extend(step.when.iter().map(|p: &FieldPredicate| p.field_name.clone()));
        // A branch's conditions read the record too.
        if let StepKind::Branch { branches, .. } = &step.kind {
            for b in branches {
                out.extend(b.when.iter().map(|p| p.field_name.clone()));
            }
        }
    }
    if let TriggerEvent::FieldChanged { field_name } = &rule.trigger {
        out.insert(field_name.clone());
    }
    out
}

/// Every problem that would stop a rule from being saved. Empty means it can be saved.
pub fn validate_rule(state: &SharedState, manifest: &AppManifest, rule: &AutomationRule) -> Vec<String> {
    let mut problems = Vec::new();
    let known = crate::service::approvers::known_sets(state, rule);
    let names = field_names(manifest);
    let computed = computed_names(manifest);

    let mut seen = HashSet::new();
    for step in scaffoldry_core::workflow::all_steps(&rule.steps) {
        if !seen.insert(step.id.as_str()) {
            problems.push(format!("step id '{}' is used twice", step.id));
        }
        if let StepKind::UserTask { role, approver, .. } = &step.kind {
            if let Err(e) = validate_approver(&effective_approver(role, approver), &known) {
                problems.push(format!("step '{}': {e}", step.id));
            }
        }
    }
    for step in scaffoldry_core::workflow::all_steps(&rule.steps) {
        let actions: Vec<&ActionType> = match &step.kind {
            StepKind::Service { action } => vec![action],
            StepKind::UserTask { approve, reject, .. } => approve.iter().chain(reject.iter()).collect(),
            StepKind::Branch { .. } => vec![],
        };
        if actions.iter().any(|a| matches!(a, ActionType::WebhookDispatch { .. })) {
            problems.push(format!("step '{}' is a webhook, which a rule may not hold", step.id));
        }
    }
    if rule.actions.iter().any(|a| matches!(a, ActionType::WebhookDispatch { .. })) {
        problems.push("a rule may not hold a webhook".to_string());
    }
    problems.extend(structure_problems(manifest, rule, state));
    if crate::service::flows::is_flow(rule) {
        let limit = crate::service::flows::max_steps(state);
        if rule.steps.len() as i64 > limit {
            problems.push(format!("the flow has {} steps, and the limit is {limit}", rule.steps.len()));
        }
        problems.extend(scaffoldry_engine::flow::validate_flow(rule, &crate::service::flows::manifests_with(state, manifest, rule)));
    }
    for name in fields_read(rule) {
        if !names.contains(&name) {
            problems.push(format!("the rule names the field '{name}', which the app does not have"));
        }
    }
    // `status` is the one field a rule sets, and every record may carry it, so the app need not
    // declare it. If the app does declare it, it must not be computed.
    for name in fields_set(rule) {
        if computed.contains(&name) {
            problems.push(format!("the rule sets the computed field '{name}'"));
        }
    }
    problems
}

/// Process v2 phase 2: branches, triggers, and records a rule creates.
fn structure_problems(manifest: &AppManifest, rule: &AutomationRule, state: &SharedState) -> Vec<String> {
    use scaffoldry_core::workflow::{all_steps, MAX_BRANCH_DEPTH, MAX_BRANCH_STEPS, MAX_RULE_STEPS};
    use scaffoldry_engine::FieldType;
    let mut problems = Vec::new();
    // Branches nest at most two levels. The step that goes deeper is named.
    fn deep(steps: &[scaffoldry_core::ProcessStep], level: usize, out: &mut Vec<String>) {
        for s in steps {
            if let StepKind::Branch { branches, otherwise } = &s.kind {
                if level > MAX_BRANCH_DEPTH {
                    out.push(format!("step '{}': branches nest {level} levels deep here, and the most is two levels", s.id));
                    continue;
                }
                for b in branches {
                    deep(&b.steps, level + 1, out);
                }
                deep(otherwise, level + 1, out);
            }
        }
    }
    deep(&rule.steps, 1, &mut problems);
    let count = all_steps(&rule.steps).len();
    if count > MAX_RULE_STEPS {
        problems.push(format!("the rule has {count} steps, and the most is {MAX_RULE_STEPS}"));
    }
    for s in all_steps(&rule.steps) {
        if let StepKind::Branch { branches, otherwise } = &s.kind {
            for steps in branches.iter().map(|b| &b.steps).chain(std::iter::once(otherwise)) {
                let n = all_steps(steps).len();
                if n > MAX_BRANCH_STEPS {
                    problems.push(format!("step '{}': one branch has {n} steps, and the most is {MAX_BRANCH_STEPS}", s.id));
                }
            }
        }
    }
    let field_in = |m: &AppManifest, name: &str| m.tables.iter().flat_map(|t| t.fields.iter()).find(|f| f.name == name).cloned();
    match &rule.trigger {
        TriggerEvent::Manual => {
            let pressed = manifest.tables.iter().flat_map(|t| t.fields.iter()).any(|f| f.field_type == FieldType::Button && f.button_rule.as_deref() == Some(rule.id.as_str()));
            if !pressed {
                problems.push(format!("the rule '{}' starts by hand, and no button field in the app names it. Add a Button field whose rule is '{}'", rule.id, rule.id));
            }
        }
        TriggerEvent::DateField { field, .. } => match field_in(manifest, field) {
            None => problems.push(format!("the date trigger names the field '{field}', which the app does not have")),
            Some(f) if !matches!(f.field_type, FieldType::Date | FieldType::DateTime) => {
                problems.push(format!("the date trigger names the field '{field}', which is not a date field"))
            }
            Some(f) if !f.indexed => problems.push(format!("the date field '{field}' is not indexed. Mark it indexed, so the hourly scan reads the index and not the table")),
            Some(_) => {}
        },
        _ => {}
    }
    // A record a rule creates must have every required field. One created in the table that starts the rule would start
    // it again.
    let manifests = crate::service::flows::manifests_with(state, manifest, rule);
    let trigger_table = scaffoldry_engine::flow::trigger_table(rule, &manifests);
    for step in all_steps(&rule.steps) {
        let StepKind::Service { action: ActionType::CreateRecord { app, table, fields, .. } } = &step.kind else { continue };
        if let Some(t) = manifests.iter().find(|m| &m.slug == app).and_then(|m| m.tables.iter().find(|t| &t.id == table)) {
            for f in t.fields.iter().filter(|f| f.required && !f.field_type.is_computed()) {
                if !fields.iter().any(|(n, _)| n == &f.name) {
                    problems.push(format!("step '{}': a record created in '{table}' needs its required field '{}'", step.id, f.name));
                }
            }
        }
        let starts_itself = app == &rule.app_slug && trigger_table.as_deref() == Some(table.as_str()) && matches!(rule.trigger, TriggerEvent::RecordCreated);
        if starts_itself {
            problems.push(format!("step '{}' creates a record in '{table}', the table whose new records start this rule, so the rule would start itself", step.id));
        }
    }
    problems
}

/// The rule an instance runs under: the version it started with, or the live rule for an old
/// instance that has none.
pub fn rule_for(state: &SharedState, inst: &ProcessInstance) -> Option<AutomationRule> {
    if inst.rule_version > 0 {
        {
        let repo = &state.repository;
            if let Ok(Some(rule)) = repo.get_rule_version(&inst.rule_id, inst.rule_version) {
                return Some(rule);
            }
        }
    }
    state.automations.read().ok()?.get(&inst.app_slug)?.iter().find(|r| r.id == inst.rule_id).cloned()
}

/// Saves one version of a rule: validates, then writes the ledger entry, the version row, and the
/// live row in one transaction. Nothing else may write a live rule.
pub fn save_rule(
    changed_by: &str,
    app_slug: &str,
    rule: AutomationRule,
    proposal_id: Option<Uuid>,
    summary: &str,
    state: &SharedState,
) -> Result<AutomationRule, ServiceError> {
    let manifest = state
        .engine
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .resolve_by_slug(app_slug)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("App '{app_slug}' not found")))?;
    let mut rule = rule;
    rule.app_slug = app_slug.to_string();
    let problems = validate_rule(state, &manifest, &rule);
    if !problems.is_empty() {
        return Err(ServiceError::BadRequest(format!("rule_schema: {}", problems.join("; "))));
    }
    crate::service::flows::derive(state, &manifest, &mut rule);
    let repo = &*state.repository;
    let (saved, _) = repo
        .save_rule(&rule, summary, proposal_id, changed_by, &manifest.organization_code)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    super::reload_after_write(state)?;
    Ok(saved)
}

/// Decides a waiting process step. The REST route and the `decide_process` tool both call this.
///
/// The resolver says who may act. The submitter and the starter never may, and a Platform Admin
/// does not bypass it. The step is read from the version of the rule the instance started under.
pub fn decide_process(
    caller: &AuthUser,
    _origin: &Origin,
    slug: &str,
    instance_id: &str,
    decision: &str,
    state: &SharedState,
) -> Result<ProcessInstance, ServiceError> {
    authorize_app(caller, slug, AppAction::Read, state)?;

    let decision = decision.to_lowercase();
    if decision != "approve" && decision != "reject" {
        return Err(ServiceError::BadRequest("Decision must be 'approve' or 'reject'".to_string()));
    }
    // Unknown affiliation is 403.
    EduPersonAffiliation::from_str(&caller.affiliation).map_err(|_| ServiceError::forbidden("Unknown affiliation"))?;

    // The instance is read from the database. The decision below locks its row and checks it again.
    let mut instance = state
        .repository
        .get_process_instance(instance_id)
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?
        .ok_or_else(|| ServiceError::NotFound("Process instance not found".to_string()))?;
    if instance.app_slug != slug {
        return Err(ServiceError::NotFound("Process instance not found in this app".to_string()));
    }
    // A record outside the caller's scope is not theirs to decide on, and is not said to exist.
    if !crate::service::scope::readable_ids(caller, slug, std::slice::from_ref(&instance.record_id), state).contains(&instance.record_id) {
        return Err(ServiceError::NotFound("Process instance not found".to_string()));
    }
    if instance.status != ProcessStatus::Waiting {
        return Err(ServiceError::conflict("Process instance is not in Waiting status"));
    }

    let waiting_step_id = instance.waiting_step_id.clone().unwrap_or_default();
    let mut found_step = false;
    let mut field_effects = Vec::new();
    if let Some(rule) = rule_for(state, &instance) {
        if let Some(step) = scaffoldry_core::workflow::find_step(&rule.steps, &waiting_step_id) {
            if let StepKind::UserTask { approve, reject, .. } = &step.kind {
                found_step = true;
                let actions = if decision == "approve" { approve } else { reject };
                for act in actions {
                    match act {
                        ActionType::UpdateRecordStatus { new_status } => {
                            field_effects.push(ActionEffect::SetFields { fields: vec![("status".into(), new_status.clone())] });
                        }
                        ActionType::NotifyCollaborator { role, message_template } => {
                            instance.log.push(format!("Notified role '{role}': {message_template}"));
                        }
                        ActionType::CreateLedgerAuditEntry { summary, oscal_control } => {
                            instance.log.push(format!("Appended audit log: {summary} (Control: {oscal_control})"));
                        }
                        ActionType::WebhookDispatch { target_url } => {
                            instance.log.push(format!("Webhook not sent: {target_url}"));
                        }
                        // A flow action is never the result of a decision: saving refuses it there.
                        ActionType::FindRecord { .. } | ActionType::CreateRecord { .. } | ActionType::UpdateRecord { .. } | ActionType::SetLink { .. } => {}
                    }
                }
            }
        }
    }
    if !found_step {
        return Err(ServiceError::BadRequest("Waiting step not found in automations".to_string()));
    }

    if !crate::service::approvers::caller_can_decide(state, caller, &instance) {
        return Err(ServiceError::forbidden("Forbidden: Caller may not decide this step"));
    }

    // The record the decision changes is a system write, so it needs no version. It commits with the decision.
    let mut record_write = None;
    if !field_effects.is_empty() {
        let current = state.find_record(slug, &instance.record_id).map_err(ServiceError::Internal)?;
        if let Some(rec) = current {
            let mut data = rec.data.clone();
            scaffoldry_core::workflow::apply_field_effects(&mut data, &field_effects);
            // The person who decided is the actor.
            let ctx = crate::service::history::ctx_for(&caller.eppn, crate::repository::ActorKind::User, slug, &data, state);
            record_write = Some((slug.to_string(), instance.record_id.clone(), data, ctx));
        }
    }

    let decision_payload = json!({ "instance_id": instance.id, "decision": decision });
    let ledger = crate::repository::AppendDecisionParams {
        principal: caller.eppn.clone(),
        organization_code: "institutional".to_string(),
        app_slug: Some(instance.app_slug.clone()),
        decision_type: DecisionType::WorkflowRuleApproved,
        oscal_control_id: "AC-03".to_string(),
        rationale: format!("{decision} by {} on {}", caller.eppn, instance.id),
        payload: decision_payload,
    };
    let was_waiting_at = waiting_step_id.clone();
    instance.status = if decision == "approve" { ProcessStatus::Completed } else { ProcessStatus::Rejected };
    instance.waiting_step_id = None;
    // One transaction: lock the instance, check it is still waiting at this step, then write the record, the new
    // state, and the ledger entry. A second decision on the same step finds it decided and writes nothing.
    match state
        .repository
        .decide_process_instance(&instance, &was_waiting_at, record_write, ledger)
        .map_err(|e| ServiceError::Internal(e.to_string()))?
    {
        crate::repository::Decided::Done => {}
        crate::repository::Decided::Stale => return Err(ServiceError::conflict("Process instance is not in Waiting status")),
    }
    // Notifications phase 4: the record's creator and the person who started the process are told the decision.
    let record = state.find_record(slug, &instance.record_id).ok().flatten();
    let creator = record.as_ref().and_then(|r| r.created_by.clone());
    let table = record.as_ref().and_then(|r| r.data.get("_table_id").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    let people: Vec<String> = creator.into_iter().chain(instance.started_by.clone()).collect();
    let notice = crate::notifications::Notice::DecisionMade {
        by: crate::notifications::PersonRef(caller.eppn.clone()),
        app: crate::notifications::AppRef(slug.to_string()),
        table: crate::notifications::TableRef(table),
        step: crate::notifications::StepRef(instance.id.clone()),
        approved: crate::notifications::Flag(decision == "approve"),
    };
    let _ = crate::notifications::notify(state, &crate::notifications::Recipients::People(people), Some(&caller.eppn), &notice);
    Ok(instance)
}

/// The most waiting decisions one call returns.
pub const MAX_WAITING: usize = 200;

/// Waiting instances in apps the caller can read, where the caller may decide now.
pub fn list_waiting_decisions(caller: &AuthUser, state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let waiting: Vec<ProcessInstance> = state
        .repository
        .list_process_instances(None, Some("Waiting"))
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    let mut readable: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut out: Vec<Value> = Vec::new();
    let mut sorted = waiting;
    sorted.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.id.cmp(&b.id)));
    for inst in sorted {
        let can_read = *readable
            .entry(inst.app_slug.clone())
            .or_insert_with(|| authorize_app(caller, &inst.app_slug, AppAction::Read, state).is_ok());
        let in_scope = crate::service::scope::readable_ids(caller, &inst.app_slug, std::slice::from_ref(&inst.record_id), state).contains(&inst.record_id);
        if can_read && in_scope && crate::service::approvers::caller_can_decide(state, caller, &inst) {
            out.push(crate::service::approvers::instance_view(state, &inst));
            if out.len() >= MAX_WAITING {
                break;
            }
        }
    }
    Ok(out)
}
