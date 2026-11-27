//! Row shares (data layer phase 10): the requests in `row_shares` and the lookup in `row_share_rows`.
//!
//! A share names rows, so both tables are behind the data door. The header holds the request, the decision and the date. The lookup
//! holds one row for each row an approved share covers, keyed by the grantee, and is the only thing a read consults. A read asks
//! whether the header is approved and its date is ahead of the database's clock, so a share ends by its own terms and no job is
//! needed for access. Nothing here decides who may share or approve. The server checks that, and writes the ledger entry in the same
//! transaction.

use postgres::GenericClient;
use serde_json::Value;
use uuid::Uuid;

const STAMP: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'";

fn at(column: &str) -> String {
    format!("to_char({column} AT TIME ZONE 'UTC', {STAMP})")
}

fn columns() -> String {
    format!(
        "id, dataset_id, app_slug, table_id, records, grantee, access, fields, kind, categories, justification, requested_by, {}, {}, assessment, status, \
         decided_by, {}, decision_note, {}",
        at("requested_at"),
        at("expires_at"),
        at("decided_at"),
        at("ended_at")
    )
}

/// A share as stored.
#[derive(Debug, Clone)]
pub struct Share {
    pub id: Uuid,
    pub dataset_id: String,
    pub app_slug: String,
    pub table_id: String,
    pub records: Vec<String>,
    pub grantee: String,
    pub access: String,
    pub fields: Vec<String>,
    pub kind: String,
    pub categories: Vec<String>,
    pub justification: String,
    pub requested_by: String,
    pub requested_at: String,
    pub expires_at: String,
    pub assessment: Value,
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
    pub ended_at: Option<String>,
}

fn share_of(r: &postgres::Row) -> Share {
    Share {
        id: r.get(0),
        dataset_id: r.get(1),
        app_slug: r.get(2),
        table_id: r.get(3),
        records: r.get(4),
        grantee: r.get(5),
        access: r.get(6),
        fields: r.get(7),
        kind: r.get(8),
        categories: r.get(9),
        justification: r.get(10),
        requested_by: r.get(11),
        requested_at: r.get(12),
        expires_at: r.get(13),
        assessment: r.get(14),
        status: r.get(15),
        decided_by: r.get(16),
        decided_at: r.get(17),
        decision_note: r.get(18),
        ended_at: r.get(19),
    }
}

/// What a request holds.
pub struct NewShare<'a> {
    pub id: Uuid,
    pub dataset_id: &'a str,
    pub app_slug: &'a str,
    pub table_id: &'a str,
    pub records: &'a [String],
    pub grantee: &'a str,
    pub access: &'a str,
    pub fields: &'a [String],
    pub kind: &'a str,
    pub categories: &'a [String],
    pub justification: &'a str,
    pub requested_by: &'a str,
    /// RFC 3339.
    pub expires_at: &'a str,
    pub assessment: &'a Value,
}

pub fn insert<C: GenericClient>(c: &mut C, n: &NewShare<'_>) -> Result<(), postgres::Error> {
    c.execute(
        "INSERT INTO row_shares (id, dataset_id, app_slug, table_id, records, grantee, access, fields, kind, categories, justification, requested_by, expires_at, assessment) \
         VALUES ($1, $2, $3, $4, $5, lower($6), $7, $8, $9, $10, $11, lower($12), $13::text::timestamptz, $14)",
        &[&n.id, &n.dataset_id, &n.app_slug, &n.table_id, &n.records, &n.grantee, &n.access, &n.fields, &n.kind, &n.categories, &n.justification, &n.requested_by, &n.expires_at, n.assessment],
    )?;
    Ok(())
}

pub fn get<C: GenericClient>(c: &mut C, id: Uuid) -> Result<Option<Share>, postgres::Error> {
    Ok(c.query_opt(&format!("SELECT {} FROM row_shares WHERE id = $1", columns()), &[&id])?.as_ref().map(share_of))
}

