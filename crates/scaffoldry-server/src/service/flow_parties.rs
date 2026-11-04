//! Workflows phase 2: a flow that touches several workspaces needs approval from each.
//!
//! A proposal that carries a flow has parties: the home workspace and every other workspace its grants reach. This
//! module works out the parties, the checks a flow raises (`flow_grants`, `cross_workspace`, `data_movement`,
//! `classification_downgrade`, `flow_target_exists`), and the sentences each party is shown.

use crate::service::ServiceError;
use crate::state::SharedState;
use scaffoldry_core::workflow::FlowGrant;
use scaffoldry_core::AutomationRule;
use scaffoldry_engine::flow::grants_sentence;
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A workspace classification's rank. A sensitive field may not move to a lower one.
pub fn classification_rank(c: &str) -> u8 {
    match c.trim().to_ascii_lowercase().as_str() {
        "public" => 0,
        "internal" => 1,
        "restricted" => 2,
        "ferpa sensitive" => 3,
        _ => 1,
    }
}

/// Whether flows may reach beyond their own workspace. Off makes any proposal with more than one party fail.
pub fn cross_workspace_enabled(state: &SharedState) -> bool {
    state.settings.read().ok().and_then(|s| s.get("flows.cross_workspace_enabled").and_then(Value::as_bool)).unwrap_or(true)
}

/// True when an administrator has stopped this flow through `flows.disabled`.
pub fn is_disabled(state: &SharedState, rule_id: &str) -> bool {
    state
        .settings
        .read()
        .ok()
        .and_then(|s| s.get("flows.disabled").and_then(Value::as_array).map(|l| l.iter().any(|v| v.as_str() == Some(rule_id))))
        .unwrap_or(false)
}

/// What a proposal's flows touch, and the checks they raise.
pub struct Plan {
    /// The home workspace first, then each other workspace in the order the grants reach it.
    pub parties: Vec<String>,
    /// The parties a flow reads from. When a flagged check is raised, the proposer may not approve these.
    pub sources: Vec<String>,
    /// Every app the flows touch, with its live version now.
    pub touched: Vec<(String, Option<i32>)>,
    pub checks: Vec<Value>,
    /// What each party is shown: only what concerns it.
    pub changes: BTreeMap<String, Vec<String>>,
}

impl Plan {
    /// The value stored in `app_proposals.parties`.
    pub fn to_json(&self) -> Value {
        json!({
            "parties": self.parties,
            "sources": self.sources,
            "touched": self.touched.iter().map(|(s, v)| json!({ "slug": s, "version": v })).collect::<Vec<_>>(),
            "changes": self.changes,
        })
    }
}

