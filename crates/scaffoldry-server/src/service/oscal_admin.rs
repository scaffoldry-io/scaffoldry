//! Admin console phase 5: the mapping from control to statement to policy, kept as an OSCAL component-definition.
//!
//! The stored document holds what a person wrote: a statement and an implementation status for each control. A control is a
//! requirement when a person put it there or an `@oscal` annotation of the active policy names it. A ledger entry is evidence
//! for a control and never makes one. Control ids are normalized with `catalog::normalize`.
//!
//! OSCAL 1.1.2 has `implementation-status` as a field of the system security plan, not of a component definition's
//! `implemented-requirement`, which allows only props. So it is a prop in the Scaffoldry namespace, as are the other
//! props added here.

use crate::service::admin::{admin_write, require_platform_admin_or_compliance};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::catalog;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const KIND: &str = "component-definition";
pub const NAMESPACE: &str = "https://scaffoldry.io/ns/oscal";
pub const STATUSES: [&str; 5] = ["implemented", "partial", "planned", "alternative", "not-applicable"];
const MAX_STATEMENT: usize = 2_000;

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

/// A version 4 shaped uuid derived from a name, so a document that has not changed keeps its ids from one export to the next.
fn stable_uuid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_random_bytes(bytes).into_uuid().to_string()
}

fn prop(name: &str, value: impl Into<String>) -> Value {
    json!({ "name": name, "ns": NAMESPACE, "value": value.into() })
}

fn skeleton(requirements: Vec<Value>) -> Value {
    json!({
        "component-definition": {
            "uuid": stable_uuid("component-definition"),
            "metadata": {
                "title": "Scaffoldry Sovereign Application Platform Component Definition",
                "last-modified": chrono::Utc::now().to_rfc3339(),
                "version": "1.0.0",
                "oscal-version": "1.1.2"
            },
            "components": [{
                "uuid": stable_uuid("component"),
                "type": "software",
                "title": "Scaffoldry Sovereign Application Platform",
                "description": "Governed collaborative workspace, tabular engine, and Cedar authorization lattice",
                "control-implementations": [{
                    "uuid": stable_uuid("control-implementation"),
                    "source": "https://doi.org/10.6028/NIST.SP.800-53r5",
                    "description": "Institutional control implementation, each control mapped to the Cedar policies that enforce it",
                    "implemented-requirements": requirements
                }]
            }]
        }
    })
}

fn requirements_of(doc: &mut Value) -> Option<&mut Vec<Value>> {
    doc.pointer_mut("/component-definition/components/0/control-implementations/0/implemented-requirements")?.as_array_mut()
}

/// What a person stored: control id (normalized) to statement and status.
fn stored(doc: &Option<Value>) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    let Some(list) = doc.as_ref().and_then(|d| d.pointer("/component-definition/components/0/control-implementations/0/implemented-requirements")).and_then(Value::as_array) else { return out };
    for r in list {
        let Some(id) = r.get("control-id").and_then(Value::as_str) else { continue };
        let status = r.get("props").and_then(Value::as_array).and_then(|p| p.iter().find(|p| p["name"] == "implementation-status")).and_then(|p| p["value"].as_str()).unwrap_or("planned");
        out.insert(catalog::normalize(id), (r.get("description").and_then(Value::as_str).unwrap_or("").to_string(), status.to_string()));
    }
    out
}

/// One row per requirement, in control id order.
pub fn rows(state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let doc = state.repository.get_governance_document(KIND).map_err(storage)?;
    let stored = stored(&doc);
    // The controls the active policy names, and the policies that name each.
    let mut linked: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in state.policy().policy_summaries() {
        if !p.oscal.trim().is_empty() {
            linked.entry(catalog::normalize(&p.oscal)).or_default().push(p.id);
        }
    }
    // Evidence: the ledger entries for each control, however the entry spelled the id.
    let mut evidence: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for (control, n, latest) in state.repository.ledger_control_evidence().map_err(storage)? {
        let slot = evidence.entry(catalog::normalize(&control)).or_insert((0, String::new()));
        slot.0 += n;
        if latest > slot.1 {
            slot.1 = latest;
        }
    }
    let mut ids: Vec<&String> = stored.keys().chain(linked.keys()).collect();
    ids.sort();
    ids.dedup();
    Ok(ids
        .into_iter()
        .map(|id| {
            let policies = linked.get(id).cloned().unwrap_or_default();
            let (statement, status) = stored.get(id).cloned().unwrap_or_else(|| {
                (format!("Enforced by {} Cedar polic{}.", policies.len(), if policies.len() == 1 { "y" } else { "ies" }), "implemented".to_string())
            });
            let (count, latest) = evidence.get(id).cloned().unwrap_or((0, String::new()));
            json!({
                "control_id": id,
                "title": catalog::title(id),
                "statement": statement,
                "status": status,
                "policy_ids": policies,
                "evidence_count": count,
                "latest_evidence": (!latest.is_empty()).then_some(latest),
                "edited": stored.contains_key(id),
            })
        })
        .collect())
}