pub fn get_for_update<C: GenericClient>(tx: &mut C, id: Uuid) -> Result<Option<Share>, postgres::Error> {
    Ok(tx.query_opt(&format!("SELECT {} FROM row_shares WHERE id = $1 FOR UPDATE", columns()), &[&id])?.as_ref().map(share_of))
}

/// Which shares a list reads. `Live` is approved and not yet at its date, by the database's clock.
pub enum Which<'a> {
    Live,
    Status(&'a str),
}

/// The shares of a dataset, newest first, optionally only those that name `record`.
pub fn list<C: GenericClient>(c: &mut C, dataset_id: &str, which: Which<'_>, record: Option<&str>, limit: i64) -> Result<Vec<Share>, postgres::Error> {
    let (status, live) = match which {
        Which::Live => ("Approved", true),
        Which::Status(s) => (s, false),
    };
    let rows = c.query(
        &format!(
            "SELECT {} FROM row_shares WHERE dataset_id = $1 AND status = $2 AND ($3 = FALSE OR expires_at > NOW()) AND ($4::text IS NULL OR $4 = ANY(records)) \
             ORDER BY requested_at DESC, id LIMIT $5",
            columns()
        ),
        &[&dataset_id, &status, &live, &record, &limit],
    )?;
    Ok(rows.iter().map(share_of).collect())
}

/// Approves a waiting request: the status, and one lookup row for each row it covers. False when it was not waiting.
pub fn approve<C: GenericClient>(tx: &mut C, share: &Share, by: &str, note: &str) -> Result<bool, postgres::Error> {
    let done = tx.execute(
        "UPDATE row_shares SET status = 'Approved', decided_by = lower($2), decided_at = NOW(), decision_note = $3 WHERE id = $1 AND status = 'Requested'",
        &[&share.id, &by, &note],
    )?;
    if done == 0 {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO row_share_rows (share_id, grantee, app_slug, table_id, record_id) SELECT $1, $2, $3, $4, r FROM unnest($5::text[]) r ON CONFLICT DO NOTHING",
        &[&share.id, &share.grantee, &share.app_slug, &share.table_id, &share.records],
    )?;
    Ok(true)
}

/// Closes a waiting request with a status that grants nothing (`Declined`, `Withdrawn` or `Lapsed`). False when it was not waiting.
pub fn close_request<C: GenericClient>(tx: &mut C, id: Uuid, status: &str, by: &str, note: &str) -> Result<bool, postgres::Error> {
    Ok(tx.execute(
        "UPDATE row_shares SET status = $2, decided_by = lower($3), decided_at = NOW(), decision_note = $4, ended_at = NOW() WHERE id = $1 AND status = 'Requested'",
        &[&id, &status, &by, &note],
    )? == 1)
}

/// Ends an approved share, by its date (`Ended`) or early (`Withdrawn`), and removes its lookup rows. False when it was not approved.
pub fn end<C: GenericClient>(tx: &mut C, id: Uuid, status: &str, by: Option<&str>, note: &str) -> Result<bool, postgres::Error> {
    let done = tx.execute(
        "UPDATE row_shares SET status = $2, ended_at = NOW(), decision_note = CASE WHEN $3::text IS NULL THEN decision_note ELSE $4 END, \
         decided_by = coalesce(lower($3), decided_by) WHERE id = $1 AND status = 'Approved'",
        &[&id, &status, &by, &note],
    )?;
    if done == 0 {
        return Ok(false);
    }
    tx.execute("DELETE FROM row_share_rows WHERE share_id = $1", &[&id])?;
    Ok(true)
}

/// The approved shares whose date has come at `now` (RFC 3339), oldest date first.
pub fn due<C: GenericClient>(c: &mut C, now: &str, limit: i64) -> Result<Vec<Uuid>, postgres::Error> {
    Ok(c.query("SELECT id FROM row_shares WHERE status = 'Approved' AND expires_at <= $1::text::timestamptz ORDER BY expires_at, id LIMIT $2", &[&now, &limit])?.iter().map(|r| r.get(0)).collect())
}

