//! `record_links`: the index of which record points at which. The truth is the record's own JSON,
//! which holds the linked ids. These rows are derived from it in the same transaction, and
//! `rebuild_links` can write them again from the JSON.

use crate::repository::{diff_fields, write_history, ActorKind, PostgresRepository, RepositoryError, Touched, WriteCtx};
use scaffoldry_engine::FieldSpec;
use serde_json::Value;
use std::collections::BTreeSet;

/// A link field of the table being written, and the field on the other table that mirrors it.
#[derive(Debug, Clone)]
pub struct LinkSpec {
    pub field: String,
    /// The mirror field on the target table, when the link is two-way.
    pub related: Option<String>,
    /// True when the mirror field holds one record.
    pub related_one: bool,
}

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

/// The context of a write the platform makes on a person's behalf: it names the same person and
/// batch, and says the system made it.
fn derived(ctx: &WriteCtx) -> WriteCtx {
    let mut sys = ctx.clone();
    sys.kind = ActorKind::System;
    sys
}

/// Takes a record's `data` and `version` under a row lock. `None` when the record is not there.
fn lock(tx: &mut postgres::Transaction<'_>, app_slug: &str, id: &str) -> Result<Option<(Value, i32, String)>, postgres::Error> {
    Ok(tx
        .query_opt("SELECT data, version, table_id FROM dataset_records WHERE app_slug = $1 AND id = $2 FOR UPDATE", &[&app_slug, &id])?
        .map(|r| (r.get(0), r.get(1), r.get(2))))
}

/// One change to a link field: `other` goes into, or out of, `field` of `record_id`.
struct Membership<'a> {
    record_id: &'a str,
    field: &'a str,
    /// True when the field holds one record.
    one: bool,
    other: &'a str,
    present: bool,
}

/// Puts `other` in or takes it out of a link field of a record, without raising its version. The
/// link rows and a history entry by the system follow, in the caller's transaction.
fn set_membership(tx: &mut postgres::Transaction<'_>, app_slug: &str, m: Membership<'_>, ctx: &WriteCtx) -> Result<bool, postgres::Error> {
    let Membership { record_id, field, one, other, present } = m;
    let Some((old, version, table)) = lock(tx, app_slug, record_id)? else { return Ok(false) };
    let mut ids = linked_ids(old.get(field));
    let has = ids.iter().any(|x| x == other);
    if has == present {
        return Ok(false);
    }
    if present {
        if one {
            ids.clear();
        }
        ids.push(other.to_string());
    } else {
        ids.retain(|x| x != other);
    }
    let mut new = old.clone();
    if let Some(map) = new.as_object_mut() {
        if one && ids.is_empty() {
            map.remove(field);
        } else if one {
            map.insert(field.to_string(), Value::String(ids[0].clone()));
        } else {
            map.insert(field.to_string(), Value::Array(ids.iter().cloned().map(Value::String).collect()));
        }
    }
    crate::repository::set_record_data_in_tx(tx, app_slug, record_id, &new)?;
    tx.execute("DELETE FROM record_links WHERE from_record = $1 AND field = $2", &[&record_id, &field])?;
    insert_links(tx, app_slug, record_id, &new, &[field.to_string()])?;
    let changes = diff_fields(Some(&old), Some(&new), &[]);
    write_history(tx, Touched { app_slug, table_id: &table, record_id, version, action: "update" }, &changes, &derived(ctx))?;
    Ok(true)
}

/// Replaces a record's link rows with what its JSON says, and mirrors the change into the records
/// on the other end of every two-way link. Called by every record write, inside its transaction.
/// `old` is the record before the write, or `None` for a new record.
pub(crate) fn write_record_links(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    record_id: &str,
    old: Option<&Value>,
    data: &Value,
    ctx: &WriteCtx,
) -> Result<(), postgres::Error> {
    let names: Vec<String> = ctx.links.iter().map(|l| l.field.clone()).collect();
    tx.execute("DELETE FROM record_links WHERE from_record = $1", &[&record_id])?;
    insert_links(tx, app_slug, record_id, data, &names)?;
    // The records on the other end, locked in one order so two writers cannot wait on each other.
    let mut changes: Vec<(&LinkSpec, Vec<String>, Vec<String>)> = Vec::new();
    for spec in ctx.links.iter().filter(|l| l.related.is_some()) {
        let before: BTreeSet<String> = linked_ids(old.and_then(|o| o.get(&spec.field))).into_iter().collect();
        let after: BTreeSet<String> = linked_ids(data.get(&spec.field)).into_iter().collect();
        let removed: Vec<String> = before.difference(&after).cloned().collect();
        let added: Vec<String> = after.difference(&before).cloned().collect();
        if !removed.is_empty() || !added.is_empty() {
            changes.push((spec, removed, added));
        }
    }
    let mut touched: Vec<&String> = changes.iter().flat_map(|(_, r, a)| r.iter().chain(a.iter())).collect();
    touched.sort();
    touched.dedup();
    for id in touched {
        lock(tx, app_slug, id)?;
    }
    for (spec, removed, added) in changes {
        let Some(related) = spec.related.as_deref() else { continue };
        for id in &removed {
            set_membership(tx, app_slug, Membership { record_id: id, field: related, one: spec.related_one, other: record_id, present: false }, ctx)?;
        }
        for id in &added {
            set_membership(tx, app_slug, Membership { record_id: id, field: related, one: spec.related_one, other: record_id, present: true }, ctx)?;
        }
    }
    Ok(())
}

