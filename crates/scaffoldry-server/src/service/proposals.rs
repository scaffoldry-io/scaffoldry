//! App proposals. An agent proposes a change to an app. Nothing it sends changes a live app until
//! a person with authority approves it. Checks are computed from the proposal and the live app.
//! They are never constants.

use crate::repository_proposals::{Approval, ProposalRow};
use crate::service::access::authorize_workspace;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::standards::ceds::CedsElement;
use scaffoldry_engine::{effective_ferpa_sensitive, validate_manifest, AppManifest, FieldSpec, HostRouter, PageKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SOURCE_BYTES: usize = 256 * 1024;

pub struct ProposeInput {
    pub workspace_id: String,
    pub manifest: Value,
    pub summary: String,
    pub page_sources: Vec<String>,
}

fn repo(state: &SharedState) -> Result<&crate::repository::PostgresRepository, ServiceError> {
    state.repository.as_deref().ok_or_else(|| ServiceError::Internal("Proposals need PostgreSQL".to_string()))
}

fn internal(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Internal(e.to_string())
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_id(id: &str) -> Result<uuid::Uuid, ServiceError> {
    uuid::Uuid::parse_str(id).map_err(|_| ServiceError::NotFound("Proposal not found".to_string()))
}

/// The workspace a proposal belongs to, for the gate.
pub fn workspace_of(proposal_id: &str, state: &SharedState) -> Result<String, ServiceError> {
    let id = parse_id(proposal_id)?;
    repo(state)?
        .get_proposal(id)
        .map_err(internal)?
        .map(|p| p.workspace_id)
        .ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))
}

fn check(name: &str, problems: Vec<String>) -> Value {
    json!({ "name": name, "outcome": if problems.is_empty() { "pass" } else { "fail" }, "messages": problems })
}

/// Fields that are protected under the effective rule, or whose CEDS code is FERPA sensitive.
fn sensitive_fields(m: &AppManifest, state: &SharedState) -> BTreeSet<(String, String)> {
    let categories = state.sensitivity_categories();
    let labels = state.labels.read().unwrap_or_else(|p| p.into_inner());
    let mut out = BTreeSet::new();
    for table in &m.tables {
        for f in &table.fields {
            let label = labels.get(&(m.slug.clone(), table.id.clone(), f.name.clone()));
            let by_ceds = m
                .ceds_mappings
                .get(&f.name)
                .and_then(|code| CedsElement::from_code(code))
                .is_some_and(|e| e.is_ferpa_sensitive());
            if effective_ferpa_sensitive(f, label, &categories).protected || by_ceds {
                out.insert((table.id.clone(), f.name.clone()));
            }
        }
    }
    out
}

fn field_of<'a>(m: &'a AppManifest, table: &str, name: &str) -> Option<&'a FieldSpec> {
    m.tables.iter().find(|t| t.id == table).and_then(|t| t.fields.iter().find(|f| f.name == name))
}

fn words(m: &AppManifest, table: &str, names: &[String]) -> String {
    let list: Vec<String> = names
        .iter()
        .map(|n| field_of(m, table, n).map(|f| f.label.to_lowercase()).unwrap_or_else(|| n.clone()))
        .collect();
    if list.is_empty() {
        "nothing".to_string()
    } else {
        list.join(", ")
    }
}

/// The grants of a custom page, in words a department chair can check.
fn grant_sentences(m: &AppManifest, page_title: &str, kind: &PageKind) -> Vec<String> {
    let PageKind::Custom { grants, .. } = kind else { return Vec::new() };
    if grants.is_empty() {
        return vec![format!("Page \"{page_title}\" has no access to any table.")];
    }
    grants
        .iter()
        .map(|g| {
            let table = m.tables.iter().find(|t| t.id == g.table_id).map(|t| t.name.clone()).unwrap_or_else(|| g.table_id.clone());
            let change = if g.write.is_empty() { "It may not change any field.".to_string() } else { format!("It may change: {}.", words(m, &g.table_id, &g.write)) };
            let records = match (g.create, g.delete) {
                (false, false) => "It may not create or delete records.",
                (true, false) => "It may create records. It may not delete records.",
                (false, true) => "It may delete records. It may not create records.",
                (true, true) => "It may create and delete records.",
            };
            format!("Page \"{page_title}\" reads {table}: {}. {change} {records}", words(m, &g.table_id, &g.read))
        })
        .collect()
}

