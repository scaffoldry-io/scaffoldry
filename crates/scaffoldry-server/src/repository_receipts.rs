//! Access model phase 8: receipts and the batches that seal them. A receipt holds ids and never values. A batch is the hash
//! of the receipts sealed into it, chained to the batch before, so an edit to a receipt or a missing batch shows.

use crate::repository::{PostgresRepository, RepositoryError};
use crate::service::receipts::NewReceipt;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The hash of the batch before the first.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One receipt as a line of text. A batch hashes these, so the text is the same on every machine: the time in UTC to the
/// microsecond, lists joined with commas, and the counters as the database prints them.
const LINE: &str = "concat_ws('|', id::text, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US'), actor, coalesce(acting_as, ''), action, \
    object_kind, object_id, outcome, array_to_string(grant_ids, ','), array_to_string(policy_ids, ','), array_to_string(narrowed_by, ','), counters::text)";

#[derive(Debug, Clone, Serialize)]
pub struct ReceiptRow {
    pub id: i64,
    pub at: String,
    pub actor: String,
    pub acting_as: Option<String>,
    pub action: String,
    pub object_kind: String,
    pub object_id: String,
    pub outcome: String,
    pub grant_ids: Vec<i64>,
    pub policy_ids: Vec<String>,
    pub narrowed_by: Vec<String>,
    pub counters: Value,
    pub batch_id: Option<i64>,
}

const COLUMNS: &str = "id, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), actor, acting_as, action, object_kind, object_id, outcome, grant_ids, policy_ids, narrowed_by, counters, batch_id";

fn row_of(r: &postgres::Row) -> ReceiptRow {
    ReceiptRow {
        id: r.get(0),
        at: r.get(1),
        actor: r.get(2),
        acting_as: r.get(3),
        action: r.get(4),
        object_kind: r.get(5),
        object_id: r.get(6),
        outcome: r.get(7),
        grant_ids: r.get(8),
        policy_ids: r.get(9),
        narrowed_by: r.get(10),
        counters: r.get(11),
        batch_id: r.get(12),
    }
}

/// The filters of the receipt list.
#[derive(Debug, Default, Clone)]
pub struct ReceiptFilter {
    pub actor: Option<String>,
    /// An object id, or an app slug to include its tables.
    pub object: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// Receipts older than this id.
    pub before: Option<i64>,
    pub limit: i64,
}

/// A sealed batch.
#[derive(Debug, Clone, Serialize)]
pub struct Batch {
    pub id: i64,
    pub first_id: i64,
    pub last_id: i64,
    pub hash: String,
    pub previous_hash: String,
    pub receipts: usize,
}

/// What a check of the chain found.
#[derive(Debug, Clone, Serialize)]
pub struct Verification {
    pub ok: bool,
    pub batches_checked: usize,
    pub failed_batch: Option<i64>,
    pub reason: Option<String>,
}