/// Closes the requests nobody decided before their date. Returns how many.
pub fn lapse<C: GenericClient>(c: &mut C, now: &str) -> Result<u64, postgres::Error> {
    c.execute(
        "UPDATE row_shares SET status = 'Lapsed', ended_at = NOW(), decision_note = 'Nobody decided this before its date' WHERE status = 'Requested' AND expires_at <= $1::text::timestamptz",
        &[&now],
    )
}

/// Shares that ended and whose owners were not told yet.
pub fn unnotified<C: GenericClient>(c: &mut C, limit: i64) -> Result<Vec<Share>, postgres::Error> {
    Ok(c.query(&format!("SELECT {} FROM row_shares WHERE status = 'Ended' AND end_notified_at IS NULL ORDER BY ended_at, id LIMIT $1", columns()), &[&limit])?.iter().map(share_of).collect())
}

pub fn mark_notified<C: GenericClient>(c: &mut C, id: Uuid) -> Result<(), postgres::Error> {
    c.execute("UPDATE row_shares SET end_notified_at = NOW() WHERE id = $1 AND end_notified_at IS NULL", &[&id])?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The read path
// ---------------------------------------------------------------------------------------------------------------------

/// A condition on a statement over `dataset_records`, true for a row an approved share that has not reached its date covers for the
/// grantee, and whose fields hold every one in `needs`. `id_expr` names the row's id, and `grantee`, `app`, `table` and `needs` are the
/// placeholders the caller bound (`needs` as text[]). It is a membership in a small keyed lookup, so a list of the whole table pays one
/// probe for each row it tests, and the planner hashes the lookup once.
pub fn live_sql(id_expr: &str, grantee: &str, app: &str, table: &str, needs: &str) -> String {
    format!(
        "{id_expr} IN (SELECT s.record_id FROM row_share_rows s JOIN row_shares h ON h.id = s.share_id \
         WHERE s.grantee = {grantee} AND s.app_slug = {app} AND s.table_id = {table} AND h.status = 'Approved' AND h.expires_at > NOW() AND h.fields @> {needs})"
    )
}

/// Whether the grantee holds a live share on this table. A reader with none adds nothing to a statement.
pub fn has_live<C: GenericClient>(c: &mut C, grantee: &str, app_slug: &str, table_id: &str) -> Result<bool, postgres::Error> {
    Ok(c.query_one(
        "SELECT EXISTS (SELECT 1 FROM row_share_rows s JOIN row_shares h ON h.id = s.share_id \
         WHERE s.grantee = $1 AND s.app_slug = $2 AND s.table_id = $3 AND h.status = 'Approved' AND h.expires_at > NOW())",
        &[&grantee, &app_slug, &table_id],
    )?
    .get(0))
}

/// A live share on one row for the grantee: what it lets them do and the fields it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cover {
    pub record_id: String,
    pub write: bool,
    pub fields: Vec<String>,
}

/// The live shares the grantee holds on these rows of the app, one entry for each share and row.
pub fn covers<C: GenericClient>(c: &mut C, grantee: &str, app_slug: &str, ids: &[String]) -> Result<Vec<Cover>, postgres::Error> {
    let rows = c.query(
        "SELECT s.record_id, h.access = 'write', h.fields FROM row_share_rows s JOIN row_shares h ON h.id = s.share_id \
         WHERE s.grantee = $1 AND s.app_slug = $2 AND s.record_id = ANY($3) AND h.status = 'Approved' AND h.expires_at > NOW() ORDER BY s.record_id, h.id",
        &[&grantee, &app_slug, &ids],
    )?;
    Ok(rows.iter().map(|r| Cover { record_id: r.get(0), write: r.get(1), fields: r.get(2) }).collect())
}