/// The change list: the proposal against the live manifest, as plain sentences.
fn change_list(proposal: &AppManifest, live: Option<&AppManifest>, state: &SharedState) -> Vec<String> {
    let mut out = Vec::new();
    let Some(live) = live else {
        out.push(format!("A new app \"{}\" with {} table(s) and {} page(s).", proposal.title, proposal.tables.len(), proposal.pages.len()));
        for page in &proposal.pages {
            out.push(format!("Page \"{}\" is added.", page.title));
            out.extend(grant_sentences(proposal, &page.title, &page.kind));
        }
        return out;
    };
    if proposal.title != live.title {
        out.push(format!("The title changes from \"{}\" to \"{}\".", live.title, proposal.title));
    }
    for t in &proposal.tables {
        match live.tables.iter().find(|l| l.id == t.id) {
            None => out.push(format!("Table \"{}\" is added with {} field(s).", t.name, t.fields.len())),
            Some(l) => {
                for f in &t.fields {
                    match l.fields.iter().find(|x| x.name == f.name) {
                        None => out.push(format!("Field \"{}\" is added to table \"{}\".", f.name, t.name)),
                        Some(x) if x.field_type != f.field_type => out.push(format!(
                            "Field \"{}\" of table \"{}\" changes type from {:?} to {:?}.",
                            f.name, t.name, x.field_type, f.field_type
                        )),
                        _ => {}
                    }
                }
                for x in &l.fields {
                    if !t.fields.iter().any(|f| f.name == x.name) {
                        out.push(format!("Field \"{}\" is removed from table \"{}\".", x.name, t.name));
                    }
                }
            }
        }
    }
    for l in &live.tables {
        if !proposal.tables.iter().any(|t| t.id == l.id) {
            out.push(format!("Table \"{}\" is removed.", l.name));
        }
    }
    for v in &proposal.views {
        if !live.views.iter().any(|x| x.id == v.id) {
            out.push(format!("View \"{}\" is added.", v.title));
        }
    }
    for v in &live.views {
        if !proposal.views.iter().any(|x| x.id == v.id) {
            out.push(format!("View \"{}\" is removed.", v.title));
        }
    }
    let sensitive = sensitive_fields(proposal, state);
    for page in &proposal.pages {
        match live.pages.iter().find(|x| x.id == page.id) {
            None => {
                out.push(format!("Page \"{}\" is added.", page.title));
                out.extend(grant_sentences(proposal, &page.title, &page.kind));
            }
            Some(x) if x != page => {
                out.push(format!("Page \"{}\" is changed.", page.title));
                out.extend(grant_sentences(proposal, &page.title, &page.kind));
            }
            _ => continue,
        }
        if let PageKind::Custom { grants, .. } = &page.kind {
            for g in grants {
                for name in &g.read {
                    if sensitive.contains(&(g.table_id.clone(), name.clone())) {
                        let label = field_of(proposal, &g.table_id, name).map(|f| f.label.to_lowercase()).unwrap_or_else(|| name.clone());
                        out.push(format!("Page \"{}\" reads a sensitive field: {label} ({}).", page.title, g.table_id));
                    }
                }
            }
        }
    }
    for x in &live.pages {
        if !proposal.pages.iter().any(|p| p.id == x.id) {
            out.push(format!("Page \"{}\" is removed.", x.title));
        }
    }
    if out.is_empty() {
        out.push("No change to tables, fields, views, or pages.".to_string());
    }
    out
}

fn live_manifest(slug: &str, state: &SharedState) -> Option<AppManifest> {
    state.engine.read().ok().and_then(|e| e.resolve_by_slug(slug).cloned())
}

fn live_version(slug: &str, state: &SharedState) -> Result<Option<i32>, ServiceError> {
    repo(state)?.app_manifest_version(slug).map_err(internal)
}

fn records_exist(slug: &str, state: &SharedState) -> bool {
    match state.repository.as_deref() {
        Some(r) => r.count_records_by_app_slugs(&[slug.to_string()]).map(|m| m.get(slug).copied().unwrap_or(0) > 0).unwrap_or(false),
        None => state.count_app_records(slug).map(|n| n > 0).unwrap_or(false),
    }
}

/// The caller's role in the workspace, lowercase. A Platform Admin counts as an admin.
fn role_in(caller: &AuthUser, ws_id: &str, state: &SharedState) -> Option<String> {
    if caller.affiliation == "central_admin" {
        return Some("admin".to_string());
    }
    state
        .collaborators
        .read()
        .ok()
        .and_then(|c| c.get(ws_id).and_then(|l| l.iter().find(|m| m.eppn == caller.eppn).map(|m| m.role.to_lowercase())))
}

