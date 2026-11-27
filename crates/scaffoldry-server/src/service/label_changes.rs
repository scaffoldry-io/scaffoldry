//! Data layer phase 6: a label change is proposed, evaluated on request, and decided by a steward
//! (`docs/plans/data-layer.md`, "The evaluation a person reads" and "Changing a label", steps 1 to 3).
//!
//! - **A proposal** names rows to tag or untag, or a dataset floor. It comes from a person or from a review finding. It is stored
//!   in `label_changes` and applies nothing.
//! - **The evaluation** is built when it is asked for. It asks the phase 3 gate the same question twice, once of the world as it is and
//!   once of the world as it would be, for each kind of reader. It never stores its answer.
//! - **A decision** is made by the dataset's steward (the people who hold its `steward_position` at its unit or above), a Platform
//!   Admin or a compliance officer. Raising protection needs one approval. Removing protection needs two, from two people, and
//!   neither is the proposer. When the change would lock someone out of open work, the steward must say how to treat that work.
//!   Approving applies the change through the phase 3 functions and writes one `DataLabelChanged` ledger entry that names the
//!   proposer, the deciders and the finding.
//!
//! Applying on a date, the notice period, the communications plan and exceptions are phase 6b, in `schedule`.

pub mod schedule;

use crate::repository::AppendDecisionParams;
use crate::repository_catalog::CatalogRow;
use crate::repository_label_changes::{DecideOutcome, DecideReq, Done, Holder, OpenProcess, OpenTask, Plan, Proposal};
use crate::service::admin::require_platform_admin_or_compliance;
use crate::service::{catalog, data_governance::flag_tables, row_labels, ServiceError};
use crate::state::{AuthUser, SharedState};
use scaffoldry_core::approver::{ApproverSpec, Origin};
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_data::changes::Change;
use scaffoldry_data::gate::Gate;
use scaffoldry_engine::sensitivity::Category;
use scaffoldry_engine::HostRouter;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

/// The most rows one proposal names, and the most categories one row or floor carries (the phase 3 limit).
const MAX_ROWS: usize = 1_000;
const MAX_TAGS: usize = row_labels::MAX_TAGS;
/// Example ids shown for a change. Never a value.
const EXAMPLES: usize = 5;
/// The longest list of named things the evaluation prints for one part.
const LISTED: usize = 50;

fn bad(m: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(m.into())
}

fn storage(e: crate::repository::RepositoryError) -> ServiceError {
    ServiceError::StorageUnavailable(e.to_string())
}

fn is_admin_or_compliance(caller: &AuthUser, state: &SharedState) -> bool {
    require_platform_admin_or_compliance(caller, state).is_ok()
}

fn reason_of(body: &Value) -> Result<String, ServiceError> {
    let reason = body.get("reason").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(bad("A reason between 1 and 500 characters is required"));
    }
    Ok(reason.to_string())
}

// ---------------------------------------------------------------------------------------------------------------------
// Who may decide
// ---------------------------------------------------------------------------------------------------------------------

/// The people who hold the dataset's steward position now, found from the dataset's unit up (the rule request access uses).
fn stewards_of(row: &CatalogRow, state: &SharedState) -> Vec<String> {
    let Some(key) = &row.steward_position else { return Vec::new() };
    let Some(unit) = catalog::unit_of(state, row) else { return Vec::new() };
    let spec = ApproverSpec::Position { key: key.clone(), from: Origin::Workspace, walk_up: true };
    crate::service::approvers::resolved_approvers(state, &spec, unit, &[], None)
}

/// A steward, a Platform Admin or a compliance officer.
fn may_decide(caller: &AuthUser, row: &CatalogRow, state: &SharedState) -> bool {
    is_admin_or_compliance(caller, state) || stewards_of(row, state).iter().any(|s| s.eq_ignore_ascii_case(&caller.eppn))
}

fn refused() -> ServiceError {
    ServiceError::forbidden("Only the dataset's steward, a Platform Admin or a compliance officer reads or decides a label change")
}

/// A change and its dataset, for a caller who may decide it. A change the caller may not decide is the same refusal whether it
/// exists or not, so the answer says nothing about it.
fn load(caller: &AuthUser, id: &str, state: &SharedState) -> Result<(Change, CatalogRow), ServiceError> {
    let admin = is_admin_or_compliance(caller, state);
    let missing = || if admin { ServiceError::NotFound("Label change not found".to_string()) } else { refused() };
    let id = Uuid::parse_str(id).map_err(|_| missing())?;
    let change = state.repository.label_change(id).map_err(storage)?.ok_or_else(missing)?;
    let row = state.repository.catalog_row(&change.dataset_id).map_err(storage)?.ok_or_else(missing)?;
    if !may_decide(caller, &row, state) {
        return Err(refused());
    }
    Ok((change, row))
}

// ---------------------------------------------------------------------------------------------------------------------
// What is asked
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Tags,
    Floor,
}

/// A proposal read into the terms the evaluation and the decision need.
struct Ask {
    kind: Kind,
    app: String,
    table: String,
    dataset_id: String,
    floor_before: Vec<String>,
    floor_after: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
    records: Vec<String>,
}

impl Ask {
    /// Whether a row is one the change touches.
    fn touches(&self, record: &str) -> bool {
        match self.kind {
            Kind::Floor => true,
            Kind::Tags => self.records.iter().any(|r| r == record),
        }
    }

