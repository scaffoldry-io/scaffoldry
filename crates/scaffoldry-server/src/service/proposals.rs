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
    /// Sentences the caller adds to the change list, for what the proposal cannot carry itself, such as a webhook that
    /// came with a bundle and is off until it is approved on its own.
    pub notes: Vec<String>,
}

/// Content a person already reviewed and approved, such as a published template. The checks that ask "did this change?"
/// compare the proposal with it when the app is not live, so they are raised for what differs from it and no more.
pub struct Reviewed {
    pub manifest: Value,
    pub rules: Vec<Value>,
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
pub(crate) fn sensitive_fields(m: &AppManifest, state: &SharedState) -> BTreeSet<(String, String)> {
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
pub(crate) fn grant_sentences(m: &AppManifest, page_title: &str, kind: &PageKind) -> Vec<String> {
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
                        None if l.retired_fields.iter().any(|x| x.name == f.name) => {
                            out.push(format!("Field \"{}\" of table \"{}\" is restored, with the values it kept.", f.name, t.name))
                        }
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
                        out.push(format!("Field \"{}\" of table \"{}\" is retired. Its values are kept.", x.name, t.name));
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

pub(crate) fn live_version(slug: &str, state: &SharedState) -> Result<Option<i32>, ServiceError> {
    repo(state)?.app_manifest_version(slug).map_err(internal)
}

fn records_exist(slug: &str, state: &SharedState) -> bool {
    { let r = &state.repository; r.count_records_by_app_slugs(&[slug.to_string()]).map(|m| m.get(slug).copied().unwrap_or(0) > 0).unwrap_or(false) }
}

/// The caller's role in the workspace, lowercase. A Platform Admin counts as an admin.
pub(crate) fn role_in(caller: &AuthUser, ws_id: &str, state: &SharedState) -> Option<String> {
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

/// What the `data_loss` check says of a retired field (lifecycle phase 2).
pub const RETIRED_KEPT: &str = "The values stay for 90 days and can be restored.";

/// Lifecycle phase 2. A field the live app has and this manifest leaves out is retired, not deleted: it moves to its
/// table's `retired_fields`, stamped `at`, and its values stay in the records. A field retired before stays retired,
/// with its first stamp, unless the manifest puts it back in `fields`, which restores it. Only the live app says what is
/// retired, so a proposal cannot add an entry of its own. A removed table is not covered and is reported as before.
pub(crate) fn retire_removed(live: Option<&AppManifest>, manifest: &mut AppManifest, at: &str) {
    for t in &mut manifest.tables {
        t.retired_fields.clear();
        for f in &mut t.fields {
            f.retired_at = None;
        }
        let Some(lt) = live.and_then(|l| l.tables.iter().find(|x| x.id == t.id)) else { continue };
        let gone = |name: &str, t: &scaffoldry_engine::AppTable| !t.fields.iter().any(|f| f.name == name) && !t.retired_fields.iter().any(|r| r.name == name);
        for lr in &lt.retired_fields {
            if gone(&lr.name, t) {
                t.retired_fields.push(lr.clone());
            }
        }
        for lf in &lt.fields {
            if gone(&lf.name, t) {
                let mut retired = lf.clone();
                retired.retired_at = Some(at.to_string());
                t.retired_fields.push(retired);
            }
        }
    }
}

fn check_manifest_part(
    input_manifest: Value,
    page_sources: Vec<String>,
    ws: &crate::state::WorkspaceRecord,
    live: Option<&AppManifest>,
    reviewed: Option<&AppManifest>,
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

    retire_removed(live, &mut manifest, &chrono::Utc::now().to_rfc3339());

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

    // What the checks below compare a page, an access rule or a mask with: the live app, else the reviewed content.
    let baseline = live.or(reviewed);
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
                            loss.push(format!("field '{}' of table '{}' is retired. {RETIRED_KEPT}", lf.name, lt.name));
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
            // A text field that is one end of a value join becomes a link. A job matches the text to records, and the
            // preview of that job says how many records it cannot match.
            if change.from == FieldType::Text && change.to == FieldType::Link {
                if let Some(relation) = l.relationships.iter().find(|r| r.source_table_id == change.table_id && r.source_field == change.field) {
                    match repo.latest_relation_preview(&manifest.slug, &relation.id).map_err(internal)? {
                        None => loss.push(format!(
                            "{what} changes from a value join to a link and no preview was run. POST /apps/{}/relations/{}/convert-preview first",
                            manifest.slug, relation.id
                        )),
                        Some(result) => {
                            let (none, several) = (result["unmatched"].as_i64().unwrap_or(0), result["ambiguous"].as_i64().unwrap_or(0));
                            if none + several > 0 {
                                loss.push(format!("{none} values of {what} match no record and {several} match several, so they would be left blank"));
                            }
                            previews.push(result);
                        }
                    }
                    continue;
                }
            }
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
        let changed = baseline.and_then(|l| l.pages.iter().find(|x| x.id == page.id)).is_none_or(|x| x != page);
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
        let changed = baseline.and_then(|l| l.pages.iter().find(|x| x.id == page.id)).is_none_or(|x| x != page);
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
    if let Some(l) = baseline {
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
            let was = baseline.and_then(|l| l.tables.iter().find(|x| x.id == t.id)).and_then(|x| x.fields.iter().find(|x| x.name == f.name)).and_then(|x| x.mask);
            let default = scaffoldry_engine::default_mask(f, &categories);
            (was != Some(mask) && mask.weaker_than(default)).then(|| format!("table '{}' field '{}': the mask shows more than its category's default. {} The default: {}", t.id, f.name, mask.sentence(), default.sentence()))
        })
        .collect();
    // Integrations phase 4: each webhook's receiver is on the allowlist and its secret exists, now.
    let (webhook_host, webhook_sensitive, webhook_scope) = webhook_checks(&manifest, baseline, state);
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
        check("webhook_host", webhook_host),
        check("webhook_sensitive", webhook_sensitive),
        warn("webhook_scope", webhook_scope),
    ];
    Ok(ManifestPart { manifest, checks, sources: provided.into_iter().collect() })
}

/// Integrations phase 4: what a manifest's webhooks raise.
///
/// `webhook_host` fails when a webhook's address is not https, its host and port are not on `egress.allow` now, its
/// secret does not exist, or it names a table or field the app does not have. `webhook_sensitive` is raised when a
/// webhook new or changed since the reviewed version sends the value of an effectively sensitive field, and needs a
/// second person. `webhook_scope` warns when a webhook sends values with no filter, so every record's values leave.
fn webhook_checks(manifest: &AppManifest, baseline: Option<&AppManifest>, state: &SharedState) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut host, mut sensitive, mut scope) = (Vec::new(), Vec::new(), Vec::new());
    let hooks = &manifest.integrations.webhooks;
    if hooks.is_empty() {
        return (host, sensitive, scope);
    }
    let (allow, allow_http) = {
        let settings = state.settings.read().unwrap_or_else(|p| p.into_inner());
        (crate::net::egress::Allow::from_settings(&settings), settings.get("webhooks.allow_http").and_then(Value::as_bool).unwrap_or(false))
    };
    let secrets: BTreeSet<String> = crate::secrets::list(&state.repository).map(|l| l.into_iter().map(|s| s.name).collect()).unwrap_or_default();
    let flagged = sensitive_fields(manifest, state);
    let mut ids = BTreeSet::new();
    for h in hooks {
        if !ids.insert(h.id.clone()) {
            host.push(format!("two webhooks have the id '{}'", h.id));
        }
        let (scheme, rest) = h.url.split_once("://").unwrap_or(("", ""));
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let (name, port) = match authority.rsplit_once(':') {
            Some((n, p)) => (n.to_string(), p.parse::<u16>().ok()),
            None => (authority.to_string(), Some(if scheme == "http" { 80 } else { 443 })),
        };
        if !(scheme == "https" || (allow_http && scheme == "http")) {
            host.push(format!("webhook '{}': its address {} is not https", h.id, h.url));
        } else if name.is_empty() || port.is_none() {
            host.push(format!("webhook '{}': {} names no host and port", h.id, h.url));
        } else if !allow.lists_host(&name, port.unwrap_or(0)) {
            host.push(format!("webhook '{}': the host {name} and port {} are not on egress.allow", h.id, port.unwrap_or(0)));
        }
        if !secrets.contains(&h.secret) {
            host.push(format!("webhook '{}': the secret {} does not exist", h.id, h.secret));
        }
        if h.events.is_empty() || h.tables.is_empty() {
            host.push(format!("webhook '{}': it names no event or no table", h.id));
        }
        for t in &h.tables {
            match manifest.tables.iter().find(|x| &x.id == t) {
                None => host.push(format!("webhook '{}': the app has no table '{t}'", h.id)),
                Some(table) => {
                    for f in h.fields.iter().filter(|f| !table.fields.iter().any(|x| &x.name == *f)) {
                        host.push(format!("webhook '{}': the table '{t}' has no field '{f}'", h.id));
                    }
                }
            }
        }
        if h.include_values {
            if h.filter.is_none() {
                scope.push(format!("webhook '{}' sends the values of {} for every record, with no filter", h.id, h.fields.join(", ")));
            }
            let reviewed = baseline.and_then(|b| b.integrations.webhooks.iter().find(|x| x.id == h.id)).map(|x| serde_json::to_value(x).unwrap_or_default());
            if reviewed != Some(serde_json::to_value(h).unwrap_or_default()) {
                for f in &h.fields {
                    if h.tables.iter().any(|t| flagged.contains(&(t.clone(), f.clone()))) {
                        sensitive.push(format!("webhook '{}' sends the sensitive field '{f}' to {name}", h.id));
                    }
                }
            }
        }
    }
    (host, sensitive, scope)
}