/// A record is gone. The links it held, in both directions, are removed and remembered in
/// `trash_links`, and every record that pointed at it forgets it.
pub(crate) fn delete_record_links(tx: &mut postgres::Transaction<'_>, app_slug: &str, record_id: &str, ctx: &WriteCtx) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM trash_links WHERE record_id = $1", &[&record_id])?;
    tx.execute(
        "INSERT INTO trash_links (record_id, field, other_record, direction) \
         SELECT from_record, field, to_record, 'out' FROM record_links WHERE from_record = $1 \
         ON CONFLICT DO NOTHING",
        &[&record_id],
    )?;
    let incoming = tx.query("SELECT from_record, field FROM record_links WHERE to_record = $1 ORDER BY from_record, field", &[&record_id])?;
    for row in &incoming {
        let (from, field): (String, String) = (row.get(0), row.get(1));
        tx.execute(
            "INSERT INTO trash_links (record_id, field, other_record, direction) VALUES ($1, $2, $3, 'in') ON CONFLICT DO NOTHING",
            &[&record_id, &field, &from],
        )?;
        let one = match lock(tx, app_slug, &from)? {
            Some((data, _, _)) => !matches!(data.get(&field), Some(Value::Array(_))),
            None => true,
        };
        set_membership(tx, app_slug, Membership { record_id: &from, field: &field, one, other: record_id, present: false }, ctx)?;
    }
    tx.execute("DELETE FROM record_links WHERE to_record = $1", &[&record_id])?;
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

    /// Brings back the links a deleted record had, now that the record is back. A link comes back only
    /// when the record on the other end is still there. A record the restored one names that is gone
    /// is dropped from it. Returns the number of links restored.
    pub fn restore_record_links(&self, app_slug: &str, record_id: &str) -> Result<u64, RepositoryError> {
        let (slug, id) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let ctx = WriteCtx::system("links.restore");
            let Some((own, _, _)) = lock(&mut tx, &slug, &id)? else {
                return Err(RepositoryError::NotFound(format!("record {id}")));
            };
            let rows = tx.query("SELECT field, other_record, direction FROM trash_links WHERE record_id = $1 ORDER BY field, other_record", &[&id])?;
            let mut restored = 0u64;
            // Links out: keep the ids that are still records, drop the rest from the restored record.
            let mut data = own.clone();
            let out_fields: BTreeSet<String> = rows.iter().filter(|r| r.get::<_, String>(2) == "out").map(|r| r.get(0)).collect();
            for field in &out_fields {
                let ids = linked_ids(data.get(field));
                let live: Vec<String> = tx
                    .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND id = ANY($2)", &[&slug, &ids])?
                    .iter()
                    .map(|r| r.get(0))
                    .collect();
                let kept: Vec<String> = ids.iter().filter(|x| live.contains(x)).cloned().collect();
                restored += kept.len() as u64;
                if let Some(map) = data.as_object_mut() {
                    match map.get(field) {
                        Some(Value::String(_)) if kept.is_empty() => {
                            map.remove(field);
                        }
                        Some(Value::String(_)) => {}
                        _ => {
                            map.insert(field.clone(), Value::Array(kept.into_iter().map(Value::String).collect()));
                        }
                    }
                }
            }
            if data != own {
                crate::repository::set_record_data_in_tx(&mut tx, &slug, &id, &data)?;
            }
            for field in &out_fields {
                tx.execute("DELETE FROM record_links WHERE from_record = $1 AND field = $2", &[&id, field])?;
                insert_links(&mut tx, &slug, &id, &data, std::slice::from_ref(field))?;
            }
            // Links in: put the restored record back into the field of each record that still exists.
            for row in rows.iter().filter(|r| r.get::<_, String>(2) == "in") {
                let (field, other): (String, String) = (row.get(0), row.get(1));
                let Some((theirs, _, _)) = lock(&mut tx, &slug, &other)? else { continue };
                let one = !matches!(theirs.get(&field), Some(Value::Array(_)));
                let occupied = one && !linked_ids(theirs.get(&field)).is_empty();
                if occupied {
                    continue;
                }
                if set_membership(&mut tx, &slug, Membership { record_id: &other, field: &field, one, other: &id, present: true }, &ctx)? {
                    restored += 1;
                }
            }
            tx.execute("DELETE FROM trash_links WHERE record_id = $1", &[&id])?;
            tx.commit()?;
            Ok(restored)
        })
    }

    /// How many records point at `record_id` through any of `fields`.
    pub fn count_links_to(&self, app_slug: &str, record_id: &str, fields: &[String]) -> Result<i64, RepositoryError> {
        let (slug, id, fields) = (app_slug.to_string(), record_id.to_string(), fields.to_vec());
        self.with_client(move |client| {
            Ok(client
                .query_one("SELECT count(*) FROM record_links WHERE app_slug = $1 AND to_record = $2 AND field = ANY($3)", &[&slug, &id, &fields])?
                .get(0))
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
