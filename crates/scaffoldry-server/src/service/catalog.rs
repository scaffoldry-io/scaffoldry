//! Data layer phase 2d: the catalog. Find what exists, and see it without seeing a value.
//!
//! One entry for each dataset, on the `datasets` table of phase 1. A dataset is an app table or a published dataset. Both
//! have a dataset id, and a published dataset keeps its own id as well, because the older routes and the connections code
//! name it that way. The entry holds metadata only: the name and description, the owning function and unit, the steward
//! position, the fields, the category labels, the freshness, an approximate size and how it is used. It never holds a
//! stored value.
//!
//! Every list, count, facet, search hit and lookup passes one gate first: whether the caller may discover the dataset. A
//! dataset the caller may not discover is skipped before anything about it is read, so it cannot reach a total, a facet or
//! an error. A Platform Admin and a compliance officer discover every dataset, because they set the policy.
//!
//! A sample is made from the schema alone. Its values come from each field's type and label, and the code that makes it is
//! never given a row. The count of a small group is masked under the threshold `catalog.small_group_min` (11 by default).
//! Search matches the words against the entry's metadata with SQL `ILIKE`. It never reads a row.

use crate::repository_catalog::{CatalogRow, PolicyChange};
use crate::service::admin::require_platform_admin_or_compliance;
use crate::service::ServiceError;
use crate::state::{AuthUser, OrganizationNode, SharedState};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::{effective_sensitivity, AppManifest, LabelSet};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// The most entries one search returns. The total says how many matched.
pub const PAGE: usize = 50;
/// The sample rows an entry shows.
const SAMPLE_ROWS: usize = 2;
/// The words a query may hold, and its length.
const MAX_WORDS: usize = 8;
const MAX_QUERY: usize = 200;
/// The setting that names the smallest group a count may show, and its default.
pub const MIN_SETTING: &str = "catalog.small_group_min";
pub const DEFAULT_MIN: i64 = 11;
/// The fields a request form may list, in the order a form shows them. A dataset's `request_fields` is a subset.
pub const REQUEST_FIELDS: [&str; 4] = ["reason", "project", "duration", "agreement_reference"];
/// What a restricted field is called.
pub const RESTRICTED: &str = "a restricted field";

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn not_found() -> ServiceError {
    ServiceError::NotFound("Dataset not found".to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// Who may discover a dataset
// ---------------------------------------------------------------------------------------------------------------------

/// The discoverability policy of a dataset. `everyone` is the default.
#[derive(Debug, Clone, PartialEq)]
pub enum Discover {
    Everyone,
    Hidden,
    /// Whoever holds a role in one of these units or below, or one of these positions.
    Reach { units: Vec<Uuid>, positions: Vec<String> },
}

impl Discover {
    /// The stored or posted form: `"everyone"`, `"hidden"`, `{"units": [...]}`, `{"positions": [...]}`, or both lists.
    /// Anything else is `None`.
    pub fn from_json(v: &Value) -> Option<Discover> {
        match v {
            Value::String(s) if s == "everyone" => Some(Discover::Everyone),
            Value::String(s) if s == "hidden" => Some(Discover::Hidden),
            Value::Object(m) if m.keys().all(|k| k == "units" || k == "positions") => {
                let list = |key: &str| -> Option<Vec<String>> {
                    match m.get(key) {
                        None => Some(Vec::new()),
                        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string)).collect(),
                        Some(_) => None,
                    }
                };
                let units: Vec<Uuid> = list("units")?.iter().map(|u| Uuid::parse_str(u).ok()).collect::<Option<_>>()?;
                let positions = list("positions")?;
                (!units.is_empty() || !positions.is_empty()).then_some(Discover::Reach { units, positions })
            }
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Discover::Everyone => json!("everyone"),
            Discover::Hidden => json!("hidden"),
            Discover::Reach { units, positions } => {
                let mut m = Map::new();
                if !units.is_empty() {
                    m.insert("units".to_string(), json!(units));
                }
                if !positions.is_empty() {
                    m.insert("positions".to_string(), json!(positions));
                }
                Value::Object(m)
            }
        }
    }
}

