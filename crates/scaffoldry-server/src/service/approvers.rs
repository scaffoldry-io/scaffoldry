//! The engine's side of approvers: build the resolver's context from server state, and ask it
//! who may decide a waiting step. The resolver itself is pure and lives in `scaffoldry-core`.

use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use chrono::Utc;
use scaffoldry_core::approver::{
    can_decide, effective_approver, resolve_approvers, validate_approver, ApproverSpec, Collaborator,
    Holding, KnownSets, NoApprover, Person, ResolveCtx, Resolution, Unit,
};
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, StepKind};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn problem_code(p: NoApprover) -> String {
    serde_json::to_value(p).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The resolver's inputs for one record of one app, read once.
pub fn context(state: &SharedState, app_slug: &str, record_id: &str, started_by: Option<&str>) -> ResolveCtx {
    use scaffoldry_engine::HostRouter;

    let units: Vec<Unit> = state
        .organizations
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .map(|o| Unit {
            id: o.id.to_string(),
            parent_id: o.parent_id.map(|p| p.to_string()),
            org_type: o.org_type.clone(),
            name: o.name.clone(),
        })
        .collect();

    let holdings: Vec<Holding> = state
        .roles
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|r| Holding {
            eppn: r.eppn.clone(),
            unit_id: r.organization_id.to_string(),
            position_key: r.position_key.clone(),
            is_primary: r.is_primary,
        })
        .collect();

    let ws_id = state
        .engine
        .read()
        .ok()
        .and_then(|e| e.resolve_by_slug(app_slug).and_then(|m| m.workspace_id.clone()));
    let workspace_unit = ws_id
        .as_ref()
        .and_then(|id| state.workspaces.read().ok().and_then(|w| w.get(id).and_then(|ws| ws.organization_id)))
        .map(|u| u.to_string());
    let collaborators: Vec<Collaborator> = ws_id
        .as_ref()
        .and_then(|id| state.collaborators.read().ok().map(|c| c.get(id).cloned().unwrap_or_default()))
        .unwrap_or_default()
        .into_iter()
        .map(|c| Collaborator { eppn: c.eppn, role: c.role, affiliation: c.scoped_affiliation })
        .collect();

    let submitter = state
        .records
        .read()
        .ok()
        .and_then(|r| r.get(app_slug).and_then(|recs| recs.iter().find(|x| x.id == record_id).and_then(|x| x.created_by.clone())));

    // People the platform holds a user record for carry their state. Anyone else, such as a
    // seeded persona, counts as active, since nothing says otherwise.
    let mut people: Vec<Person> = state
        .users
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .map(|u| Person {
            eppn: u.user_name.clone(),
            display_name: crate::service::people::display_name(u),
            active: u.active,
            on_hold: u.admin_hold,
        })
        .collect();
    let known: HashSet<String> = people.iter().map(|p| p.eppn.to_lowercase()).collect();
    let mut extra: HashSet<String> = HashSet::new();
    for e in holdings.iter().map(|h| &h.eppn).chain(collaborators.iter().map(|c| &c.eppn)) {
        extra.insert(e.clone());
    }
    extra.extend(submitter.iter().cloned());
    extra.extend(started_by.map(str::to_string));
    for e in extra {
        if !known.contains(&e.to_lowercase()) {
            people.push(Person { eppn: e.clone(), display_name: e, active: true, on_hold: false });
        }
    }

    ResolveCtx {
        units,
        holdings,
        people,
        delegations: vec![],
        collaborators,
        now: Utc::now().timestamp(),
        submitter,
        starter: started_by.map(str::to_string),
        workspace_unit,
    }
}

/// The approver spec of the user task an instance waits on.
pub fn spec_of(state: &SharedState, inst: &ProcessInstance) -> Option<ApproverSpec> {
    let step_id = inst.waiting_step_id.as_deref()?;
    let automations = state.automations.read().ok()?;
    let rule = automations.get(&inst.app_slug)?.iter().find(|r| r.id == inst.rule_id)?;
    let step = rule.steps.iter().find(|s| s.id == step_id)?;
    match &step.kind {
        StepKind::UserTask { role, approver, .. } => Some(effective_approver(role, approver)),
        StepKind::Service { .. } => None,
    }
}

