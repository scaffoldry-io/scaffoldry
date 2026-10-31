//! Approved app templates (import-export phase 4).
//!
//! A template is a bundle in the institution's catalog. Publishing one makes it `pending`. Someone other than the publisher
//! approves it, and every step is a ledger entry written before the change. Using one makes an ordinary proposal, with the
//! template as content that was reviewed already: the checks that ask "did this change?" compare with it, so what matches
//! the template is not raised again and what differs is. The classification check is not relaxed: it asks about the
//! workspace, and a template does not change what a workspace may hold.
//! NIST SP 800-53 CM-2, CM-3 and CM-14.

use crate::repository_templates::TemplateRow;
use crate::service::admin::{admin_write, require_platform_admin};
use crate::service::bundle::{self, digest_of};
use crate::service::organizations::{is_platform_admin, unit_in_reach, OrgCaller};
use crate::service::proposals::{self, ProposeInput, Reviewed};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_core::{AutomationRule, StepKind};
use scaffoldry_engine::{validate_manifest, AppManifest};
use serde_json::{json, Value};
use uuid::Uuid;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::bad_request(m)
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn parse_id(id: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(id).map_err(|_| ServiceError::not_found("Template not found"))
}

fn org_caller(caller: &AuthUser) -> OrgCaller {
    OrgCaller { eppn: caller.eppn.clone(), affiliation: caller.affiliation.clone() }
}

fn is_admin(caller: &AuthUser, state: &SharedState) -> bool {
    let orgs: Vec<_> = state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    is_platform_admin(&org_caller(caller), &orgs, &state.roles_of(&caller.eppn))
}

/// A Platform Admin may manage any template. An Org Unit Admin may manage the templates limited to their unit or below.
fn require_manager(caller: &AuthUser, unit: Option<Uuid>, state: &SharedState) -> Result<(), ServiceError> {
    let Some(unit) = unit else { return require_platform_admin(caller, state) };
    if is_admin(caller, state) {
        return Ok(());
    }
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    if unit_in_reach(&org_caller(caller), unit, &tree, &state.roles_of(&caller.eppn)) {
        Ok(())
    } else {
        Err(ServiceError::forbidden("Publishing a template needs a Platform Admin, or an Org Unit Admin for the unit it is limited to"))
    }
}

/// A template limited to a unit is seen by people placed in that unit or below, and by Platform Admins.
fn visible_to(caller: &AuthUser, t: &TemplateRow, state: &SharedState) -> bool {
    let Some(unit) = t.unit_id else { return true };
    if is_admin(caller, state) {
        return true;
    }
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    state.roles_of(&caller.eppn).iter().any(|r| tree.contains(unit, r.organization_id))
}

fn manifest_of(bundle: &Value) -> Result<AppManifest, ServiceError> {
    serde_json::from_value(bundle["manifest"].clone()).map_err(|e| bad(format!("schema: the bundle's manifest is not an app manifest: {e}")))
}

/// What a template holds, in sentences: its tables, who sees what, and which approvals its rules ask for.
fn contents(bundle: &Value, state: &SharedState) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(m) = manifest_of(bundle) else { return out };
    let sensitive = proposals::sensitive_fields(&m, state);
    for t in &m.tables {
        let names: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).collect();
        out.push(format!("Table \"{}\" has {} field(s): {}.", t.name, names.len(), names.join(", ")));
        let flagged: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).filter(|n| sensitive.contains(&(t.id.clone(), (*n).to_string()))).collect();
        if !flagged.is_empty() {
            out.push(format!("Sensitive in \"{}\": {}.", t.name, flagged.join(", ")));
        }
    }
    for page in &m.pages {
        let sentences = proposals::grant_sentences(&m, &page.title, &page.kind);
        if sentences.is_empty() {
            out.push(format!("Page \"{}\".", page.title));
        }
        out.extend(sentences);
    }
    for rule in &m.access.rows {
        out.push(scaffoldry_core::access::rule_sentence(rule));
    }
    for raw in bundle["rules"].as_array().into_iter().flatten() {
        let Ok(rule) = serde_json::from_value::<AutomationRule>(raw.clone()) else { continue };
        out.push(format!("Rule \"{}\".", rule.name));
        let asks = rule.steps.iter().filter(|s| matches!(s.kind, StepKind::UserTask { .. })).count();
        if asks > 0 {
            out.push(format!("Rule \"{}\" asks a person to decide at {asks} step(s).", rule.name));
        }
    }
    let hooks = bundle["integrations"]["webhooks"].as_array().map_or(0, Vec::len);
    if hooks > 0 {
        out.push(format!("{hooks} webhook(s), off until each is approved on its own."));
    }
    out
}