/// The caller, as far as discovery is concerned.
struct Who {
    manager: bool,
    units: Vec<Uuid>,
    positions: HashSet<String>,
}

impl Who {
    fn of(caller: &AuthUser, state: &SharedState) -> Who {
        let roles = state.roles_of(&caller.eppn);
        Who {
            manager: require_platform_admin_or_compliance(caller, state).is_ok(),
            units: roles.iter().map(|r| r.organization_id).collect(),
            positions: roles.iter().filter_map(|r| r.position_key.clone()).collect(),
        }
    }

    /// Whether the caller may discover a dataset with this stored policy. A policy that cannot be read hides the dataset.
    fn may(&self, stored: &Value, tree: &scaffoldry_core::tree::OrgTree) -> bool {
        if self.manager {
            return true;
        }
        match Discover::from_json(stored) {
            Some(Discover::Everyone) => true,
            Some(Discover::Reach { units, positions }) => {
                units.iter().any(|u| self.units.iter().any(|held| tree.contains(*u, *held))) || positions.iter().any(|p| self.positions.contains(p))
            }
            Some(Discover::Hidden) | None => false,
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The entry
// ---------------------------------------------------------------------------------------------------------------------

/// One field as the sample and the entry need it.
struct FieldView {
    name: String,
    label: String,
    field_type: String,
    options: Vec<String>,
}

/// An entry before its size, freshness and sample are added.
struct Base {
    name: String,
    department: String,
    unit: String,
    categories: Vec<String>,
    hay: String,
    entry: Map<String, Value>,
    fields: Vec<FieldView>,
    /// The app and the table ids whose live rows give the size. `None` for a published dataset.
    rows_of: Option<(String, Vec<String>)>,
    declared: Option<i64>,
    freshness: Option<String>,
}

struct Ctx {
    manifests: HashMap<String, AppManifest>,
    published: HashMap<String, scaffoldry_core::PublishedDataset>,
    orgs: Vec<OrganizationNode>,
    positions: HashMap<String, String>,
    labels: LabelSet,
    categories: Vec<scaffoldry_engine::sensitivity::Category>,
    min: i64,
}

impl Ctx {
    fn of(state: &SharedState) -> Result<Ctx, ServiceError> {
        // A lock that a panic poisoned is an error, not an empty catalog.
        let stuck = || ServiceError::Internal("The catalog could not be read".to_string());
        Ok(Ctx {
            manifests: state.engine.read().map_err(|_| stuck())?.list_manifests().into_iter().map(|m| (m.slug.clone(), m)).collect(),
            published: state.datasets.read().map_err(|_| stuck())?.clone(),
            orgs: state.organizations.read().map_err(|_| stuck())?.values().cloned().collect(),
            positions: state.position_types.read().map_err(|_| stuck())?.values().map(|t| (t.key.clone(), t.name.clone())).collect(),
            labels: state.labels.read().map_err(|_| stuck())?.clone(),
            categories: state.sensitivity_categories(),
            min: threshold(state),
        })
    }

    /// The unit a code or a name stands for, as the entry shows it.
    fn unit(&self, text: &str) -> Value {
        let found = self.orgs.iter().find(|o| o.code.eq_ignore_ascii_case(text)).or_else(|| self.orgs.iter().find(|o| o.name.eq_ignore_ascii_case(text)));
        match found {
            Some(o) => json!({ "id": o.id, "name": o.name, "code": o.code }),
            None => json!({ "id": null, "name": text, "code": null }),
        }
    }

    fn category_name(&self, id: &str) -> String {
        self.categories.iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string())
    }
}

/// The smallest group a count may show.
pub fn threshold(state: &SharedState) -> i64 {
    state.settings.read().ok().and_then(|s| s.get(MIN_SETTING).and_then(Value::as_i64)).filter(|n| *n >= 2).unwrap_or(DEFAULT_MIN)
}

/// Builds the part of an entry that needs no row. `None` when the dataset's source is gone.
fn build(row: &CatalogRow, ctx: &Ctx) -> Option<Base> {
    let restricted = |name: &str| row.restricted_fields.iter().any(|r| r == name);
    let mut entry = Map::new();
    let mut fields = Vec::new();
    let mut field_json = Vec::new();
    let mut categories: Vec<String> = Vec::new();
    let (id, name, description, department, organization, herm, level, kind);
    let mut rows_of = None;
    let mut declared = None;
    let mut freshness = None;
    let mut app_title = String::new();

    if let Some(pid) = &row.published_id {
        let ds = ctx.published.get(pid)?;
        id = ds.id.clone();
        name = ds.name.clone();
        description = ds.description.clone();
        department = ds.department.clone();
        organization = ds.organization.clone();
        herm = ds.herm_capability_id.clone();
        level = ds.sensitivity_level.clone();
        kind = "published";
        declared = Some(ds.record_count as i64);
        freshness = ds.published_at.get(..10).map(str::to_string);
        for f in &ds.fields {
            if restricted(&f.name) {
                field_json.push(restricted_field());
                continue;
            }
            // A published field is not a manifest field and has no label, so its stored flag is its whole sensitivity.
            // It is moved out by destructuring, so `effective_sensitivity_test` still holds every read of a manifest flag.
            let scaffoldry_core::DatasetField { ferpa_sensitive: flagged, .. } = f.clone();
            if flagged {
                categories.push(scaffoldry_engine::LEGACY_FLAG_CATEGORY.to_string());
            }
            field_json.push(json!({ "name": f.name, "label": f.label, "description": f.label, "field_type": f.field_type, "required": f.required, "ferpa_sensitive": flagged, "ceds_code": f.ceds_code, "restricted": false }));
            fields.push(FieldView { name: f.name.clone(), label: f.label.clone(), field_type: f.field_type.clone(), options: Vec::new() });
        }
    } else {
        let m = ctx.manifests.get(&row.app_slug)?;
        let (index, table) = m.tables.iter().enumerate().find(|(_, t)| t.id == row.table_id)?;
        id = row.id.clone();
        name = table.name.clone();
        description = table.description.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| m.description.clone());
        department = m.department.clone();
        organization = m.organization_code.clone();
        herm = m.herm_capability_id.clone();
        kind = "table";
        app_title = m.title.clone();
        let mut ids = vec![table.id.clone()];
        if index == 0 {
            ids.push(String::new());
        }
        rows_of = Some((m.slug.clone(), ids));
        let mut protected = false;
        for f in &table.fields {
            let sensitivity = effective_sensitivity(f, ctx.labels.get(&(m.slug.clone(), table.id.clone(), f.name.clone())), &ctx.categories);
            let field_protected = sensitivity.protected;
            protected |= field_protected;
            if restricted(&f.name) {
                field_json.push(restricted_field());
                continue;
            }
            categories.extend(sensitivity.categories);
            let ty = serde_json::to_value(f.field_type).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            field_json.push(json!({ "name": f.name, "label": f.label, "description": f.label, "field_type": ty, "required": f.required, "ferpa_sensitive": field_protected, "ceds_code": null, "restricted": false }));
            fields.push(FieldView { name: f.name.clone(), label: f.label.clone(), field_type: ty, options: f.select_options.iter().map(|o| o.label.clone()).collect() });
        }
        level = if protected { "Restricted / FERPA" } else { "Internal" }.to_string();
    }

    categories.sort();
    categories.dedup();
    let unit = ctx.unit(&organization);
    let unit_name = unit["name"].as_str().unwrap_or("").to_string();
    let unit_code = unit["code"].as_str().unwrap_or("").to_string();
    let category_json: Vec<Value> = categories.iter().map(|c| json!({ "id": c, "name": ctx.category_name(c) })).collect();
    let steward = row.steward_position.as_ref().map(|k| json!({ "position_key": k, "position": ctx.positions.get(k) }));

    let mut hay = vec![name.clone(), description.clone(), department.clone(), organization.clone(), unit_name.clone(), unit_code, app_title.clone()];
    hay.extend(herm.iter().cloned());
    hay.extend(categories.iter().flat_map(|c| [c.clone(), ctx.category_name(c)]));
    hay.extend(fields.iter().flat_map(|f| [f.name.clone(), f.label.clone()]));
    if let Some(Some(p)) = row.steward_position.as_ref().map(|k| ctx.positions.get(k)) {
        hay.push(p.clone());
    }

    entry.insert("id".into(), json!(id));
    entry.insert("dataset_id".into(), json!(row.id));
    entry.insert("kind".into(), json!(kind));
    entry.insert("name".into(), json!(name));
    entry.insert("description".into(), json!(description));
    entry.insert("department".into(), json!(department));
    entry.insert("function".into(), json!(department));
    entry.insert("organization".into(), json!(organization));
    entry.insert("unit".into(), unit);
    entry.insert("steward".into(), steward.unwrap_or(Value::Null));
    entry.insert("request_fields".into(), json!(row.request_fields));
    entry.insert("sensitivity_level".into(), json!(level));
    entry.insert("herm_capability_id".into(), json!(herm));
    entry.insert("fields".into(), Value::Array(field_json));
    entry.insert("categories".into(), Value::Array(category_json));
    entry.insert("used_by_apps".into(), json!(used_by(row, &id, ctx)));
    if kind == "table" {
        entry.insert("app_slug".into(), json!(row.app_slug));
        entry.insert("app_title".into(), json!(app_title));
        entry.insert("table_id".into(), json!(row.table_id));
    }
    Some(Base { name, department, unit: unit_name, categories, hay: hay.join(" | "), entry, fields, rows_of, declared, freshness })
}

fn restricted_field() -> Value {
    json!({ "label": RESTRICTED, "description": RESTRICTED, "restricted": true })
}

/// How many other apps link to the dataset, from their manifests. It is a count of apps.
fn used_by(row: &CatalogRow, id: &str, ctx: &Ctx) -> usize {
    ctx.manifests
        .values()
        .filter(|m| m.slug != row.app_slug)
        .filter(|m| {
            m.tables.iter().flat_map(|t| t.fields.iter()).any(|f| match &row.published_id {
                Some(pid) => f.linked_dataset_id.as_deref() == Some(pid.as_str()) || f.linked_dataset_id.as_deref() == Some(id),
                None => f.link_target_app.as_deref() == Some(row.app_slug.as_str()) && f.link_target_table.as_deref() == Some(row.table_id.as_str()),
            })
        })
        .count()
}

/// The count as the entry shows it: `record_count`, and `size` with a label. Under the threshold the count is not given.
fn size(rows: i64, over_cap: bool, min: i64) -> (Value, Value) {
    if rows < min {
        return (Value::Null, json!({ "masked": true, "label": format!("fewer than {min} rows") }));
    }
    if over_cap {
        let cap = scaffoldry_data::catalog::COUNT_CAP;
        return (json!(cap), json!({ "masked": false, "approx": cap, "label": "over 100,000 rows" }));
    }
    let approx = two_figures(rows);
    (json!(approx), json!({ "masked": false, "approx": approx, "label": format!("about {approx} rows") }))
}

/// A count rounded to two significant figures.
fn two_figures(n: i64) -> i64 {
    let mut step = 1;
    while n / step >= 100 {
        step *= 10;
    }
    (n + step / 2) / step * step
}

// ---------------------------------------------------------------------------------------------------------------------
// The sample: made from the schema, never from a row
// ---------------------------------------------------------------------------------------------------------------------

/// A made-up value for a field of this type. It uses the field's type, its label and its declared options, and nothing else.
fn synthetic(f: &FieldView, i: usize) -> Value {
    let label = &f.label;
    match f.field_type.as_str() {
        "Number" | "Count" | "Rollup" | "Formula" | "Autonumber" | "Lookup" => json!(i),
        "Currency" => json!(i * 100),
        "Percent" => json!(i * 10),
        "Rating" => json!(i.min(5)),
        "Duration" => json!(i * 60),
        "Boolean" | "Checkbox" => json!(i % 2 == 1),
        "Date" => json!(format!("2000-01-{i:02}")),
        "DateTime" | "CreatedTime" | "LastModifiedTime" => json!(format!("2000-01-{i:02}T09:00:00Z")),
        "Select" => json!(f.options.get((i - 1) % f.options.len().max(1)).cloned().unwrap_or_else(|| format!("Option {i}"))),
        "MultiSelect" => json!(f.options.first().cloned().into_iter().collect::<Vec<_>>()),
        "Email" | "Collaborator" | "CreatedBy" | "LastModifiedBy" => json!(format!("person{i}@example.org")),
        "Phone" => json!(format!("555-010{i}")),
        "Url" => json!(format!("https://example.org/{i}")),
        "Attachment" => json!([]),
        "Link" | "Relation" | "Unit" => json!(format!("example-{i}")),
        "Button" => Value::Null,
        _ => json!(format!("Sample {label} {i}")),
    }
}

fn sample(fields: &[FieldView]) -> Value {
    let rows: Vec<Value> = (1..=SAMPLE_ROWS).map(|i| Value::Object(fields.iter().map(|f| (f.name.clone(), synthetic(f, i))).collect())).collect();
    json!({ "synthetic": true, "note": "Made up from the schema. It holds no stored value.", "rows": rows })
}

// ---------------------------------------------------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------------------------------------------------

/// What to look for. Both parts are optional.
#[derive(Debug, Default, Clone)]
pub struct Search {
    pub query: Option<String>,
    pub department: Option<String>,
}

/// A `LIKE` pattern for one word: the word is only itself, whatever characters it holds.
fn like_pattern(word: &str) -> String {
    let mut p = String::from("%");
    for c in word.chars() {
        if matches!(c, '\\' | '%' | '_') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

fn words(query: &Option<String>) -> Result<Vec<String>, ServiceError> {
    let q = query.as_deref().unwrap_or("").trim();
    if q.chars().count() > MAX_QUERY {
        return Err(bad(format!("A query holds at most {MAX_QUERY} characters")));
    }
    let list: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    if list.len() > MAX_WORDS {
        return Err(bad(format!("A query holds at most {MAX_WORDS} words")));
    }
    Ok(list)
}

fn facet(counts: BTreeMap<String, usize>) -> Value {
    Value::Array(counts.into_iter().map(|(name, count)| json!({ "name": name, "count": count })).collect())
}

/// The entries the caller may discover that match, with the facets of those entries. A dataset the caller may not discover
/// is not read at all.
pub fn search(caller: &AuthUser, s: &Search, state: &SharedState) -> Result<Value, ServiceError> {
    let words = words(&s.query)?;
    let who = Who::of(caller, state);
    let ctx = Ctx::of(state)?;
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner()).clone();
    let department = s.department.as_deref().map(str::trim).filter(|d| !d.is_empty());

    let mut bases: Vec<(CatalogRow, Base)> = Vec::new();
    for row in state.repository.catalog_rows().map_err(storage)? {
        if !who.may(&row.discoverability, &tree) {
            continue;
        }
        let Some(base) = build(&row, &ctx) else { continue };
        if department.is_some_and(|d| !base.department.eq_ignore_ascii_case(d)) {
            continue;
        }
        bases.push((row, base));
    }

    if !words.is_empty() {
        let ids = bases.iter().map(|(r, _)| r.id.clone()).collect();
        let hay = bases.iter().map(|(_, b)| b.hay.clone()).collect();
        let keep: HashSet<String> = state.repository.catalog_match(ids, hay, words.iter().map(|w| like_pattern(w)).collect()).map_err(storage)?.into_iter().collect();
        bases.retain(|(r, _)| keep.contains(&r.id));
    }
    bases.sort_by(|(ra, a), (rb, b)| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| ra.id.cmp(&rb.id)));

    let (mut units, mut functions, mut categories) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for (_, b) in &bases {
        *units.entry(b.unit.clone()).or_insert(0) += 1;
        *functions.entry(b.department.clone()).or_insert(0) += 1;
        for c in &b.categories {
            *categories.entry(ctx.category_name(c)).or_insert(0) += 1;
        }
    }
    let total = bases.len();
    let mut datasets = Vec::new();
    for (row, base) in bases.into_iter().take(PAGE) {
        datasets.push(finish(&row, base, &ctx, who.manager, state)?);
    }
    Ok(json!({
        "total": total,
        "truncated": total > PAGE,
        "datasets": datasets,
        "facets": { "units": facet(units), "functions": facet(functions), "categories": facet(categories) },
    }))
}

/// The dataset the caller may discover, named by its dataset id or its published id. A dataset the caller may not discover
/// is the same not found as one that does not exist.
pub fn discoverable(caller: &AuthUser, id: &str, state: &SharedState) -> Result<CatalogRow, ServiceError> {
    let row = state.repository.catalog_row(id).map_err(storage)?.ok_or_else(not_found)?;
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner()).clone();
    if !Who::of(caller, state).may(&row.discoverability, &tree) {
        return Err(not_found());
    }
    Ok(row)
}