pub fn propose(caller: &AuthUser, input: ProposeInput, state: &SharedState) -> Result<Value, ServiceError> {
    let repo = repo(state)?;
    let ws = state
        .workspaces
        .read()
        .map_err(internal)?
        .get(&input.workspace_id)
        .cloned()
        .ok_or_else(|| ServiceError::NotFound(format!("Workspace '{}' not found", input.workspace_id)))?;
    if !matches!(role_in(caller, &ws.id, state).as_deref(), Some("owner" | "admin" | "editor")) {
        return Err(ServiceError::forbidden("Forbidden: proposing a change needs the owner, admin, or editor role"));
    }
    if input.summary.trim().is_empty() {
        return Err(ServiceError::BadRequest("Argument 'summary' must say what changes".to_string()));
    }

    let mut manifest: AppManifest =
        serde_json::from_value(input.manifest).map_err(|e| ServiceError::BadRequest(format!("schema: the manifest is not an app manifest: {e}")))?;
    let live = live_manifest(&manifest.slug, state);
    if let Some(l) = &live {
        if l.workspace_id.as_deref() != Some(ws.id.as_str()) {
            return Err(ServiceError::BadRequest(format!("The app '{}' belongs to another workspace", manifest.slug)));
        }
    }
    manifest.workspace_id = Some(ws.id.clone());
    // A proposal can never verify a domain.
    manifest.custom_domain = live.as_ref().and_then(|l| l.custom_domain.clone());
    manifest.custom_domain_verified = live.as_ref().is_some_and(|l| l.custom_domain_verified);

    // schema: a failure stores no row.
    if let Err(problems) = validate_manifest(&manifest) {
        return Err(ServiceError::BadRequest(format!("schema: the manifest has {} problem(s): {}", problems.len(), problems.join("; "))));
    }

    // page_source: a failure stores no row.
    let mut source_problems = Vec::new();
    let mut provided: BTreeMap<String, String> = BTreeMap::new();
    for source in input.page_sources {
        if source.len() > MAX_SOURCE_BYTES {
            source_problems.push(format!("a page source is {} bytes, the limit is {MAX_SOURCE_BYTES}", source.len()));
            continue;
        }
        if source.to_lowercase().contains("</script") {
            source_problems.push("a page source contains the text '</script'".to_string());
            continue;
        }
        provided.insert(sha256_hex(&source), source);
    }
    let wanted: Vec<String> = manifest
        .pages
        .iter()
        .filter_map(|p| match &p.kind {
            PageKind::Custom { source_sha256, .. } => Some(source_sha256.to_lowercase()),
            _ => None,
        })
        .collect();
    let missing: Vec<&String> = wanted.iter().filter(|h| !provided.contains_key(*h)).collect();
    if !missing.is_empty() {
        let stored = repo.page_sources_present(&missing.iter().map(|h| (*h).clone()).collect::<Vec<_>>()).map_err(internal)?;
        for h in missing {
            if !stored.contains(h) {
                source_problems.push(format!("a custom page names the source {h}, which was not sent and is not stored"));
            }
        }
    }
    if !source_problems.is_empty() {
        return Err(ServiceError::BadRequest(format!("page_source: {}", source_problems.join("; "))));
    }

    let base_version = live_version(&manifest.slug, state)?;
    let proposed_sensitive = sensitive_fields(&manifest, state);
    let live_sensitive = live.as_ref().map(|l| sensitive_fields(l, state)).unwrap_or_default();

    // sensitivity: a sensitive field the live app did not have, in an Internal or Public workspace.
    let classification = ws.data_classification.as_str();
    let sensitivity_problems: Vec<String> = if matches!(classification, "Internal" | "Public") {
        proposed_sensitive
            .difference(&live_sensitive)
            .map(|(t, f)| format!("field '{f}' of table '{t}' is sensitive and the workspace is classified {classification}"))
            .collect()
    } else {
        Vec::new()
    };

    // data_loss: something in the live app is gone, and records exist.
    let mut loss = Vec::new();
    if let Some(l) = &live {
        for lt in &l.tables {
            match manifest.tables.iter().find(|t| t.id == lt.id) {
                None => loss.push(format!("table '{}' is removed", lt.name)),
                Some(t) => {
                    for lf in &lt.fields {
                        if !t.fields.iter().any(|f| f.name == lf.name) {
                            loss.push(format!("field '{}' of table '{}' is removed", lf.name, lt.name));
                        }
                    }
                }
            }
        }
    }
    let loss = if loss.is_empty() || !records_exist(&manifest.slug, state) { Vec::new() } else { loss };

    // page_sensitivity: a new or changed custom page reads a sensitive field.
    let mut page_problems = Vec::new();
    for page in &manifest.pages {
        let changed = live.as_ref().and_then(|l| l.pages.iter().find(|x| x.id == page.id)).is_none_or(|x| x != page);
        if !changed {
            continue;
        }
        if let PageKind::Custom { grants, .. } = &page.kind {
            for g in grants {
                for name in &g.read {
                    if proposed_sensitive.contains(&(g.table_id.clone(), name.clone())) {
                        page_problems.push(format!("page '{}' reads the sensitive field '{name}' of table '{}'", page.title, g.table_id));
                    }
                }
            }
        }
    }

    let changes = change_list(&manifest, live.as_ref(), state);
    let checks = json!({
        "checks": [
            check("schema", vec![]),
            check("sensitivity", sensitivity_problems),
            check("data_loss", loss),
            check("stale", vec![]),
            check("page_source", vec![]),
            check("page_sensitivity", page_problems),
        ],
        "changes": changes,
    });

    let row = ProposalRow {
        id: uuid::Uuid::new_v4(),
        workspace_id: ws.id.clone(),
        app_slug: manifest.slug.clone(),
        base_version,
        manifest: serde_json::to_value(&manifest).map_err(internal)?,
        checks,
        summary: input.summary,
        status: "Pending".to_string(),
        proposed_by: caller.eppn.clone(),
        decided_by: None,
        created_at: String::new(),
        decided_at: None,
    };
    let sources: Vec<(String, String)> = provided.into_iter().collect();
    repo.insert_proposal(&row, &sources).map_err(internal)?;
    let stored = repo.get_proposal(row.id).map_err(internal)?.ok_or_else(|| ServiceError::Internal("The proposal was not stored".to_string()))?;
    view(&stored, state)
}

