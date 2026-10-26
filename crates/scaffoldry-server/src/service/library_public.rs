//! Policy library phase 4: what enforces each rule, and the library everyone reads.
//!
//! A rule is Enforced when at least one of its bindings points at something that exists and is in use right now. The
//! state is worked out on every read from the target itself. Nothing about it is stored or cached.

use crate::service::admin::{admin_write, require_platform_admin_or_compliance};
use crate::service::authority::unit_path;
use crate::service::library::{internal, parse_id, Problems, OSCAL_CONTROL};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::guards::ALLOWED_TEMPLATES;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::library::{binding_state, context_around, BindingState};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const NAMESPACE: &str = "https://scaffoldry.io/ns/oscal";
const CONTEXT_CHARS: usize = 300;
const CANDIDATES: i64 = 1000;
const KINDS: &[&str] = &["cedar_policy", "guard_template", "sensitivity_category", "retention_policy", "authority_row"];

fn require_admin(state: &SharedState, caller: &AuthUser) -> Result<(), ServiceError> {
    require_platform_admin_or_compliance(caller, state).map_err(|_| ServiceError::forbidden("Only a Platform Admin or compliance may do this"))
}

/// A name for what a binding points at, when it exists and is in use. `None` means Missing.
fn target_name(state: &SharedState, kind: &str, target: &str) -> Option<String> {
    match kind {
        "cedar_policy" => state.policy_engine.policy_summaries().into_iter().find(|p| p.id == target).map(|p| if p.description.is_empty() { format!("Policy {}", p.id) } else { p.description }),
        "guard_template" => ALLOWED_TEMPLATES.iter().find(|t| **t == target).map(|t| format!("Workspace guard: {t}")),
        "authority_row" => {
            let id = Uuid::parse_str(target).ok()?;
            let (tier, unit) = state.repository.authority_row(id).ok().flatten()?;
            let name = state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&unit).map(|o| o.name.clone()).unwrap_or_else(|| "a unit".to_string());
            Some(format!("Decision authority: {tier} changes in {name}"))
        }
        // These kinds are added by briefs that have not run, so nothing can be bound to them yet.
        _ => None,
    }
}

fn check_target(state: &SharedState, kind: &str, target: &str) -> Result<(), ServiceError> {
    match kind {
        "sensitivity_category" => Err(ServiceError::BadRequest("Sensitivity categories are added by sensitive-content.md phase 1, which has not run. Bind to a policy, a guard template or a decision-authority row".to_string())),
        "retention_policy" => Err(ServiceError::BadRequest("Retention policies are added by lifecycle.md phase 3, which has not run. Bind to a policy, a guard template or a decision-authority row".to_string())),
        k if KINDS.contains(&k) => {
            if target_name(state, k, target).is_some() {
                Ok(())
            } else {
                let mut p = Problems::default();
                p.add("target_ref", format!("no {} named that exists", k.replace('_', " ")));
                p.done()
            }
        }
        _ => {
            let mut p = Problems::default();
            p.add("target_kind", "target_kind is cedar_policy, guard_template, sensitivity_category, retention_policy or authority_row");
            p.done()
        }
    }
}