/// The name an entry shows, for a notice. It reads no row and checks nobody's access, so it is only for people who are
/// already told about the dataset. `None` when its source is gone.
pub fn name_of(state: &SharedState, id: &str) -> Option<String> {
    let row = state.repository.catalog_row(id).ok()??;
    if let Some(pid) = &row.published_id {
        return state.datasets.read().ok()?.get(pid).map(|d| d.name.clone());
    }
    let engine = state.engine.read().ok()?;
    use scaffoldry_engine::HostRouter;
    engine.resolve_by_slug(&row.app_slug)?.tables.iter().find(|t| t.id == row.table_id).map(|t| t.name.clone())
}

/// The unit a table dataset sits at: the unit its app names by code or name, else its workspace's unit. `None` for a
/// published dataset, which has no app.
pub fn unit_of(state: &SharedState, row: &CatalogRow) -> Option<Uuid> {
    if row.published_id.is_some() {
        return None;
    }
    unit_of_app(state, &row.app_slug)
}

/// The unit an app's datasets sit at (data layer phase 3 reads it for a row that names no unit of its own).
pub fn unit_of_app(state: &SharedState, app_slug: &str) -> Option<Uuid> {
    use scaffoldry_engine::HostRouter;
    let (code, workspace) = {
        let engine = state.engine.read().ok()?;
        let m = engine.resolve_by_slug(app_slug)?;
        (m.organization_code.clone(), m.workspace_id.clone())
    };
    let named = {
        let orgs = state.organizations.read().ok()?;
        orgs.values().find(|o| o.code.eq_ignore_ascii_case(&code)).or_else(|| orgs.values().find(|o| o.name.eq_ignore_ascii_case(&code))).map(|o| o.id)
    };
    named.or_else(|| state.workspaces.read().ok()?.get(&workspace?)?.organization_id)
}