/// The checks on a rules part. A rule that fails `rule_schema` stores no proposal.
pub(crate) struct RulesPart {
    pub(crate) rules: Vec<(AutomationRule, u32)>,
    pub(crate) checks: Vec<Value>,
}

pub(crate) fn check_rules_part(
    caller: &AuthUser,
    raw: Vec<Value>,
    slug: &str,
    effective: &AppManifest,
    reviewed_rules: &[Value],
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
        // The grants are derived here, so the proposal a reviewer approves holds them, and its digest covers them.
        crate::service::flows::derive(state, effective, &mut rule);
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

    let mut task_problems = Vec::new();
    let mut loops = Vec::new();
    let mut approvers = Vec::new();
    // A rule equal to one that was reviewed has had its effects reviewed.
    let reviewed_rules: Vec<AutomationRule> = reviewed_rules
        .iter()
        .filter_map(|v| serde_json::from_value::<AutomationRule>(v.clone()).ok())
        .map(|mut r| {
            r.app_slug = slug.to_string();
            r.version = 0;
            r
        })
        .collect();
    for (rule, _) in &rules {
        let set = crate::service::processes::fields_set(rule);
        let already = reviewed_rules.contains(rule);
        for name in set.iter().filter(|_| !already) {
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
        // Workflows phase 4: a task that shows or asks for a sensitive field opens it to its assignee.
        for step in scaffoldry_core::workflow::all_steps(&rule.steps).into_iter().filter(|_| !already) {
            if let StepKind::UserTask { show, fill, .. } = &step.kind {
                for name in show.iter().chain(fill.iter()) {
                    if effective.tables.iter().any(|t| sensitive.contains(&(t.id.clone(), name.clone()))) {
                        task_problems.push(format!("rule '{}', step '{}': the task shows the sensitive field '{name}' to its assignee", rule.name, step.id));
                    }
                }
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
    let checks = vec![check("rule_schema", vec![]), check("rule_effects", effects), check("task_sensitivity", task_problems), warn("rule_loop", loops), warn("rule_approver", approvers)];
    Ok(RulesPart { rules, checks })
}

/// What a flow may read and change, as the reviewer approves it (workflows phase 1). Nothing for a rule that is not a flow.
fn grant_sentence(rule: &AutomationRule, state: &SharedState) -> Option<String> {
    crate::service::flows::is_flow(rule)
        .then(|| scaffoldry_engine::flow::grants_sentence(&rule.grants, &rule.app_slug, &crate::service::flows::manifests_for(state, rule)))
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
            out.extend(grant_sentence(proposed, state));
        }
        Some(old) => {
            // Compare the rule without its version and its switch, which have their own sentences.
            let (mut a, mut b) = (old.clone(), proposed.clone());
            (a.version, b.version) = (0, 0);
            (a.enabled, b.enabled) = (true, true);
            if a != b {
                out.push(format!("Rule \"{}\" is changed.", proposed.name));
                out.extend(describe_rule(proposed));
                out.extend(grant_sentence(proposed, state));
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

/// What differs between a proposed manifest and the reviewed one, in sentences. Views, the slug and the workspace are
/// left out: a new app always has its own.
fn differences(proposed: &AppManifest, reviewed: &AppManifest) -> Vec<String> {
    fn same<T: serde::Serialize>(a: &T, b: &T) -> bool {
        serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
    }
    let mut out = Vec::new();
    if proposed.title != reviewed.title {
        out.push(format!("the title is \"{}\", the template's is \"{}\"", proposed.title, reviewed.title));
    }
    for t in &proposed.tables {
        let Some(r) = reviewed.tables.iter().find(|x| x.id == t.id) else {
            out.push(format!("table '{}' is not in the template", t.id));
            continue;
        };
        if t.name != r.name {
            out.push(format!("table '{}' is named differently from the template's", t.id));
        }
        for f in &t.fields {
            match r.fields.iter().find(|x| x.name == f.name) {
                None => out.push(format!("field '{}' of table '{}' is not in the template", f.name, t.id)),
                Some(x) if !same(x, f) => out.push(format!("field '{}' of table '{}' differs from the template", f.name, t.id)),
                _ => {}
            }
        }
        for x in &r.fields {
            if !t.fields.iter().any(|f| f.name == x.name) {
                out.push(format!("field '{}' of table '{}' is in the template and not here", x.name, t.id));
            }
        }
    }
    for r in &reviewed.tables {
        if !proposed.tables.iter().any(|t| t.id == r.id) {
            out.push(format!("table '{}' is in the template and not here", r.id));
        }
    }
    for p in &proposed.pages {
        match reviewed.pages.iter().find(|x| x.id == p.id) {
            None => out.push(format!("page '{}' is not in the template", p.id)),
            Some(x) if !same(x, p) => out.push(format!("page '{}' differs from the template", p.id)),
            _ => {}
        }
    }
    for x in &reviewed.pages {
        if !proposed.pages.iter().any(|p| p.id == x.id) {
            out.push(format!("page '{}' is in the template and not here", x.id));
        }
    }
    if !same(&proposed.access, &reviewed.access) {
        out.push("the access rules differ from the template's".to_string());
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
    let (ws, app) = (input.workspace_id.clone(), proposal_app(&input));
    let made = propose_with(caller, input, false, None, state)?;
    tell_pending(caller, &ws, app, &made, state);
    Ok(made)
}

/// The app a proposal is about: the one it names, or the one its manifest describes.
fn proposal_app(input: &ProposeInput) -> Option<String> {
    input.app_slug.clone().or_else(|| input.manifest.as_ref().and_then(|m| m.get("slug")).and_then(Value::as_str).map(str::to_string))
}

/// Notifications phase 4: a proposal that waits tells the workspace's owners and admins other than the proposer.
fn tell_pending(caller: &AuthUser, workspace: &str, app: Option<String>, made: &Value, state: &SharedState) {
    let pending = made["status"].as_str().map(|s| s.eq_ignore_ascii_case("pending")).unwrap_or(false);
    let (Some(app), true) = (app, pending) else { return };
    let notice = crate::notifications::Notice::ProposalPending { by: crate::notifications::PersonRef(caller.eppn.clone()), app: crate::notifications::AppRef(app) };
    let to = crate::notifications::Recipients::Workspace { workspace: workspace.to_string(), roles: vec!["owner".into(), "admin".into()] };
    let _ = crate::notifications::notify(state, &to, Some(&caller.eppn), &notice);
}

/// A proposal made from content that was reviewed already. What matches it is not raised again. What differs is.
pub fn propose_reviewed(caller: &AuthUser, input: ProposeInput, reviewed: Reviewed, state: &SharedState) -> Result<Value, ServiceError> {
    let (ws, app) = (input.workspace_id.clone(), proposal_app(&input));
    let made = propose_with(caller, input, false, Some(reviewed), state)?;
    tell_pending(caller, &ws, app, &made, state);
    Ok(made)
}

/// Runs every check `propose` runs and says what the change would need, without storing anything.
pub fn dry_run(caller: &AuthUser, input: ProposeInput, state: &SharedState) -> Result<Value, ServiceError> {
    propose_with(caller, input, true, None, state)
}

fn propose_with(caller: &AuthUser, input: ProposeInput, dry_run: bool, reviewed: Option<Reviewed>, state: &SharedState) -> Result<Value, ServiceError> {
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
    if !dry_run {
        crate::service::access::archived_refusal(&slug, Some(&ws.id), state)?;
    }
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

    let reviewed_manifest: Option<AppManifest> = match &reviewed {
        Some(r) => Some(serde_json::from_value(r.manifest.clone()).map_err(|e| ServiceError::Internal(format!("The reviewed manifest is not a manifest: {e}")))?),
        None => None,
    };
    if let Some(raw) = input.manifest {
        let part = check_manifest_part(raw, input.page_sources, &ws, live.as_ref(), reviewed_manifest.as_ref(), state)?;
        if let Some(r) = &reviewed_manifest {
            checks.push(warn("template_pristine", differences(&part.manifest, r)));
        }
        changes.extend(change_list(&part.manifest, live.as_ref(), state));
        checks.extend(part.checks);
        sources = part.sources;
        manifest_json = Some(serde_json::to_value(&part.manifest).map_err(internal)?);
        effective = Some(part.manifest);
    }

    let mut rules_json: Option<Value> = None;
    let mut flow_plan: Option<crate::service::flow_parties::Plan> = None;
    let mut proposed_rules: Vec<AutomationRule> = Vec::new();
    if !input.rules.is_empty() {
        let effective = effective.as_ref().ok_or_else(|| ServiceError::NotFound(format!("App '{slug}' not found")))?;
        let part = check_rules_part(caller, input.rules, &slug, effective, reviewed.as_ref().map_or(&[][..], |r| &r.rules), state)?;
        for (rule, _) in &part.rules {
            changes.extend(rule_sentences(rule, state));
        }
        checks.extend(part.checks);
        // Workflows phase 2: what the flows touch, and which workspaces must approve.
        let rules: Vec<AutomationRule> = part.rules.iter().map(|(r, _)| r.clone()).collect();
        proposed_rules = rules.clone();
        if let Some(plan) = crate::service::flow_parties::plan(state, &ws.id, effective, &rules)? {
            checks.extend(plan.checks.clone());
            flow_plan = Some(plan);
        }
        rules_json = Some(Value::Array(
            part.rules.iter().map(|(r, base)| json!({ "rule": r, "base_version": base })).collect(),
        ));
    }
    // Field types phase 5: a button names a rule of its own app with a Manual trigger, proposed here or live.
    if let Some(m) = effective.as_ref() {
        let problems = button_problems(m, &proposed_rules, state);
        if !problems.is_empty() {
            return Err(ServiceError::BadRequest(format!("button_rule: {}", problems.join("; "))));
        }
    }
    checks.push(check("stale", vec![]));
    changes.extend(input.notes);

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
        parties: flow_plan.as_ref().filter(|p| p.parties.len() > 1).map(|p| p.to_json()),
    };
    // Workflows phase 5: each party's owners and admins review it, and the proposer waits, as work items.
    let mut parties = crate::service::flow_parties::parties_of(&row.parties);
    if parties.is_empty() {
        parties.push(ws.id.clone());
    }
    let work = crate::service::work::proposal_items(state, &parties, &slug, &caller.eppn, row.id);
    repo.insert_proposal(&row, &sources, &work).map_err(internal)?;
    // Each other party's owners and admins are told that a proposal needs them.
    for party in crate::service::flow_parties::parties_of(&row.parties).iter().filter(|w| **w != ws.id) {
        let notice = crate::notifications::Notice::ProposalPending { by: crate::notifications::PersonRef(caller.eppn.clone()), app: crate::notifications::AppRef(slug.clone()) };
        let to = crate::notifications::Recipients::Workspace { workspace: party.clone(), roles: vec!["owner".into(), "admin".into()] };
        let _ = crate::notifications::notify(state, &to, Some(&caller.eppn), &notice);
    }
    let stored = repo.get_proposal(row.id).map_err(internal)?.ok_or_else(|| ServiceError::Internal("The proposal was not stored".to_string()))?;
    view(&stored, state)
}

/// What is wrong with the buttons of an app: a button that names no rule, a rule the app does not have, or a rule that
/// does not start by hand.
fn button_problems(m: &AppManifest, proposed: &[AutomationRule], state: &SharedState) -> Vec<String> {
    let live = state.automations.read().map(|a| a.get(&m.slug).cloned().unwrap_or_default()).unwrap_or_default();
    let mut problems = Vec::new();
    for f in m.tables.iter().flat_map(|t| t.fields.iter()).filter(|f| f.field_type == scaffoldry_engine::FieldType::Button) {
        let Some(id) = f.button_rule.as_deref() else {
            problems.push(format!("the button '{}' names no rule", f.name));
            continue;
        };
        match proposed.iter().chain(live.iter()).find(|r| r.id == id) {
            None => problems.push(format!("the button '{}' names the rule '{id}', which this app does not have", f.name)),
            Some(r) if r.trigger != scaffoldry_core::TriggerEvent::Manual => {
                problems.push(format!("the button '{}' names the rule '{id}', which does not have a Manual trigger. A button starts a rule that starts by hand", f.name))
            }
            Some(_) => {}
        }
    }
    problems
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
            for (slug, version) in crate::service::flow_parties::touched_of(&p.parties) {
                let live = live_version(&slug, state)?;
                if live != version {
                    messages.push(format!("the app '{slug}' that the flow touches is at version {live:?}, this proposal was made against {version:?}"));
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
        "parties": parties_view(p, state)?,
    }))
}

/// Each party of a flow across workspaces, with its decision so far. Empty for a proposal with one party.
fn parties_view(p: &ProposalRow, state: &SharedState) -> Result<Value, ServiceError> {
    let parties = crate::service::flow_parties::parties_of(&p.parties);
    if parties.is_empty() {
        return Ok(json!([]));
    }
    let decided = repo(state)?.party_decisions(p.id).map_err(internal)?;
    let sources = crate::service::flow_parties::sources_of(&p.parties);
    Ok(Value::Array(
        parties
            .iter()
            .map(|ws| {
                let d = decided.iter().find(|(w, ..)| w == ws);
                json!({
                    "workspace_id": ws,
                    "source": sources.contains(ws),
                    "decision": d.map(|(_, _, decision, _)| decision.clone()),
                    "decided_by": d.map(|(_, by, _, _)| by.clone()),
                    "at": d.map(|(_, _, _, at)| at.clone()),
                })
            })
            .collect(),
    ))
}

/// The proposal as `caller` may see it. An owner or admin of the home workspace sees all of it. An owner or admin of
/// another party sees the change list for their own workspace only.
pub fn get_for(caller: &AuthUser, proposal_id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let row = repo(state)?.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    let home = authorize_workspace(caller, &row.workspace_id, "manage_workspace", state);
    if home.is_ok() {
        return view(&row, state);
    }
    let party = crate::service::flow_parties::parties_of(&row.parties).into_iter().find(|ws| authorize_workspace(caller, ws, "manage_workspace", state).is_ok());
    match party {
        Some(ws) => {
            let mut v = view(&row, state)?;
            v["changes"] = row.parties.as_ref().map(|p| p["changes"][&ws].clone()).unwrap_or_else(|| json!([]));
            v["party"] = json!(ws);
            Ok(v)
        }
        None => home.map(|_| Value::Null),
    }
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
    decide_as(caller, proposal_id, decision, rationale, digest, None, state)
}

/// Decides a proposal. For a flow that touches several workspaces, `party` names the workspace the caller decides for.
/// Without it, the caller decides for the first undecided party they own or administer.
pub fn decide_as(caller: &AuthUser, proposal_id: &str, decision: &str, rationale: &str, digest: Option<&str>, party: Option<&str>, state: &SharedState) -> Result<Value, ServiceError> {
    let id = parse_id(proposal_id)?;
    let repo = repo(state)?;
    let row = repo.get_proposal(id).map_err(internal)?.ok_or_else(|| ServiceError::NotFound("Proposal not found".to_string()))?;
    // Workflows phase 2: a flow across workspaces is decided party by party. The last party's approval goes on below,
    // through the same requirement, snapshot hold and transaction as any approval, carrying every party with it.
    let mut flow_final: Option<crate::repository_proposals::FlowFinal> = None;
    if !crate::service::flow_parties::parties_of(&row.parties).is_empty() {
        match decide_party(caller, &row, decision, rationale, digest, party, state)? {
            PartyStep::Last(flow) => flow_final = Some(flow),
            PartyStep::Recorded => {
                if decision == "reject" {
                    tell_decided(caller, &row, decision, state);
                }
                return get_for(caller, proposal_id, state);
            }
        }
    }
    // For a flow across workspaces the home party's own approver is the one the requirement is asked about.
    let home_approver = flow_final.as_ref().and_then(|f| f.approvals.iter().find(|(w, _)| *w == row.workspace_id).map(|(_, by)| by.clone()));
    let deciding = home_approver.clone().unwrap_or_else(|| caller.eppn.clone());
    if row.status == "Approving" {
        return Err(ServiceError::Conflict("This proposal is waiting for its snapshot. It is approved when the snapshot is done".to_string()));
    }
    // What is required is worked out from the tables as they are now, not as they were when the proposal was made.
    let need = if matches!(decision, "approve" | "reject") { Some(crate::service::authority::requirement_for(state, &row.workspace_id, &row.checks, &row.proposed_by)?) } else { None };
    let approver_path = need.as_ref().is_some_and(|n| n.requirement.path == scaffoldry_core::authority::Path::Approver);
    let is_resolved = need.as_ref().is_some_and(|n| n.resolved.iter().any(|a| a.eppn.eq_ignore_ascii_case(&deciding)));
    // A proposer may withdraw their own proposal. A person the approver spec names needs no role in the workspace.
    let withdrawing = decision == "reject" && row.proposed_by.eq_ignore_ascii_case(&caller.eppn);
    // The last party of a flow was authorized for its own workspace above.
    if !(withdrawing || approver_path && is_resolved || flow_final.is_some()) {
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
        if decision == "approve" && need.requirement.path != scaffoldry_core::authority::Path::Direct && row.proposed_by.eq_ignore_ascii_case(&deciding) {
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
            let other_person = need.as_ref().is_some_and(|n| n.requirement.path != scaffoldry_core::authority::Path::Direct) || flow_final.is_some() && crate::repository_proposals::has_flagged_failure(&row.checks);
            // A change that loses data needs a snapshot from the last seven days. The approval waits for one.
            if crate::service::snapshots::hold_for_snapshot(caller, &row, rationale, &stored, other_person, state)? {
                return get(proposal_id, state);
            }
            match repo.approve_proposal(id, &caller.eppn, rationale, &stored, other_person, flow_final.clone()).map_err(|e| match e {
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
    tell_decided(caller, &row, decision, state);
    get(proposal_id, state)
}

/// Notifications phase 4: the proposer is told the decision. A proposer who withdrew is not told of their own act.
fn tell_decided(caller: &AuthUser, row: &ProposalRow, decision: &str, state: &SharedState) {
    let notice = crate::notifications::Notice::ProposalDecided {
        by: crate::notifications::PersonRef(caller.eppn.clone()),
        app: crate::notifications::AppRef(row.app_slug.clone()),
        approved: crate::notifications::Flag(decision == "approve"),
    };
    let _ = crate::notifications::notify(state, &crate::notifications::Recipients::Person(row.proposed_by.clone()), Some(&caller.eppn), &notice);
}

/// One party's decision on a flow that touches several workspaces (workflows phase 2). An owner or admin of the party
/// decides for it; a Platform Admin may too. When a flagged check is raised, the proposer may not approve a party the
/// flow reads from. Any party rejecting rejects the proposal. The last approval goes on through `decide_as`, which
/// publishes it in one transaction that checks every app the flow touches is still at the version it was proposed against.
fn decide_party(caller: &AuthUser, row: &ProposalRow, decision: &str, rationale: &str, digest: Option<&str>, party: Option<&str>, state: &SharedState) -> Result<PartyStep, ServiceError> {
    let repo = repo(state)?;
    if row.status != "Pending" {
        return Err(ServiceError::Conflict(format!("This proposal is already {}", row.status)));
    }
    if decision != "approve" && decision != "reject" {
        return Err(ServiceError::BadRequest("Argument 'decision' must be 'approve' or 'reject'".to_string()));
    }
    if rationale.trim().is_empty() {
        return Err(ServiceError::BadRequest("Argument 'rationale' must say why".to_string()));
    }
    let parties = crate::service::flow_parties::parties_of(&row.parties);
    let decided = repo.party_decisions(row.id).map_err(internal)?;
    let withdrawing = decision == "reject" && row.proposed_by.eq_ignore_ascii_case(&caller.eppn);
    let party = match party {
        Some(p) => p.to_string(),
        None => parties
            .iter()
            .find(|ws| !decided.iter().any(|(w, ..)| w == *ws) && authorize_workspace(caller, ws, "manage_workspace", state).is_ok())
            .cloned()
            .or_else(|| withdrawing.then(|| row.workspace_id.clone()))
            .ok_or_else(|| ServiceError::forbidden("You are not an owner or admin of any workspace this flow still needs"))?,
    };
    if !parties.contains(&party) {
        return Err(ServiceError::BadRequest(format!("The workspace '{party}' is not a party to this proposal. Its parties are {}", parties.join(", "))));
    }
    if !withdrawing {
        authorize_workspace(caller, &party, "manage_workspace", state)?;
    }
    if decision == "reject" {
        repo.record_party_decision(row.id, &party, &caller.eppn, "reject", rationale).map_err(internal)?;
        repo.reject_proposal(row.id, &caller.eppn).map_err(internal)?.map_err(|status| ServiceError::Conflict(format!("This proposal is already {status}")))?;
        return Ok(PartyStep::Recorded);
    }
    let flagged = crate::repository_proposals::has_flagged_failure(&row.checks);
    if flagged && row.proposed_by.eq_ignore_ascii_case(&caller.eppn) && crate::service::flow_parties::sources_of(&row.parties).contains(&party) {
        let others = crate::service::members::people_records(&party, state)
            .iter()
            .filter(|c| matches!(c.role.to_ascii_lowercase().as_str(), "owner" | "admin") && !c.eppn.eq_ignore_ascii_case(&row.proposed_by))
            .count();
        return Err(ServiceError::forbidden(if others == 0 {
            format!("A flagged check is raised and the flow reads from {party}, so its approver must not be the proposer. You are the only owner or admin there, so this party waits for a Platform Admin")
        } else {
            format!("A flagged check is raised and the flow reads from {party}, so another owner or admin of {party} must approve it")
        }));
    }
    let stored = digest_of(row, state)?;
    match digest {
        None => return Err(ServiceError::BadRequest("Argument 'digest' is required to approve. Send the digest of the proposal you reviewed".to_string())),
        Some(named) if !named.trim().eq_ignore_ascii_case(&stored) => {
            return Err(ServiceError::ProposalChanged(format!(
                "The proposal is not what you reviewed. You named the digest {named}, and its digest is {stored}. Nothing was applied. Read the proposal again and approve its current digest"
            )))
        }
        Some(_) => {}
    }
    if !repo.record_party_decision(row.id, &party, &caller.eppn, "approve", rationale).map_err(internal)? {
        return Err(ServiceError::Conflict(format!("The workspace '{party}' has decided this proposal already")));
    }
    let decided = repo.party_decisions(row.id).map_err(internal)?;
    let approvals: Vec<(String, String)> = parties
        .iter()
        .filter_map(|ws| decided.iter().find(|(w, _, d, _)| w == ws && d == "approve").map(|(w, by, _, _)| (w.clone(), by.clone())))
        .collect();
    if approvals.len() < parties.len() {
        // Not live until every party has approved.
        return Ok(PartyStep::Recorded);
    }
    Ok(PartyStep::Last(crate::repository_proposals::FlowFinal {
        parties: json!(approvals.iter().map(|(w, by)| json!({ "workspace_id": w, "approved_by": by })).collect::<Vec<_>>()),
        approvals,
        touched: crate::service::flow_parties::touched_of(&row.parties),
    }))
}

/// What one party's decision did.
enum PartyStep {
    /// Recorded. The proposal waits for other parties, or was rejected.
    Recorded,
    /// The last party approved: the proposal is published with these parties.
    Last(crate::repository_proposals::FlowFinal),
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
    let proposed = propose(caller, input, state)?;
    if approve_if_direct(caller, &proposed, state)? {
        Ok(Direct::Applied)
    } else {
        Ok(Direct::Pending(proposed))
    }
}

/// Approves a proposal just made, in the same request, when its caller owns or administers the workspace, nothing failed,
/// and what the change needs is the owner's say. Returns whether it did.
pub fn approve_if_direct(caller: &AuthUser, proposed: &Value, state: &SharedState) -> Result<bool, ServiceError> {
    let ws_id = proposed["workspace_id"].as_str().unwrap_or_default();
    let any_fail = proposed["checks"].as_array().is_some_and(|l| l.iter().any(|c| c["outcome"] == "fail"));
    let owner_or_admin = matches!(role_in(caller, ws_id, state).as_deref(), Some("owner" | "admin"));
    // Applied in the same request only when what the change needs is its owner's say.
    let direct = proposed["authority"]["path"] == "direct";
    if any_fail || !owner_or_admin || !direct {
        return Ok(false);
    }
    let id = proposed["proposal_id"].as_str().unwrap_or_default().to_string();
    // The direct path reviewed the content it just made, so it names that content's digest itself.
    let digest = proposed["digest"].as_str().unwrap_or_default().to_string();
    decide(caller, &id, "approve", "Applied directly by an owner or admin: no check was raised", Some(&digest), state)?;
    Ok(true)
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