/// The lowest classification that can hold the template without raising `sensitivity`.
fn needs_classification(bundle: &Value, state: &SharedState) -> &'static str {
    match manifest_of(bundle) {
        Ok(m) if !proposals::sensitive_fields(&m, state).is_empty() => "FERPA Sensitive",
        _ => "Any",
    }
}

fn view(t: &TemplateRow, state: &SharedState) -> Value {
    json!({
        "id": t.id,
        "template_id": t.id,
        "name": t.name,
        "description": t.description,
        "category": t.category,
        "version": t.version,
        "status": t.status,
        "unit_id": t.unit_id,
        "published_by": t.published_by,
        "published_at": t.published_at,
        "decided_by": t.decided_by,
        "decided_at": t.decided_at,
        "bundle_sha256": t.bundle_sha256,
        "contents": contents(&t.bundle, state),
        "needs_classification": needs_classification(&t.bundle, state),
    })
}

fn text(body: &Value, key: &str, max: usize) -> Result<Option<String>, ServiceError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let s = v.as_str().ok_or_else(|| bad(format!("'{key}' must be text")))?.trim().to_string();
            if s.chars().count() > max {
                return Err(bad(format!("'{key}' is at most {max} characters")));
            }
            Ok(Some(s))
        }
    }
}

/// Publishes a bundle as a template. It waits for someone other than the publisher.
pub fn publish(caller: &AuthUser, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let unit_id = match text(body, "unit_id", 64)?.filter(|s| !s.is_empty()) {
        Some(u) => {
            let u = Uuid::parse_str(&u).map_err(|_| bad("'unit_id' must be a unit id"))?;
            if !state.organizations.read().unwrap_or_else(|p| p.into_inner()).contains_key(&u) {
                return Err(ServiceError::not_found("That unit does not exist"));
            }
            Some(u)
        }
        None => None,
    };
    require_manager(caller, unit_id, state)?;
    let name = text(body, "name", 120)?.filter(|s| !s.is_empty()).ok_or_else(|| bad("A template needs a 'name'"))?;
    let bundle = body.get("bundle").ok_or_else(|| bad("The request needs a 'bundle'"))?;
    bundle::verify(bundle)?;
    if bundle.get("sample_data").is_some() {
        return Err(bad("A template holds no sample data. Export the bundle without it"));
    }
    let manifest = manifest_of(bundle)?;
    if let Err(problems) = validate_manifest(&manifest) {
        return Err(bad(format!("schema: the bundle's manifest has {} problem(s): {}", problems.len(), problems.join("; "))));
    }
    bundle::checked_page_sources(bundle, &bundle["manifest"])?;

    let row = TemplateRow {
        id: Uuid::new_v4(),
        name: name.clone(),
        description: text(body, "description", 2000)?.unwrap_or_default(),
        category: text(body, "category", 60)?.filter(|s| !s.is_empty()).unwrap_or_else(|| "General".to_string()),
        bundle: bundle.clone(),
        bundle_sha256: digest_of(bundle),
        version: 1,
        status: "pending".to_string(),
        unit_id,
        published_by: caller.eppn.clone(),
        published_at: String::new(),
        decided_by: None,
        decided_at: None,
        rationale: String::new(),
    };
    let payload = json!({ "template_id": row.id, "name": name, "bundle_sha256": row.bundle_sha256, "unit_id": unit_id });
    admin_write(state, caller, DecisionType::TemplatePublished, "CM-03", &format!("Published the template \"{name}\" for review"), &payload, || {
        state.repository.insert_template(&row).map_err(internal)
    })?;
    let stored = state.repository.get_template(row.id).map_err(internal)?.ok_or_else(|| ServiceError::Internal("The template was not stored".to_string()))?;
    Ok(view(&stored, state))
}

fn load(id: &str, state: &SharedState) -> Result<TemplateRow, ServiceError> {
    state.repository.get_template(parse_id(id)?).map_err(internal)?.ok_or_else(|| ServiceError::not_found("Template not found"))
}

