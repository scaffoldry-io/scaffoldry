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
    let submitter = state.find_record(app_slug, record_id).ok().flatten().and_then(|r| r.created_by);
    context_for(state, app_slug, submitter, started_by, named)
}

/// The resolver's inputs for a record of `app_slug` that `submitter` made.
pub fn context_for(state: &SharedState, app_slug: &str, submitter: Option<String>, started_by: Option<&str>, named: &[String]) -> ResolveCtx {
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

    // The resolver walks from the workspace's unit, or the submitter's own, up to the root. It needs the holdings
    // on that path and the submitter's own, not everyone's.
    let start_units: Vec<uuid::Uuid> = workspace_unit
        .as_deref()
        .and_then(|u| u.parse().ok())
        .into_iter()
        .collect();
    let holdings: Vec<Holding> = path_holdings(state, &start_units, submitter.as_slice());
    let delegations = delegations_of(state, &holdings);

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
    extra.extend(delegations.iter().map(|d| d.delegate.clone()));
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
        delegations,
        collaborators,
        now: Utc::now().timestamp(),
        submitter,
        starter: started_by.map(str::to_string),
        workspace_unit,
    }
}

/// Approvers phase 5: who a step with `spec` would go to today, for a record `submitter` made. Nothing is created.
pub fn preview(caller: &AuthUser, app_slug: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::access::authorize_app(caller, app_slug, crate::service::access::AppAction::Manage, state)?;
    let spec: ApproverSpec = serde_json::from_value(body.get("approver").cloned().unwrap_or(Value::Null))
        .map_err(|e| ServiceError::bad_request(format!("approver: {e}")))?;
    let submitter = body.get("created_by").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&caller.eppn).to_string();
    let named: Vec<String> = match &spec {
        ApproverSpec::Person { eppn } => vec![eppn.clone()],
        _ => vec![],
    };
    let ctx = context_for(state, app_slug, Some(submitter), None, &named);
    let resolution = resolve_approvers(&spec, &ctx);
    let name_of = |id: &str| ctx.units.iter().find(|x| x.id == id).map(|u| u.name.clone());
    let unit_name = resolution.unit_id.as_deref().and_then(name_of);
    // Where a position search starts: the submitter's own unit, or the workspace's. A problem is said from there.
    let start = match &spec {
        ApproverSpec::Position { from: scaffoldry_core::approver::Origin::Submitter, .. } => ctx
            .holdings
            .iter()
            .find(|h| ctx.submitter.as_deref().is_some_and(|s| h.eppn.eq_ignore_ascii_case(s)) && h.is_primary && h.position_key.is_none())
            .map(|h| h.unit_id.clone())
            .or_else(|| ctx.workspace_unit.clone()),
        _ => ctx.workspace_unit.clone(),
    };
    let start_unit_name = start.as_deref().and_then(name_of);
    let approvers: Vec<Value> = resolution
        .approvers
        .iter()
        .map(|a| json!({ "eppn": a.eppn, "display_name": a.display_name, "via": a.via }))
        .collect();
    Ok(json!({
        "approvers": approvers,
        "unit_id": resolution.unit_id,
        "unit_name": unit_name,
        "start_unit_name": start_unit_name,
        "problem": resolution.problem.map(problem_code),
    }))
}

/// What the builder's `Who decides` control offers: the position types in use, by key and name.
pub fn options(caller: &AuthUser, app_slug: &str, state: &SharedState) -> Result<Value, ServiceError> {
    crate::service::access::authorize_app(caller, app_slug, crate::service::access::AppAction::Manage, state)?;
    let positions: Vec<Value> = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.retired_at.is_none())
        .map(|p| json!({ "key": p.key, "name": p.name }))
        .collect();
    let mut positions = positions;
    positions.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({ "positions": positions }))
}

