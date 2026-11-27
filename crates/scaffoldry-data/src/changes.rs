//! Label changes (data layer phase 6): the proposals in `label_changes`, and the reads and writes of rows they need.
//!
//! A proposal names rows, so its table is behind the data door. Reads for the evaluation (the units and tags of the rows a change
//! touches) and the one write a decision makes to rows (their tags) live here too. Nothing here decides who may do what. The server
//! checks that, and builds the ledger entry in the same transaction.

use postgres::GenericClient;
use serde_json::{json, Value};
use uuid::Uuid;

const COLUMNS: &str = "id, dataset_id, kind, spec, reason, source, finding_id, proposed_by, proposed_at::text, status, approvals, amended, open_work, decided_by, decided_at::text, decision_note, \
    to_char(effective_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), plan, plan_approved_by, to_char(plan_approved_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')";

/// A proposal as stored.
#[derive(Debug, Clone)]
pub struct Change {
    pub id: Uuid,
    pub dataset_id: String,
    pub kind: String,
    pub spec: Value,
    pub reason: String,
    pub source: String,
    pub finding_id: Option<Uuid>,
    pub proposed_by: String,
    pub proposed_at: String,
    pub status: String,
    pub approvals: Vec<Approval>,
    pub amended: Value,
    pub open_work: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
    /// Phase 6b: the date a scheduled change applies, as UTC text. `None` for a change that applies when it is decided.
    pub effective_at: Option<String>,
    /// Phase 6b: the communications plan the steward reads, as the schema `label-change-plan.schema.json` describes it.
    pub plan: Option<Value>,
    pub plan_approved_by: Option<String>,
    pub plan_approved_at: Option<String>,
}

/// One approval: who, when, and the note they gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub by: String,
    pub at: String,
    pub note: String,
}

impl Approval {
    pub fn to_json(&self) -> Value {
        json!({ "by": self.by, "at": self.at, "note": self.note })
    }
}

fn approvals_of(v: &Value) -> Vec<Approval> {
    let text = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    v.as_array().map(|l| l.iter().map(|a| Approval { by: text(a, "by"), at: text(a, "at"), note: text(a, "note") }).collect()).unwrap_or_default()
}

fn change_of(r: &postgres::Row) -> Change {
    Change {
        id: r.get(0),
        dataset_id: r.get(1),
        kind: r.get(2),
        spec: r.get(3),
        reason: r.get(4),
        source: r.get(5),
        finding_id: r.get(6),
        proposed_by: r.get(7),
        proposed_at: r.get(8),
        status: r.get(9),
        approvals: approvals_of(&r.get::<_, Value>(10)),
        amended: r.get(11),
        open_work: r.get(12),
        decided_by: r.get(13),
        decided_at: r.get(14),
        decision_note: r.get(15),
        effective_at: r.get(16),
        plan: r.get(17),
        plan_approved_by: r.get(18),
        plan_approved_at: r.get(19),
    }
}

/// What a new proposal holds.
pub struct NewChange<'a> {
    pub dataset_id: &'a str,
    pub kind: &'a str,
    pub spec: &'a Value,
    pub reason: &'a str,
    pub source: &'a str,
    pub finding_id: Option<Uuid>,
    pub proposed_by: &'a str,
}

/// Stores a proposal. It applies nothing. Returns its id. A finding that already has a live proposal is refused by the unique index.
pub fn insert<C: GenericClient>(c: &mut C, n: &NewChange<'_>) -> Result<Uuid, postgres::Error> {
    let id = Uuid::new_v4();
    c.execute(
        "INSERT INTO label_changes (id, dataset_id, kind, spec, reason, source, finding_id, proposed_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[&id, &n.dataset_id, &n.kind, n.spec, &n.reason, &n.source, &n.finding_id, &n.proposed_by],
    )?;
    Ok(id)
}

pub fn get<C: GenericClient>(c: &mut C, id: Uuid) -> Result<Option<Change>, postgres::Error> {
    Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM label_changes WHERE id = $1"), &[&id])?.as_ref().map(change_of))
}

/// One proposal, locked for the decision that follows.
pub fn get_for_update<C: GenericClient>(tx: &mut C, id: Uuid) -> Result<Option<Change>, postgres::Error> {
    Ok(tx.query_opt(&format!("SELECT {COLUMNS} FROM label_changes WHERE id = $1 FOR UPDATE"), &[&id])?.as_ref().map(change_of))
}