/// `POST /admin/library/policies/{id}/rules/{key}/bindings`.
pub fn add_binding(state: &SharedState, caller: &AuthUser, policy_id: &str, rule_key: &str, body: &Value) -> Result<Value, ServiceError> {
    require_admin(state, caller)?;
    let policy_id = parse_id(policy_id, "Policy")?;
    let policy = state.repository.library_policy(policy_id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Policy not found".to_string()))?;
    if !state.repository.library_rule_key_exists(policy_id, rule_key).map_err(internal)? {
        return Err(ServiceError::NotFound("Rule not found".to_string()));
    }
    let mut p = Problems::default();
    let kind = p.text(body, "target_kind", 24, true).unwrap_or_default();
    let target = p.text(body, "target_ref", 255, true).unwrap_or_default();
    let reason = p.text(body, "reason", 500, true).unwrap_or_default();
    p.done()?;
    check_target(state, &kind, &target)?;
    let payload = json!({ "action": "binding_added", "policy_key": policy["key"], "rule_key": rule_key, "target_kind": kind, "target_ref": target });
    let binding = admin_write(state, caller, DecisionType::PolicyLibraryChanged, OSCAL_CONTROL, &reason, &payload, || {
        state.repository.library_add_binding(policy_id, rule_key, &kind, &target, &caller.eppn).map_err(|e| {
            let text = e.to_string();
            if text.contains("duplicate key") {
                ServiceError::Conflict("That rule is already bound to that target".to_string())
            } else {
                internal(text)
            }
        })
    })?;
    Ok(json!({ "binding": binding }))
}

/// `DELETE /admin/library/bindings/{id}`.
pub fn delete_binding(state: &SharedState, caller: &AuthUser, id: &str, body: &Value) -> Result<Value, ServiceError> {
    require_admin(state, caller)?;
    let id = parse_id(id, "Binding")?;
    let (policy_id, binding) = state.repository.library_binding(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Binding not found".to_string()))?;
    let mut p = Problems::default();
    let reason = p.text(body, "reason", 500, true).unwrap_or_default();
    p.done()?;
    let key = state.repository.library_policy(policy_id).map_err(internal)?.map(|p| p["key"].clone()).unwrap_or(Value::Null);
    let payload = json!({ "action": "binding_removed", "policy_key": key, "rule_key": binding["rule_key"], "target_kind": binding["target_kind"], "target_ref": binding["target_ref"] });
    admin_write(state, caller, DecisionType::PolicyLibraryChanged, OSCAL_CONTROL, &reason, &payload, || {
        if !state.repository.library_delete_binding(id).map_err(internal)? {
            return Err(ServiceError::NotFound("Binding not found".to_string()));
        }
        Ok(json!({ "binding": binding }))
    })
}

/// What enforces a rule, as it is now: `(enforcement, enforced_by, warning)`.
fn enforcement_of(state: &SharedState, rule_key: &str, bindings: &[Value]) -> (&'static str, Vec<Value>, Option<String>) {
    let mine: Vec<&Value> = bindings.iter().filter(|b| b["rule_key"] == rule_key).collect();
    let lookup = |kind: &str, target: &str| target_name(state, kind, target).is_some();
    let by: Vec<Value> = mine
        .iter()
        .map(|b| {
            let (kind, target) = (b["target_kind"].as_str().unwrap_or_default(), b["target_ref"].as_str().unwrap_or_default());
            let live = binding_state(kind, target, &lookup) == BindingState::Enforced;
            json!({ "id": b["id"], "target_kind": kind, "name": target_name(state, kind, target).unwrap_or_else(|| format!("{} {target}", kind.replace('_', " "))), "state": if live { "Enforced" } else { "Missing" } })
        })
        .collect();
    let enforced = by.iter().any(|b| b["state"] == "Enforced");
    let warning = (!by.is_empty() && !enforced).then(|| "What enforced this rule is gone. It is guidance until the policy owner binds it again.".to_string());
    (if enforced { "Enforced" } else { "Guidance" }, by, warning)
}

fn decorate(state: &SharedState, rule: &mut Value, cache: &mut HashMap<Uuid, Vec<Value>>) {
    let pid = rule["policy_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default();
    let bindings = cache.entry(pid).or_insert_with(|| state.repository.library_bindings_of(pid).unwrap_or_default());
    let (enforcement, by, warning) = enforcement_of(state, rule["rule_key"].as_str().or_else(|| rule["key"].as_str()).unwrap_or_default(), bindings);
    rule["enforcement"] = json!(enforcement);
    rule["enforced_by"] = json!(by);
    rule["warning"] = json!(warning);
}

/// `enforcement` for the agent's `find_library_rules` tool.
pub fn enforcement_for(state: &SharedState, policy_id: Uuid, rule_key: &str) -> &'static str {
    let bindings = state.repository.library_bindings_of(policy_id).unwrap_or_default();
    enforcement_of(state, rule_key, &bindings).0
}

/// The policies a person may read: those of their own units, of every unit above, and of the institution. A Platform Admin
/// or compliance reads all of them.
fn visible(state: &SharedState, caller: &AuthUser) -> Result<Vec<Value>, ServiceError> {
    let all: Vec<Value> = state.repository.library_policies().map_err(internal)?.into_iter().filter(|p| p["status"] == "Active").collect();
    if require_platform_admin_or_compliance(caller, state).is_ok() {
        return Ok(all);
    }
    let mut units: HashSet<Uuid> = HashSet::new();
    for r in state.roles_of(&caller.eppn) {
        units.extend(unit_path(state, Some(r.organization_id)));
    }
    units.extend(state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().filter(|o| o.parent_id.is_none()).map(|o| o.id));
    Ok(all.into_iter().filter(|p| p["organization_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).is_some_and(|u| units.contains(&u))).collect())
}

/// `GET /library`: published rules a person may read, paged. Query: `q`, `policy`, `unit`, `enforcement`, `cursor`, `limit`.
pub fn library(state: &SharedState, caller: &AuthUser, q: &HashMap<String, String>) -> Result<Value, ServiceError> {
    let get = |k: &str| q.get(k).map(String::as_str).unwrap_or_default();
    let unit = get("unit");
    let policies: Vec<Uuid> = visible(state, caller)?
        .iter()
        .filter(|p| unit.is_empty() || p["organization_id"] == unit)
        .filter_map(|p| p["id"].as_str().and_then(|u| Uuid::parse_str(u).ok()))
        .collect();
    let mut rules = if policies.is_empty() { Vec::new() } else { state.repository.library_published_in(&policies, get("q"), get("policy"), CANDIDATES, 0).map_err(internal)? };
    let mut cache = HashMap::new();
    for r in &mut rules {
        decorate(state, r, &mut cache);
    }
    let want = get("enforcement");
    if want == "Enforced" || want == "Guidance" {
        rules.retain(|r| r["enforcement"] == want);
    }
    let offset: usize = get("cursor").parse().unwrap_or(0);
    let limit: usize = get("limit").parse().unwrap_or(50).clamp(1, 200);
    let page: Vec<Value> = rules.iter().skip(offset).take(limit).cloned().collect();
    let next = (offset + limit < rules.len()).then(|| (offset + limit).to_string());
    Ok(json!({ "rules": page, "next_cursor": next }))
}

fn readable_policy(state: &SharedState, caller: &AuthUser, key: &str) -> Result<Value, ServiceError> {
    visible(state, caller)?.into_iter().find(|p| p["key"] == key).ok_or_else(|| ServiceError::NotFound("Policy not found".to_string()))
}

/// `GET /library/policies/{key}`.
pub fn policy_page(state: &SharedState, caller: &AuthUser, key: &str) -> Result<Value, ServiceError> {
    let policy = readable_policy(state, caller, key)?;
    let id = policy["id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("policy without id"))?;
    let mut rules = state.repository.library_published_of(id).map_err(internal)?;
    let mut cache = HashMap::new();
    for r in &mut rules {
        decorate(state, r, &mut cache);
    }
    let sources: Vec<Value> = state
        .repository
        .library_sources_of(id)
        .map_err(internal)?
        .iter()
        .map(|s| json!({ "title": s["title"], "href": s["href"], "kind": s["kind"], "state": s["state"], "last_checked_at": s["latest_snapshot"]["taken_at"] }))
        .collect();
    Ok(json!({ "policy": { "key": policy["key"], "title": policy["title"], "summary": policy["summary"], "last_reviewed_at": policy["last_reviewed_at"], "organization_id": policy["organization_id"] }, "rules": rules, "sources": sources }))
}

/// `GET /library/rules/{policy_key}/{rule_key}`.
pub fn rule_page(state: &SharedState, caller: &AuthUser, policy_key: &str, rule_key: &str) -> Result<Value, ServiceError> {
    let policy = readable_policy(state, caller, policy_key)?;
    let pid = policy["id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("policy without id"))?;
    let rule_id = state.repository.library_published_rule(pid, rule_key).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Rule not found".to_string()))?;
    let mut rule = state.repository.library_rule(rule_id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Rule not found".to_string()))?;
    rule["rule_key"] = rule["key"].clone();
    rule["policy_key"] = policy["key"].clone();
    decorate(state, &mut rule, &mut HashMap::new());
    let mut citations = Vec::new();
    for c in rule["citations"].as_array().cloned().unwrap_or_default() {
        let snapshot = c["snapshot_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).and_then(|u| state.repository.library_snapshot(u).ok().flatten());
        let (before, after, title) = match &snapshot {
            Some((_, s)) => {
                let (b, a) = context_around(s["text"].as_str().unwrap_or_default(), c["passage"].as_str().unwrap_or_default(), CONTEXT_CHARS).unwrap_or_default();
                (b, a, s["source_title"].clone())
            }
            None => (String::new(), String::new(), Value::Null),
        };
        citations.push(json!({ "passage": c["passage"], "locator": c["locator"], "before": before, "after": after, "source_title": title }));
    }
    let history = state.repository.library_rule_history(pid, rule_key).map_err(internal)?;
    Ok(json!({ "rule": rule, "policy": { "key": policy["key"], "title": policy["title"] }, "citations": citations, "history": history }))
}

/// The address of the rule a Cedar policy is bound to, for a denial to link to.
pub fn rule_address_for_cedar_policy(state: &SharedState, policy_id: &str) -> Option<String> {
    state.repository.library_rule_bound_to("cedar_policy", policy_id).ok().flatten().map(|(p, r)| format!("/library/rules/{p}/{r}"))
}

/// `GET /admin/library/catalog`: the library as an OSCAL 1.1.2 catalog. A group is a policy and a control is a published
/// rule. Each control links to the sources it cites. A back-matter resource holds each source and a hash of its latest text.
pub fn catalog(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    require_admin(state, caller)?;
    let mut groups = Vec::new();
    let mut resources = Vec::new();
    for policy in state.repository.library_policies().map_err(internal)?.into_iter().filter(|p| p["status"] == "Active") {
        let pid = policy["id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or_else(|| internal("policy without id"))?;
        let rules = state.repository.library_published_of(pid).map_err(internal)?;
        if rules.is_empty() {
            continue;
        }
        let sources = state.repository.library_sources_of(pid).map_err(internal)?;
        for s in &sources {
            let id = s["id"].as_str().unwrap_or_default();
            let href = s["href"].as_str().map(str::to_string).unwrap_or_else(|| format!("urn:scaffoldry:source:{id}"));
            let mut rlink = json!({ "href": href });
            if let Some(hash) = s["latest_snapshot"]["text_sha256"].as_str() {
                rlink["hashes"] = json!([{ "algorithm": "SHA-256", "value": hash.trim() }]);
            }
            resources.push(json!({ "uuid": id, "title": s["title"], "rlinks": [rlink] }));
        }
        let bindings = state.repository.library_bindings_of(pid).map_err(internal)?;
        let mut controls = Vec::new();
        for rule in rules {
            let key = rule["rule_key"].as_str().unwrap_or_default();
            let full = state.repository.library_rule(rule["id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).unwrap_or_default()).map_err(internal)?.unwrap_or(Value::Null);
            let mut cited: Vec<String> = Vec::new();
            for c in full["citations"].as_array().cloned().unwrap_or_default() {
                if let Some((_, snap)) = c["snapshot_id"].as_str().and_then(|u| Uuid::parse_str(u).ok()).and_then(|u| state.repository.library_snapshot(u).ok().flatten()) {
                    let source = snap["source_id"].as_str().unwrap_or_default().to_string();
                    if !cited.contains(&source) {
                        cited.push(source);
                    }
                }
            }
            let (_, by, _) = enforcement_of(state, key, &bindings);
            let props: Vec<Value> = by.iter().filter(|b| b["state"] == "Enforced").map(|b| json!({ "name": "enforced-by", "ns": NAMESPACE, "value": b["name"] })).collect();
            let statement = rule["statement"].as_str().unwrap_or_default();
            let id = format!("{}-{key}", policy["key"].as_str().unwrap_or_default());
            let mut control = json!({
                "id": id,
                "title": statement.chars().take(120).collect::<String>(),
                "parts": [{ "id": format!("{id}-statement"), "name": "statement", "prose": statement }],
                "links": cited.iter().map(|s| json!({ "href": format!("#{s}"), "rel": "reference" })).collect::<Vec<_>>(),
            });
            if !props.is_empty() {
                control["props"] = Value::Array(props);
            }
            controls.push(control);
        }
        groups.push(json!({ "id": policy["key"], "title": policy["title"], "controls": controls }));
    }
    let mut cat = json!({
        "uuid": Uuid::new_v4(),
        "metadata": { "title": "Scaffoldry policy library", "last-modified": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), "version": "1", "oscal-version": "1.1.2" },
    });
    if !groups.is_empty() {
        cat["groups"] = Value::Array(groups);
        cat["back-matter"] = json!({ "resources": resources });
    }
    Ok(json!({ "catalog": cat }))
}