    /// The tags a row would carry after the change.
    fn tags_after(&self, before: &[String]) -> Vec<String> {
        if self.kind == Kind::Floor {
            return before.to_vec();
        }
        let mut out: Vec<String> = before.iter().filter(|t| !self.remove.contains(t)).cloned().chain(self.add.iter().cloned()).collect();
        out.sort();
        out.dedup();
        out
    }
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn ask_of(change: &Change, row: &CatalogRow, state: &SharedState) -> Result<Ask, ServiceError> {
    let floor_before = state
        .repository
        .with_client({
            let (app, table) = (row.app_slug.clone(), row.table_id.clone());
            move |c| Ok(scaffoldry_data::labels::floor_of(c, &app, &table)?)
        })
        .map_err(storage)?;
    let spec = &change.spec;
    let kind = if change.kind == "floor" { Kind::Floor } else { Kind::Tags };
    let floor_after = if kind == Kind::Floor { strings(spec, "floor") } else { floor_before.clone() };
    Ok(Ask {
        kind,
        app: row.app_slug.clone(),
        table: row.table_id.clone(),
        dataset_id: row.id.clone(),
        floor_before,
        floor_after,
        add: strings(spec, "add"),
        remove: strings(spec, "remove"),
        records: strings(spec, "records"),
    })
}

/// A category that protects: marked protected, or carrying a requirement. One that settings no longer define is held to protect, as the
/// rule for removing a tag holds it.
fn protective(id: &str, cats: &[Category]) -> bool {
    cats.iter().find(|c| c.id == id).is_none_or(|c| c.protected || !c.requires.is_empty())
}

/// Whether the change takes protection away: it lowers the floor (any category leaves it), or it removes a protective tag.
fn removes_protection(ask: &Ask, cats: &[Category]) -> bool {
    match ask.kind {
        Kind::Floor => ask.floor_before.iter().any(|c| !ask.floor_after.contains(c)),
        Kind::Tags => ask.remove.iter().any(|t| protective(t, cats)),
    }
}

fn approvals_needed(ask: &Ask, cats: &[Category]) -> usize {
    if removes_protection(ask, cats) {
        2
    } else {
        1
    }
}

/// The ask from a request body: the kind, and what it names, checked against the settings and the dataset. A body that names parts
/// replaces them in `base`, so the same checks serve a proposal and a changed proposal.
fn spec_of(kind: &Kind, body: &Value, base: Option<&Value>, row: &CatalogRow, floor_now: &[String], state: &SharedState) -> Result<Value, ServiceError> {
    let cats = state.sensitivity_categories();
    let pick = |key: &str| -> Result<Option<Vec<String>>, ServiceError> {
        match body.get(key) {
            None => Ok(base.map(|b| strings(b, key))),
            Some(Value::Array(a)) => {
                let mut v: Vec<String> = a.iter().map(|s| s.as_str().map(str::to_string)).collect::<Option<_>>().ok_or_else(|| bad(format!("{key} is a list of text")))?;
                v.sort();
                v.dedup();
                Ok(Some(v))
            }
            Some(_) => Err(bad(format!("{key} is a list of text"))),
        }
    };
    match kind {
        Kind::Floor => {
            let floor = pick("floor")?.ok_or_else(|| bad("floor is a list of category ids"))?;
            if floor.len() > MAX_TAGS {
                return Err(bad(format!("A floor holds at most {MAX_TAGS} categories")));
            }
            if let Some(unknown) = floor.iter().find(|t| !cats.iter().any(|c| &c.id == *t)) {
                return Err(bad(format!("'{}' is not a category of this organization", unknown.chars().take(64).collect::<String>())));
            }
            if floor == floor_now {
                return Err(bad("The dataset already has this floor, so there is nothing to change"));
            }
            Ok(json!({ "floor": floor }))
        }
        Kind::Tags => {
            let add = pick("add")?.unwrap_or_default();
            let remove = pick("remove")?.unwrap_or_default();
            let records = pick("records")?.unwrap_or_default();
            if add.is_empty() && remove.is_empty() {
                return Err(bad("Name tags to add or tags to remove"));
            }
            if add.len() > MAX_TAGS || remove.len() > MAX_TAGS {
                return Err(bad(format!("A change names at most {MAX_TAGS} tags to add and {MAX_TAGS} to remove")));
            }
            if let Some(unknown) = add.iter().find(|t| !cats.iter().any(|c| &c.id == *t)) {
                return Err(bad(format!("'{}' is not a category of this organization", unknown.chars().take(64).collect::<String>())));
            }
            if remove.iter().any(|t| t.is_empty() || t.chars().count() > 64) {
                return Err(bad("A tag to remove is a category id of at most 64 characters"));
            }
            if add.iter().any(|t| remove.contains(t)) {
                return Err(bad("A tag cannot be added and removed in one change"));
            }
            if records.is_empty() || records.len() > MAX_ROWS {
                return Err(bad(format!("Name 1 to {MAX_ROWS} rows in `records`")));
            }
            let found = state.repository.label_rows(&row.app_slug, &row.table_id, records.clone()).map_err(storage)?;
            let found: BTreeSet<&str> = found.iter().map(|(id, _, _)| id.as_str()).collect();
            if let Some(gone) = records.iter().find(|r| !found.contains(r.as_str())) {
                return Err(bad(format!("'{}' is not a live row of this dataset", gone.chars().take(64).collect::<String>())));
            }
            Ok(json!({ "add": add, "remove": remove, "records": records }))
        }
    }
}

fn change_json(c: &Change, ask: &Ask, cats: &[Category]) -> Value {
    json!({
        "id": c.id, "dataset_id": c.dataset_id, "kind": c.kind, "spec": c.spec, "reason": c.reason, "source": c.source, "finding_id": c.finding_id,
        "proposed_by": c.proposed_by, "proposed_at": c.proposed_at, "status": c.status,
        "removes_protection": removes_protection(ask, cats), "approvals_needed": approvals_needed(ask, cats),
        "approved_by": c.approvals.iter().map(|a| json!({ "by": a.by, "at": a.at, "note": a.note })).collect::<Vec<_>>(),
        "open_work": c.open_work, "decided_by": c.decided_by, "decided_at": c.decided_at, "decision_note": c.decision_note,
        "effective_at": c.effective_at, "plan_approved_at": c.plan_approved_at,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Propose
// ---------------------------------------------------------------------------------------------------------------------

/// `POST /data/datasets/{id}/label-changes`. The body names `kind` (`tags` or `floor`), what it asks, and a `reason`. It is for the
/// dataset's steward, a Platform Admin or a compliance officer. The change is stored as pending. Nothing is applied.
pub fn propose(caller: &AuthUser, dataset: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let admin = is_admin_or_compliance(caller, state);
    let row = state.repository.catalog_row(dataset).map_err(storage)?;
    let Some(row) = row.filter(|r| admin || may_decide(caller, r, state)) else {
        return Err(if admin { ServiceError::NotFound("Dataset not found".to_string()) } else { refused() });
    };
    if row.published_id.is_some() {
        return Err(bad("A published dataset holds no rows, so it has no labels to change"));
    }
    let reason = reason_of(body)?;
    let kind = match body.get("kind").and_then(Value::as_str) {
        Some("tags") => Kind::Tags,
        Some("floor") => Kind::Floor,
        _ => return Err(bad("kind is tags or floor")),
    };
    let floor_now = state
        .repository
        .with_client({
            let (app, table) = (row.app_slug.clone(), row.table_id.clone());
            move |c| Ok(scaffoldry_data::labels::floor_of(c, &app, &table)?)
        })
        .map_err(storage)?;
    let spec = spec_of(&kind, body, None, &row, &floor_now, state)?;
    let kind_name = if kind == Kind::Floor { "floor" } else { "tags" };
    let proposal = Proposal { dataset_id: row.id.clone(), kind: kind_name.to_string(), spec, reason, source: "person".to_string(), finding: None, by: caller.eppn.clone() };
    let id = state.repository.propose_label_change(proposal).map_err(storage)?;
    let change = state.repository.label_change(id).map_err(storage)?.ok_or_else(|| ServiceError::Internal("The proposal was not stored".to_string()))?;
    let ask = ask_of(&change, &row, state)?;
    Ok(change_json(&change, &ask, &state.sensitivity_categories()))
}

/// Confirming a review finding (data layer phase 5 found it, phase 6 acts on it). The finding is confirmed and a proposal is made in one
/// transaction. The proposal tags the rows the finding names, which are its sample rows. It applies nothing.
pub fn from_finding(caller: &AuthUser, finding: &Value, reason: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let text = |k: &str| finding[k].as_str().unwrap_or("").to_string();
    let (app, table, category, field) = (text("app_slug"), text("table_id"), text("category"), text("field"));
    let id = Uuid::parse_str(&text("id")).map_err(|_| ServiceError::NotFound("Finding not found".to_string()))?;
    let rows = state.repository.catalog_rows().map_err(storage)?;
    let dataset = rows.iter().find(|r| r.app_slug == app && r.table_id == table).ok_or_else(|| ServiceError::Conflict("The dataset is gone, so there is nothing to tag. Dismiss the finding".to_string()))?;
    if !state.sensitivity_categories().iter().any(|c| c.id == category) {
        return Err(ServiceError::Conflict("The category is no longer one of this organization's. Dismiss the finding".to_string()));
    }
    let named: Vec<String> = finding["sample_record_ids"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let mut live: Vec<String> = state.repository.label_rows(&app, &table, named).map_err(storage)?.into_iter().map(|(id, _, _)| id).collect();
    live.sort();
    if live.is_empty() {
        return Err(ServiceError::Conflict("The rows the finding names are gone. Dismiss the finding".to_string()));
    }
    let spec = json!({ "add": [category], "remove": [], "records": live });
    let why = format!("The review found content in `{field}` that looks like {category}. {reason}");
    let why: String = why.chars().take(500).collect();
    let by = caller.eppn.clone();
    let principal = by.clone();
    let proposal = Proposal { dataset_id: dataset.id.clone(), kind: "tags".to_string(), spec, reason: why, source: "review".to_string(), finding: Some(id), by: by.clone() };
    let entry_reason = reason.to_string();
    let payload = json!({ "finding": id, "decision": "confirm", "app": app, "table": table, "field": field, "detector": text("detector_id"), "category": category });
    let change = state
        .repository
        .confirm_review_finding(id, &by, reason, proposal, move |change| {
            let mut payload = payload;
            payload["change"] = json!(change);
            AppendDecisionParams { principal, organization_code: "DIV-ADMIN-CONSOLE".to_string(), app_slug: None, decision_type: DecisionType::ContentFindingDecided, oscal_control_id: "RA-02".to_string(), rationale: entry_reason, payload }
        })
        .map_err(storage)?;
    match change {
        Some(change) => Ok(json!({ "id": id, "status": "confirmed", "change_id": change, "next": "A steward reads the evaluation and decides. Nothing is tagged yet" })),
        None => Err(ServiceError::Conflict("This finding was already decided".to_string())),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The evaluation
// ---------------------------------------------------------------------------------------------------------------------

/// The (position, unit) pairs a person holds.
type Signature = Vec<(String, Uuid)>;

/// A group of readers in the totals: a position and the unit it is held at, or neither for a reader who holds none a requirement lists.
type Bucket = (Option<String>, Option<String>, Option<Uuid>);

/// A kind of reader, how many there are, and the gate they meet before the change and after it.
struct Kindred {
    sig: Signature,
    /// The people who hold exactly these positions. Their number is what the evaluation shows. The plan of a scheduled change names them.
    members: Vec<String>,
    before: Option<Gate>,
    after: Option<Gate>,
}

/// What the gate says of a kind of reader for the rows a change touches.
struct Verdict {
    kind: Kindred,
    gains: bool,
    loses: bool,
}

/// The relevant (position, unit) pairs a person holds today. A position no requirement lists cannot change what the gate says.
fn signature(holders: &[&Holder], relevant: &BTreeSet<String>) -> Signature {
    let mut out: Vec<(String, Uuid)> = holders.iter().filter(|h| h.in_force()).filter_map(|h| h.position_key.as_ref().filter(|k| relevant.contains(*k)).map(|k| (k.clone(), h.unit))).collect();
    out.sort();
    out.dedup();
    out
}

fn is_root_admin(holders: &[&Holder], state: &SharedState) -> bool {
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    holders.iter().any(|h| h.affiliation == "platform_admin" && orgs.get(&h.unit).is_some_and(|o| o.parent_id.is_none()))
}

/// Whether a reader meets the gate for a row, given a gate that may be absent (a reader with nothing to meet).
fn admitted(gate: &Option<Gate>, unit: Option<Uuid>, tags: &[String], contains: &dyn Fn(Uuid, Uuid) -> bool) -> bool {
    let Some(g) = gate else { return true };
    let mut data = Map::new();
    if let (Some(field), Some(unit)) = (&g.unit_field, unit) {
        data.insert(field.clone(), json!(unit.to_string()));
    }
    if !tags.is_empty() {
        data.insert("_tags".to_string(), json!(tags));
    }
    g.admits(&Value::Object(data), contains)
}

fn requirement_positions(cats: &[Category]) -> BTreeSet<String> {
    cats.iter().flat_map(|c| c.requires.iter().map(|r| r.position.clone())).collect()
}

fn names_of(state: &SharedState) -> (HashMap<String, String>, HashMap<Uuid, String>) {
    let positions = state.position_types.read().map(|p| p.iter().map(|(k, t)| (k.clone(), t.name.clone())).collect()).unwrap_or_default();
    let units = state.organizations.read().map(|o| o.iter().map(|(id, n)| (*id, n.name.clone())).collect()).unwrap_or_default();
    (positions, units)
}

/// The readers, grouped by what they hold, and how many each group has. A reader is a person who holds a role, or is a member of the
/// app's workspace. A Platform Admin reads every row and is left out. Two people who hold the same relevant positions at the same
/// units get the same answer from the gate, so each group is asked once.
fn population(app: &str, relevant: &BTreeSet<String>, state: &SharedState) -> Result<(BTreeMap<Signature, Vec<String>>, i64), ServiceError> {
    let holders = state.repository.all_holders().map_err(storage)?;
    let mut by_person: HashMap<String, Vec<&Holder>> = HashMap::new();
    for h in &holders {
        by_person.entry(h.eppn.clone()).or_default().push(h);
    }
    let mut groups: BTreeMap<Signature, Vec<String>> = BTreeMap::new();
    let mut counted = 0;
    for (person, list) in &by_person {
        if is_root_admin(list, state) {
            continue;
        }
        groups.entry(signature(list, relevant)).or_default().push(person.clone());
        counted += 1;
    }
    let workspace = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine.resolve_by_slug(app).and_then(|m| m.workspace_id.clone())
    };
    if let Some(ws) = workspace {
        let members: Vec<String> = crate::service::members::people_with_roles(&ws, state).into_keys().collect();
        let mut seen = BTreeSet::new();
        for m in members {
            if by_person.contains_key(&m) || !seen.insert(m.clone()) {
                continue;
            }
            groups.entry(Vec::new()).or_default().push(m);
            counted += 1;
        }
    }
    Ok((groups, counted))
}

struct Reach {
    people: i64,
    by: BTreeMap<Bucket, i64>,
}

fn bucket(sig: &[(String, Uuid)], names: &(HashMap<String, String>, HashMap<Uuid, String>), n: i64, into: &mut Reach) {
    into.people += n;
    if sig.is_empty() {
        *into.by.entry((None, None, None)).or_default() += n;
    }
    for (position, unit) in sig {
        let name = names.0.get(position).cloned().unwrap_or_else(|| position.clone());
        let place = names.1.get(unit).cloned().unwrap_or_else(|| unit.to_string());
        *into.by.entry((Some(name), Some(place), Some(*unit))).or_default() += n;
    }
}

fn reach_json(r: &Reach) -> Value {
    let mut list: Vec<Value> = r.by.iter().map(|((p, u, _), n)| json!({ "position": p, "unit": u, "people": n })).collect();
    list.sort_by_key(|v| std::cmp::Reverse(v["people"].as_i64().unwrap_or(0)));
    json!({ "people": r.people, "by_position": list })
}

/// What the gate says of each kind of reader, for the rows the change touches: the readers the gate admits before and the readers it
/// admits after, in a world with the new floor and the new tags. Returns the verdicts and how many people were counted.
///
/// Cost: one read of every holding, one grouped read of the rows the change touches (by key for named rows, a scan of the table for a
/// floor), then one gate for each kind of reader, before and after, asked of each distinct pair of unit and tags. It does not
/// read a row twice and it does not ask the gate about a person.
fn verdicts(ask: &Ask, cats: &[Category], state: &SharedState) -> Result<(Vec<Verdict>, i64), ServiceError> {
    let relevant = requirement_positions(cats);
    let (readers, counted) = population(&ask.app, &relevant, state)?;
    let ids = (ask.kind == Kind::Tags).then(|| ask.records.clone());
    let groups = state.repository.label_groups(&ask.app, &ask.table, ids).map_err(storage)?;
    let gates: Vec<Kindred> = readers
        .into_iter()
        .map(|(sig, members)| {
            let before = row_labels::gate_over(&sig, &ask.app, &ask.table, Some(ask.floor_before.clone()), cats, state);
            let after = row_labels::gate_over(&sig, &ask.app, &ask.table, Some(ask.floor_after.clone()), cats, state);
            Kindred { sig, members, before, after }
        })
        .collect();
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    let contains = |a: Uuid, u: Uuid| tree.contains(a, u);
    let out = gates
        .into_iter()
        .map(|k| {
            let (mut gains, mut loses) = (false, false);
            for g in &groups {
                let (was, will) = (admitted(&k.before, g.unit, &g.tags, &contains), admitted(&k.after, g.unit, &ask.tags_after(&g.tags), &contains));
                gains |= !was && will;
                loses |= was && !will;
            }
            Verdict { kind: k, gains, loses }
        })
        .collect();
    Ok((out, counted))
}

/// Who gains and who loses access to at least one row the change touches, by position and unit. It names no person.
fn access(ask: &Ask, cats: &[Category], state: &SharedState) -> Result<Value, ServiceError> {
    let (verdicts, counted) = verdicts(ask, cats, state)?;
    let names = names_of(state);
    let (mut gain, mut lose) = (Reach { people: 0, by: BTreeMap::new() }, Reach { people: 0, by: BTreeMap::new() });
    for v in &verdicts {
        let n = v.kind.members.len() as i64;
        if v.gains {
            bucket(&v.kind.sig, &names, n, &mut gain);
        }
        if v.loses {
            bucket(&v.kind.sig, &names, n, &mut lose);
        }
    }
    Ok(json!({
        "people_counted": counted,
        "gain": reach_json(&gain),
        "lose": reach_json(&lose),
        "basis": "Counted from the label requirements in settings, by the position and unit a person holds, with the gate that reads every row. A Platform Admin reads every row and is not counted. App grants and row rules narrow access further and are not counted.",
    }))
}

/// The people who lose access to at least one row the change touches. The plan of a scheduled change tells them.
fn losers(ask: &Ask, cats: &[Category], state: &SharedState) -> Result<BTreeSet<String>, ServiceError> {
    Ok(verdicts(ask, cats, state)?.0.into_iter().filter(|v| v.loses).flat_map(|v| v.kind.members).collect())
}

/// What depends on the data, and who would be locked out of unfinished work.
struct Work {
    tasks: Vec<OpenTask>,
    processes: Vec<OpenProcess>,
    /// The people who read an affected row now and would not after it, with the open work that is theirs.
    locked: BTreeMap<String, Vec<Value>>,
}

fn work(ask: &Ask, cats: &[Category], state: &SharedState) -> Result<Work, ServiceError> {
    let tasks = state.repository.open_tasks(&ask.app, &ask.table).map_err(storage)?;
    let processes = state.repository.open_processes(&ask.app).map_err(storage)?;
    let ids: BTreeSet<String> = tasks.iter().map(|t| t.record.clone()).chain(processes.iter().map(|p| p.record.clone())).collect();
    let rows: HashMap<String, (Option<Uuid>, Vec<String>)> = if ids.is_empty() {
        HashMap::new()
    } else {
        state.repository.label_rows(&ask.app, &ask.table, ids.into_iter().collect()).map_err(storage)?.into_iter().map(|(id, unit, tags)| (id, (unit, tags))).collect()
    };
    let tasks: Vec<OpenTask> = tasks.into_iter().filter(|t| rows.contains_key(&t.record) && ask.touches(&t.record)).collect();
    let processes: Vec<OpenProcess> = processes.into_iter().filter(|p| rows.contains_key(&p.record) && ask.touches(&p.record)).collect();

    // For each person with open work on a row the change touches: do they read the row now, and not after?
    let mut mine: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    for t in &tasks {
        mine.entry(t.person.to_lowercase()).or_default().push((t.record.clone(), json!({ "kind": "task", "instance": t.instance, "step": t.step, "note": "The task's own grant still opens this one record. The person loses the rest of their reading of it" })));
    }
    for p in &processes {
        if let Some(by) = &p.started_by {
            mine.entry(by.to_lowercase()).or_default().push((p.record.clone(), json!({ "kind": "process", "instance": p.id, "rule": p.rule_id })));
        }
    }
    let mut locked: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (person, items) in mine {
        let list = state.repository.holdings_of(&person).map_err(storage)?;
        let holders: Vec<Holder> = {
            let now = chrono::Utc::now().timestamp();
            list.iter()
                .map(|(h, a)| Holder { eppn: person.clone(), unit: h.unit, position_key: h.position_key.clone(), effective: h.effective, end: h.end, affiliation: a.clone() })
                .filter(|h| h.effective.is_none_or(|e| e <= now) && h.end.is_none_or(|e| e >= now))
                .collect()
        };
        if is_root_admin(&holders.iter().collect::<Vec<_>>(), state) {
            continue;
        }
        let held: Vec<(String, Uuid)> = holders.iter().filter_map(|h| h.position_key.clone().map(|k| (k, h.unit))).collect();
        let before = row_labels::gate_over(&held, &ask.app, &ask.table, Some(ask.floor_before.clone()), cats, state);
        let after = row_labels::gate_over(&held, &ask.app, &ask.table, Some(ask.floor_after.clone()), cats, state);
        let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
        let contains = |a: Uuid, u: Uuid| tree.contains(a, u);
        for (record, item) in items {
            let Some((unit, tags)) = rows.get(&record) else { continue };
            if admitted(&before, *unit, tags, &contains) && !admitted(&after, *unit, &ask.tags_after(tags), &contains) {
                locked.entry(person.clone()).or_default().push(item);
            }
        }
    }
    Ok(Work { tasks, processes, locked })
}

fn sentence(change: &Change, finding: &Option<Value>) -> String {
    match finding {
        Some(f) => format!(
            "The review found content in the field '{}' that looks like {}: {} matches in {} rows. {}",
            f["field"].as_str().unwrap_or(""),
            f["category"].as_str().unwrap_or(""),
            f["match_count"],
            f["record_count"],
            change.reason
        ),
        None => format!("{} proposed it. {}", change.proposed_by, change.reason),
    }
}

fn undo_text(ask: &Ask, cats: &[Category]) -> (String, usize) {
    // The opposite change: tags added are removed and the other way round, and a floor goes back to what it was.
    let reverse = Ask {
        kind: ask.kind.clone(),
        app: ask.app.clone(),
        table: ask.table.clone(),
        dataset_id: ask.dataset_id.clone(),
        floor_before: ask.floor_after.clone(),
        floor_after: ask.floor_before.clone(),
        add: ask.remove.clone(),
        remove: ask.add.clone(),
        records: ask.records.clone(),
    };
    let needs = approvals_needed(&reverse, cats);
    let how = if needs == 2 { "It takes protection away, so it needs two approvers, and neither is the person who proposes it" } else { "It raises protection, so one steward approves it" };
    let step = match ask.kind {
        Kind::Floor => "Propose a floor change that sets the floor back to what it was",
        Kind::Tags => "Propose a tags change that does the opposite to the same rows",
    };
    (format!("{step} (POST /data/datasets/{}/label-changes). {how}. Nothing is lost by a change that is applied: the history of each row shows its tags, and the ledger names who decided.", ask.dataset_id), needs)
}

/// The evaluation of a change, built now.
fn evaluate(change: &Change, ask: &Ask, state: &SharedState) -> Result<Value, ServiceError> {
    let cats = state.sensitivity_categories();
    let access = access(ask, &cats, state)?;
    let work = work(ask, &cats, state)?;

    // What changes
    let (rows, examples) = match ask.kind {
        Kind::Tags => {
            let mut ids = ask.records.clone();
            ids.sort();
            let live = state.repository.label_rows(&ask.app, &ask.table, ids).map_err(storage)?;
            (live.len() as i64, live.into_iter().take(EXAMPLES).map(|(id, _, _)| id).collect::<Vec<_>>())
        }
        Kind::Floor => (state.repository.label_live_count(&ask.app, &ask.table).map_err(storage)?, state.repository.label_examples(&ask.app, &ask.table, EXAMPLES as i64).map_err(storage)?),
    };
    let (add, remove) = match ask.kind {
        Kind::Tags => (ask.add.clone(), ask.remove.clone()),
        Kind::Floor => (ask.floor_after.iter().filter(|c| !ask.floor_before.contains(c)).cloned().collect::<Vec<_>>(), ask.floor_before.iter().filter(|c| !ask.floor_after.contains(c)).cloned().collect::<Vec<_>>()),
    };
    let mut what = json!({
        "kind": change.kind, "rows": rows, "add": add, "remove": remove, "examples": examples,
        "fields": "A tag labels the whole row, so every field of each row", "values": "hidden",
    });
    if ask.kind == Kind::Floor {
        what["floor_from"] = json!(ask.floor_before);
        what["floor_to"] = json!(ask.floor_after);
    }

    // Why
    let finding = match change.finding_id {
        Some(id) => state.repository.get_finding(id).map_err(storage)?.map(|f| json!({ "id": f["id"], "field": f["field"], "category": f["category"], "match_count": f["match_count"], "record_count": f["record_count"] })),
        None => None,
    };
    let why = json!({ "source": change.source, "finding": finding, "reason": change.reason, "sentence": sentence(change, &finding) });

    // What depends on it
    let manifest_title = {
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        engine.resolve_by_slug(&ask.app).map(|m| m.title.clone()).unwrap_or_default()
    };
    let dependents = state.repository.label_dependents(&ask.app).map_err(storage)?;
    let cut = |v: Vec<Value>| -> Value { json!(v.into_iter().take(LISTED).collect::<Vec<_>>()) };
    let depends = json!({
        "apps": [{ "slug": ask.app, "title": manifest_title }],
        "processes": cut(work.processes.iter().map(|p| json!({ "id": p.id, "rule_id": p.rule_id })).collect()),
        "tasks": cut(work.tasks.iter().map(|t| json!({ "instance": t.instance, "step": t.step, "person": t.person })).collect()),
        "automations": cut(dependents.automations.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect()),
        "shares": cut(dependents.shares.iter().map(|(id, view, scope)| json!({ "id": id, "view_id": view, "scope": scope })).collect()),
        "exports": cut(dependents.exports.iter().map(|id| json!({ "job_id": id })).collect()),
        "not_yet_listed": "System users, app grants and row shares are later phases of the data layer and are not listed",
    });

    // Who would be locked out
    let locked = json!({
        "count": work.locked.len(),
        "people": work.locked.iter().take(LISTED).map(|(person, items)| json!({ "person": person, "work": items })).collect::<Vec<_>>(),
    });

    let options = json!([
        { "id": "now", "available": true, "text": "Apply now. People with open work on these rows lose their access when it applies" },
        { "id": "new_work_only", "available": true, "text": "Apply for new work only. The change is approved and held, and it applies when the open work on these rows has ended" },
        { "id": "date", "available": true, "min_days": schedule::min_days(ask, &cats, state), "text": format!("Apply on a date. Give `effective_at` at least {} days from now. A plan tells the people it affects, you approve it before anyone is told, and they may ask for an exception until the date", schedule::min_days(ask, &cats, state)) },
    ]);
    let (undo, undo_needs) = undo_text(ask, &cats);
    let removes = removes_protection(ask, &cats);
    let order = if work.locked.is_empty() {
        json!(["what_changes", "why", "access", "depends_on", "locked_out", "options", "undo"])
    } else {
        json!(["locked_out", "what_changes", "why", "access", "depends_on", "options", "undo"])
    };
    Ok(json!({
        "order": order,
        "locked_out": locked,
        "what_changes": what,
        "why": why,
        "access": access,
        "depends_on": depends,
        "options": options,
        "undo": { "text": undo, "approvals_needed": undo_needs, "who": "The dataset's steward, a Platform Admin or a compliance officer" },
        "decision": {
            "removes_protection": removes, "approvals_needed": approvals_needed(ask, &cats), "approved_by": change.approvals.iter().map(|a| a.by.clone()).collect::<Vec<_>>(),
            "open_work_choice_required": !work.locked.is_empty(),
        },
    }))
}

/// `GET /data/label-changes/{id}`: the change, and its evaluation built now.
pub fn get(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    let ask = ask_of(&change, &row, state)?;
    let cats = state.sensitivity_categories();
    let mut out = json!({ "change": change_json(&change, &ask, &cats), "evaluation": evaluate(&change, &ask, state)? });
    if let Some(s) = schedule::block(&change, state)? {
        out["schedule"] = s;
    }
    Ok(out)
}

/// `GET /data/label-changes`: the changes the caller may decide, newest first. Filters: `status`, `dataset`.
pub fn list(caller: &AuthUser, q: &HashMap<String, String>, state: &SharedState) -> Result<Value, ServiceError> {
    if let Some(s) = q.get("status") {
        if !["Pending", "Approved", "Scheduled", "Applied", "Declined", "Withdrawn"].contains(&s.as_str()) {
            return Err(bad("status is Pending, Approved, Scheduled, Applied, Declined or Withdrawn"));
        }
    }
    let admin = is_admin_or_compliance(caller, state);
    let all = state.repository.list_label_changes(q.get("status").cloned(), q.get("dataset").cloned(), 200).map_err(storage)?;
    let rows = state.repository.catalog_rows().map_err(storage)?;
    let cats = state.sensitivity_categories();
    let mut out = Vec::new();
    for change in all {
        let Some(row) = rows.iter().find(|r| r.id == change.dataset_id) else { continue };
        if !admin && !stewards_of(row, state).iter().any(|s| s.eq_ignore_ascii_case(&caller.eppn)) {
            continue;
        }
        let ask = ask_of(&change, row, state)?;
        out.push(change_json(&change, &ask, &cats));
    }
    Ok(json!({ "changes": out }))
}

// ---------------------------------------------------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------------------------------------------------

fn plan_of(ask: &Ask, state: &SharedState) -> Result<Plan, ServiceError> {
    let manifests = state.engine.read().map_err(|e| ServiceError::Internal(e.to_string()))?.list_manifests();
    let touched: BTreeSet<(String, String)> = std::iter::once((ask.app.clone(), ask.table.clone())).collect();
    let labels = state.labels.read().map_err(|e| ServiceError::Internal(e.to_string()))?.clone();
    let flag = flag_tables(state, &manifests, &touched, &labels).into_iter().next();
    let keep: Vec<String> = state.sensitivity_categories().iter().filter(|c| c.protected).map(|c| c.id.clone()).collect();
    let names = flag.map(|f| f.names).unwrap_or_default();
    Ok(match ask.kind {
        Kind::Floor => Plan::Floor { dataset_id: ask.dataset_id.clone(), app_slug: ask.app.clone(), table_id: ask.table.clone(), floor: ask.floor_after.clone(), names, keep },
        Kind::Tags => Plan::Tags { app_slug: ask.app.clone(), table_id: ask.table.clone(), ids: ask.records.clone(), add: ask.add.clone(), remove: ask.remove.clone(), names, keep },
    })
}

/// The ledger entry of an applied change: it names the proposer, the deciders, the finding and the choice about open work.
fn ledger_of(id: Uuid, ask: &Ask, by: &str, note: &str) -> Box<dyn FnOnce(&Done) -> AppendDecisionParams + Send> {
    let (by, note) = (by.to_string(), note.to_string());
    let (kind, app, dataset) = (ask.kind.clone(), ask.app.clone(), ask.dataset_id.clone());
    let (floor_after, add, remove, records) = (ask.floor_after.clone(), ask.add.clone(), ask.remove.clone(), ask.records.clone());
    Box::new(move |d: &Done| {
        let mut payload = json!({
            "change": id.to_string(), "dataset": dataset, "proposed_by": d.proposed_by, "decided_by": d.deciders, "finding": d.finding.map(|f| f.to_string()), "open_work": d.open_work,
        });
        match kind {
            Kind::Floor => {
                payload["setting"] = json!("label_floor");
                payload["from"] = json!(d.floor_was);
                payload["to"] = json!(floor_after);
            }
            Kind::Tags => {
                payload["setting"] = json!("row_tags");
                payload["add"] = json!(add);
                payload["remove"] = json!(remove);
                payload["rows"] = json!(records.len());
                payload["changed"] = json!(d.tagged.len());
                payload["examples"] = json!(d.tagged.iter().take(EXAMPLES).collect::<Vec<_>>());
            }
        }
        AppendDecisionParams { principal: by, organization_code: "DIV-ADMIN-CONSOLE".to_string(), app_slug: Some(app), decision_type: DecisionType::DataLabelChanged, oscal_control_id: "AC-03".to_string(), rationale: note, payload }
    })
}

fn applied(id: Uuid, done: &Done, open_work: &Option<String>) -> Value {
    json!({
        "id": id, "status": "Applied", "approvals": done.deciders.len(), "decided_by": done.deciders, "open_work": open_work,
        "rows_changed": done.tagged.len(), "floor": done.floor_was.as_ref().map(|_| "set"),
    })
}

fn outcome(id: Uuid, out: DecideOutcome, needs: usize, open_work: Option<String>) -> Result<Value, ServiceError> {
    match out {
        DecideOutcome::Missing => Err(ServiceError::NotFound("Label change not found".to_string())),
        DecideOutcome::NotPending(s) => Err(ServiceError::Conflict(format!("This change is {s}, so it cannot be decided now"))),
        DecideOutcome::Proposer => Err(ServiceError::forbidden("The person who proposed a change that removes protection cannot approve it. Two other people do")),
        DecideOutcome::Duplicate => Err(ServiceError::Conflict("You have approved this change already. A different person approves it next".to_string())),
        DecideOutcome::Waiting { approvals } => Ok(json!({ "id": id, "status": "Pending", "approvals": approvals, "approvals_needed": needs, "next": "It removes protection, so a second approver, who is not the proposer or the first approver, completes it" })),
        DecideOutcome::Held { approvals } => Ok(json!({ "id": id, "status": "Approved", "approvals": approvals, "approvals_needed": needs, "open_work": open_work, "next": "It is held. It applies when the open work has ended: POST /data/label-changes/{id}/apply" })),
        DecideOutcome::Scheduled { approvals } => Ok(json!({ "id": id, "status": "Scheduled", "approvals": approvals, "approvals_needed": needs, "open_work": open_work })),
        DecideOutcome::Applied { done } => Ok(applied(id, &done, &open_work)),
    }
}

/// `POST /data/label-changes/{id}/decide`. The body names `decision` (`approve`, `decline` or `change`) and a `reason`.
///
/// - `approve` adds the caller's approval. Raising protection needs one. Removing protection needs two, from two people, and the
///   proposer is neither. When the change would lock someone out of open work, `open_work` is required: `now`, or
///   `new_work_only`, which holds the change until the work has ended. Nothing decides it for the steward.
/// - `decline` closes it.
/// - `change` replaces what it asks (`add`, `remove`, `records`, or `floor`) and takes back every approval.
pub fn decide(caller: &AuthUser, id: &str, body: &Value, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    let reason = reason_of(body)?;
    let which = body.get("decision").and_then(Value::as_str).unwrap_or("");
    if !["approve", "decline", "change"].contains(&which) {
        return Err(bad("decision is approve, decline or change"));
    }
    if change.status != "Pending" && !(which == "decline" && change.status == "Approved") {
        return Err(ServiceError::Conflict(format!("This change is {}, so it cannot be decided now", change.status.to_lowercase())));
    }
    let cats = state.sensitivity_categories();
    let ask = ask_of(&change, &row, state)?;
    let needs = approvals_needed(&ask, &cats);

    match which {
        "decline" => match state.repository.decline_label_change(change.id, &caller.eppn, &reason).map_err(storage)? {
            None => Ok(json!({ "id": change.id, "status": "Declined" })),
            Some(s) => Err(ServiceError::Conflict(format!("This change is {}, so it cannot be decided now", s.to_lowercase()))),
        },
        "change" => {
            let kind = ask.kind.clone();
            let spec = spec_of(&kind, body, Some(&change.spec), &row, &ask.floor_before, state)?;
            match state.repository.amend_label_change(change.id, spec, &caller.eppn, &reason).map_err(storage)? {
                Ok(()) => Ok(json!({ "id": change.id, "status": "Pending", "approvals": 0, "next": "The approvals were taken back. Read the new evaluation, then approve" })),
                Err(s) => Err(ServiceError::Conflict(format!("This change is {}, so it cannot be changed now", s.to_lowercase()))),
            }
        }
        _ => {
            if needs == 2 && change.proposed_by.eq_ignore_ascii_case(&caller.eppn) {
                return Err(ServiceError::forbidden("The person who proposed a change that removes protection cannot approve it. Two other people do"));
            }
            // A date: the notice period is checked now, and the change is held for the date once the approvals are complete.
            let kept = change.effective_at.as_deref().filter(|_| change.status == "Pending");
            let effective = schedule::effective_of(body, kept, &ask, &cats, state, chrono::Utc::now())?;
            let open_work = match (&effective, open_work_of(body)?) {
                (Some(_), None) => Some("date".to_string()),
                (Some(_), Some(w)) if w == "date" => Some(w),
                (Some(_), Some(_)) => return Err(bad("A date and `open_work` of `now` or `new_work_only` cannot both be asked. Give `effective_at` alone")),
                (None, Some(w)) if w == "date" => return Err(bad("Apply on a date needs `effective_at`, a date and time such as 2026-12-30T09:00:00Z")),
                (None, w) => w,
            };
            let locked = work(&ask, &cats, state)?.locked.len();
            if locked > 0 && open_work.is_none() {
                return Err(bad(format!("{locked} people would be locked out of open work. Say how to treat it: open_work is `now` or `new_work_only`, or give `effective_at` to apply on a date. No default decides this for you")));
            }
            let hold = locked > 0 && open_work.as_deref() == Some("new_work_only");
            let complete = change.approvals.iter().filter(|a| !a.by.eq_ignore_ascii_case(&caller.eppn)).count() + 1 >= needs;
            let (held, described) = match &effective {
                Some(at) if complete => {
                    let (held, described) = schedule::hold(caller, &change, &ask, &row, *at, &reason, state)?;
                    (Some(held), Some(described))
                }
                _ => (None, None),
            };
            let req = DecideReq {
                id: change.id,
                by: caller.eppn.clone(),
                note: reason.clone(),
                open_work: open_work.clone(),
                needs,
                hold,
                release: false,
                scheduled: false,
                schedule: held,
                date: effective.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                plan: plan_of(&ask, state)?,
                ledger: ledger_of(change.id, &ask, &caller.eppn, &reason),
            };
            let out = state.repository.decide_label_change(req).map_err(storage)?;
            let scheduled = matches!(out, DecideOutcome::Scheduled { .. });
            let mut answer = outcome(change.id, out, needs, open_work)?;
            if let (true, Some(d)) = (scheduled, described) {
                if let (Some(a), Some(d)) = (answer.as_object_mut(), d.as_object()) {
                    a.extend(d.clone());
                }
            }
            Ok(answer)
        }
    }
}

fn open_work_of(body: &Value) -> Result<Option<String>, ServiceError> {
    match body.get("open_work") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s == "now" || s == "new_work_only" || s == "date" => Ok(Some(s.clone())),
        Some(_) => Err(bad("open_work is `now`, `new_work_only` or `date`")),
    }
}

/// `POST /data/label-changes/{id}/apply`: applies a change that was approved and held for new work only, once the open work on its rows has
/// ended. It adds no approval.
pub fn apply(caller: &AuthUser, id: &str, state: &SharedState) -> Result<Value, ServiceError> {
    let (change, row) = load(caller, id, state)?;
    if change.status != "Approved" {
        return Err(ServiceError::Conflict(format!("This change is {}. Only a change that was approved and held is applied here", change.status.to_lowercase())));
    }
    let cats = state.sensitivity_categories();
    let ask = ask_of(&change, &row, state)?;
    let locked = work(&ask, &cats, state)?.locked.len();
    if locked > 0 {
        return Err(ServiceError::Conflict(format!("{locked} people still have open work that this change would stop. It applies when that work has ended")));
    }
    let note = format!("Applied after the open work ended. {}", change.decision_note.clone().unwrap_or_default());
    let req = DecideReq {
        id: change.id,
        by: caller.eppn.clone(),
        note: note.chars().take(500).collect(),
        open_work: change.open_work.clone(),
        needs: 0,
        hold: false,
        release: true,
        scheduled: false,
        schedule: None,
        date: None,
        plan: plan_of(&ask, state)?,
        ledger: ledger_of(change.id, &ask, &caller.eppn, &note.chars().take(500).collect::<String>()),
    };
    let out = state.repository.decide_label_change(req).map_err(storage)?;
    outcome(change.id, out, 0, change.open_work.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(kind: Kind, before: &[&str], after: &[&str], add: &[&str], remove: &[&str]) -> Ask {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        Ask { kind, app: "a".into(), table: "t".into(), dataset_id: "ds".into(), floor_before: v(before), floor_after: v(after), add: v(add), remove: v(remove), records: vec!["r1".into()] }
    }

    fn cat(id: &str, protected: bool) -> Category {
        Category { id: id.to_string(), name: id.to_string(), source: String::new(), protected, detectors: vec![], default_mask: Default::default(), requires: vec![] }
    }

    #[test]
    fn raising_needs_one_approver_and_removing_protection_needs_two() {
        let cats = [cat("phi", true), cat("open", false)];
        assert_eq!(approvals_needed(&ask(Kind::Tags, &[], &[], &["phi"], &[]), &cats), 1);
        assert_eq!(approvals_needed(&ask(Kind::Tags, &[], &[], &[], &["phi"]), &cats), 2);
        assert_eq!(approvals_needed(&ask(Kind::Tags, &[], &[], &[], &["open"]), &cats), 1, "a tag that protects nothing is removed by one steward");
        assert_eq!(approvals_needed(&ask(Kind::Tags, &[], &[], &[], &["gone"]), &cats), 2, "a category settings dropped is held to protect");
        assert_eq!(approvals_needed(&ask(Kind::Floor, &["phi"], &["phi", "open"], &[], &[]), &cats), 1);
        assert_eq!(approvals_needed(&ask(Kind::Floor, &["phi", "open"], &["phi"], &[], &[]), &cats), 2, "any category leaving a floor is a lowering");
    }

    #[test]
    fn a_change_to_tags_works_out_the_tags_a_row_would_have() {
        let a = ask(Kind::Tags, &[], &[], &["phi"], &["hr"]);
        assert_eq!(a.tags_after(&["hr".to_string(), "open".to_string()]), vec!["open".to_string(), "phi".to_string()]);
        let f = ask(Kind::Floor, &[], &["phi"], &[], &[]);
        assert_eq!(f.tags_after(&["open".to_string()]), vec!["open".to_string()], "a floor change leaves a row's tags alone");
        assert!(a.touches("r1") && !a.touches("r2") && f.touches("anything"));
    }
}
