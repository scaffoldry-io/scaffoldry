//! Decision authority phase 4: the load on real decisions, for the last 90 days. Counts, names of approvers, and times.
//! The matrix is read as it is now, so a proposal made under an older matrix is counted under today's. Nothing here
//! reads a proposal's content.

use crate::service::admin::require_platform_admin_or_compliance;
use crate::service::approvers::resolution_at;
use crate::service::authority::unit_path;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::authority::{load_summary, required, Path};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

const WINDOW_DAYS: i32 = 90;
const WEEKS: usize = 13;

/// Who a spec resolves to at a unit for a proposer: eppn and display name.
type Resolved = HashMap<(String, Uuid, String), Vec<(String, String)>>;

fn internal(e: impl ToString) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

/// `GET /admin/authority/load`.
pub fn load(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let tables = state.repository.authority_tables().map_err(internal)?;
    let groups = state.repository.authority_load_groups(WINDOW_DAYS).map_err(internal)?;
    let times = state.repository.authority_decision_times(WINDOW_DAYS).map_err(internal)?;

    let units: HashMap<String, Option<Uuid>> = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).iter().map(|(id, w)| (id.clone(), w.organization_id)).collect();
    let (mut direct, mut second_person, mut approver, mut unassigned, mut proposals) = (0i64, 0i64, 0i64, 0i64, 0i64);
    let mut weekly: BTreeMap<String, (String, [u64; WEEKS])> = BTreeMap::new();
    // The same spec, unit and proposer resolve to the same people, whatever the week.
    let mut resolved = Resolved::new();

    for g in &groups {
        proposals += g.count;
        let unit = units.get(&g.workspace_id).copied().flatten();
        let need = required(&g.raised, &unit_path(state, unit), &tables.0, &tables.1);
        match need.path {
            Path::Direct => direct += g.count,
            Path::SecondPerson => second_person += g.count,
            Path::Approver => {
                approver += g.count;
                let people = match (&need.approver, unit) {
                    (Some(spec), Some(unit)) => resolve(state, &mut resolved, spec, unit, &g.proposed_by),
                    _ => Vec::new(),
                };
                if people.is_empty() {
                    unassigned += g.count;
                }
                for (eppn, name) in people {
                    weekly.entry(eppn).or_insert_with(|| (name, [0; WEEKS])).1[g.weeks_ago.min(WEEKS - 1)] += g.count as u64;
                }
            }
        }
    }

    let approvers: Vec<Value> = weekly
        .into_iter()
        .map(|(eppn, (name, weeks))| {
            let s = load_summary(&weeks);
            json!({ "eppn": eppn, "display_name": name, "weeks": weeks, "median": s.median, "p95": s.p95, "largest": s.largest })
        })
        .collect();
    Ok(json!({
        "window_days": WINDOW_DAYS,
        "proposals": proposals,
        "paths": { "direct": direct, "second_person": second_person, "approver": approver },
        "unassigned": unassigned,
        "approvers": approvers,
        "decision_time": { "decided": times.decided, "median_seconds": times.median, "p95_seconds": times.p95 },
    }))
}

fn resolve(state: &SharedState, cache: &mut Resolved, spec: &ApproverSpec, unit: Uuid, proposer: &str) -> Vec<(String, String)> {
    let key = (serde_json::to_string(spec).unwrap_or_default(), unit, proposer.to_lowercase());
    cache.entry(key).or_insert_with(|| resolution_at(state, spec, unit, &[], Some(proposer)).approvers.into_iter().map(|a| (a.eppn, a.display_name)).collect()).clone()
}
