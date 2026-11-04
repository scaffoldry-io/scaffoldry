//! Workflows phase 7: what the server says about a flow the editor holds. It writes nothing.
//!
//! The editor sends its flow after each edit. The answer holds each problem with the path to its step, the grants as
//! sentences, the parties that would approve it, the checks a proposal would raise, and the flow in sentences. The
//! browser never decides whether a flow is valid: it shows this answer.
//!
//! A path is a list of numbers. The first is the step's index in `steps`. When that step is a branch, the next number
//! is the arm, with `otherwise` after the last `when` arm, then the step's index in that arm, and so on.

use crate::service::access::{authorize_app, authorize_workspace, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::{AutomationRule, ProcessStep, StepKind};
use scaffoldry_engine::{AppManifest, HostRouter};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Every step's path, by step id. A repeated id keeps its first path.
pub fn step_paths(steps: &[ProcessStep]) -> HashMap<String, Vec<usize>> {
    fn walk(steps: &[ProcessStep], prefix: &[usize], out: &mut HashMap<String, Vec<usize>>) {
        for (i, s) in steps.iter().enumerate() {
            let mut at = prefix.to_vec();
            at.push(i);
            out.entry(s.id.clone()).or_insert_with(|| at.clone());
            if let StepKind::Branch { branches, otherwise } = &s.kind {
                for (arm, b) in branches.iter().enumerate() {
                    walk(&b.steps, &[at.as_slice(), &[arm]].concat(), out);
                }
                walk(otherwise, &[at.as_slice(), &[branches.len()]].concat(), out);
            }
        }
    }
    let mut out = HashMap::new();
    walk(steps, &[], &mut out);
    out
}

/// The step a problem is about. Problems about a step start `step '<id>'` or `step id '<id>'`.
fn step_of(problem: &str) -> Option<&str> {
    let rest = problem.strip_prefix("step id '").or_else(|| problem.strip_prefix("step '"))?;
    rest.split('\'').next()
}

/// The flow as it would be saved, and the app it belongs to. A flow with no app is checked against its home
/// workspace alone.
fn flow_and_manifest(body: Value, state: &SharedState) -> Result<(AutomationRule, AppManifest), ServiceError> {
    let rule: AutomationRule = serde_json::from_value(body).map_err(|e| ServiceError::BadRequest(format!("rule_schema: the body is not a flow: {e}")))?;
    let manifest = manifest_of(&rule, state)?;
    Ok((rule, manifest))
}

/// The app a flow belongs to. A flow with no app gets an empty app in its home workspace.
pub(crate) fn manifest_of(rule: &AutomationRule, state: &SharedState) -> Result<AppManifest, ServiceError> {
    if !rule.app_slug.is_empty() {
        let manifest = state
            .engine
            .read()
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .resolve_by_slug(&rule.app_slug)
            .cloned()
            .ok_or_else(|| ServiceError::NotFound(format!("App '{}' not found", rule.app_slug)))?;
        return Ok(manifest);
    }
    let ws = rule.home_workspace.clone().ok_or_else(|| ServiceError::BadRequest("rule_schema: a flow names its app or its home workspace".into()))?;
    serde_json::from_value(json!({
        "slug": "", "title": "", "description": "", "organization_code": "", "department": "", "workspace_id": ws, "views": [], "tables": [],
    }))
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// An app manager, the owner of its workspace, or an administrator may validate. A flow with no app needs its home
/// workspace's owner or an administrator.
pub(crate) fn authorize(caller: &AuthUser, rule: &AutomationRule, manifest: &AppManifest, state: &SharedState) -> Result<(), ServiceError> {
    if !rule.app_slug.is_empty() && authorize_app(caller, &rule.app_slug, AppAction::Manage, state).is_ok() {
        return Ok(());
    }
    match manifest.workspace_id.as_deref().or(rule.home_workspace.as_deref()) {
        Some(ws) => authorize_workspace(caller, ws, "manage_workspace", state),
        None => Err(ServiceError::forbidden("Only someone who manages this app may check its flows")),
    }
}

pub fn validate(caller: &AuthUser, body: Value, state: &SharedState) -> Result<Value, ServiceError> {
    let raw = body.clone();
    let (mut rule, manifest) = flow_and_manifest(body, state)?;
    authorize(caller, &rule, &manifest, state)?;
    crate::service::flows::derive(state, &manifest, &mut rule);

    let paths = step_paths(&rule.steps);
    let problems: Vec<Value> = crate::service::processes::validate_rule(state, &manifest, &rule)
        .into_iter()
        .map(|message| {
            let step = step_of(&message).filter(|s| paths.contains_key(*s)).map(str::to_string);
            let path = step.as_ref().and_then(|s| paths.get(s)).cloned();
            json!({ "message": message, "step": step, "path": path })
        })
        .collect();

    let is_flow = crate::service::flows::is_flow(&rule);
    let grants = is_flow.then(|| scaffoldry_engine::flow::grants_sentence(&rule.grants, &rule.app_slug, &crate::service::flows::manifests_with(state, &manifest, &rule)));

    // The checks a proposal holding this flow would raise. A flow with problems fails `rule_schema` and raises no more.
    let mut checks = match crate::service::proposals::check_rules_part(caller, vec![raw], &rule.app_slug, &manifest, &[], state) {
        Ok(part) => part.checks,
        Err(ServiceError::BadRequest(m)) => vec![json!({ "name": "rule_schema", "outcome": "fail", "messages": [m] })],
        Err(e) => return Err(e),
    };
    let home = manifest.workspace_id.clone().or(rule.home_workspace.clone()).unwrap_or_default();
    let mut parties = vec![home.clone()];
    if problems.is_empty() {
        if let Some(plan) = crate::service::flow_parties::plan(state, &home, &manifest, std::slice::from_ref(&rule))? {
            parties = plan.parties;
            checks.extend(plan.checks);
        }
    }

    Ok(json!({
        "problems": problems,
        "grants": grants,
        "parties": parties,
        "checks": checks,
        "description": scaffoldry_core::workflow::describe_rule(&rule),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scaffoldry_core::workflow::Branch;

    fn s(id: &str, kind: StepKind) -> ProcessStep {
        ProcessStep { id: id.into(), when: vec![], kind }
    }

    #[test]
    fn paths_walk_into_arms_with_otherwise_last() {
        let leaf = || StepKind::Service { action: scaffoldry_core::ActionType::UpdateRecordStatus { new_status: "x".into() } };
        let branch = StepKind::Branch { branches: vec![Branch { when: vec![], steps: vec![s("a", leaf())] }], otherwise: vec![s("b", leaf()), s("c", leaf())] };
        let paths = step_paths(&[s("first", leaf()), s("split", branch)]);
        assert_eq!(paths["first"], vec![0]);
        assert_eq!(paths["split"], vec![1]);
        assert_eq!(paths["a"], vec![1, 0, 0]);
        assert_eq!(paths["c"], vec![1, 1, 1]);
    }

    #[test]
    fn a_problem_names_its_step() {
        assert_eq!(step_of("step 'inner': the table 'logs' has no field 'nope'"), Some("inner"));
        assert_eq!(step_of("step id 'a' is used twice"), Some("a"));
        assert_eq!(step_of("a rule may not hold a webhook"), None);
    }
}
