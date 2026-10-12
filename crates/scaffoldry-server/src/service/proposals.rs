//! App proposals. An agent proposes a change to an app. Nothing it sends changes a live app until
//! a person with authority approves it. Checks are computed from the proposal and the live app.
//! They are never constants.

use crate::repository_proposals::{rule_changes, Approval, ProposalRow};
use crate::service::access::authorize_workspace;
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::standards::ceds::CedsElement;
use scaffoldry_core::workflow::describe_rule;
use scaffoldry_core::{AutomationRule, StepKind};
use scaffoldry_engine::{effective_ferpa_sensitive, validate_manifest, AppManifest, FieldSpec, FieldType, HostRouter, PageKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SOURCE_BYTES: usize = 256 * 1024;

pub struct ProposeInput {
    pub workspace_id: String,
    /// Required when there is no manifest.
    pub app_slug: Option<String>,
    pub manifest: Option<Value>,
    pub rules: Vec<Value>,
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

/// A check that warns and never blocks.
fn warn(name: &str, messages: Vec<String>) -> Value {
    json!({ "name": name, "outcome": if messages.is_empty() { "pass" } else { "warn" }, "messages": messages })
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
        for rule in &proposal.access.rows {
            out.push(scaffoldry_core::access::rule_sentence(rule));
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
    for rule in &proposal.access.rows {
        match live.access.rows.iter().find(|r| r.id == rule.id) {
            None => out.push(format!("Access rule \"{}\" is added: {}", rule.id, scaffoldry_core::access::rule_sentence(rule))),
            Some(old) if old != rule => out.push(format!("Access rule \"{}\" is changed: {}", rule.id, scaffoldry_core::access::rule_sentence(rule))),
            _ => {}
        }
    }
    for old in &live.access.rows {
        if !proposal.access.rows.iter().any(|r| r.id == old.id) {
            out.push(format!("Access rule \"{}\" is removed.", old.id));
        }
    }
    if out.is_empty() {
        out.push("No change to tables, fields, views, or pages.".to_string());
    }
    out
}

pub fn live_app_exists(slug: &str, state: &SharedState) -> bool {
    live_manifest(slug, state).is_some()
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
    crate::service::members::member_role(caller, ws_id, state).map(|r| r.as_str().to_string())
}

/// The checks on a manifest part, and what it changed.
struct ManifestPart {
    manifest: AppManifest,
    checks: Vec<Value>,
    sources: Vec<(String, String)>,
}


fn count_records_using_option(
    app_slug: &str,
    table_id: &str,
    field_name: &str,
    field_type: FieldType,
    option_id: &str,
    state: &SharedState,
) -> i64 {
    let Some(repo) = state.repository.as_deref() else {
        return 0;
    };
    let app = app_slug.to_string();
    let table = table_id.to_string();
    let field = field_name.to_string();
    let opt = option_id.to_string();
    let is_multi = field_type == FieldType::MultiSelect;
    repo.with_client(move |c| {
        let count: i64 = if is_multi {
            c.query_one(
                "SELECT count(*) FROM dataset_records                  WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL                    AND (data->$3 ? $4 OR data->>$3 = $4)",
                &[&app, &table, &field, &opt],
            )?.get(0)
        } else {
            c.query_one(
                "SELECT count(*) FROM dataset_records                  WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL                    AND data->>$3 = $4",
                &[&app, &table, &field, &opt],
            )?.get(0)
        };
        Ok(count)
    }).unwrap_or(0)
}

fn check_manifest_part(
    input_manifest: Value,
    page_sources: Vec<String>,
    ws: &crate::state::WorkspaceRecord,
    live: Option<&AppManifest>,
    state: &SharedState,
) -> Result<ManifestPart, ServiceError> {
    let repo = repo(state)?;
    let mut manifest: AppManifest = serde_json::from_value(input_manifest)
        .map_err(|e| ServiceError::BadRequest(format!("schema: the manifest is not an app manifest: {e}")))?;
    if let Some(l) = live {
        if l.workspace_id.as_deref() != Some(ws.id.as_str()) {
            return Err(ServiceError::BadRequest(format!("The app '{}' belongs to another workspace", manifest.slug)));
        }
    }
    manifest.workspace_id = Some(ws.id.clone());
    // A proposal can never verify a domain. Nothing in the platform verifies one.
    let live_verified = live.is_some_and(|l| l.custom_domain_verified);
    if manifest.custom_domain_verified && !live_verified {
        return Err(ServiceError::BadRequest("schema: custom_domain_verified cannot be set by a proposal".to_string()));
    }
    manifest.custom_domain_verified = live_verified;

    // schema: a failure stores no row.
    if let Err(problems) = validate_manifest(&manifest) {
        return Err(ServiceError::BadRequest(format!("schema: the manifest has {} problem(s): {}", problems.len(), problems.join("; "))));
    }

    // page_source: a failure stores no row.
    let mut source_problems = Vec::new();
    let mut provided: BTreeMap<String, String> = BTreeMap::new();
    for source in page_sources {
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

    let proposed_sensitive = sensitive_fields(&manifest, state);
    let live_sensitive = live.map(|l| sensitive_fields(l, state)).unwrap_or_default();

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
    if let Some(l) = live {
        for lt in &l.tables {
            match manifest.tables.iter().find(|t| t.id == lt.id) {
                None => loss.push(format!("table '{}' is removed", lt.name)),
                Some(t) => {
                    for lf in &lt.fields {
                        if !t.fields.iter().any(|f| f.name == lf.name) {
                            loss.push(format!("field '{}' of table '{}' is removed", lf.name, lt.name));
                        } else if matches!(lf.field_type, FieldType::Select | FieldType::MultiSelect) {
                            if let Some(f) = t.fields.iter().find(|f| f.name == lf.name) {
                                for live_opt in &lf.select_options {
                                    if !f.select_options.iter().any(|o| o.id == live_opt.id) {
                                        let count = count_records_using_option(&manifest.slug, &lt.id, &lf.name, lf.field_type, &live_opt.id, state);
                                        if count > 0 {
                                            loss.push(format!("option '{}' of field '{}' in table '{}' is removed and used by {} records", live_opt.label, lf.name, lt.name, count));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // A field changes type: every value that would become blank is data lost. The owner runs a preview
    // first, and its result is attached to this check.
    let mut previews: Vec<Value> = Vec::new();
    if let (Some(l), true) = (live, records_exist(&manifest.slug, state)) {
        for change in scaffoldry_engine::recalc::type_changes(l, &manifest) {
            let what = format!("field '{}' of table '{}'", change.field, change.table_id);
            if !scaffoldry_engine::recalc::conversion_supported(change.from, change.to) {
                loss.push(format!("{what} cannot change from {:?} to {:?}", change.from, change.to));
                continue;
            }
            match repo.latest_convert_preview(&manifest.slug, &change.table_id, &change.field, change.to).map_err(internal)? {
                None => loss.push(format!("{what} changes type and no preview was run. POST /apps/{}/fields/convert-preview first", manifest.slug)),
                Some(result) => {
                    let blanks = result["blanks"].as_i64().unwrap_or(0);
                    if blanks > 0 {
                        loss.push(format!("{blanks} values of {what} would become blank"));
                    }
                    previews.push(result);
                }
            }
        }
    }
    let loss = if loss.is_empty() || !records_exist(&manifest.slug, state) { Vec::new() } else { loss };

    // page_sensitivity: a new or changed custom page reads a sensitive field.
    let mut page_problems = Vec::new();
    for page in &manifest.pages {
        let changed = live.and_then(|l| l.pages.iter().find(|x| x.id == page.id)).is_none_or(|x| x != page);
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

    let mut data_loss = check("data_loss", loss);
    if !previews.is_empty() {
        data_loss["previews"] = Value::Array(previews);
    }

    let access_problems = scaffoldry_engine::detect_access_problems(&manifest);
    let mut loosened = Vec::new();
    if let Some(l) = live {
        for old in &l.access.rows {
            match manifest.access.rows.iter().find(|r| r.id == old.id) {
                None => loosened.push(format!("rule '{}' was removed", old.id)),
                Some(new_r) => {
                    if new_r.visible_when != old.visible_when
                        || new_r.ops != old.ops
                        || new_r.except_roles != old.except_roles
                    {
                        loosened.push(format!("rule '{}' was changed", new_r.id));
                    }
                }
            }
        }
        for old in &l.access.columns {
            match manifest.access.columns.iter().find(|r| r.id == old.id) {
                None => loosened.push(format!("column rule '{}' was removed", old.id)),
                Some(new_r) => {
                    if new_r.fields != old.fields
                        || new_r.mode != old.mode
                        || new_r.unless != old.unless
                    {
                        loosened.push(format!("column rule '{}' was changed", new_r.id));
                    }
                }
            }
        }
    }

    let checks = vec![
        check("schema", vec![]),
        check("sensitivity", sensitivity_problems),
        data_loss,
        check("page_source", vec![]),
        check("page_sensitivity", page_problems),
        warn("relationships", scaffoldry_engine::manifest_warnings(&manifest)),
        warn("access_rules", access_problems),
        check("access_loosened", loosened),
    ];
    Ok(ManifestPart { manifest, checks, sources: provided.into_iter().collect() })
}

/// The checks on a rules part. A rule that fails `rule_schema` stores no proposal.
struct RulesPart {
    rules: Vec<(AutomationRule, u32)>,
    checks: Vec<Value>,
}

fn check_rules_part(
    caller: &AuthUser,
    raw: Vec<Value>,
    slug: &str,
    effective: &AppManifest,
    state: &SharedState,
) -> Result<RulesPart, ServiceError> {
    let repo = repo(state)?;
    let sensitive = sensitive_fields(effective, state);
    let mut schema_problems = Vec::new();
    let mut rules = Vec::new();
    for value in raw {
        let mut rule: AutomationRule = serde_json::from_value(value)
            .map_err(|e| ServiceError::BadRequest(format!("rule_schema: a rule is not a rule: {e}")))?;
        rule.app_slug = slug.to_string();
        rule.version = 0;
        for p in crate::service::processes::validate_rule(state, effective, &rule) {
            schema_problems.push(format!("rule '{}': {p}", rule.id));
        }
        let owner_app = state
            .automations
            .read()
            .ok()
            .and_then(|a| a.iter().find(|(_, list)| list.iter().any(|r| r.id == rule.id)).map(|(k, _)| k.clone()));
        if owner_app.is_some_and(|a| a != slug) {
            schema_problems.push(format!("rule id '{}' belongs to another app", rule.id));
        }
        let base = repo.rule_live_version(&rule.id).map_err(internal)?;
        rules.push((rule, base));
    }
    if !schema_problems.is_empty() {
        return Err(ServiceError::BadRequest(format!("rule_schema: {}", schema_problems.join("; "))));
    }

    let mut effects = Vec::new();
    let mut loops = Vec::new();
    let mut approvers = Vec::new();
    for (rule, _) in &rules {
        let set = crate::service::processes::fields_set(rule);
        for name in &set {
            let flagged = effective.tables.iter().any(|t| sensitive.contains(&(t.id.clone(), name.clone())));
            if flagged {
                effects.push(format!("rule '{}' sets the sensitive field '{name}'", rule.name));
            }
        }
        if matches!(rule.trigger, scaffoldry_core::workflow::TriggerEvent::RecordUpdated) {
            let read = crate::service::processes::fields_read(rule);
            for name in set.intersection(&read) {
                loops.push(format!("rule '{}' runs on every update and sets '{name}', which its own condition reads", rule.name));
            }
        }
        for step in &rule.steps {
            if let StepKind::UserTask { role, approver, .. } = &step.kind {
                let spec = scaffoldry_core::approver::effective_approver(role, approver);
                let ctx = crate::service::approvers::context(state, slug, "", Some(&caller.eppn));
                let resolution = scaffoldry_core::approver::resolve_approvers(&spec, &ctx);
                if resolution.approvers.is_empty() {
                    approvers.push(format!("rule '{}', step '{}': nobody can decide this today", rule.name, step.id));
                } else if resolution.approvers.iter().all(|a| a.eppn.eq_ignore_ascii_case(&caller.eppn)) {
                    approvers.push(format!("rule '{}', step '{}': only the proposer can decide this today", rule.name, step.id));
                }
            }
        }
    }
    let checks = vec![check("rule_schema", vec![]), check("rule_effects", effects), warn("rule_loop", loops), warn("rule_approver", approvers)];
    Ok(RulesPart { rules, checks })
}

fn rule_sentences(proposed: &AutomationRule, state: &SharedState) -> Vec<String> {
    let live = state
        .automations
        .read()
        .ok()
        .and_then(|a| a.get(&proposed.app_slug).and_then(|l| l.iter().find(|r| r.id == proposed.id).cloned()));
    let mut out = Vec::new();
    match live {
        None => {
            out.push(format!("Rule \"{}\" is added.", proposed.name));
            out.extend(describe_rule(proposed));
        }
        Some(old) => {
            // Compare the rule without its version and its switch, which have their own sentences.
            let (mut a, mut b) = (old.clone(), proposed.clone());
            (a.version, b.version) = (0, 0);
            (a.enabled, b.enabled) = (true, true);
            if a != b {
                out.push(format!("Rule \"{}\" is changed.", proposed.name));
                out.extend(describe_rule(proposed));
            }
            if old.enabled != proposed.enabled {
                out.push(format!("Rule \"{}\" is {}.", proposed.name, if proposed.enabled { "enabled" } else { "disabled" }));
            }
            if out.is_empty() {
                out.push(format!("Rule \"{}\" does not change.", proposed.name));
            }
        }
    }
    out
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
    if input.manifest.is_none() && input.rules.is_empty() {
        return Err(ServiceError::BadRequest("A proposal needs a manifest, rules, or both".to_string()));
    }

    // The app this proposal is about.
    let slug = match (&input.manifest, &input.app_slug) {
        (Some(m), given) => {
            let from_manifest = m.get("slug").and_then(|s| s.as_str()).unwrap_or("").to_string();
            if let Some(g) = given {
                if *g != from_manifest {
                    return Err(ServiceError::BadRequest("Argument 'app_slug' does not match the manifest's slug".to_string()));
                }
            }
            from_manifest
        }
        (None, Some(g)) => g.clone(),
        (None, None) => return Err(ServiceError::BadRequest("Argument 'app_slug' is required when there is no manifest".to_string())),
    };
    let live = live_manifest(&slug, state);
    if input.manifest.is_none() {
        match &live {
            None => return Err(ServiceError::NotFound(format!("App '{slug}' not found"))),
            Some(l) if l.workspace_id.as_deref() != Some(ws.id.as_str()) => {
                return Err(ServiceError::BadRequest(format!("The app '{slug}' belongs to another workspace")));
            }
            _ => {}
        }
    }

    let mut checks: Vec<Value> = Vec::new();
    let mut changes: Vec<String> = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut manifest_json: Option<Value> = None;
    let mut effective: Option<AppManifest> = live.clone();

    if let Some(raw) = input.manifest {
        let part = check_manifest_part(raw, input.page_sources, &ws, live.as_ref(), state)?;
        changes.extend(change_list(&part.manifest, live.as_ref(), state));
        checks.extend(part.checks);
        sources = part.sources;
        manifest_json = Some(serde_json::to_value(&part.manifest).map_err(internal)?);
        effective = Some(part.manifest);
    }

    let mut rules_json: Option<Value> = None;
    if !input.rules.is_empty() {
        let effective = effective.as_ref().ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))?;
        let part = check_rules_part(caller, input.rules, &slug, effective, state)?;
        for (rule, _) in &part.rules {
            changes.extend(rule_sentences(rule, state));
        }
        checks.extend(part.checks);
        rules_json = Some(Value::Array(
            part.rules.iter().map(|(r, base)| json!({ "rule": r, "base_version": base })).collect(),
        ));
    }
    checks.push(check("stale", vec![]));

    let base_version = if manifest_json.is_some() { live_version(&slug, state)? } else { None };
    let row = ProposalRow {
        id: uuid::Uuid::new_v4(),
        workspace_id: ws.id.clone(),
        app_slug: slug.clone(),
        base_version,
        manifest: manifest_json,
        checks: json!({ "checks": checks, "changes": changes }),
        summary: input.summary,
        status: "Pending".to_string(),
        proposed_by: caller.eppn.clone(),
        decided_by: None,
        created_at: String::new(),
        decided_at: None,
        rules: rules_json,
    };
    repo.insert_proposal(&row, &sources).map_err(internal)?;
    let stored = repo.get_proposal(row.id).map_err(internal)?.ok_or_else(|| ServiceError::Internal("The proposal was not stored".to_string()))?;
    view(&stored, state)
}

/// The proposal as a caller sees it. `stale` is worked out now, against the live app and rules.
fn view(p: &ProposalRow, state: &SharedState) -> Result<Value, ServiceError> {
    let mut checks = p.checks["checks"].clone();
    if let Some(list) = checks.as_array_mut() {
        let mut messages = Vec::new();
        if p.status == "Pending" {
            if p.manifest.is_some() {
                let live = live_version(&p.app_slug, state)?;
                if live != p.base_version {
                    messages.push(format!("the app is at version {live:?}, this proposal was made against {:?}", p.base_version));
                }
            }
            for (rule, base) in rule_changes(&p.rules) {
                let live = repo(state)?.rule_live_version(&rule.id).map_err(internal)?;
                if live != base {
                    messages.push(format!("rule '{}' is at version {live}, this proposal was made against {base}", rule.name));
                }
            }
        }
        for c in list.iter_mut() {
            if c["name"] == "stale" {
                *c = check("stale", messages.clone());
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
        "approve" => {
            // The rules are checked again against the app as it is now. A manifest approved since
            // the proposal was made may have removed a field a rule reads.
            let effective: Option<AppManifest> = match &row.manifest {
                Some(m) => serde_json::from_value(m.clone()).ok(),
                None => live_manifest(&row.app_slug, state),
            };
            if let Some(effective) = &effective {
                let mut problems = Vec::new();
                for (rule, _) in rule_changes(&row.rules) {
                    problems.extend(crate::service::processes::validate_rule(state, effective, &rule).into_iter().map(|p| format!("rule '{}': {p}", rule.id)));
                }
                if !problems.is_empty() {
                    return Err(ServiceError::BadRequest(format!("rule_schema: {}", problems.join("; "))));
                }
            }
            match repo.approve_proposal(id, &caller.eppn, rationale).map_err(|e| match e {
                // The approval would start more background work than the person may have. Nothing was kept.
                crate::repository::RepositoryError::Quota(message) => ServiceError::QuotaExceeded(message),
                other => ServiceError::Internal(format!("The approval was not kept: {other}")),
            })? {
                Approval::Approved(done) => {
                    state.ledger.write().unwrap_or_else(|p| p.into_inner()).extend(done.entries);
                    if let Some(m) = &row.manifest {
                        let manifest: AppManifest = serde_json::from_value(m.clone()).map_err(internal)?;
                        state.engine.write().map_err(internal)?.register_manifest(manifest.clone()).map_err(internal)?;
                        crate::service::scope::set_rules(&row.app_slug, manifest.access.rows, state);
                        crate::service::scope::set_column_rules(&row.app_slug, manifest.access.columns, state);
                    }
                    if !done.rules.is_empty() {
                        crate::service::processes::reload_rules(state, &row.app_slug);
                    }
                }
                Approval::NotPending(status) => return Err(ServiceError::Conflict(format!("This proposal is already {status}"))),
                Approval::Superseded { live } => {
                    return Err(ServiceError::Conflict(format!(
                        "The app or a rule changed since this was proposed: the app is at version {live:?}, the proposal was made against {:?}. The proposal is now superseded",
                        row.base_version
                    )))
                }
                Approval::SecondPersonRequired => {
                    return Err(ServiceError::forbidden("A change with a flagged check needs a second person to approve it. The proposer cannot"));
                }
            }
        }
        _ => return Err(ServiceError::BadRequest("Argument 'decision' must be 'approve' or 'reject'".to_string())),
    }
    get(proposal_id, state)
}

/// What a direct route got from the proposal it made.
pub enum Direct {
    /// The caller is an owner or admin, nothing failed, and nothing was flagged. It is live.
    Applied,
    /// A person must decide. The change is not live.
    Pending(Value),
}

/// A direct route is a wrapper: it makes a proposal with the same checks, and approves it in the
/// same request when its caller owns or administers the workspace and no check is raised.
/// A `fail` that stores no proposal comes back as the error `propose` returns.
pub fn submit_direct(caller: &AuthUser, input: ProposeInput, state: &SharedState) -> Result<Direct, ServiceError> {
    let ws_id = input.workspace_id.clone();
    let proposed = propose(caller, input, state)?;
    let any_fail = proposed["checks"].as_array().is_some_and(|l| l.iter().any(|c| c["outcome"] == "fail"));
    let owner_or_admin = matches!(role_in(caller, &ws_id, state).as_deref(), Some("owner" | "admin"));
    if any_fail || !owner_or_admin {
        return Ok(Direct::Pending(proposed));
    }
    let id = proposed["proposal_id"].as_str().unwrap_or_default().to_string();
    decide(caller, &id, "approve", "Applied directly by an owner or admin: no check was raised", state)?;
    Ok(Direct::Applied)
}
