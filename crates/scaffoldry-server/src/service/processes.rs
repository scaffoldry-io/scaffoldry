//! Process definitions: what a rule must satisfy to be saved, and saving a version of one.
//!
//! A rule is governed like a manifest. It is proposed, checked, and approved. `save_rule` is the
//! one function that writes a rule version, and approval and the administrator's enable and
//! disable are the only callers.

use crate::service::access::{authorize_app, AppAction};
use crate::service::history::Origin;
use crate::service::ServiceError;
use crate::state::{AuthUser, RecordDecisionInput, SharedState};
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
    for step in &rule.steps {
        match &step.kind {
            StepKind::Service { action } => out.push(action),
            StepKind::UserTask { approve, reject, .. } => {
                out.extend(approve.iter());
                out.extend(reject.iter());
            }
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
    for step in &rule.steps {
        out.extend(step.when.iter().map(|p: &FieldPredicate| p.field_name.clone()));
    }
    if let TriggerEvent::FieldChanged { field_name } = &rule.trigger {
        out.insert(field_name.clone());
    }
    out
}

/// Every problem that would stop a rule from being saved. Empty means it can be saved.
pub fn validate_rule(state: &SharedState, manifest: &AppManifest, rule: &AutomationRule) -> Vec<String> {
    let mut problems = Vec::new();
    let known = crate::service::approvers::known_sets(state);
    let names = field_names(manifest);
    let computed = computed_names(manifest);

    let mut seen = HashSet::new();
    for step in &rule.steps {
        if !seen.insert(step.id.as_str()) {
            problems.push(format!("step id '{}' is used twice", step.id));
        }
        if let StepKind::UserTask { role, approver, .. } = &step.kind {
            if let Err(e) = validate_approver(&effective_approver(role, approver), &known) {
                problems.push(format!("step '{}': {e}", step.id));
            }
        }
    }
    for step in &rule.steps {
        let actions: Vec<&ActionType> = match &step.kind {
            StepKind::Service { action } => vec![action],
            StepKind::UserTask { approve, reject, .. } => approve.iter().chain(reject.iter()).collect(),
        };
        if actions.iter().any(|a| matches!(a, ActionType::WebhookDispatch { .. })) {
            problems.push(format!("step '{}' is a webhook, which a rule may not hold", step.id));
        }
    }
    if rule.actions.iter().any(|a| matches!(a, ActionType::WebhookDispatch { .. })) {
        problems.push("a rule may not hold a webhook".to_string());
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

/// The rule an instance runs under: the version it started with, or the live rule for an old
/// instance that has none.
pub fn rule_for(state: &SharedState, inst: &ProcessInstance) -> Option<AutomationRule> {
    if inst.rule_version > 0 {
        if let Some(repo) = state.repository.as_deref() {
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
    let repo = state.repository.as_deref().ok_or_else(|| ServiceError::Internal("Process definitions need PostgreSQL".to_string()))?;
    let (saved, entry) = repo
        .save_rule(&rule, summary, proposal_id, changed_by, &manifest.organization_code)
        .map_err(|e| ServiceError::Internal(e.to_string()))?;
    state.ledger.write().unwrap_or_else(|p| p.into_inner()).push(entry);
    reload_rules(state, app_slug);
    Ok(saved)
}

/// Replaces the in-memory rules of an app with the live rows.
pub fn reload_rules(state: &SharedState, app_slug: &str) {
    if let Some(repo) = state.repository.as_deref() {
        if let Ok(rows) = repo.list_workflow_automations(Some(app_slug)) {
            state.automations.write().unwrap_or_else(|p| p.into_inner()).insert(app_slug.to_string(), rows);
        }
    }
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

    let mut instance = state
        .process_instances
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .get(instance_id)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound("Process instance not found".to_string()))?;
    if instance.app_slug != slug {
        return Err(ServiceError::NotFound("Process instance not found in this app".to_string()));
    }
    if instance.status != ProcessStatus::Waiting {
        return Err(ServiceError::conflict("Process instance is not in Waiting status"));
    }

    let waiting_step_id = instance.waiting_step_id.clone().unwrap_or_default();
    let mut found_step = false;
    let mut field_effects = Vec::new();
    if let Some(rule) = rule_for(state, &instance) {
        if let Some(step) = rule.steps.iter().find(|s| s.id == waiting_step_id) {
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

    // Apply field effects and write the record. This is a system write, so it needs no version.
    if !field_effects.is_empty() {
        let current = state.find_record(slug, &instance.record_id).map_err(ServiceError::Internal)?;
        if let Some(rec) = current {
            let mut data = rec.data.clone();
            scaffoldry_core::workflow::apply_field_effects(&mut data, &field_effects);
            // The person who decided is the actor.
            let ctx = crate::service::history::ctx_for(&caller.eppn, crate::repository::ActorKind::User, slug, &data, state);
            state.overwrite_record_data(slug, &instance.record_id, &data, &ctx).map_err(ServiceError::Internal)?;
        }
    }

    let decision_payload = json!({ "instance_id": instance.id, "decision": decision });
    state
        .append_ledger_entry(RecordDecisionInput {
            principal: caller.eppn.clone(),
            organization_code: "institutional".to_string(),
            app_slug: Some(instance.app_slug.clone()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: format!("{decision} by {} on {}", caller.eppn, instance.id),
            payload: &decision_payload,
        })
        .map_err(|e| ServiceError::Internal(e.to_string()))?;

    instance.status = if decision == "approve" { ProcessStatus::Completed } else { ProcessStatus::Rejected };
    instance.waiting_step_id = None;
    state.persist_process_instance(instance.clone()).map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(instance)
}

/// The most waiting decisions one call returns.
pub const MAX_WAITING: usize = 200;

/// Waiting instances in apps the caller can read, where the caller may decide now.
pub fn list_waiting_decisions(caller: &AuthUser, state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let waiting: Vec<ProcessInstance> = state
        .process_instances
        .read()
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .values()
        .filter(|i| i.status == ProcessStatus::Waiting)
        .cloned()
        .collect();
    let mut readable: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut out: Vec<Value> = Vec::new();
    let mut sorted = waiting;
    sorted.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.id.cmp(&b.id)));
    for inst in sorted {
        let can_read = *readable
            .entry(inst.app_slug.clone())
            .or_insert_with(|| authorize_app(caller, &inst.app_slug, AppAction::Read, state).is_ok());
        if can_read && crate::service::approvers::caller_can_decide(state, caller, &inst) {
            out.push(crate::service::approvers::instance_view(state, &inst));
            if out.len() >= MAX_WAITING {
                break;
            }
        }
    }
    Ok(out)
}