fn batch_hash(previous: &str, lines: &[String]) -> String {
    let mut h = Sha256::new();
    h.update(previous.as_bytes());
    for line in lines {
        h.update(b"\n");
        h.update(line.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

const INSERT_SQL: &str = "INSERT INTO access_receipts (actor, acting_as, action, object_kind, object_id, outcome, grant_ids, policy_ids, narrowed_by, counters) \
     SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, COALESCE((SELECT jsonb_object_agg(name, version) FROM cache_versions), '{}'::jsonb) RETURNING id";

impl PostgresRepository {
    /// Writes one receipt. The counters in force are read in the same statement, and the statement is prepared once for the
    /// connection, so a receipt costs one round trip and a durable commit. The median of 1,000 is 0.76 to 0.82 ms here.
    pub fn insert_access_receipt(&self, r: &NewReceipt) -> Result<i64, RepositoryError> {
        thread_local! {
            // The statement is prepared once for the worker's connection, so a write is one round trip, not two.
            static INSERT: std::cell::RefCell<Option<postgres::Statement>> = const { std::cell::RefCell::new(None) };
        }
        let r = r.clone();
        self.with_client(move |c| {
            for attempt in 0..2 {
                let prepared = INSERT.with(|s| s.borrow().clone());
                let statement = match prepared {
                    Some(s) => s,
                    None => {
                        let s = c.prepare(INSERT_SQL)?;
                        INSERT.with(|cell| *cell.borrow_mut() = Some(s.clone()));
                        s
                    }
                };
                match c.query_one(&statement, &[&r.actor, &r.acting_as, &r.action, &r.object_kind, &r.object_id, &r.outcome, &r.grant_ids, &r.policy_ids, &r.narrowed_by]) {
                    Ok(row) => return Ok(row.get(0)),
                    // A statement from a connection that was replaced is gone. Prepare it again, once.
                    Err(_) if attempt == 0 => INSERT.with(|cell| *cell.borrow_mut() = None),
                    Err(e) => return Err(e.into()),
                }
            }
            unreachable!("the second attempt returns")
        })
    }

    /// The cache counters in force now.
    pub fn cache_counters(&self) -> Result<Value, RepositoryError> {
        self.with_client(|c| Ok(c.query_one("SELECT COALESCE(jsonb_object_agg(name, version), '{}'::jsonb) FROM cache_versions", &[])?.get(0)))
    }

    pub fn access_receipt(&self, id: i64) -> Result<Option<ReceiptRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM access_receipts WHERE id = $1"), &[&id])?.as_ref().map(row_of)))
    }

    /// Newest first. One more than `limit` comes back when there is a next page.
    pub fn list_access_receipts(&self, f: &ReceiptFilter) -> Result<Vec<ReceiptRow>, RepositoryError> {
        let f = f.clone();
        self.with_client(move |c| {
            let rows = c.query(
                &format!(
                    "SELECT {COLUMNS} FROM access_receipts WHERE ($1::text IS NULL OR lower(actor) = lower($1)) \
                     AND ($2::text IS NULL OR object_id = $2 OR object_id LIKE $2 || '/%') \
                     AND ($3::text IS NULL OR outcome = $3) \
                     AND ($4::text IS NULL OR at >= $4::text::timestamptz) AND ($5::text IS NULL OR at < $5::text::timestamptz) \
                     AND ($6::bigint IS NULL OR id < $6) ORDER BY id DESC LIMIT $7"
                ),
                &[&f.actor, &f.object, &f.outcome, &f.since, &f.until, &f.before, &(f.limit + 1)],
            )?;
            Ok(rows.iter().map(row_of).collect())
        })
    }

    /// Seals the receipts that are not in a batch into the next one. `None` when there are none. A receipt that commits late
    /// belongs to the next batch: a batch is the receipts tagged with it, not a range of ids.
    pub fn seal_access_receipts(&self, most: i64) -> Result<Option<Batch>, RepositoryError> {
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute("SELECT pg_advisory_xact_lock(7708001)", &[])?;
            let previous: String = tx.query_opt("SELECT hash FROM access_receipt_batches ORDER BY id DESC LIMIT 1", &[])?.map_or_else(|| GENESIS.to_string(), |r| r.get::<_, String>(0).trim().to_string());
            let rows = tx.query(&format!("SELECT id, {LINE} FROM access_receipts WHERE batch_id IS NULL ORDER BY id LIMIT $1"), &[&most])?;
            if rows.is_empty() {
                return Ok(None);
            }
            let ids: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
            let lines: Vec<String> = rows.iter().map(|r| r.get(1)).collect();
            let hash = batch_hash(&previous, &lines);
            let (first, last) = (ids[0], ids[ids.len() - 1]);
            let id: i64 = tx
                .query_one("INSERT INTO access_receipt_batches (first_id, last_id, hash, previous_hash) VALUES ($1, $2, $3, $4) RETURNING id", &[&first, &last, &hash, &previous])?
                .get(0);
            tx.execute("UPDATE access_receipts SET batch_id = $1 WHERE id = ANY($2)", &[&id, &ids])?;
            tx.commit()?;
            Ok(Some(Batch { id, first_id: first, last_id: last, hash, previous_hash: previous, receipts: ids.len() }))
        })
    }

    /// Records which ledger entry holds a batch's hash.
    pub fn set_batch_ledger_sequence(&self, batch: i64, sequence: i64) -> Result<(), RepositoryError> {
        self.with_client(move |c| {
            c.execute("UPDATE access_receipt_batches SET ledger_sequence = $2 WHERE id = $1", &[&batch, &sequence])?;
            Ok(())
        })
    }

    /// Recomputes every batch from its receipts and checks that each names the one before. The first batch that does not
    /// hold is named.
    pub fn verify_access_receipts(&self) -> Result<Verification, RepositoryError> {
        self.with_client(|c| {
            let batches = c.query("SELECT id, hash, previous_hash FROM access_receipt_batches ORDER BY id", &[])?;
            let mut chain = GENESIS.to_string();
            let mut checked = 0;
            for b in &batches {
                let (id, hash, previous): (i64, String, String) = (b.get(0), b.get::<_, String>(1).trim().to_string(), b.get::<_, String>(2).trim().to_string());
                let fail = |reason: &str| Verification { ok: false, batches_checked: checked, failed_batch: Some(id), reason: Some(reason.to_string()) };
                if previous != chain {
                    return Ok(fail("The batch before this one is missing or was changed."));
                }
                let lines: Vec<String> = c.query(&format!("SELECT {LINE} FROM access_receipts WHERE batch_id = $1 ORDER BY id"), &[&id])?.iter().map(|r| r.get(0)).collect();
                if batch_hash(&previous, &lines) != hash {
                    return Ok(fail("A receipt in this batch was changed or removed."));
                }
                chain = hash;
                checked += 1;
            }
            Ok(Verification { ok: true, batches_checked: checked, failed_batch: None, reason: None })
        })
    }
}
