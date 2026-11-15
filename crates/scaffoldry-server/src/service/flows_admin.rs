//! Workflows phase 11: the Flows panel of the console. Every flow across the workspaces, who approved it and when, what it
//! may touch in sentences, how its runs have gone, and a switch with a reason. A Platform Admin only. Switching a flow off
//! or on is the setting `flows.disabled`, so the runner already honors it on its next run, on every server, and the change
//! is a settings ledger entry.

use crate::service::admin::require_platform_admin;
use crate::service::{flow_parties, flows, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::AutomationRule;
use scaffoldry_engine::flow::grants_sentence;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// Every flow in every app, by id.
fn all_flows(state: &SharedState) -> Vec<AutomationRule> {
    let mut out: Vec<AutomationRule> = state.automations.read().map(|m| m.values().flatten().filter(|r| flows::is_flow(r)).cloned().collect()).unwrap_or_default();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn workspace_name(state: &SharedState, id: &str) -> String {
    state.workspaces.read().ok().and_then(|m| m.get(id).map(|w| w.name.clone())).unwrap_or_else(|| id.to_string())
}

/// `GET /admin/flows`. Query: `crosses_workspaces=true`, `reads_sensitive=true`, `state`, `search`.
pub fn list(caller: &AuthUser, query: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let rules = all_flows(state);
    let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    let since = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    let runs: HashMap<String, crate::repository_index::FlowPanelRuns> = state.repository.flow_panel_runs(&ids, &since).map_err(storage)?.into_iter().map(|r| (r.rule_id.clone(), r)).collect();
    let tripped: BTreeSet<String> = state.repository.flows_tripped(&ids).map_err(storage)?.into_iter().collect();
    let versions: Vec<(String, i32)> = rules.iter().map(|r| (r.id.clone(), r.version as i32)).collect();
    let mut approvals: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for (rule, ws, who, at) in state.repository.flow_approvals(&versions).map_err(storage)? {
        approvals.entry(rule).or_default().push((ws, who, at));
    }
    let want = |key: &str| query.get(key).is_some_and(|v| v == "true");
    let search = query.get("search").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for rule in &rules {
        let manifests = flows::manifests_for(state, rule);
        let ws_of = |slug: &str| manifests.iter().find(|m| m.slug == slug).and_then(|m| m.workspace_id.clone());
        let home = rule.home_workspace.clone().or_else(|| ws_of(&rule.app_slug)).unwrap_or_default();
        let mut parties: Vec<String> = vec![home.clone()];
        for g in &rule.grants {
            if let Some(ws) = ws_of(&g.app_slug) {
                if !parties.contains(&ws) {
                    parties.push(ws);
                }
            }
        }
        let crosses = parties.len() > 1;
        let reads_sensitive = rule.grants.iter().any(|g| {
            let sensitive = crate::service::records::sensitive_fields_of(&g.app_slug, &g.table_id, state);
            g.read.iter().any(|f| sensitive.contains(f))
        });
        let disabled = flow_parties::is_disabled(state, &rule.id);
        let is_tripped = tripped.contains(&rule.id);
        let flow_state = if disabled { "Disabled" } else if is_tripped { "Tripped" } else { "Active" };
        if (want("crosses_workspaces") && !crosses) || (want("reads_sensitive") && !reads_sensitive) || query.get("state").is_some_and(|s| !s.is_empty() && s != flow_state) {
            continue;
        }
        if search.as_ref().is_some_and(|s| !rule.name.to_lowercase().contains(s) && !rule.id.to_lowercase().contains(s) && !rule.app_slug.to_lowercase().contains(s)) {
            continue;
        }
        let approved = approvals.get(&rule.id).cloned().unwrap_or_default();
        let party_rows: Vec<Value> = parties
            .iter()
            .map(|ws| {
                let got = approved.iter().find(|(w, _, _)| w == ws);
                json!({ "workspace_id": ws, "name": workspace_name(state, ws), "approved_by": got.map(|g| g.1.clone()), "approved_at": got.map(|g| g.2.clone()) })
            })
            .collect();
        let last = runs.get(&rule.id);
        out.push(json!({
            "rule_id": rule.id,
            "name": rule.name,
            "app_slug": rule.app_slug,
            "version": rule.version,
            "home_workspace": home,
            "home_workspace_name": workspace_name(state, &home),
            "crosses_workspaces": crosses,
            "reads_sensitive": reads_sensitive,
            "parties": party_rows,
            "grants": grants_sentence(&rule.grants, &rule.app_slug, &manifests),
            "last_run": last.and_then(|l| l.last_started.as_ref().map(|at| json!({ "at": at, "status": l.last_status }))),
            "runs_last_7_days": last.map_or(0, |l| l.runs),
            "failures_last_7_days": last.map_or(0, |l| l.failed),
            "state": flow_state,
        }));
    }
    Ok(json!({ "flows": out }))
}

/// `POST /admin/flows/{rule_id}/disable` and `/enable`. The reason is required. The change is one settings ledger entry and takes
/// effect on the next run, on every server, because every server reads the setting.
pub fn set_disabled(caller: &AuthUser, rule_id: &str, disabled: bool, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(ServiceError::BadRequest("A reason between 1 and 500 characters is required for administrative actions".to_string()));
    }
    let rule = flows::find_flow(state, rule_id).filter(flows::is_flow).ok_or_else(|| ServiceError::NotFound("Flow not found".to_string()))?;
    let mut list: Vec<String> = state.settings.read().ok().and_then(|s| s.get("flows.disabled").and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())).unwrap_or_default();
    list.retain(|id| id != rule_id);
    if disabled {
        list.push(rule_id.to_string());
    }
    let verb = if disabled { "disabled" } else { "enabled" };
    state
        .repository
        .put_platform_setting(
            "flows.disabled",
            &json!(list),
            &caller.eppn,
            crate::repository::AppendDecisionParams {
                principal: caller.eppn.clone(),
                organization_code: "INST".to_string(),
                app_slug: Some(rule.app_slug.clone()),
                decision_type: DecisionType::PolicyRevision,
                oscal_control_id: "AC-03".to_string(),
                rationale: format!("Flow '{}' ({rule_id}) {verb} by {}. Reason: {reason}", rule.name, caller.eppn),
                payload: json!({ "key": "flows.disabled", "flow": rule_id, "action": verb }),
            },
        )
        .map_err(|e| ServiceError::Internal(format!("Database write failed: {e}")))?;
    state.refresh().map_err(|e| ServiceError::StorageUnavailable(format!("The setting was saved, but this server could not reload: {e}")))?;
    Ok(json!({ "rule_id": rule_id, "state": if disabled { "Disabled" } else { "Active" } }))
}

/// For the Overview: how many flows there are, how many cross workspaces, how many the breaker has turned off, and how many work
/// items the last nightly reconcile repaired.
pub fn counts(state: &SharedState) -> Value {
    let rules = all_flows(state);
    let ids: Vec<String> = rules.iter().map(|r| r.id.clone()).collect();
    let tripped = state.repository.flows_tripped(&ids).map(|t| t.len()).unwrap_or(0);
    let crossing = rules
        .iter()
        .filter(|r| {
            let manifests = flows::manifests_for(state, r);
            let ws: BTreeSet<String> = r.grants.iter().filter_map(|g| manifests.iter().find(|m| m.slug == g.app_slug).and_then(|m| m.workspace_id.clone())).chain(r.home_workspace.clone()).collect();
            ws.len() > 1
        })
        .count();
    json!({ "total": rules.len(), "cross_workspace": crossing, "tripped": tripped, "work_drift_repaired": state.repository.last_reconcile_repairs().unwrap_or(None) })
}