/// One entry, named by its dataset id or its published id. A dataset the caller may not discover is the same not found as
/// one that does not exist.
pub fn get(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let row = discoverable(caller, id, state)?;
    let ctx = Ctx::of(state)?;
    let base = build(&row, &ctx).ok_or_else(not_found)?;
    finish(&row, base, &ctx, Who::of(caller, state).manager, state)
}

/// Adds the size, the freshness and the sample. The policy is added for the people who set it.
fn finish(row: &CatalogRow, base: Base, ctx: &Ctx, manager: bool, state: &SharedState) -> Result<Value, ServiceError> {
    let mut entry = base.entry;
    let (mut freshness, mut counted) = (base.freshness, base.declared.map(|n| size(n, false, ctx.min)));
    if let Some((app, tables)) = base.rows_of {
        let f = state.repository.catalog_footprint(&app, tables).map_err(storage)?;
        counted = Some(size(f.rows, f.over_cap, ctx.min));
        freshness = f.latest_day;
    }
    let (record_count, size) = counted.unwrap_or((Value::Null, json!({ "masked": true, "label": "not known" })));
    entry.insert("record_count".into(), record_count);
    entry.insert("size".into(), size);
    entry.insert("freshness".into(), json!(freshness));
    entry.insert("sample".into(), sample(&base.fields));
    if manager {
        entry.insert("discoverability".into(), row.discoverability.clone());
        entry.insert("restricted_fields".into(), json!(row.restricted_fields));
    }
    Ok(Value::Object(entry))
}

