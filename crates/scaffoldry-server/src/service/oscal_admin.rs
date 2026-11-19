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
use std::collections::{BTreeMap, HashMap};

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
                    "source": catalog::source_url("sp800-53r5"),
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

/// The baselines that select a control, lowest first.
fn baselines_of(id: &str) -> Vec<&'static str> {
    ["low", "moderate", "high"].into_iter().filter(|b| catalog::baseline(b).is_some_and(|ids| ids.iter().any(|i| i == id))).collect()
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
            let known = catalog::control(id);
            json!({
                "control_id": id,
                "control_title": known.map(|c| c.title.as_str()),
                "catalog": known.map(|c| c.catalog),
                "family_title": known.map(|c| c.family_title.as_str()),
                "baselines": baselines_of(id),
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

/// The OSCAL document: one `implemented-requirement` for each row whose control is in a loaded catalog, grouped under one
/// `control-implementation` for each catalog, whose `source` is that catalog's canonical URL. Called by the console, the
/// governance route and the governance report tool, so all three say the same thing.
pub fn export_component_definition(state: &SharedState) -> Result<Value, ServiceError> {
    let mut by_catalog: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for r in rows(state)? {
        let (Some(id), Some(name)) = (r["control_id"].as_str(), r["catalog"].as_str()) else { continue };
        let mut props = vec![prop("implementation-status", r["status"].as_str().unwrap_or("planned"))];
        for p in r["policy_ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            props.push(prop("cedar-policy-id", p));
        }
        props.push(prop("ledger-evidence-count", r["evidence_count"].as_i64().unwrap_or(0).to_string()));
        if let Some(t) = r["latest_evidence"].as_str() {
            props.push(prop("ledger-latest-evidence", t));
        }
        let name = if name == "sp800-171r3" { "sp800-171r3" } else { "sp800-53r5" };
        by_catalog.entry(name).or_default().push(json!({ "uuid": stable_uuid(&format!("requirement:{id}")), "control-id": id, "description": r["statement"], "props": props }));
    }
    let mut doc = skeleton(Vec::new());
    let implementations: Vec<Value> = by_catalog
        .into_iter()
        .map(|(name, reqs)| {
            json!({
                "uuid": stable_uuid(&format!("control-implementation:{name}")),
                "source": catalog::source_url(name),
                "description": "Institutional control implementation, each control mapped to the Cedar policies that enforce it",
                "implemented-requirements": reqs
            })
        })
        .collect();
    doc["component-definition"]["components"][0]["control-implementations"] = json!(implementations);
    Ok(doc)
}

pub fn component_definition(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    export_component_definition(state)
}

pub fn put_requirement(caller: &AuthUser, control_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let id = catalog::normalize(control_id);
    if catalog::control(&id).is_none() {
        return Err(ServiceError::BadRequest(format!("unknown control: {control_id} is in neither the NIST SP 800-53 Rev 5 nor the SP 800-171 Rev 3 catalog")));
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

const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 200;

fn page_of(q: &HashMap<String, String>) -> (usize, usize) {
    let limit = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let start = q.get("cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
    (start, limit)
}

fn non_empty<'a>(q: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    q.get(key).map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// Search the catalogs. Each row says whether it already has a requirement, and its status.
pub fn search_catalog(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let catalog_name = non_empty(q, "catalog");
    let family = non_empty(q, "family");
    let baseline = match non_empty(q, "baseline") {
        Some(b) => Some(catalog::baseline(b).ok_or_else(|| ServiceError::BadRequest("baseline is low, moderate or high".to_string()))?),
        None => None,
    };
    let held: BTreeMap<String, Value> = rows(state)?.into_iter().filter(|r| r["edited"] == true).map(|r| (r["control_id"].as_str().unwrap_or_default().to_string(), r)).collect();
    let hits: Vec<&catalog::Control> = catalog::search(q.get("q").map(String::as_str).unwrap_or(""), family, catalog_name).into_iter().filter(|c| baseline.is_none_or(|ids| ids.iter().any(|i| i == &c.id))).collect();
    let (start, limit) = page_of(q);
    let page: Vec<Value> = hits
        .iter()
        .skip(start)
        .take(limit)
        .map(|c| {
            let id = catalog::normalize(&c.id);
            json!({
                "control_id": id,
                "control_title": c.title,
                "catalog": c.catalog,
                "family_id": c.family_id,
                "family_title": c.family_title,
                "baselines": baselines_of(&id),
                "has_requirement": held.contains_key(&id),
                "status": held.get(&id).map(|r| r["status"].clone()),
            })
        })
        .collect();
    let next = (start + limit < hits.len()).then(|| (start + limit).to_string());
    Ok(json!({ "rows": page, "total": hits.len(), "next_cursor": next }))
}

/// The catalog name, the baseline asked for, and each control with its status and ledger evidence count.
type CoverageSet = (&'static str, Option<String>, Vec<(&'static catalog::Control, String, i64)>);

/// The controls of a catalog, or of a baseline, each with its status: the requirement's, or `none`.
fn coverage_set(q: &HashMap<String, String>, state: &SharedState) -> Result<CoverageSet, ServiceError> {
    let baseline = non_empty(q, "baseline").map(str::to_ascii_lowercase);
    let ids = match &baseline {
        Some(b) => Some(catalog::baseline(b).ok_or_else(|| ServiceError::BadRequest("baseline is low, moderate or high".to_string()))?),
        None => None,
    };
    let name = match non_empty(q, "catalog") {
        Some("sp800-53r5") | None => "sp800-53r5",
        Some("sp800-171r3") if baseline.is_none() => "sp800-171r3",
        Some("sp800-171r3") => return Err(ServiceError::BadRequest("The baselines select SP 800-53 controls, so a baseline needs catalog sp800-53r5".to_string())),
        Some(other) => return Err(ServiceError::BadRequest(format!("catalog is sp800-53r5 or sp800-171r3, not {other}"))),
    };
    let held: BTreeMap<String, (String, i64)> = rows(state)?.into_iter().map(|r| (r["control_id"].as_str().unwrap_or_default().to_string(), (r["status"].as_str().unwrap_or("planned").to_string(), r["evidence_count"].as_i64().unwrap_or(0)))).collect();
    let set = catalog::search("", None, Some(name))
        .into_iter()
        .filter(|c| ids.is_none_or(|ids| ids.iter().any(|i| i == &c.id)))
        .map(|c| {
            let (status, evidence) = held.get(&catalog::normalize(&c.id)).cloned().unwrap_or(("none".to_string(), 0));
            (c, status, evidence)
        })
        .collect();
    Ok((name, baseline, set))
}

fn counts_json(counts: &BTreeMap<String, i64>) -> Value {
    let mut out = serde_json::Map::new();
    for k in STATUSES.iter().chain(["none"].iter()) {
        out.insert((*k).to_string(), json!(counts.get(*k).copied().unwrap_or(0)));
    }
    Value::Object(out)
}

/// How many controls of the set are in each status, in each family, and how many have ledger evidence.
pub fn coverage(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let (name, baseline, set) = coverage_set(q, state)?;
    let mut total: BTreeMap<String, i64> = BTreeMap::new();
    let mut families: BTreeMap<String, (String, i64, BTreeMap<String, i64>)> = BTreeMap::new();
    let mut with_evidence = 0;
    for (c, status, evidence) in &set {
        *total.entry(status.clone()).or_default() += 1;
        let f = families.entry(c.family_id.clone()).or_insert_with(|| (c.family_title.clone(), 0, BTreeMap::new()));
        f.1 += 1;
        *f.2.entry(status.clone()).or_default() += 1;
        if *evidence > 0 {
            with_evidence += 1;
        }
    }
    let families: Vec<Value> = families.into_iter().map(|(id, (title, n, counts))| json!({ "family_id": id, "family_title": title, "total": n, "counts": counts_json(&counts) })).collect();
    Ok(json!({ "catalog": name, "baseline": baseline, "total": set.len(), "counts": counts_json(&total), "families": families, "with_evidence": with_evidence }))
}

/// A page of the controls of the set that have no requirement.
pub fn coverage_gaps(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let (name, baseline, set) = coverage_set(q, state)?;
    let gaps: Vec<_> = set.iter().filter(|(_, status, _)| status == "none").collect();
    let (start, limit) = page_of(q);
    let rows: Vec<Value> = gaps
        .iter()
        .skip(start)
        .take(limit)
        .map(|(c, _, _)| json!({ "control_id": catalog::normalize(&c.id), "control_title": c.title, "catalog": c.catalog, "family_id": c.family_id, "family_title": c.family_title }))
        .collect();
    let next = (start + limit < gaps.len()).then(|| (start + limit).to_string());
    Ok(json!({ "catalog": name, "baseline": baseline, "rows": rows, "total": gaps.len(), "next_cursor": next }))
}
