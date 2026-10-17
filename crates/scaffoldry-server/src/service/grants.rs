//! Every grant that names a person. Access is meant to come from positions and roles, so a grant to one
//! named person is allowed but must be visible. This builds the list an administrator reviews.

use crate::service::members;
use crate::state::SharedState;
use scaffoldry_core::approver::ApproverSpec;
use scaffoldry_core::{ProcessStatus, StepKind};
use serde_json::{json, Value};
use std::collections::HashMap;

/// One row per grant to a named person: a workspace or app membership, an approver rule that names a person,
/// and an open instance assigned to a person. Who made a membership and when comes from the ledger. An older
/// grant with no ledger entry shows no granter.
pub fn direct_grants(state: &SharedState) -> Vec<Value> {
    let made: HashMap<(String, String, String), (String, String, String)> = state
        .repository
        .membership_grant_entries()
        .unwrap_or_default()
        .into_iter()
        .map(|e| ((e.workspace_id, e.target, e.app_slug), (e.principal, e.timestamp, e.rationale)))
        .collect();

    let mut rows: Vec<Value> = Vec::new();

    for c in members::direct_memberships(state) {
        let entry = made.get(&(c.workspace_id.clone(), c.eppn.clone(), c.app_slug.clone()));
        rows.push(json!({
            "kind": "membership",
            "scope": if c.app_slug.is_empty() { "workspace" } else { "app" },
            "workspace_id": c.workspace_id,
            "app_slug": c.app_slug,
            "who": c.eppn,
            "role": c.role,
            "granted_by": entry.map(|e| e.0.clone()),
            "granted_at": entry.map(|e| e.1.clone()).unwrap_or_else(|| c.added_at.clone()),
            "rationale": entry.map(|e| e.2.clone()),
        }));
    }

    if let Ok(rules) = state.automations.read() {
        for (app, list) in rules.iter() {
            for rule in list {
                for step in &rule.steps {
                    if let StepKind::UserTask { approver: Some(ApproverSpec::Person { eppn }), .. } = &step.kind {
                        rows.push(json!({
                            "kind": "approver", "scope": "app", "app_slug": app, "who": eppn,
                            "rule_id": rule.id, "rule_name": rule.name, "step_id": step.id,
                            "granted_by": null, "granted_at": null, "rationale": null,
                        }));
                    }
                }
            }
        }
    }

    if let Ok(instances) = state.process_instances.read() {
        for inst in instances.values().filter(|i| i.status == ProcessStatus::Waiting) {
            if let Some(who) = &inst.assigned_to {
                rows.push(json!({
                    "kind": "assignment", "scope": "app", "app_slug": inst.app_slug, "who": who,
                    "instance_id": inst.id, "record_id": inst.record_id,
                    "granted_by": null, "granted_at": null, "rationale": null,
                }));
            }
        }
    }

    let key = |v: &Value, f: &str| v[f].as_str().unwrap_or("").to_string();
    rows.sort_by_key(|v| (key(v, "kind"), key(v, "workspace_id"), key(v, "app_slug"), key(v, "who")));
    rows
}