/// The delegations made by the people who hold a position on the path (approvers phase 4).
fn delegations_of(state: &SharedState, holdings: &[Holding]) -> Vec<scaffoldry_core::approver::Delegation> {
    let mut holders: Vec<String> = holdings.iter().filter(|h| h.position_key.is_some()).map(|h| h.eppn.clone()).collect();
    holders.sort();
    holders.dedup();
    crate::service::delegations::for_resolution(state, &holders)
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

/// How the user task an instance waits on is decided.
pub fn decision_of(state: &SharedState, inst: &ProcessInstance) -> scaffoldry_core::workflow::DecisionRule {
    let rule = inst.waiting_step_id.as_deref().and_then(|step| crate::service::processes::rule_for(state, inst).map(|r| (step.to_string(), r)));
    match rule.as_ref().and_then(|(step, r)| scaffoldry_core::workflow::find_step(&r.steps, step).map(|s| s.kind.clone())) {
        Some(StepKind::UserTask { decision, .. }) => decision,
        _ => scaffoldry_core::workflow::DecisionRule::Single,
    }
}

/// Whether the step an instance waits on is a committee's vote (process v2 phase 4).
pub fn is_committee(state: &SharedState, inst: &ProcessInstance) -> bool {
    !decision_of(state, inst).is_single()
}

/// The approver spec of the user task an instance waits on.
pub fn spec_of(state: &SharedState, inst: &ProcessInstance) -> Option<ApproverSpec> {
    let step_id = inst.waiting_step_id.as_deref()?;
    let rule = crate::service::processes::rule_for(state, inst)?;
    let step = scaffoldry_core::workflow::find_step(&rule.steps, step_id)?;
    match &step.kind {
        StepKind::UserTask { role, approver, .. } => Some(effective_approver(role, approver)),
        StepKind::Service { .. } | StepKind::Branch { .. } => None,
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
    // Process v2 phase 4: a committee step is voted on by the voters fixed when it started, and nobody else.
    if !inst.voters.is_empty() || is_committee(state, inst) {
        return inst.voters.iter().any(|v| v.eq_ignore_ascii_case(&caller.eppn));
    }
    // Process v2 phase 3: a timer escalated the step to these people. The resolver chose them, so it already left out
    // the submitter and the starter.
    if inst.escalated_to.iter().any(|e| e.eq_ignore_ascii_case(&caller.eppn)) {
        return true;
    }
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
/// How many people may decide the step an instance waits at now (process v2, phase 1). An assignee counts as one.
pub fn approver_count(state: &SharedState, inst: &ProcessInstance) -> usize {
    let escalated = inst.escalated_to.len();
    if inst.assigned_to.is_some() {
        return 1 + escalated;
    }
    resolution_for(state, inst).map(|(_, _, r)| r.approvers.len()).unwrap_or(0) + escalated
}

/// Everyone who may decide the step an instance waits at now: the assignee or the resolved approvers, and the people a
/// timer escalated it to. Also the submitter, whom a notice leaves out.
pub fn current_approvers(state: &SharedState, inst: &ProcessInstance) -> (Vec<String>, Option<String>) {
    let (mut people, submitter) = match (&inst.assigned_to, resolution_for(state, inst)) {
        (Some(who), Some((_, ctx, _))) => (vec![who.clone()], ctx.submitter.clone()),
        (None, Some((_, ctx, r))) => (r.approvers.into_iter().map(|a| a.eppn).collect(), ctx.submitter.clone()),
        (Some(who), None) => (vec![who.clone()], None),
        (None, None) => (vec![], None),
    };
    for e in &inst.escalated_to {
        if !people.iter().any(|p| p.eq_ignore_ascii_case(e)) {
            people.push(e.clone());
        }
    }
    (people, submitter)
}

pub fn refresh_no_approver(state: &SharedState, inst: &mut ProcessInstance) -> bool {
    if inst.status != ProcessStatus::Waiting {
        return false;
    }
    let Some((_, _, resolution)) = resolution_for(state, inst) else {
        return false;
    };
    // An assignment overrides the position, so a reassigned step has an approver.
    let needed = decision_of(state, inst).voters_needed();
    let problem = if needed > 0 {
        // Process v2 phase 4: a committee needs enough voters, counted when the step was entered.
        (inst.voters.len() < needed).then(|| "too_few_voters".to_string())
    } else if inst.assigned_to.is_some() {
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
    let mut approvers = approvers;
    if inst.status == ProcessStatus::Waiting {
        for e in &inst.escalated_to {
            approvers.push(json!({ "eppn": e, "display_name": crate::service::process_timers::display_name(state, e), "via": "Escalated" }));
        }
    }
    if let Some(map) = v.as_object_mut() {
        map.insert("approvers".to_string(), Value::Array(approvers));
        // Process v2 phase 4: how far a committee's vote has got, as counts.
        if let Some(tally) = crate::service::process_votes::tally_view(state, inst) {
            map.insert("tally".to_string(), tally);
        }
        // Process v2 phase 3: what the step's next timer will do, in words.
        if inst.status == ProcessStatus::Waiting {
            if let Some(next) = crate::service::process_timers::next_timer_view(state, inst) {
                map.insert("next_timer".to_string(), next);
            }
        }
    }
    v
}

/// What the platform knows, for checking this rule: the position types, and the people the rule names.
pub fn known_sets(state: &SharedState, rule: &AutomationRule) -> KnownSets {
    let types = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner());
    let named: Vec<String> = scaffoldry_core::workflow::all_steps(&rule.steps)
        .into_iter()
        .flat_map(|step| match &step.kind {
            // The step's approver, and the people its timers escalate to (process v2 phase 3).
            StepKind::UserTask { role, approver, on_due, .. } => std::iter::once(effective_approver(role, approver))
                .chain(on_due.iter().filter_map(|d| match d {
                    scaffoldry_core::workflow::DueAction::Escalate { approver } => Some(approver.clone()),
                    _ => None,
                }))
                .filter_map(|spec| match spec {
                    ApproverSpec::Person { eppn } => Some(eppn),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            StepKind::Service { .. } | StepKind::Branch { .. } => vec![],
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
