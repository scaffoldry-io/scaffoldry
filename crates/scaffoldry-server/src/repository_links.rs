//! `record_links`: the index of which record points at which. The truth is the record's own JSON,
//! which holds the linked ids. These rows are derived from it in the same transaction, and
//! `rebuild_links` can write them again from the JSON.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_engine::FieldSpec;
use serde_json::Value;

/// The records a record points at through one field, in order. Parameters: record id, field.
pub const LINKS_FROM_SQL: &str = "SELECT to_record FROM record_links WHERE from_record = $1 AND field = $2 ORDER BY ord, to_record";
/// The records that point at a record through one field. Parameters: record id, field.
pub const LINKS_TO_SQL: &str = "SELECT from_record FROM record_links WHERE to_record = $1 AND field = $2 ORDER BY from_record";

/// How many records one link field may hold.
pub const MAX_LINKS_PER_FIELD: usize = 1_000;

/// The ids a link field holds: one id, a list of ids, or nothing.
pub fn linked_ids(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

/// Replaces a record's link rows with what its JSON says. Called by every record write, inside
/// its transaction.
pub(crate) fn write_record_links(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    record_id: &str,
    data: &Value,
    link_fields: &[String],
) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_links WHERE from_record = $1", &[&record_id])?;
    insert_links(tx, app_slug, record_id, data, link_fields)
}

/// A record is gone, so the links it held are gone with it.
pub(crate) fn delete_record_links(tx: &mut postgres::Transaction<'_>, record_id: &str) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_links WHERE from_record = $1", &[&record_id])?;
    Ok(())
}

fn insert_links(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    record_id: &str,
    data: &Value,
    link_fields: &[String],
) -> Result<(), postgres::Error> {
    let (mut fields, mut tos, mut ords): (Vec<String>, Vec<String>, Vec<i16>) = (Vec::new(), Vec::new(), Vec::new());
    for field in link_fields {
        for (ord, to) in linked_ids(data.get(field)).into_iter().enumerate() {
            fields.push(field.clone());
            tos.push(to);
            ords.push(i16::try_from(ord).unwrap_or(i16::MAX));
        }
    }
    if fields.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO record_links (app_slug, field, from_record, to_record, ord) \
         SELECT $1, u.f, $2, u.t, u.o FROM unnest($3::text[], $4::text[], $5::int2[]) AS u(f, t, o) \
         ON CONFLICT (from_record, field, to_record) DO NOTHING",
        &[&app_slug, &record_id, &fields, &tos, &ords],
    )?;
    Ok(())
}

impl PostgresRepository {
    /// Writes a table's link rows again from the records' JSON. Returns the number of rows written.
    pub fn rebuild_links(&self, app_slug: &str, table_id: &str, fields: &[FieldSpec]) -> Result<u64, RepositoryError> {
        let (slug, table) = (app_slug.to_string(), table_id.to_string());
        let names: Vec<String> = fields.iter().filter(|f| f.field_type == scaffoldry_engine::FieldType::Link).map(|f| f.name.clone()).collect();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            tx.execute(
                "DELETE FROM record_links WHERE app_slug = $1 AND from_record IN \
                 (SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2)",
                &[&slug, &table],
            )?;
            let mut written = 0u64;
            let mut after: Option<String> = None;
            loop {
                let rows = match &after {
                    None => tx.query("SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 ORDER BY id LIMIT 5000", &[&slug, &table])?,
                    Some(id) => tx.query("SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND id > $3 ORDER BY id LIMIT 5000", &[&slug, &table, id])?,
                };
                for row in &rows {
                    let (id, data): (String, Value) = (row.get(0), row.get(1));
                    written += names.iter().map(|n| linked_ids(data.get(n)).len() as u64).sum::<u64>();
                    insert_links(&mut tx, &slug, &id, &data, &names)?;
                }
                if rows.len() < 5000 {
                    break;
                }
                after = rows.last().map(|r| r.get::<_, String>(0));
            }
            tx.commit()?;
            Ok(written)
        })
    }

    /// The ids a record points at through one field, in order.
    pub fn links_from(&self, record_id: &str, field: &str) -> Result<Vec<String>, RepositoryError> {
        self.link_ids(LINKS_FROM_SQL, record_id, field)
    }

    /// The ids of the records that point at a record through one field.
    pub fn links_to(&self, record_id: &str, field: &str) -> Result<Vec<String>, RepositoryError> {
        self.link_ids(LINKS_TO_SQL, record_id, field)
    }

    fn link_ids(&self, sql: &'static str, record_id: &str, field: &str) -> Result<Vec<String>, RepositoryError> {
        let (id, field) = (record_id.to_string(), field.to_string());
        self.with_client(move |client| Ok(client.query(sql, &[&id, &field])?.iter().map(|r| r.get(0)).collect()))
    }

    /// Which of `ids` are records of this table of this app.
    pub fn existing_record_ids(&self, app_slug: &str, table_id: &str, ids: &[String]) -> Result<Vec<String>, RepositoryError> {
        let (slug, table, ids) = (app_slug.to_string(), table_id.to_string(), ids.to_vec());
        self.with_client(move |client| {
            Ok(client
                .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND id = ANY($3)", &[&slug, &table, &ids])?
                .iter()
                .map(|r| r.get(0))
                .collect())
        })
    }
}
