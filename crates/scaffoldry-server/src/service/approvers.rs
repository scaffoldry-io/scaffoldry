//! The engine's side of approvers: build the resolver's context from server state, and ask it
//! who may decide a waiting step. The resolver itself is pure and lives in `scaffoldry-core`.

use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use chrono::Utc;
use scaffoldry_core::approver::{
    can_decide, effective_approver, resolve_approvers, validate_approver, ApproverSpec,
    Collaborator, Holding, KnownSets, NoApprover, Person, Resolution, ResolveCtx, Unit,
};
use scaffoldry_core::{AutomationRule, ProcessInstance, ProcessStatus, StepKind};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn problem_code(p: NoApprover) -> String {
    serde_json::to_value(p)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The holdings the resolver may need when it starts at `units` and walks to the root: those at each unit on the way,
/// and every holding of the `people` named, whose own unit is another place to start.
pub fn path_holdings(state: &SharedState, units: &[uuid::Uuid], people: &[String]) -> Vec<Holding> {
    let own = state.repository.roles_of_people(people).unwrap_or_default();
    let mut path: Vec<uuid::Uuid> = Vec::new();
    {
        let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
        for start in units.iter().copied().chain(
            own.iter()
                .filter(|r| r.is_primary)
                .map(|r| r.organization_id),
        ) {
            for anc in tree.ancestors(start) {
                if !path.contains(&anc) {
                    path.push(anc);
                }
            }
        }
    }
    let mut roles = state.repository.roles_in_units(&path).unwrap_or_default();
    for r in own {
        if !roles.iter().any(|x| x.id == r.id) {
            roles.push(r);
        }
    }
    roles
        .into_iter()
        .map(|r| Holding {
            eppn: r.eppn,
            unit_id: r.organization_id.to_string(),
            position_key: r.position_key,
            is_primary: r.is_primary,
        })
        .collect()
}

/// The resolver's inputs for one record of one app, read once.
pub fn context(
    state: &SharedState,
    app_slug: &str,
    record_id: &str,
    started_by: Option<&str>,
    named: &[String],
) -> ResolveCtx {
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

    let ws_id = state.engine.read().ok().and_then(|e| {
        e.resolve_by_slug(app_slug)
            .and_then(|m| m.workspace_id.clone())
    });
    let workspace_unit = ws_id
        .as_ref()
        .and_then(|id| {
            state
                .workspaces
                .read()
                .ok()
                .and_then(|w| w.get(id).and_then(|ws| ws.organization_id))
        })
        .map(|u| u.to_string());
    let collaborators: Vec<Collaborator> = ws_id
        .as_ref()
        .map(|id| crate::service::members::people_records(id, state))
        .unwrap_or_default()
        .into_iter()
        .map(|c| Collaborator {
            eppn: c.eppn,
            role: c.role,
            affiliation: c.scoped_affiliation,
        })
        .collect();

    let submitter = state
        .find_record(app_slug, record_id)
        .ok()
        .flatten()
        .and_then(|r| r.created_by);

    // The resolver walks from the workspace's unit, or the submitter's own, up to the root. It needs the holdings
    // on that path and the submitter's own, not everyone's.
    let start_units: Vec<uuid::Uuid> = workspace_unit
        .as_deref()
        .and_then(|u| u.parse().ok())
        .into_iter()
        .collect();
    let holdings: Vec<Holding> = path_holdings(state, &start_units, submitter.as_slice());

    // People the platform holds a user record for carry their state. Anyone else, such as a
    // seeded persona, counts as active, since nothing says otherwise.
    let mut extra: HashSet<String> = HashSet::new();
    for e in holdings
        .iter()
        .map(|h| &h.eppn)
        .chain(collaborators.iter().map(|c| &c.eppn))
    {
        extra.insert(e.clone());
    }
    extra.extend(submitter.iter().cloned());
    extra.extend(named.iter().cloned());
    extra.extend(started_by.map(str::to_string));
    let wanted: Vec<String> = extra.iter().cloned().collect();
    let mut people: Vec<Person> = state
        .repository
        .users_by_names(&wanted)
        .unwrap_or_default()
        .iter()
        .map(|u| Person {
            eppn: u.user_name.clone(),
            display_name: crate::service::people::display_name(u),
            active: u.active,
            on_hold: u.admin_hold,
        })
        .collect();
    let known: HashSet<String> = people.iter().map(|p| p.eppn.to_lowercase()).collect();
    for e in extra {
        if !known.contains(&e.to_lowercase()) {
            people.push(Person {
                eppn: e.clone(),
                display_name: e,
                active: true,
                on_hold: false,
            });
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

/// The people an instance names itself: the person in the spec and the one it was assigned to. Their state
/// must be loaded even when they hold nothing on the path.
fn named_in(spec: &ApproverSpec, inst: &ProcessInstance) -> Vec<String> {
    let mut named: Vec<String> = inst.assigned_to.iter().cloned().collect();
    if let ApproverSpec::Person { eppn } = spec {
        named.push(eppn.clone());
    }
    named
}

/// The approver spec of the user task an instance waits on.
pub fn spec_of(state: &SharedState, inst: &ProcessInstance) -> Option<ApproverSpec> {
    let step_id = inst.waiting_step_id.as_deref()?;
    let rule = crate::service::processes::rule_for(state, inst)?;
    let step = rule.steps.iter().find(|s| s.id == step_id)?;
    match &step.kind {
        StepKind::UserTask { role, approver, .. } => Some(effective_approver(role, approver)),
        StepKind::Service { .. } => None,
    }
}

pub fn resolution_for(
    state: &SharedState,
    inst: &ProcessInstance,
) -> Option<(ApproverSpec, ResolveCtx, Resolution)> {
    let spec = spec_of(state, inst)?;
    let ctx = context(
        state,
        &inst.app_slug,
        &inst.record_id,
        inst.started_by.as_deref(),
        &named_in(&spec, inst),
    );
    let resolution = resolve_approvers(&spec, &ctx);
    Some((spec, ctx, resolution))
}

/// Whether this caller may decide the instance's waiting step now.
pub fn caller_can_decide(state: &SharedState, caller: &AuthUser, inst: &ProcessInstance) -> bool {
    if inst.status != ProcessStatus::Waiting {
        return false;
    }
    let Some(spec) = spec_of(state, inst) else {
        return false;
    };
    let ctx = context(
        state,
        &inst.app_slug,
        &inst.record_id,
        inst.started_by.as_deref(),
        &named_in(&spec, inst),
    );
    can_decide(
        &caller.eppn,
        &caller.affiliation,
        inst.assigned_to.as_deref(),
        &spec,
        &ctx,
    )
}

/// Sets `no_approver` from the resolver. Returns true when the stored value changed. The first
/// time it is set, a line goes in the instance log.
pub fn refresh_no_approver(state: &SharedState, inst: &mut ProcessInstance) -> bool {
    if inst.status != ProcessStatus::Waiting {
        return false;
    }
    let Some((_, _, resolution)) = resolution_for(state, inst) else {
        return false;
    };
    // An assignment overrides the position, so a reassigned step has an approver.
    let problem = if inst.assigned_to.is_some() {
        None
    } else {
        resolution.problem.map(problem_code)
    };
    if inst.no_approver == problem {
        return false;
    }
    if let Some(code) = &problem {
        if inst.no_approver.is_none() {
            inst.log.push(format!(
                "No approver for step '{}': {code}",
                inst.waiting_step_id.clone().unwrap_or_default()
            ));
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
                let name = ctx
                    .people
                    .iter()
                    .find(|p| p.eppn.eq_ignore_ascii_case(who))
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| who.clone());
                vec![json!({ "eppn": who, "display_name": name, "via": "Named" })]
            }
            (None, Some((_, _, r))) => r
                .approvers
                .iter()
                .map(|a| serde_json::to_value(a).unwrap_or(Value::Null))
                .collect(),
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

/// What the platform knows, for checking this rule: the position types, and the people the rule names.
pub fn known_sets(state: &SharedState, rule: &AutomationRule) -> KnownSets {
    let types = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner());
    let named: Vec<String> = rule
        .steps
        .iter()
        .filter_map(|step| match &step.kind {
            StepKind::UserTask { role, approver, .. } => match effective_approver(role, approver) {
                ApproverSpec::Person { eppn } => Some(eppn),
                _ => None,
            },
            StepKind::Service { .. } => None,
        })
        .collect();
    KnownSets {
        position_keys: types
            .values()
            .filter(|p| p.retired_at.is_none())
            .map(|p| p.key.clone())
            .collect(),
        retired_keys: types
            .values()
            .filter(|p| p.retired_at.is_some())
            .map(|p| p.key.clone())
            .collect(),
        eppns: state
            .repository
            .users_by_names(&named)
            .unwrap_or_default()
            .into_iter()
            .map(|u| u.user_name)
            .collect(),
    }
}

/// Every user task's approver must be valid. A position that resolves to nobody today is fine.
pub fn validate_rule(state: &SharedState, rule: &AutomationRule) -> Result<(), ServiceError> {
    let known = known_sets(state, rule);
    for step in &rule.steps {
        if let StepKind::UserTask { role, approver, .. } = &step.kind {
            validate_approver(&effective_approver(role, approver), &known)
                .map_err(|e| ServiceError::bad_request(format!("Step '{}': {e}", step.id)))?;
        }
    }
    Ok(())
}

/// The people `spec` resolves to today for a thing that belongs to `unit`. `extra` names people the resolver should know
/// about. `submitter`, when given, is never one of them. The policy library asks it who owns and who reviews a policy.
pub fn resolution_at(state: &SharedState, spec: &ApproverSpec, unit: uuid::Uuid, extra: &[&str], submitter: Option<&str>) -> scaffoldry_core::approver::Resolution {
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
    let mut named: Vec<String> = extra.iter().map(|e| e.to_string()).collect();
    if let ApproverSpec::Person { eppn } = spec {
        named.push(eppn.clone());
    }
    let submitters: Vec<String> = submitter.map(str::to_string).into_iter().collect();
    let holdings = path_holdings(state, &[unit], &submitters);
    let mut wanted: HashSet<String> = holdings.iter().map(|h| h.eppn.clone()).collect();
    wanted.extend(named);
    let wanted: Vec<String> = wanted.into_iter().collect();
    let mut people: Vec<Person> = state
        .repository
        .users_by_names(&wanted)
        .unwrap_or_default()
        .iter()
        .map(|u| Person {
            eppn: u.user_name.clone(),
            display_name: crate::service::people::display_name(u),
            active: u.active,
            on_hold: u.admin_hold,
        })
        .collect();
    for e in wanted {
        if !people.iter().any(|p| p.eppn.eq_ignore_ascii_case(&e)) {
            people.push(Person {
                eppn: e.clone(),
                display_name: e,
                active: true,
                on_hold: false,
            });
        }
    }
    let ctx = ResolveCtx {
        units,
        holdings,
        people,
        delegations: vec![],
        collaborators: vec![],
        now: Utc::now().timestamp(),
        submitter: submitter.map(str::to_string),
        starter: None,
        workspace_unit: Some(unit.to_string()),
    };
    resolve_approvers(spec, &ctx)
}

/// The eppns of `resolution_at`.
pub fn resolved_approvers(state: &SharedState, spec: &ApproverSpec, unit: uuid::Uuid, extra: &[&str], submitter: Option<&str>) -> Vec<String> {
    resolution_at(state, spec, unit, extra, submitter).approvers.into_iter().map(|a| a.eppn).collect()
}

pub fn resolves_to(state: &SharedState, spec: &ApproverSpec, unit: uuid::Uuid, who: &str) -> bool {
    resolved_approvers(state, spec, unit, &[who], None)
        .iter()
        .any(|a| a.eq_ignore_ascii_case(who))
}

/// A spec the platform knows how to resolve: its position exists and is in use, or its person exists.
pub fn check_spec(state: &SharedState, spec: &ApproverSpec) -> Result<(), String> {
    let types = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner());
    let named: Vec<String> = match spec {
        ApproverSpec::Person { eppn } => vec![eppn.clone()],
        _ => Vec::new(),
    };
    let known = KnownSets {
        position_keys: types
            .values()
            .filter(|p| p.retired_at.is_none())
            .map(|p| p.key.clone())
            .collect(),
        retired_keys: types
            .values()
            .filter(|p| p.retired_at.is_some())
            .map(|p| p.key.clone())
            .collect(),
        eppns: state
            .repository
            .users_by_names(&named)
            .unwrap_or_default()
            .into_iter()
            .map(|u| u.user_name)
            .collect(),
    };
    validate_approver(spec, &known)
}