/// The proposals, newest first. `status` and `dataset` filter when given.
pub fn list<C: GenericClient>(c: &mut C, status: Option<&str>, dataset: Option<&str>, limit: i64) -> Result<Vec<Change>, postgres::Error> {
    Ok(c.query(
        &format!("SELECT {COLUMNS} FROM label_changes WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR dataset_id = $2) ORDER BY proposed_at DESC, id LIMIT $3"),
        &[&status, &dataset, &limit],
    )?
    .iter()
    .map(change_of)
    .collect())
}

/// Replaces the approvals.
pub fn set_approvals<C: GenericClient>(tx: &mut C, id: Uuid, approvals: &[Approval]) -> Result<(), postgres::Error> {
    let list = Value::Array(approvals.iter().map(Approval::to_json).collect());
    tx.execute("UPDATE label_changes SET approvals = $2 WHERE id = $1", &[&id, &list])?;
    Ok(())
}

/// Changes what is asked and takes back every approval. The old ask is kept in `amended`.
pub fn amend<C: GenericClient>(tx: &mut C, id: Uuid, spec: &Value, by: &str, note: &str) -> Result<(), postgres::Error> {
    tx.execute(
        "UPDATE label_changes SET amended = amended || jsonb_build_array(jsonb_build_object('by', $3::text, 'at', NOW()::text, 'note', $4::text, 'was', spec)), \
         spec = $2, approvals = '[]'::jsonb, effective_at = NULL WHERE id = $1",
        &[&id, spec, &by, &note],
    )?;
    Ok(())
}

/// Closes a proposal as `Declined`, or `Applied`, or holds it as `Approved`. `decided_by` is the person who completed it.
pub fn set_status<C: GenericClient>(tx: &mut C, id: Uuid, status: &str, decided_by: &str, note: &str, open_work: Option<&str>) -> Result<(), postgres::Error> {
    tx.execute(
        "UPDATE label_changes SET status = $2, decided_by = $3, decided_at = NOW(), decision_note = $4, open_work = COALESCE($5, open_work) WHERE id = $1",
        &[&id, &status, &decided_by, &note, &open_work],
    )?;
    Ok(())
}

/// Holds an approved change for its date (phase 6b): the status becomes `Scheduled`, with the date and the plan the steward will read.
/// `at` is RFC 3339 text. The plan is approved separately, and nothing is sent before then.
pub fn schedule<C: GenericClient>(tx: &mut C, id: Uuid, at: &str, plan: &Value, decided_by: &str, note: &str) -> Result<(), postgres::Error> {
    tx.execute(
        "UPDATE label_changes SET status = 'Scheduled', effective_at = $2::text::timestamptz, plan = $3, open_work = 'date', decided_by = $4, decided_at = NOW(), decision_note = $5 WHERE id = $1",
        &[&id, &at, plan, &decided_by, &note],
    )?;
    Ok(())
}

/// Keeps the date a first approver asked for while the change waits for a second. The approval that completes the change takes it unless
/// that approver names another. `amend` clears it.
pub fn set_effective<C: GenericClient>(tx: &mut C, id: Uuid, at: &str) -> Result<(), postgres::Error> {
    tx.execute("UPDATE label_changes SET effective_at = $2::text::timestamptz WHERE id = $1 AND status = 'Pending'", &[&id, &at])?;
    Ok(())
}

/// Records that the steward approved the plan. False when it was approved already, so a second call sends nothing new.
pub fn approve_plan<C: GenericClient>(tx: &mut C, id: Uuid, by: &str) -> Result<bool, postgres::Error> {
    Ok(tx.execute("UPDATE label_changes SET plan_approved_by = $2, plan_approved_at = NOW() WHERE id = $1 AND plan_approved_at IS NULL", &[&id, &by])? == 1)
}

/// The scheduled changes whose plan is approved and whose date is `now` or earlier, oldest first. `now` is RFC 3339 text. A change
/// whose plan was never approved is left alone, because nobody was told.
pub fn due<C: GenericClient>(c: &mut C, now: &str, limit: i64) -> Result<Vec<Uuid>, postgres::Error> {
    Ok(c.query(
        "SELECT id FROM label_changes WHERE status = 'Scheduled' AND plan_approved_at IS NOT NULL AND effective_at <= $1::text::timestamptz ORDER BY effective_at, id LIMIT $2",
        &[&now, &limit],
    )?
    .iter()
    .map(|r| r.get(0))
    .collect())
}