pub fn requirements(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    Ok(json!({ "rows": rows(state)?, "next_cursor": null }))
}

/// The OSCAL document: one `implemented-requirement` for each row. Called by the console, the governance route and the
/// governance report tool, so all three say the same thing.
pub fn export_component_definition(state: &SharedState) -> Result<Value, ServiceError> {
    let requirements: Vec<Value> = rows(state)?
        .iter()
        .map(|r| {
            let id = r["control_id"].as_str().unwrap_or_default();
            let mut props = vec![prop("implementation-status", r["status"].as_str().unwrap_or("planned"))];
            for p in r["policy_ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                props.push(prop("cedar-policy-id", p));
            }
            props.push(prop("ledger-evidence-count", r["evidence_count"].as_i64().unwrap_or(0).to_string()));
            if let Some(t) = r["latest_evidence"].as_str() {
                props.push(prop("ledger-latest-evidence", t));
            }
            json!({ "uuid": stable_uuid(&format!("requirement:{id}")), "control-id": id, "description": r["statement"], "props": props })
        })
        .collect();
    Ok(skeleton(requirements))
}

pub fn component_definition(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    export_component_definition(state)
}

pub fn put_requirement(caller: &AuthUser, control_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let id = catalog::normalize(control_id);
    if id.is_empty() || id.len() > 32 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '(' | ')')) {
        return Err(ServiceError::BadRequest("The control id is not an OSCAL control id, such as ac-2 or ac-2.1".to_string()));
    }
    let statement = body.get("statement").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if statement.is_empty() || statement.chars().count() > MAX_STATEMENT {
        return Err(ServiceError::BadRequest(format!("statement is required and is at most {MAX_STATEMENT} characters")));
    }
    let status = body.get("status").and_then(Value::as_str).unwrap_or("");
    if !STATUSES.contains(&status) {
        return Err(ServiceError::BadRequest(format!("status is one of {}", STATUSES.join(", "))));
    }
    let reason = body.get("reason").and_then(Value::as_str).unwrap_or("");
    let (statement, status) = (statement.to_string(), status.to_string());
    let payload = json!({ "change": "oscal_requirement", "control_id": id, "status": status });
    admin_write(state, caller, DecisionType::PolicyRevision, "AC-03", reason, &payload, || {
        let target = id.clone();
        state
            .repository
            .edit_governance_document(KIND, &caller.eppn, move |held| {
                let mut doc = held.unwrap_or_else(|| skeleton(Vec::new()));
                let list = requirements_of(&mut doc).ok_or_else(|| crate::repository::RepositoryError::Conflict("The stored component definition is damaged".to_string()))?;
                let entry = json!({
                    "uuid": stable_uuid(&format!("requirement:{target}")),
                    "control-id": target,
                    "description": statement,
                    "props": [prop("implementation-status", status)],
                });
                match list.iter_mut().find(|r| r["control-id"].as_str().map(catalog::normalize).as_deref() == Some(target.as_str())) {
                    Some(slot) => *slot = entry,
                    None => list.push(entry),
                }
                doc["component-definition"]["metadata"]["last-modified"] = json!(chrono::Utc::now().to_rfc3339());
                Ok(doc)
            })
            .map_err(storage)
    })?;
    rows(state)?.into_iter().find(|r| r["control_id"] == id.as_str()).ok_or_else(|| ServiceError::Internal("The requirement was not stored".to_string()))
}
