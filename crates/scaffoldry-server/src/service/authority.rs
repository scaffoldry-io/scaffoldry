//! Decision authority: what a proposal needs, asked when it is made and again when it is decided.
//!
//! `scaffoldry_core::authority::required` is the one pure function. This module reads the two small tables and the org
//! tree for it, and resolves the approver spec the answer names. Nothing here is cached: a proposal is not a hot path.

use crate::repository_proposals::ProposalRow;
use crate::service::approvers::resolution_at;
use crate::service::ServiceError;
use crate::state::SharedState;
use scaffoldry_core::approver::Approver;
use scaffoldry_core::authority::{remedy_for, required, MatrixRow, Path, Requirement, SignalRow, FLAGGED};
use serde_json::{json, Value};
use uuid::Uuid;

fn internal_error(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::internal(e.to_string())
}

/// What a proposal needs now, and who can give it.
pub struct Need {
    pub requirement: Requirement,
    /// The people the approver spec resolves to, other than the proposer. Empty unless the path is `approver`.
    pub resolved: Vec<Approver>,
    /// The path is `approver` and nobody resolves.
    pub no_approver: bool,
    pub sentences: Vec<String>,
    /// The workspace's unit, when it has one.
    pub unit: Option<Uuid>,
}

type Tables = (Vec<SignalRow>, Vec<MatrixRow>);

/// The workspace's unit first, then each unit above it, ending at the root.
pub(crate) fn unit_path(state: &SharedState, unit: Option<Uuid>) -> Vec<Uuid> {
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    let mut path = Vec::new();
    let mut next = unit;
    while let Some(id) = next {
        if path.contains(&id) || path.len() > 64 {
            break;
        }
        path.push(id);
        next = orgs.get(&id).and_then(|o| o.parent_id);
    }
    path
}

pub(crate) fn raised_in(checks: &Value) -> Vec<String> {
    checks["checks"]
        .as_array()
        .map(|list| list.iter().filter(|c| c["outcome"] == "fail").filter_map(|c| c["name"].as_str()).filter(|n| FLAGGED.iter().any(|(f, _)| f == n)).map(str::to_string).collect())
        .unwrap_or_default()
}

fn compute(state: &SharedState, workspace_id: &str, checks: &Value, proposer: &str, tables: &Tables) -> Need {
    let unit = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(workspace_id).and_then(|w| w.organization_id);
    let path = unit_path(state, unit);
    let requirement = required(&raised_in(checks), &path, &tables.0, &tables.1);
    let resolved: Vec<Approver> = match (&requirement.approver, unit) {
        (Some(spec), Some(unit)) if requirement.path == Path::Approver => resolution_at(state, spec, unit, &[], Some(proposer)).approvers,
        _ => Vec::new(),
    };
    let no_approver = requirement.path == Path::Approver && resolved.is_empty();
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    let sentences = requirement.sentences(&|id| orgs.get(&id).map(|o| o.name.clone()).unwrap_or_else(|| "A unit".to_string()));
    Need { requirement, resolved, no_approver, sentences, unit }
}

/// What `checks` (the stored `{ checks, changes }` of a proposal) needs from the tables as they are now.
pub fn requirement_for(state: &SharedState, workspace_id: &str, checks: &Value, proposer: &str) -> Result<Need, ServiceError> {
    let tables = state.repository.authority_tables().map_err(internal_error)?;
    Ok(compute(state, workspace_id, checks, proposer, &tables))
}

/// The people who can say who holds a vacant position: the unit admins of the unit and of every unit above it.
fn contact(state: &SharedState, unit: Option<Uuid>) -> Vec<String> {
    let path = unit_path(state, unit);
    let mut eppns: Vec<String> = state.repository.roles_in_units(&path).unwrap_or_default().into_iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.eppn).collect();
    eppns.sort();
    eppns.dedup();
    eppns
}

impl Need {
    /// What the builder reads. `state` is read for the people to contact when nobody can decide.
    pub fn to_json(&self, state: &SharedState) -> Value {
        let detail: Vec<Value> = self
            .requirement
            .reasons
            .iter()
            .zip(&self.sentences)
            .map(|(r, sentence)| {
                json!({
                    "check": r.check,
                    "sentence": sentence,
                    "remedy": r.check.as_deref().and_then(remedy_for),
                    "control_id": r.control_id,
                    "library_rule": r.library_rule.as_ref().map(|(policy, rule)| json!({ "policy_key": policy, "rule_key": rule })),
                })
            })
            .collect();
        json!({
            "tier": self.requirement.tier.as_str(),
            "path": self.requirement.path.as_str(),
            "path_sentence": self.requirement.path_sentence(),
            "reasons_detail": detail,
            "contact": if self.no_approver { contact(state, self.unit) } else { Vec::new() },
            "approver": self.requirement.approver,
            "resolved": self.resolved.iter().map(|a| json!({ "eppn": a.eppn, "display_name": a.display_name, "via": a.via })).collect::<Vec<_>>(),
            "no_approver": self.no_approver,
            "reasons": self.sentences,
        })
    }
}

/// Pending proposals whose approver path resolves to nobody, as they are now.
pub fn unassigned(state: &SharedState) -> Result<Vec<ProposalRow>, ServiceError> {
    let tables = state.repository.authority_tables().map_err(internal_error)?;
    let pending = state.repository.pending_proposals(500).map_err(internal_error)?;
    Ok(pending.into_iter().filter(|p| compute(state, &p.workspace_id, &p.checks, &p.proposed_by, &tables).no_approver).collect())
}

/// What a pending proposal asks of the caller, if they may decide it under the matrix as it is now.
///
/// This follows `proposals::decide`: on the approver path only the people the spec resolves to; on any other path an
/// owner or admin of the workspace, and on the second-person path never the proposer.
fn may_decide(state: &SharedState, caller: &crate::state::AuthUser, p: &ProposalRow, need: &Need) -> bool {
    match need.requirement.path {
        Path::Approver => need.resolved.iter().any(|a| a.eppn.eq_ignore_ascii_case(&caller.eppn)),
        Path::SecondPerson if p.proposed_by.eq_ignore_ascii_case(&caller.eppn) => false,
        _ => crate::service::access::authorize_workspace(caller, &p.workspace_id, "manage_workspace", state).is_ok(),
    }
}

/// The pending proposals `caller` may decide, across workspaces. Each carries who proposed it and why it needs a decider,
/// and no content: a decider reads the proposal itself, where the digest is.
pub fn to_decide(state: &SharedState, caller: &crate::state::AuthUser) -> Result<Vec<Value>, ServiceError> {
    let tables = state.repository.authority_tables().map_err(internal_error)?;
    let pending = state.repository.pending_proposals(500).map_err(internal_error)?;
    let mut out = Vec::new();
    for p in pending {
        let need = compute(state, &p.workspace_id, &p.checks, &p.proposed_by, &tables);
        if !may_decide(state, caller, &p, &need) {
            continue;
        }
        out.push(json!({
            "proposal_id": p.id,
            "workspace_id": p.workspace_id,
            "app_slug": p.app_slug,
            "summary": p.summary,
            "proposed_by": p.proposed_by,
            "created_at": p.created_at,
            "tier": need.requirement.tier.as_str(),
            "path": need.requirement.path.as_str(),
            "reasons": need.sentences,
        }));
    }
    Ok(out)
}