// ---------------------------------------------------------------------------------------------------------------------
// The policy
// ---------------------------------------------------------------------------------------------------------------------

/// `PUT /admin/datasets/{id}/discoverability`. The body names any of `discoverability`, `restricted_fields`,
/// `steward_position` and `request_fields`, and a `reason`. A part left out stays as it is. A `steward_position` of null clears it.
pub fn set_policy(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    require_platform_admin_or_compliance(caller, state)?;
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(bad("A reason between 1 and 500 characters is required"));
    }
    let row = state.repository.catalog_row(id).map_err(storage)?.ok_or_else(not_found)?;
    let ctx = Ctx::of(state)?;
    let mut change = PolicyChange::default();

    if let Some(v) = body.get("discoverability") {
        let policy = Discover::from_json(v).ok_or_else(|| bad("discoverability is \"everyone\", \"hidden\", {\"units\": [unit ids]} or {\"positions\": [position keys]}"))?;
        if let Discover::Reach { units, positions } = &policy {
            if let Some(u) = units.iter().find(|u| !ctx.orgs.iter().any(|o| o.id == **u)) {
                return Err(bad(format!("{u} is not a unit")));
            }
            let live = |k: &str| state.position_types.read().ok().and_then(|p| p.get(k).map(|t| t.retired_at.is_none())).unwrap_or(false);
            if let Some(p) = positions.iter().find(|p| !live(p)) {
                return Err(bad(format!("'{p}' is not a position")));
            }
        }
        change.discoverability = Some(policy.to_json());
    }
    if let Some(v) = body.get("restricted_fields") {
        let names: Vec<String> = v.as_array().and_then(|a| a.iter().map(|x| x.as_str().map(str::to_string)).collect()).ok_or_else(|| bad("restricted_fields is a list of field names"))?;
        let known = known_fields(&row, &ctx).ok_or_else(not_found)?;
        if let Some(n) = names.iter().find(|n| !known.contains(*n)) {
            return Err(bad(format!("'{n}' is not a field of this dataset")));
        }
        let mut names = names;
        names.sort();
        names.dedup();
        change.restricted_fields = Some(names);
    }
    match body.get("steward_position") {
        None => {}
        Some(Value::Null) => change.steward_position = Some(None),
        Some(Value::String(k)) => {
            let live = state.position_types.read().ok().and_then(|p| p.get(k).map(|t| t.retired_at.is_none())).unwrap_or(false);
            if !live {
                return Err(bad(format!("'{k}' is not a position")));
            }
            change.steward_position = Some(Some(k.clone()));
        }
        Some(_) => return Err(bad("steward_position is a position key or null")),
    }
    if let Some(v) = body.get("request_fields") {
        let names: Option<Vec<&str>> = v.as_array().and_then(|a| a.iter().map(Value::as_str).collect());
        let names = names.filter(|n| n.iter().all(|f| REQUEST_FIELDS.contains(f))).ok_or_else(|| bad(format!("request_fields is a list of {}", REQUEST_FIELDS.join(", "))))?;
        // Kept in the order a form shows them, each once.
        change.request_fields = Some(REQUEST_FIELDS.iter().filter(|f| names.contains(f)).map(|f| f.to_string()).collect());
    }
    if change.discoverability.is_none() && change.restricted_fields.is_none() && change.steward_position.is_none() && change.request_fields.is_none() {
        return Err(bad("Name discoverability, restricted_fields, steward_position or request_fields"));
    }

    let (principal, why) = (caller.eppn.clone(), reason.to_string());
    let now = state
        .repository
        .set_catalog_policy(id, change, move |was, now| crate::repository::AppendDecisionParams {
            principal,
            organization_code: "DIV-ADMIN-CONSOLE".to_string(),
            app_slug: (was.published_id.is_none()).then(|| was.app_slug.clone()),
            decision_type: DecisionType::DataLabelChanged,
            oscal_control_id: "AC-03".to_string(),
            rationale: why,
            payload: json!({
                "dataset": was.id, "setting": "discoverability",
                "from": { "discoverability": was.discoverability, "restricted_fields": was.restricted_fields, "steward_position": was.steward_position, "request_fields": was.request_fields },
                "to": { "discoverability": now.discoverability, "restricted_fields": now.restricted_fields, "steward_position": now.steward_position, "request_fields": now.request_fields },
            }),
        })
        .map_err(storage)?
        .ok_or_else(not_found)?;
    Ok(json!({
        "id": id, "dataset_id": now.id, "discoverability": now.discoverability,
        "restricted_fields": now.restricted_fields, "steward_position": now.steward_position, "request_fields": now.request_fields,
    }))
}

