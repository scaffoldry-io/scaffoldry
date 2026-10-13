//! The disclosure log (record-history phase 4). One row per release of sensitive fields, and one
//! row per record in it. An entry above `MAX_RECORDS_PER_ENTRY` records keeps its count and its
//! fields but lists no records.

use crate::repository::{PostgresRepository, RepositoryError};
use serde::Serialize;
use uuid::Uuid;

/// The most records one entry lists. Above this the entry is marked `records_truncated`.
pub const MAX_RECORDS_PER_ENTRY: usize = 100_000;

/// What a release writes.
pub struct NewDisclosure {
    pub kind: String,
    pub actor: String,
    pub app_slug: String,
    pub table_id: Option<String>,
    pub record_ids: Vec<String>,
    pub fields: Vec<String>,
    pub recipient: Option<String>,
    pub note: String,
}

/// One entry as stored.
#[derive(Debug, Clone, Serialize)]
pub struct DisclosureRow {
    pub id: Uuid,
    pub at: String,
    pub actor: String,
    pub kind: String,
    pub app_slug: String,
    pub table_id: Option<String>,
    pub record_count: i64,
    pub fields: Vec<String>,
    pub recipient: Option<String>,
    pub note: String,
    pub records_truncated: bool,
}

/// The filters of `GET /admin/disclosures`. `from` and `to` are RFC 3339 times.
#[derive(Debug, Clone, Default)]
pub struct DisclosureFilter {
    pub record: Option<String>,
    pub actor: Option<String>,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

const COLUMNS: &str = "l.id, to_char(l.at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), l.actor, l.kind, l.app_slug, \
    l.table_id, l.record_count, l.fields, l.recipient, l.note, l.records_truncated";

fn row_of(r: &postgres::Row) -> DisclosureRow {
    DisclosureRow {
        id: r.get(0),
        at: r.get(1),
        actor: r.get(2),
        kind: r.get(3),
        app_slug: r.get(4),
        table_id: r.get(5),
        record_count: r.get(6),
        fields: r.get(7),
        recipient: r.get(8),
        note: r.get(9),
        records_truncated: r.get(10),
    }
}

impl PostgresRepository {
    /// Writes an entry and its record rows in one transaction.
    pub fn insert_disclosure(&self, d: NewDisclosure) -> Result<Uuid, RepositoryError> {
        self.with_client(move |c| {
            let mut ids = d.record_ids;
            ids.sort_unstable();
            ids.dedup();
            let count = i64::try_from(ids.len()).unwrap_or(i64::MAX);
            let truncated = ids.len() > MAX_RECORDS_PER_ENTRY;
            let id = Uuid::new_v4();
            let mut tx = c.transaction()?;
            tx.execute(
                "INSERT INTO disclosure_log (id, actor, kind, app_slug, table_id, record_count, fields, recipient, note, records_truncated) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[&id, &d.actor, &d.kind, &d.app_slug, &d.table_id, &count, &d.fields, &d.recipient, &d.note, &truncated],
            )?;
            if !truncated && !ids.is_empty() {
                tx.execute("INSERT INTO disclosure_records (disclosure_id, record_id) SELECT $1, unnest($2::text[])", &[&id, &ids])?;
            }
            tx.commit()?;
            Ok(id)
        })
    }

    /// Entries, newest first, that match every filter given.
    pub fn list_disclosures(&self, f: DisclosureFilter, limit: i64, offset: i64) -> Result<Vec<DisclosureRow>, RepositoryError> {
        self.with_client(move |c| {
            Ok(c.query(
                &format!(
                    "SELECT {COLUMNS} FROM disclosure_log l \
                     WHERE ($1::text IS NULL OR EXISTS (SELECT 1 FROM disclosure_records r WHERE r.disclosure_id = l.id AND r.record_id = $1)) \
                       AND ($2::text IS NULL OR l.actor = $2) \
                       AND ($3::text IS NULL OR l.kind = $3) \
                       AND ($4::text IS NULL OR l.app_slug = $4) \
                       AND ($5::text IS NULL OR l.at >= $5::text::timestamptz) \
                       AND ($6::text IS NULL OR l.at <= $6::text::timestamptz) \
                     ORDER BY l.at DESC, l.id LIMIT $7 OFFSET $8"
                ),
                &[&f.record, &f.actor, &f.kind, &f.app, &f.from, &f.to, &limit, &offset],
            )?
            .iter()
            .map(row_of)
            .collect())
        })
    }

    /// Every entry of an app that lists this record, newest first.
    pub fn record_disclosures(&self, app_slug: &str, record_id: &str) -> Result<Vec<DisclosureRow>, RepositoryError> {
        let (app, record) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |c| {
            Ok(c.query(
                &format!(
                    "SELECT {COLUMNS} FROM disclosure_log l JOIN disclosure_records r ON r.disclosure_id = l.id \
                     WHERE l.app_slug = $1 AND r.record_id = $2 ORDER BY l.at DESC, l.id"
                ),
                &[&app, &record],
            )?
            .iter()
            .map(row_of)
            .collect())
        })
    }
}
