//! Decision authority: what a proposal needs, asked when it is made and again when it is decided.
//!
//! `scaffoldry_core::authority::required` is the one pure function. This module reads the two small tables and the org
//! tree for it, and resolves the approver spec the answer names. Nothing here is cached: a proposal is not a hot path.

use crate::repository_proposals::ProposalRow;
use crate::service::approvers::resolution_at;
use crate::service::ServiceError;
use crate::state::SharedState;
use scaffoldry_core::approver::Approver;
use scaffoldry_core::authority::{required, MatrixRow, Path, Requirement, SignalRow, FLAGGED};
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
    Need { requirement, resolved, no_approver, sentences }
}

/// What `checks` (the stored `{ checks, changes }` of a proposal) needs from the tables as they are now.
pub fn requirement_for(state: &SharedState, workspace_id: &str, checks: &Value, proposer: &str) -> Result<Need, ServiceError> {
    let tables = state.repository.authority_tables().map_err(internal_error)?;
    Ok(compute(state, workspace_id, checks, proposer, &tables))
}

impl Need {
    /// What the builder reads.
    pub fn to_json(&self) -> Value {
        json!({
            "tier": self.requirement.tier.as_str(),
            "path": self.requirement.path.as_str(),
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