pub fn resolution_for(state: &SharedState, inst: &ProcessInstance) -> Option<(ApproverSpec, ResolveCtx, Resolution)> {
    let spec = spec_of(state, inst)?;
    let ctx = context(state, &inst.app_slug, &inst.record_id, inst.started_by.as_deref());
    let resolution = resolve_approvers(&spec, &ctx);
    Some((spec, ctx, resolution))
}

/// Whether this caller may decide the instance's waiting step now.
pub fn caller_can_decide(state: &SharedState, caller: &AuthUser, inst: &ProcessInstance) -> bool {
    if inst.status != ProcessStatus::Waiting {
        return false;
    }
    let Some(spec) = spec_of(state, inst) else { return false };
    let ctx = context(state, &inst.app_slug, &inst.record_id, inst.started_by.as_deref());
    can_decide(&caller.eppn, &caller.affiliation, inst.assigned_to.as_deref(), &spec, &ctx)
}

/// Sets `no_approver` from the resolver. Returns true when the stored value changed. The first
/// time it is set, a line goes in the instance log.
pub fn refresh_no_approver(state: &SharedState, inst: &mut ProcessInstance) -> bool {
    if inst.status != ProcessStatus::Waiting {
        return false;
    }
    let Some((_, _, resolution)) = resolution_for(state, inst) else { return false };
    // An assignment overrides the position, so a reassigned step has an approver.
    let problem = if inst.assigned_to.is_some() { None } else { resolution.problem.map(problem_code) };
    if inst.no_approver == problem {
        return false;
    }
    if let Some(code) = &problem {
        if inst.no_approver.is_none() {
            inst.log.push(format!("No approver for step '{}': {code}", inst.waiting_step_id.clone().unwrap_or_default()));
        }
    }
    inst.no_approver = problem;
    true
}

/// An instance as a list row: the instance, and who may decide it now.
pub fn instance_view(state: &SharedState, inst: &ProcessInstance) -> Value {
    let mut v = serde_json::to_value(inst).unwrap_or(Value::Null);
    let approvers: Vec<Value> = if inst.status == ProcessStatus::Waiting {
        match (&inst.assigned_to, resolution_for(state, inst)) {
            (Some(who), Some((_, ctx, _))) => {
                let name = ctx.people.iter().find(|p| p.eppn.eq_ignore_ascii_case(who)).map(|p| p.display_name.clone()).unwrap_or_else(|| who.clone());
                vec![json!({ "eppn": who, "display_name": name, "via": "Named" })]
            }
            (None, Some((_, _, r))) => r.approvers.iter().map(|a| serde_json::to_value(a).unwrap_or(Value::Null)).collect(),
            _ => vec![],
        }
    } else {
        vec![]
    };
    if let Some(map) = v.as_object_mut() {
        map.insert("approvers".to_string(), Value::Array(approvers));
    }
    v
}

pub fn known_sets(state: &SharedState) -> KnownSets {
    let types = state.position_types.read().unwrap_or_else(|p| p.into_inner());
    KnownSets {
        position_keys: types.values().filter(|p| p.retired_at.is_none()).map(|p| p.key.clone()).collect(),
        retired_keys: types.values().filter(|p| p.retired_at.is_some()).map(|p| p.key.clone()).collect(),
        eppns: state.users.read().unwrap_or_else(|p| p.into_inner()).values().map(|u| u.user_name.clone()).collect(),
    }
}

/// Every user task's approver must be valid. A position that resolves to nobody today is fine.
pub fn validate_rule(state: &SharedState, rule: &AutomationRule) -> Result<(), ServiceError> {
    let known = known_sets(state);
    for step in &rule.steps {
        if let StepKind::UserTask { role, approver, .. } = &step.kind {
            validate_approver(&effective_approver(role, approver), &known)
                .map_err(|e| ServiceError::bad_request(format!("Step '{}': {e}", step.id)))?;
        }
    }
    Ok(())
}