/// The names of the fields a dataset has, or `None` when its source is gone.
fn known_fields(row: &CatalogRow, ctx: &Ctx) -> Option<HashSet<String>> {
    if let Some(pid) = &row.published_id {
        return Some(ctx.published.get(pid)?.fields.iter().map(|f| f.name.clone()).collect());
    }
    let table = ctx.manifests.get(&row.app_slug)?.tables.iter().find(|t| t.id == row.table_id)?;
    Some(table.fields.iter().map(|f| f.name.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_rounded_to_two_figures() {
        assert_eq!(two_figures(12), 12);
        assert_eq!(two_figures(99), 99);
        assert_eq!(two_figures(1234), 1200);
        assert_eq!(two_figures(98_765), 99_000);
    }

    #[test]
    fn a_small_group_has_no_count() {
        assert_eq!(size(10, false, 11).0, Value::Null);
        assert_eq!(size(11, false, 11).0, json!(11));
        assert_eq!(size(0, false, 11).1["label"], json!("fewer than 11 rows"));
    }

    #[test]
    fn a_word_is_only_itself_in_a_pattern() {
        assert_eq!(like_pattern("a%b_c\\d"), "%a\\%b\\_c\\\\d%");
    }

    #[test]
    fn the_policy_round_trips_and_a_bad_one_is_none() {
        let u = Uuid::new_v4();
        for v in [json!("everyone"), json!("hidden"), json!({ "units": [u] }), json!({ "positions": ["chair"] })] {
            assert_eq!(Discover::from_json(&v).unwrap().to_json(), v);
        }
        for v in [json!("sometimes"), json!({ "units": [] }), json!({ "units": ["x"] }), json!({ "other": [1] }), json!(3)] {
            assert!(Discover::from_json(&v).is_none(), "{v}");
        }
    }
}