/// Whether a live row of the table that is not one of `except` already carries one of `tags`. An exception that waives a category for a
/// dataset would open such a row, so the steward is told it cannot be granted.
pub fn others_carry<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, tags: &[String], except: &[String]) -> Result<bool, postgres::Error> {
    Ok(c.query_one(
        "SELECT EXISTS (SELECT 1 FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND tags && $3 AND NOT (id = ANY($4)))",
        &[&app_slug, &table_id, &tags, &except],
    )?
    .get(0))
}

// ---------------------------------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------------------------------

/// A set of live rows that share a unit and a tag list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub unit: Option<Uuid>,
    pub tags: Vec<String>,
    pub rows: i64,
}

/// The live rows of a table grouped by unit and tags. With `ids` it reads only those rows, by key. Without, it reads the whole table
/// in one pass, so it costs a scan of the table and one row for each distinct pair.
pub fn groups<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, ids: Option<&[String]>) -> Result<Vec<Group>, postgres::Error> {
    let rows = c.query(
        "SELECT unit_id, tags, count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($3::text[] IS NULL OR id = ANY($3)) \
         GROUP BY unit_id, tags ORDER BY unit_id NULLS FIRST, tags",
        &[&app_slug, &table_id, &ids],
    )?;
    Ok(rows.iter().map(|r| Group { unit: r.get(0), tags: r.get(1), rows: r.get(2) }).collect())
}

/// A live row's id, its unit and its tags.
pub type RowLabel = (String, Option<Uuid>, Vec<String>);

/// Each named live row of the table with its unit and tags. An id that names no live row of the table is left out.
pub fn rows_of<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, ids: &[String]) -> Result<Vec<RowLabel>, postgres::Error> {
    let rows = c.query("SELECT id, unit_id, tags FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND id = ANY($3) ORDER BY id", &[&app_slug, &table_id, &ids])?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
}

/// How many live rows a table holds.
pub fn live_count<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<i64, postgres::Error> {
    Ok(c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL", &[&app_slug, &table_id])?.get(0))
}

/// The ids of the first rows of a table, in paging order. A few examples. Never a value.
pub fn examples<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str, n: i64) -> Result<Vec<String>, postgres::Error> {
    Ok(c.query("SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL ORDER BY created_at, id LIMIT $3", &[&app_slug, &table_id, &n])?.iter().map(|r| r.get(0)).collect())
}

/// A row whose tags a decision changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    pub id: String,
    pub version: i32,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

/// Adds and removes tags on the named live rows of a table, in the caller's transaction. Each row is locked, rewritten with its new
/// `_tags` (always named, so the trigger copies exactly that list), and its version raised. A row whose tags would not change is left
/// alone. The caller writes the history rows, sets the stored flags again and appends the ledger entry in the same transaction.
pub fn set_tags<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, ids: &[String], add: &[String], remove: &[String]) -> Result<Vec<Tagged>, postgres::Error> {
    let mut out = Vec::new();
    for id in ids {
        let Some(row) = tx.query_opt("SELECT data, version, tags FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND id = $3 AND deleted_at IS NULL FOR UPDATE", &[&app_slug, &table_id, id])? else { continue };
        let (mut data, version, before): (Value, i32, Vec<String>) = (row.get(0), row.get(1), row.get(2));
        let mut after: Vec<String> = before.iter().filter(|t| !remove.contains(t)).cloned().chain(add.iter().cloned()).collect();
        after.sort();
        after.dedup();
        if after == before {
            continue;
        }
        if let Some(map) = data.as_object_mut() {
            map.insert("_tags".to_string(), json!(after));
        }
        let now: i32 = tx.query_one("UPDATE dataset_records SET data = $3, version = version + 1 WHERE app_slug = $1 AND id = $2 RETURNING version", &[&app_slug, id, &data])?.get(0);
        debug_assert_eq!(now, version + 1);
        out.push(Tagged { id: id.clone(), version: now, before, after });
    }
    Ok(out)
}
