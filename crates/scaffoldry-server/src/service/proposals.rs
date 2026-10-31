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
/// A bundle carries its own libraries, so it may be larger than a module.
const MAX_BUNDLE_BYTES: usize = 2 * 1024 * 1024;

/// The licenses a bundled library may carry, as `governance/scripts/audit-licenses.py` lists them. A test keeps the two the same.
pub const APPROVED_LICENSES: &[&str] = &["apache-2.0", "mit", "bsd-2-clause", "bsd-3-clause", "isc", "postgresql", "cc0-1.0"];

/// True when the license, or one choice of an `A OR B` expression, is approved. An `AND` expression is not read.
fn license_approved(license: &str) -> bool {
    let lower = license.to_lowercase();
    if lower.contains(" and ") {
        return false;
    }
    lower.split(" or ").any(|part| APPROVED_LICENSES.contains(&part.trim().trim_matches(|c| c == '(' || c == ')').trim()))
}

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
    Ok(&*state.repository)
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
    { let r = &state.repository; r.count_records_by_app_slugs(&[slug.to_string()]).map(|m| m.get(slug).copied().unwrap_or(0) > 0).unwrap_or(false) }
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
    let repo = &state.repository;
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

    let placed = crate::service::coordinates::manifest_problems(state, ws, &manifest)?;
    if !placed.is_empty() {
        return Err(ServiceError::BadRequest(format!("coordinates: the manifest has {} problem(s): {}", placed.len(), placed.join("; "))));
    }

    for table in &manifest.tables {
        for f in &table.fields {
            if f.field_type == scaffoldry_engine::FieldType::Formula {
                if let Some(ref expr) = f.formula_expression {
                    if scaffoldry_engine::is_formula_volatile(expr) {
                        let total_rows: i64 = repo.with_client({
                            let (slug, tid) = (manifest.slug.clone(), table.id.clone());
                            move |c| {
                                Ok(c.query_one(
                                    "SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL",
                                    &[&slug, &tid],
                                )?.get(0))
                            }
                        }).unwrap_or(0);
                        if total_rows > 500_000 {
                            return Err(ServiceError::BadRequest(format!(
                                "field '{}' of table '{}': volatile formulas (TODAY, NOW) are not supported on tables with more than 500,000 rows (has {} rows)",
                                f.name, table.name, total_rows
                            )));
                        }
                    }
                }
            }
        }
    }

    // page_source: a failure stores no row.
    let mut source_problems = Vec::new();
    let mut provided: BTreeMap<String, String> = BTreeMap::new();
    // A bundle may be larger than a module, so the limit depends on the page that names the source.
    let limits: BTreeMap<String, usize> = manifest
        .pages
        .iter()
        .filter_map(|p| match &p.kind {
            PageKind::Custom { source_sha256, format, .. } => {
                Some((source_sha256.to_lowercase(), if *format == scaffoldry_engine::PageFormat::Bundle { MAX_BUNDLE_BYTES } else { MAX_SOURCE_BYTES }))
            }
            _ => None,
        })
        .collect();
    for source in page_sources {
        let limit = limits.get(&sha256_hex(&source)).copied().unwrap_or(MAX_SOURCE_BYTES);
        if source.len() > limit {
            source_problems.push(format!("a page source is {} bytes, the limit is {limit}", source.len()));
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
    if let (Some(l), true) = (live, records_exist(&manifest.slug, state)) {
        for t in &manifest.tables {
            let old_table = l.tables.iter().find(|ot| ot.id == t.id);
            for f in &t.fields {
                let is_unique = f.constraints.as_ref().is_some_and(|c| c.unique);
                let was_unique = old_table
                    .and_then(|ot| ot.fields.iter().find(|of| of.name == f.name))
                    .and_then(|of| of.constraints.as_ref())
                    .map(|c| c.unique)
                    .unwrap_or(false);
                if is_unique && !was_unique {
                    let dupes: Result<(i64, Vec<Value>), _> = repo.with_client({
                        let (slug, tid, fname) = (manifest.slug.clone(), t.id.clone(), f.name.clone());
                        move |c| {
                            let rows = c.query(
                                "SELECT data->>$1 AS val, count(*) AS cnt FROM dataset_records WHERE app_slug = $2 AND table_id = $3 AND deleted_at IS NULL AND data->>$1 IS NOT NULL GROUP BY data->>$1 HAVING count(*) > 1 LIMIT 50",
                                &[&fname, &slug, &tid],
                            )?;
                            let mut count = 0;
                            let mut examples = Vec::new();
                            for r in rows {
                                let val: Option<String> = r.get(0);
                                let cnt: i64 = r.get(1);
                                count += cnt;
                                examples.push(json!({ "value": val, "count": cnt }));
                            }
                            Ok((count, examples))
                        }
                    });
                    if let Ok((cnt, examples)) = dupes {
                        if cnt > 0 {
                            loss.push(format!("field '{}' of table '{}' has {} duplicate values and cannot be made unique", f.name, t.name, cnt));
                            previews.push(json!({
                                "table_id": t.id,
                                "field": f.name,
                                "duplicates": cnt,
                                "examples": examples,
                            }));
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

    // page_dependencies: a new or changed bundle names its libraries, and each license is approved.
    let mut dependency_problems = Vec::new();
    for page in &manifest.pages {
        let changed = live.and_then(|l| l.pages.iter().find(|x| x.id == page.id)).is_none_or(|x| x != page);
        if !changed {
            continue;
        }
        if let PageKind::Custom { format: scaffoldry_engine::PageFormat::Bundle, dependencies, .. } = &page.kind {
            if dependencies.is_empty() {
                dependency_problems.push(format!("page '{}' is a bundle and declares no libraries", page.title));
            }
            for d in dependencies {
                if !license_approved(&d.license) {
                    dependency_problems.push(format!("page '{}': the library '{}' {} has the license '{}', which is not on the approved list", page.title, d.name, d.version, d.license));
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

    let coordinates_widened = match live {
        Some(l) => crate::service::coordinates::widened(state, ws, l, &manifest)?,
        None => vec![],
    };
    // A field that shows more than its category's default, and did not already, needs a second person.
    let categories = state.sensitivity_categories();
    let mask_weakened: Vec<String> = manifest
        .tables
        .iter()
        .flat_map(|t| t.fields.iter().map(move |f| (t, f)))
        .filter_map(|(t, f)| {
            let mask = f.mask?;
            let was = live.and_then(|l| l.tables.iter().find(|x| x.id == t.id)).and_then(|x| x.fields.iter().find(|x| x.name == f.name)).and_then(|x| x.mask);
            let default = scaffoldry_engine::default_mask(f, &categories);
            (was != Some(mask) && mask.weaker_than(default)).then(|| format!("table '{}' field '{}': the mask shows more than its category's default. {} The default: {}", t.id, f.name, mask.sentence(), default.sentence()))
        })
        .collect();
    let checks = vec![
        check("schema", vec![]),
        check("sensitivity", sensitivity_problems),
        data_loss,
        check("page_source", vec![]),
        check("page_sensitivity", page_problems),
        check("page_dependencies", dependency_problems),
        warn("relationships", scaffoldry_engine::manifest_warnings(&manifest)),
        warn("access_rules", access_problems),
        check("access_loosened", loosened),
        check("coordinates_widened", coordinates_widened),
        check("mask_weakened", mask_weakened),
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
                let ctx = crate::service::approvers::context(state, slug, "", Some(&caller.eppn), &[]);
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

/// The source text of each custom page of a manifest, keyed by page id. `local` holds sources that came with the
/// proposal and are not stored yet. The others are read from the store.
fn page_texts(manifest: Option<&Value>, local: &[(String, String)], state: &SharedState) -> Result<BTreeMap<String, String>, ServiceError> {
    let mut out = BTreeMap::new();
    let Some(pages) = manifest.and_then(|m| m.get("pages")).and_then(Value::as_array) else { return Ok(out) };
    for page in pages {
        let (Some(id), Some(sha)) = (page["id"].as_str(), page["kind"]["Custom"]["source_sha256"].as_str()) else { continue };
        let sha = sha.to_lowercase();
        let text = match local.iter().find(|(h, _)| *h == sha) {
            Some((_, text)) => text.clone(),
            None => repo(state)?
                .page_source(&sha)
                .map_err(internal)?
                .ok_or_else(|| ServiceError::Internal(format!("The source {sha} of page '{id}' is not stored")))?,
        };
        out.insert(id.to_string(), text);
    }
    Ok(out)
}

/// The digest of a proposal's content.
fn digest_of_content(manifest: Option<&Value>, rules: Option<&Value>, local: &[(String, String)], state: &SharedState) -> Result<String, ServiceError> {
    Ok(crate::repository_proposals::proposal_digest(manifest, &page_texts(manifest, local, state)?, rules))
}

/// The digest of a stored proposal. A proposal made before digests were stored has none, and its content cannot have
/// changed, so it is worked out from the content.
pub(crate) fn digest_of(p: &ProposalRow, state: &SharedState) -> Result<String, ServiceError> {
    if !p.digest.is_empty() {
        return Ok(p.digest.clone());
    }
    digest_of_content(p.manifest.as_ref(), p.rules.as_ref(), &[], state)
}

pub fn propose(caller: &AuthUser, input: ProposeInput, state: &SharedState) -> Result<Value, ServiceError> {
    propose_with(caller, input, false, state)
}

/// Runs every check `propose` runs and says what the change would need, without storing anything.
pub fn dry_run(caller: &AuthUser, input: ProposeInput, state: &SharedState) -> Result<Value, ServiceError> {
    propose_with(caller, input, true, state)
}

fn propose_with(caller: &AuthUser, input: ProposeInput, dry_run: bool, state: &SharedState) -> Result<Value, ServiceError> {
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
    let digest = digest_of_content(manifest_json.as_ref(), rules_json.as_ref(), &sources, state)?;
    let mut stored_checks = json!({ "checks": checks, "changes": changes });
    // What the builder is told: the tier, the path, who can give it, and why. Decided again when the proposal is.
    let need = crate::service::authority::requirement_for(state, &ws.id, &stored_checks, &caller.eppn)?;
    stored_checks["authority"] = need.to_json(state);
    if dry_run {
        return Ok(json!({
            "dry_run": true,
            "proposal_id": Value::Null,
            "workspace_id": ws.id,
            "app_slug": slug,
            "summary": input.summary,
            "checks": stored_checks["checks"],
            "changes": stored_checks["changes"],
            "authority": stored_checks["authority"],
        }));
    }
    let row = ProposalRow {
        id: uuid::Uuid::new_v4(),
        workspace_id: ws.id.clone(),
        app_slug: slug.clone(),
        base_version,
        manifest: manifest_json,
        checks: stored_checks,
        summary: input.summary,
        status: "Pending".to_string(),
        proposed_by: caller.eppn.clone(),
        decided_by: None,
        created_at: String::new(),
        decided_at: None,
        rules: rules_json,
        digest,
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
        "digest": digest_of(p, state)?,
        "checks": checks,
        "changes": p.checks["changes"],
        "authority": authority_now(p, state),
    }))
}

/// What the proposal needs from the tables as they are now while it is pending, and what was recorded once it is not.
fn authority_now(p: &ProposalRow, state: &SharedState) -> Value {
    if p.status == "Pending" {
        if let Ok(need) = crate::service::authority::requirement_for(state, &p.workspace_id, &p.checks, &p.proposed_by) {
            return need.to_json(state);
        }
    }
    p.checks["authority"].clone()
}

pub fn list(workspace_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let rows = repo(state)?.list_proposals(workspace_id, 100).map_err(internal)?;
    let views: Result<Vec<Value>, ServiceError> = rows.iter().map(|p| view(p, state)).collect();
    Ok(Value::Array(views?))
}

/// Pending proposals as callers see them. `unassigned` keeps those whose approver path resolves to nobody today.
pub fn list_pending(unassigned: bool, state: &SharedState) -> Result<Vec<Value>, ServiceError> {
    let rows = if unassigned { crate::service::authority::unassigned(state)? } else { repo(state)?.pending_proposals(500).map_err(internal)? };
    rows.iter().map(|p| view(p, state)).collect()
}

pub fn get(proposal_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let row = repo(state)?.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    view(&row, state)
}

/// `digest` is what the decider reviewed. An approval must name it. A different one means the content is not what was
/// reviewed, so nothing is applied.
pub fn decide(caller: &AuthUser, proposal_id: &str, decision: &str, rationale: &str, digest: Option<&str>, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let repo = repo(state)?;
    let row = repo.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    // What is required is worked out from the tables as they are now, not as they were when the proposal was made.
    let need = if matches!(decision, "approve" | "reject") { Some(crate::service::authority::requirement_for(state, &row.workspace_id, &row.checks, &row.proposed_by)?) } else { None };
    let approver_path = need.as_ref().is_some_and(|n| n.requirement.path == scaffoldry_core::authority::Path::Approver);
    let is_resolved = need.as_ref().is_some_and(|n| n.resolved.iter().any(|a| a.eppn.eq_ignore_ascii_case(&caller.eppn)));
    // A proposer may withdraw their own proposal. A person the approver spec names needs no role in the workspace.
    let withdrawing = decision == "reject" && row.proposed_by.eq_ignore_ascii_case(&caller.eppn);
    if !(withdrawing || approver_path && is_resolved) {
        authorize_workspace(caller, &row.workspace_id, "manage_workspace", state)?;
    }
    if let Some(need) = &need {
        if approver_path && !withdrawing && !is_resolved {
            return Err(ServiceError::forbidden(if need.no_approver {
                "Nobody can decide this change yet: the approver it needs resolves to no one. It stays pending until a person holds that position".to_string()
            } else {
                format!("This change needs an approver other than the proposer. Who can decide: {}", need.resolved.iter().map(|a| a.eppn.as_str()).collect::<Vec<_>>().join(", "))
            }));
        }
        if decision == "approve" && need.requirement.path != scaffoldry_core::authority::Path::Direct && row.proposed_by.eq_ignore_ascii_case(&caller.eppn) {
            return Err(ServiceError::forbidden("A change with a flagged check needs a second person to approve it. The proposer cannot"));
        }
    }
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
            let stored = digest_of(&row, state)?;
            match digest {
                None => return Err(ServiceError::BadRequest("Argument 'digest' is required to approve. Send the digest of the proposal you reviewed".to_string())),
                Some(named) if !named.trim().eq_ignore_ascii_case(&stored) => {
                    return Err(ServiceError::ProposalChanged(format!(
                        "The proposal is not what you reviewed. You named the digest {named}, and its digest is {stored}. Nothing was applied. Read the proposal again and approve its current digest"
                    )))
                }
                Some(_) => {}
            }
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
            let other_person = need.as_ref().is_some_and(|n| n.requirement.path != scaffoldry_core::authority::Path::Direct);
            match repo.approve_proposal(id, &caller.eppn, rationale, &stored, other_person).map_err(|e| match e {
                // The approval would start more background work than the person may have. Nothing was kept.
                crate::repository::RepositoryError::Quota(message) => ServiceError::QuotaExceeded(message),
                other => ServiceError::Internal(format!("The approval was not kept: {other}")),
            })? {
                Approval::Approved(_) => {
                    // The approval committed the manifest, its rules and the process definitions, and bumped their
                    // counters. This instance reads them back as every other instance will, by their stored versions.
                    crate::service::reload_after_write(state)?;
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
                Approval::ApproverRequired { resolved } => {
                    return Err(ServiceError::forbidden(format!("This change needs an approver other than the proposer. Who can decide: {}", resolved.join(", "))));
                }
            }
        }
        _ => return Err(ServiceError::BadRequest("Argument 'decision' must be 'approve' or 'reject'".to_string())),
    }
    // A trial copy lasts while its proposal waits.
    crate::service::trials::destroy_for_proposal(state, id);
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
    // Applied in the same request only when what the change needs is its owner's say.
    let direct = proposed["authority"]["path"] == "direct";
    if any_fail || !owner_or_admin || !direct {
        return Ok(Direct::Pending(proposed));
    }
    let id = proposed["proposal_id"].as_str().unwrap_or_default().to_string();
    // The direct path reviewed the content it just made, so it names that content's digest itself.
    let digest = proposed["digest"].as_str().unwrap_or_default().to_string();
    decide(caller, &id, "approve", "Applied directly by an owner or admin: no check was raised", Some(&digest), state)?;
    Ok(Direct::Applied)
}

#[cfg(test)]
mod license_tests {
    use super::*;

    #[test]
    fn the_license_list_is_the_one_the_audit_script_uses() {
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../governance/scripts/audit-licenses.py")).expect("the audit script");
        let block = script.split("APPROVED_LICENSES = {").nth(1).and_then(|b| b.split('}').next()).expect("the list in the script");
        let mut in_script: Vec<&str> = block.split('"').skip(1).step_by(2).collect();
        let mut here: Vec<&str> = APPROVED_LICENSES.to_vec();
        in_script.sort();
        here.sort();
        assert_eq!(in_script, here);
    }

    #[test]
    fn a_choice_of_licenses_passes_when_one_is_approved_and_a_conjunction_is_not_read() {
        assert!(license_approved("MIT"));
        assert!(license_approved("(MIT OR GPL-3.0)"));
        assert!(license_approved("Apache-2.0"));
        assert!(!license_approved("AGPL-3.0"));
        assert!(!license_approved("MIT AND GPL-3.0"));
        assert!(!license_approved(""));
    }
}
