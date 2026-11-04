//! Process v2 phase 4: committee votes.
//!
//! A user task whose `decision` is `AllOf` or `AtLeast(n)` is voted on by the voters fixed when it was entered: the
//! people its approver resolved to then, with their delegates, less the submitter and the starter. A person votes once
//! and may change the vote until the step closes. The step closes when `vote_outcome` says so, in the transaction of the
//! vote that closes it, with one ledger entry that holds the tally and every vote.
//!
//! The votes are visible to the voters and to the app's owners and admins, and never to the submitter or the starter.

use crate::repository::{ActorKind, AppendDecisionParams};
use scaffoldry_core::DecisionType;
use crate::repository_votes::{Ballot, Closing, Voted};
use crate::service::access::{authorize_app, AppAction};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::workflow::{DecisionRule, RejectRule};
use scaffoldry_core::{ActionType, ProcessInstance, ProcessStatus};
use serde_json::{json, Value};

/// The parts of a committee step a vote needs.
pub struct Committee<'a> {
    pub decision: DecisionRule,
    pub reject_rule: RejectRule,
    pub approve: &'a [ActionType],
    pub reject: &'a [ActionType],
}

/// Whether a vote to reject needs a comment: the `process.require_reject_comment` setting, on unless set off.
pub fn reject_needs_comment(state: &SharedState) -> bool {
    state.settings.read().ok().and_then(|s| s.get("process.require_reject_comment").and_then(Value::as_bool)).unwrap_or(true)
}

/// The voters of a committee step entered now: everyone who may decide it, delegates included. The resolver already
/// leaves out the submitter and the starter.
pub fn voters_for(state: &SharedState, inst: &ProcessInstance) -> Vec<String> {
    crate::service::approvers::current_approvers(state, inst).0
}

fn is_voter(inst: &ProcessInstance, who: &str) -> bool {
    inst.voters.iter().any(|v| v.eq_ignore_ascii_case(who))
}

/// What the step becomes when the votes close it, approved or not.
fn vote_closing(caller: &AuthUser, slug: &str, inst: &ProcessInstance, approved: bool, actions: &[ActionType], state: &SharedState) -> Result<Closing, ServiceError> {
    let mut next = inst.clone();
    next.status = if approved { ProcessStatus::Completed } else { ProcessStatus::Rejected };
    next.waiting_step_id = None;
    let effects = crate::service::processes::step_effects(actions, &mut next.log);
    let mut record = None;
    if !effects.is_empty() {
        if let Some(rec) = state.find_record(slug, &inst.record_id).map_err(ServiceError::Internal)? {
            let mut data = rec.data.clone();
            scaffoldry_core::workflow::apply_field_effects(&mut data, &effects);
            // The voter whose vote closed the step is the actor, as the decider is for one person's decision.
            let ctx = crate::service::history::ctx_for(&caller.eppn, ActorKind::User, slug, &data, state);
            record = Some((slug.to_string(), inst.record_id.clone(), data, ctx));
        }
    }
    Ok(Closing { next, record })
}