/// Approves or rejects a pending template. Not its publisher.
pub fn decide(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let t = load(id, state)?;
    require_manager(caller, t.unit_id, state)?;
    let status = match body.get("decision").and_then(Value::as_str) {
        Some("approve") => "approved",
        Some("reject") => "rejected",
        _ => return Err(bad("'decision' must be 'approve' or 'reject'")),
    };
    let rationale = text(body, "rationale", 500)?.filter(|s| !s.is_empty()).ok_or_else(|| bad("'rationale' must say why"))?;
    if t.published_by.eq_ignore_ascii_case(&caller.eppn) {
        return Err(ServiceError::forbidden("A template needs a second person to approve it. The publisher cannot"));
    }
    if t.status != "pending" {
        return Err(ServiceError::conflict(format!("This template is already {}", t.status)));
    }
    let payload = json!({ "template_id": t.id, "decision": status, "bundle_sha256": t.bundle_sha256 });
    admin_write(state, caller, DecisionType::TemplateDecided, "CM-03", &rationale, &payload, || {
        state
            .repository
            .decide_template(t.id, status, &caller.eppn, &rationale)
            .map_err(internal)?
            .map_err(|found| ServiceError::conflict(format!("This template is already {found}")))
    })?;
    Ok(view(&load(id, state)?, state))
}

/// Takes an approved template out of the catalog. What was made from it stays.
pub fn retire(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let t = load(id, state)?;
    require_manager(caller, t.unit_id, state)?;
    let reason = text(body, "reason", 500)?.filter(|s| !s.is_empty()).ok_or_else(|| bad("'reason' must say why"))?;
    let payload = json!({ "template_id": t.id, "bundle_sha256": t.bundle_sha256 });
    admin_write(state, caller, DecisionType::TemplateRetired, "CM-03", &reason, &payload, || {
        state
            .repository
            .retire_template(t.id, &caller.eppn, &reason)
            .map_err(internal)?
            .map_err(|found| ServiceError::conflict(format!("Only an approved template can be retired. This one is {found}")))
    })?;
    Ok(view(&load(id, state)?, state))
}

/// The approved templates this person may use, by category.
pub fn list(caller: &AuthUser, state: &SharedState) -> Result<Value, ServiceError> {
    let rows = state.repository.approved_templates().map_err(internal)?;
    let templates: Vec<Value> = rows.iter().filter(|t| visible_to(caller, t, state)).map(|t| view(t, state)).collect();
    Ok(json!({ "templates": templates }))
}

/// Makes a proposal from a template. `manifest` may carry the template's manifest with changes. `approve` asks for the
/// owner's approval in the same request, which stands only when nothing failed and what the change needs is the owner's say.
pub fn instantiate(caller: &AuthUser, workspace_id: &str, template_id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let t = load(template_id, state)?;
    if !matches!(t.status.as_str(), "approved" | "retired") || !visible_to(caller, &t, state) {
        return Err(ServiceError::not_found("Template not found"));
    }
    if t.status == "retired" {
        return Err(ServiceError::conflict("This template is retired. Ask for a current one"));
    }
    let p = bundle::prepare(&t.bundle, workspace_id, body, body.get("manifest"), state)?;
    let summary = bundle::summary_of(body, &p.manifest, &p.slug, "Use the template");
    let reviewed = Reviewed { manifest: t.bundle["manifest"].clone(), rules: p.rules.clone() };
    let input = ProposeInput {
        workspace_id: workspace_id.to_string(),
        app_slug: None,
        manifest: Some(p.manifest.clone()),
        rules: p.rules.clone(),
        summary,
        page_sources: p.page_sources.clone(),
        notes: p.notes.clone(),
    };
    let mut proposal = proposals::propose_reviewed(caller, input, reviewed, state)?;
    let wants = body.get("approve").and_then(Value::as_bool).unwrap_or(false);
    let approved = wants && proposals::approve_if_direct(caller, &proposal, state)?;
    if approved {
        let id = proposal["proposal_id"].as_str().unwrap_or_default().to_string();
        proposal = proposals::get(&id, state)?;
    }
    proposal["approved"] = json!(approved);
    proposal["template"] = json!({ "id": t.id, "name": t.name, "version": t.version });
    proposal["bundle"] = bundle::describe(&p);
    Ok(proposal)
}