/// The proposal as a caller sees it. `stale` is worked out now, against the live app.
fn view(p: &ProposalRow, state: &SharedState) -> Result<Value, ServiceError> {
    let mut checks = p.checks["checks"].clone();
    if let Some(list) = checks.as_array_mut() {
        let live = if p.status == "Pending" { live_version(&p.app_slug, state)? } else { p.base_version };
        let stale = live != p.base_version;
        for c in list.iter_mut() {
            if c["name"] == "stale" {
                *c = check("stale", if stale { vec![format!("the app is at version {live:?}, this proposal was made against {:?}", p.base_version)] } else { vec![] });
            }
        }
    }
    Ok(json!({
        "id": p.id,
        "proposal_id": p.id,
        "workspace_id": p.workspace_id,
        "app_slug": p.app_slug,
        "base_version": p.base_version,
        "status": p.status,
        "summary": p.summary,
        "proposed_by": p.proposed_by,
        "decided_by": p.decided_by,
        "created_at": p.created_at,
        "decided_at": p.decided_at,
        "checks": checks,
        "changes": p.checks["changes"],
    }))
}

pub fn list(workspace_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let rows = repo(state)?.list_proposals(workspace_id, 100).map_err(internal)?;
    let views: Result<Vec<Value>, ServiceError> = rows.iter().map(|p| view(p, state)).collect();
    Ok(Value::Array(views?))
}

pub fn get(proposal_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let row = repo(state)?.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    view(&row, state)
}

pub fn decide(caller: &AuthUser, proposal_id: &str, decision: &str, rationale: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let repo = repo(state)?;
    let row = repo.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    authorize_workspace(caller, &row.workspace_id, "manage_workspace", state)?;
    if rationale.trim().is_empty() {
        return Err(ServiceError::BadRequest("Argument 'rationale' must say why".to_string()));
    }
    match decision {
        "reject" => {
            repo.reject_proposal(id, &caller.eppn)
                .map_err(internal)?
                .map_err(|status| ServiceError::Conflict(format!("This proposal is already {status}")))?;
        }
        "approve" => match repo.approve_proposal(id, &caller.eppn, rationale).map_err(|e| ServiceError::Internal(format!("The approval was not kept: {e}")))? {
            Approval::Approved { entry, .. } => {
                state.ledger.write().unwrap_or_else(|p| p.into_inner()).push(entry);
                let manifest: AppManifest = serde_json::from_value(row.manifest.clone()).map_err(internal)?;
                state.engine.write().map_err(internal)?.register_manifest(manifest).map_err(internal)?;
            }
            Approval::NotPending(status) => return Err(ServiceError::Conflict(format!("This proposal is already {status}"))),
            Approval::Superseded { live } => {
                return Err(ServiceError::Conflict(format!(
                    "The app changed since this was proposed: it is at version {live:?}, the proposal was made against {:?}. The proposal is now superseded",
                    row.base_version
                )))
            }
            Approval::SecondPersonRequired => {
                return Err(ServiceError::forbidden("A change with a flagged check needs a second person to approve it. The proposer cannot"));
            }
        },
        _ => return Err(ServiceError::BadRequest("Argument 'decision' must be 'approve' or 'reject'".to_string())),
    }
    get(proposal_id, state)
}
