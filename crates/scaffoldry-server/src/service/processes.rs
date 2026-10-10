//! Process definitions: what a rule must satisfy to be saved, and saving a version of one.
//!
//! A rule is governed like a manifest. It is proposed, checked, and approved. `save_rule` is the
//! one function that writes a rule version, and approval and the administrator's enable and
//! disable are the only callers.

use crate::service::ServiceError;
use crate::state::SharedState;
use scaffoldry_core::approver::{effective_approver, validate_approver};
use scaffoldry_core::workflow::{ActionType, FieldPredicate, StepKind, TriggerEvent};
use scaffoldry_core::{AutomationRule, ProcessInstance};
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