/// Casts the caller's vote on the committee step `step` of `inst`. Returns the instance as it is after the vote.
#[allow(clippy::too_many_arguments)]
pub fn vote(caller: &AuthUser, slug: &str, inst: ProcessInstance, step: &str, vote: &str, comment: Option<&str>, committee: &Committee<'_>, state: &SharedState) -> Result<ProcessInstance, ServiceError> {
    if !is_voter(&inst, &caller.eppn) {
        return Err(ServiceError::forbidden("Forbidden: Caller is not a voter on this step. The voters were fixed when it started."));
    }
    let comment = comment.map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    if vote == "reject" && comment.is_none() && reject_needs_comment(state) {
        return Err(ServiceError::bad_request("A vote to reject needs a comment that says why."));
    }
    let ballot = Ballot {
        instance_id: inst.id.clone(),
        step_id: step.to_string(),
        voter: caller.eppn.clone(),
        vote: vote.to_string(),
        comment,
        decision: committee.decision,
        reject_rule: committee.reject_rule,
        voters: inst.voters.clone(),
        on_approve: vote_closing(caller, slug, &inst, true, committee.approve, state)?,
        on_reject: vote_closing(caller, slug, &inst, false, committee.reject, state)?,
        ledger: AppendDecisionParams {
            principal: caller.eppn.clone(),
            organization_code: "institutional".to_string(),
            app_slug: Some(slug.to_string()),
            decision_type: DecisionType::WorkflowRuleApproved,
            oscal_control_id: "AC-03".to_string(),
            rationale: String::new(),
            payload: Value::Null,
        },
    };
    let (approve_next, reject_next) = (ballot.on_approve.next.clone(), ballot.on_reject.next.clone());
    match state.repository.cast_vote(ballot).map_err(|e| ServiceError::Internal(e.to_string()))? {
        Voted::Counted(_) => Ok(inst),
        Voted::Same => Err(ServiceError::conflict(format!("You already voted {vote} on this step. A vote may change, not repeat."))),
        Voted::Stale => Err(ServiceError::conflict("Process instance is not in Waiting status")),
        Voted::Closed { approved, .. } => {
            let next = if approved { approve_next } else { reject_next };
            let decision = if approved { "approve" } else { "reject" };
            crate::service::flow_chains::on_decided(state, slug, &next.rule_id, decision, &next.record_id, &caller.eppn);
            crate::service::processes::tell_decision(state, slug, &next, decision, &caller.eppn);
            Ok(next)
        }
    }
}

/// The votes on an instance, for its voters and for the app's owners and admins. The submitter and the starter may not
/// see them, whatever else they hold.
pub fn committee_votes(caller: &AuthUser, slug: &str, instance_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    authorize_app(caller, slug, AppAction::Read, state)?;
    let inst = state
        .repository
        .get_process_instance(instance_id)
        .map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?
        .filter(|i| i.app_slug == slug)
        .ok_or_else(|| ServiceError::NotFound("Process instance not found".to_string()))?;
    if !crate::service::scope::readable_ids(caller, slug, std::slice::from_ref(&inst.record_id), state).contains(&inst.record_id) {
        return Err(ServiceError::NotFound("Process instance not found".to_string()));
    }
    let submitter = state.find_record(slug, &inst.record_id).ok().flatten().and_then(|r| r.created_by);
    let own = |who: &Option<String>| who.as_deref().is_some_and(|w| w.eq_ignore_ascii_case(&caller.eppn));
    if own(&submitter) || own(&inst.started_by) {
        return Err(ServiceError::forbidden("Forbidden: the person a vote is about does not see who voted how."));
    }
    if !is_voter(&inst, &caller.eppn) && authorize_app(caller, slug, AppAction::Manage, state).is_err() {
        return Err(ServiceError::forbidden("Forbidden: only the committee and the app's owners and admins see the votes."));
    }
    let rows = state.repository.instance_votes(&inst.id).map_err(|e| ServiceError::StorageUnavailable(e.to_string()))?;
    let tally = crate::repository_votes::tally_of(&rows, &inst.voters);
    Ok(json!({ "instance_id": inst.id, "voters": inst.voters, "tally": tally, "votes": rows }))
}

/// How far a committee's vote has got, for the process list: counts only, which anyone who can see the instance may see.
pub fn tally_view(state: &SharedState, inst: &ProcessInstance) -> Option<Value> {
    if inst.voters.is_empty() {
        return None;
    }
    let rows = state.repository.instance_votes(&inst.id).ok()?;
    serde_json::to_value(crate::repository_votes::tally_of(&rows, &inst.voters)).ok()
}

/// What is wrong with a rule's committee steps, for the rule validator.
pub fn problems(steps: &[scaffoldry_core::ProcessStep]) -> Vec<String> {
    let mut out = vec![];
    for step in scaffoldry_core::workflow::all_steps(steps) {
        if let scaffoldry_core::StepKind::UserTask { decision: DecisionRule::AtLeast(0), .. } = &step.kind {
            out.push(format!("Step '{}' needs at least 0 approvals, which decides nothing. Use 1 or more.", step.id));
        }
    }
    out
}