/// The stored parties of a proposal, read back.
pub fn parties_of(stored: &Option<Value>) -> Vec<String> {
    stored.as_ref().and_then(|p| p["parties"].as_array()).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn sources_of(stored: &Option<Value>) -> Vec<String> {
    stored.as_ref().and_then(|p| p["sources"].as_array()).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn touched_of(stored: &Option<Value>) -> Vec<(String, Option<i32>)> {
    stored
        .as_ref()
        .and_then(|p| p["touched"].as_array())
        .map(|l| l.iter().filter_map(|t| Some((t["slug"].as_str()?.to_string(), t["version"].as_i64().map(|v| v as i32)))).collect())
        .unwrap_or_default()
}

fn check(name: &str, outcome: &str, messages: Vec<String>) -> Value {
    json!({ "name": name, "outcome": outcome, "messages": messages })
}

fn workspace_label(state: &SharedState, ws: &str) -> (String, String) {
    state
        .workspaces
        .read()
        .ok()
        .and_then(|m| m.get(ws).map(|w| (w.name.clone(), w.data_classification.clone())))
        .unwrap_or_else(|| (ws.to_string(), "Internal".to_string()))
}

/// The checks and parties of the flows a proposal carries. `None` when it carries no flow. `home` is the app as the
/// proposal would make it. A classification downgrade, or a second workspace while cross-workspace flows are off, is a
/// failure that stores nothing.
pub fn plan(state: &SharedState, home_ws: &str, home: &AppManifest, rules: &[AutomationRule]) -> Result<Option<Plan>, ServiceError> {
    let flows: Vec<&AutomationRule> = rules.iter().filter(|r| crate::service::flows::is_flow(r)).collect();
    if flows.is_empty() {
        return Ok(None);
    }
    let mut parties = vec![home_ws.to_string()];
    let mut sources: Vec<String> = Vec::new();
    let mut touched: Vec<(String, Option<i32>)> = Vec::new();
    let mut by_ws: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut changes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut movement, mut downgrade) = (Vec::new(), Vec::new());
    for rule in flows {
        let manifests = crate::service::flows::manifests_with(state, home, rule);
        let ws_of = |slug: &str| -> String {
            if slug == home.slug {
                return home_ws.to_string();
            }
            manifests.iter().find(|m| m.slug == slug).and_then(|m| m.workspace_id.clone()).unwrap_or_else(|| home_ws.to_string())
        };
        let title = |slug: &str| manifests.iter().find(|m| m.slug == slug).map(|m| m.title.clone()).unwrap_or_else(|| slug.to_string());
        for g in &rule.grants {
            let ws = ws_of(&g.app_slug);
            if !parties.contains(&ws) {
                parties.push(ws.clone());
            }
            if !g.read.is_empty() && !sources.contains(&ws) {
                sources.push(ws.clone());
            }
            if !touched.iter().any(|(s, _)| s == &g.app_slug) {
                let version = state.repository.app_manifest_version(&g.app_slug).map_err(|e| ServiceError::Internal(e.to_string()))?;
                touched.push((g.app_slug.clone(), version));
            }
        }
        // Each workspace's part of the flow, in sentences.
        for ws in &parties {
            let own: Vec<FlowGrant> = rule.grants.iter().filter(|g| &ws_of(&g.app_slug) == ws).cloned().collect();
            if own.is_empty() {
                continue;
            }
            let (name, _) = workspace_label(state, ws);
            by_ws.entry(ws.clone()).or_default().push(format!("{name}: {}", grants_sentence(&own, &rule.app_slug, &manifests)));
            let elsewhere: Vec<String> = rule
                .grants
                .iter()
                .filter(|g| &ws_of(&g.app_slug) != ws && !g.read.is_empty())
                .map(|g| {
                    let table = manifests.iter().find(|m| m.slug == g.app_slug).and_then(|m| m.tables.iter().find(|t| t.id == g.table_id)).map(|t| t.name.clone()).unwrap_or_else(|| g.table_id.clone());
                    format!("{table}: {}", g.read.join(", "))
                })
                .collect();
            let mut mine = vec![format!("Flow \"{}\" in {}. {}", rule.name, title(&rule.app_slug), grants_sentence(&own, &rule.app_slug, &manifests))];
            if !elsewhere.is_empty() {
                mine.push(format!("To do this it reads from other workspaces: {}.", elsewhere.join("; ")));
            }
            changes.entry(ws.clone()).or_default().extend(mine);
        }
        // Sensitive data read in one app and written into another.
        let writes: Vec<&FlowGrant> = rule.grants.iter().filter(|g| g.create || !g.write.is_empty() || !g.link.is_empty()).collect();
        for g in rule.grants.iter().filter(|g| !g.read.is_empty()) {
            let Some(m) = manifests.iter().find(|m| m.slug == g.app_slug) else { continue };
            let sensitive = crate::service::proposals::sensitive_fields(m, state);
            for field in g.read.iter().filter(|f| sensitive.contains(&(g.table_id.clone(), (*f).clone()))) {
                for w in writes.iter().filter(|w| w.app_slug != g.app_slug) {
                    movement.push(format!("Flow \"{}\" reads the sensitive field {}.{field} in {} and writes into {}", rule.name, g.table_id, title(&g.app_slug), title(&w.app_slug)));
                    let (from_name, from_class) = workspace_label(state, &ws_of(&g.app_slug));
                    let (to_name, to_class) = workspace_label(state, &ws_of(&w.app_slug));
                    if classification_rank(&to_class) < classification_rank(&from_class) {
                        downgrade.push(format!(
                            "Flow \"{}\" would move the sensitive field {}.{field} from {from_name} ({from_class}) into {to_name} ({to_class}), which is classified lower",
                            rule.name, g.table_id
                        ));
                    }
                }
            }
        }
    }
    if !downgrade.is_empty() {
        return Err(ServiceError::BadRequest(format!("classification_downgrade: {}", downgrade.join("; "))));
    }
    let names: Vec<String> = parties.iter().map(|ws| workspace_label(state, ws).0).collect();
    let mut cross = Vec::new();
    if parties.len() > 1 {
        if !cross_workspace_enabled(state) {
            return Err(ServiceError::BadRequest(format!(
                "cross_workspace: flows that reach beyond their own workspace are turned off here (flows.cross_workspace_enabled). This flow touches {}",
                names.join(" and ")
            )));
        }
        cross.push(format!("The flow touches {} workspaces: {}. An owner or admin of each must approve it.", parties.len(), names.join(", ")));
    }
    let checks = vec![
        check("flow_grants", "pass", parties.iter().flat_map(|ws| by_ws.get(ws).cloned().unwrap_or_default()).collect()),
        check("cross_workspace", if cross.is_empty() { "pass" } else { "fail" }, cross),
        check("data_movement", if movement.is_empty() { "pass" } else { "fail" }, movement),
        check("classification_downgrade", "pass", vec![]),
        // A target table, field or link that does not exist fails `rule_schema` before a proposal is stored.
        check("flow_target_exists", "pass", vec![]),
    ];
    Ok(Some(Plan { parties, sources, touched, checks, changes }))
}
