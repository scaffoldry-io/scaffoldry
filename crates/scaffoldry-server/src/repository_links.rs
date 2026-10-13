//! `record_links`: the index of which record points at which. The truth is the record's own JSON,
//! which holds the linked ids. These rows are derived from it in the same transaction, and
//! `rebuild_links` can write them again from the JSON.

use crate::repository::{diff_fields, write_history, ActorKind, PostgresRepository, RepositoryError, Touched, WriteCtx};
use scaffoldry_engine::FieldSpec;
use serde_json::Value;
use std::collections::BTreeSet;

/// One aggregate over a parent's link rows and the target values in `record_index`. Parameters:
/// owner record id, link field, target field.
pub const ROLLUP_SUM_SQL: &str = "SELECT COALESCE(SUM(i.num), 0)::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
const ROLLUP_AVG_SQL: &str = "SELECT COALESCE(AVG(i.num), 0)::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
const ROLLUP_MIN_SQL: &str = "SELECT MIN(i.num)::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
const ROLLUP_MAX_SQL: &str = "SELECT MAX(i.num)::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
const ROLLUP_COUNT_SQL: &str = "SELECT COUNT(*) FILTER (WHERE i.num IS NOT NULL OR i.txt IS NOT NULL OR i.ts IS NOT NULL)::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
const ROLLUP_DISTINCT_SQL: &str = "SELECT COUNT(DISTINCT COALESCE(i.num::text, i.txt, i.ts::text))::float8 FROM record_links l \
    JOIN record_index i ON i.record_id = l.to_record AND i.field = $3 AND i.ord = 0 \
    WHERE l.from_record = $1 AND l.field = $2";
/// The number of links a record holds in a field. Parameters: owner record id, link field.
const COUNT_SQL: &str = "SELECT COUNT(*)::float8 FROM record_links WHERE from_record = $1 AND field = $2";

/// The values a page of records looks up, in one statement. Parameters: the owners on the page, the
/// link field, the target field.
pub const LOOKUP_SQL: &str = "SELECT l.from_record, r.data -> $3::text FROM record_links l \
    JOIN dataset_records r ON r.id = l.to_record AND r.deleted_at IS NULL \
    WHERE l.from_record = ANY($1::text[]) AND l.field = $2 ORDER BY l.from_record, l.ord";

static LOOKUP_STATEMENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many lookup statements this process has run. A test reads it before and after a page.
pub fn lookup_statements() -> u64 {
    LOOKUP_STATEMENTS.load(std::sync::atomic::Ordering::SeqCst)
}

/// A `Rollup` or `Count` field, and what it needs to be recomputed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComputedSpec {
    pub owner_table: String,
    pub field: String,
    /// The link field of the owner it goes through.
    pub via: String,
    pub count: bool,
    pub function: String,
    pub target: Option<String>,
    pub indexed: bool,
}

/// What a table needs when one of its records is recomputed because a record it reads changed.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TableCalc {
    /// The stored formulas, as `(field, expression)`, each after everything it reads.
    pub formulas: Vec<(String, String)>,
    /// The indexed fields, so the index rows of a recomputed record follow its values.
    pub index_fields: Vec<FieldSpec>,
}

/// How many owners a write recomputes before it hands the rest to a job.
pub const INLINE_OWNERS: usize = 200;

/// A link field of the table being written, and the field on the other table that mirrors it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LinkSpec {
    pub field: String,
    /// The mirror field on the target table, when the link is two-way.
    pub related: Option<String>,
    /// True when the mirror field holds one record.
    pub related_one: bool,
}

/// The `Link` fields of a table in a manifest, each with the field that mirrors it.
pub fn link_specs_in(manifest: &scaffoldry_engine::AppManifest, table_id: &str) -> Vec<LinkSpec> {
    use scaffoldry_engine::{FieldType, LinkCardinality};
    let Some(table) = manifest.tables.iter().find(|t| t.id == table_id) else { return Vec::new() };
    table
        .fields
        .iter()
        .filter(|f| f.field_type == FieldType::Link)
        .map(|f| {
            let mirror = f.link_related_field.as_deref().and_then(|name| {
                let target = manifest.tables.iter().find(|t| Some(t.id.as_str()) == f.link_target_table.as_deref())?;
                target.fields.iter().find(|x| x.name == name)
            });
            LinkSpec {
                field: f.name.clone(),
                related: mirror.map(|m| m.name.clone()),
                related_one: mirror.is_some_and(|m| m.link_cardinality == LinkCardinality::One),
            }
        })
        .collect()
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
        .query_opt("SELECT data, version, table_id FROM dataset_records WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL FOR UPDATE", &[&app_slug, &id])?
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
    // Who rolled this record up before the write.
    let owners = if ctx.computed.is_empty() { Vec::new() } else { owners_of(tx, record_id)? };
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
    if !ctx.computed.is_empty() {
        // Who rolls it up now, and the record itself, which may roll up its own links.
        let now = owners_of(tx, record_id)?;
        let table = data.get("_table_id").and_then(Value::as_str).unwrap_or("");
        let own: Vec<(String, String)> = ctx.computed.iter().filter(|c| c.owner_table == table).map(|c| (record_id.to_string(), c.via.clone())).collect();
        refresh_owners(tx, app_slug, record_id, owners, now, own, ctx)?;
    }
    Ok(())
}

/// Recomputes the owners of a changed record: inline when there are few, by the job `recalc_owners`
/// when there are many. `before` and `after` are the records that rolled it up before and after the
/// write. `own` is the record itself, which is always recomputed here.
fn refresh_owners(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    record_id: &str,
    before: Vec<(String, String)>,
    after: Vec<(String, String)>,
    own: Vec<(String, String)>,
    ctx: &WriteCtx,
) -> Result<(), postgres::Error> {
    let distinct = |v: &[(String, String)]| v.iter().map(|(id, _)| id.as_str()).collect::<BTreeSet<_>>().len();
    let all: Vec<(String, String)> = before.iter().chain(after.iter()).cloned().collect();
    if distinct(&all) <= INLINE_OWNERS {
        let mut owners = all;
        owners.extend(own);
        return recompute_owners(tx, app_slug, owners, ctx);
    }
    recompute_owners(tx, app_slug, own, ctx)?;
    // The owners the record no longer has cannot be found from its links, so the job is told.
    let kept: BTreeSet<&(String, String)> = after.iter().collect();
    let dropped: Vec<&(String, String)> = before.iter().filter(|o| !kept.contains(o)).take(INLINE_OWNERS * 5).collect();
    let payload = serde_json::json!({
        "app_slug": app_slug,
        "record_id": record_id,
        "actor": ctx.actor,
        "dropped": dropped,
        "plan": { "computed": ctx.computed, "tables": &*ctx.calc },
    });
    // The job row is part of this transaction, so it exists if and only if the write commits.
    tx.execute(
        "INSERT INTO jobs (id, kind, payload, created_by) VALUES ($1, 'recalc_owners', $2, $3)",
        &[&uuid::Uuid::new_v4(), &payload, &ctx.actor],
    )?;
    Ok(())
}

/// The records that link to `record_id`, each with the field it links through.
fn owners_of(tx: &mut postgres::Transaction<'_>, record_id: &str) -> Result<Vec<(String, String)>, postgres::Error> {
    Ok(tx
        .query("SELECT from_record, field FROM record_links WHERE to_record = $1", &[&record_id])?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect())
}

/// The most times a recomputation repeats outward, when an owner's change reaches the records that
/// roll it up. The graph has no cycle, so it ends sooner. This only guards a bad manifest.
const MAX_ROUNDS: usize = 16;

/// Writes the rollups and counts of each owner again, from its link rows and the indexed target values,
/// then its stored formulas in run order, so a formula that reads a rollup follows it. When that changes
/// a value that other records roll up, they are recomputed next. The owner's version does not change,
/// and nothing is written to history: a computed value is not a person's edit, and it must not make
/// someone's save conflict.
pub(crate) fn recompute_owners(
    tx: &mut postgres::Transaction<'_>,
    app_slug: &str,
    owners: Vec<(String, String)>,
    ctx: &WriteCtx,
) -> Result<(), postgres::Error> {
    let mut pending = owners;
    for _ in 0..MAX_ROUNDS {
        pending.sort();
        pending.dedup();
        if pending.is_empty() {
            break;
        }
        let mut next: Vec<(String, String)> = Vec::new();
        let mut ids: Vec<&String> = pending.iter().map(|(id, _)| id).collect();
        ids.dedup();
        for id in ids {
            let Some((data, _, table)) = lock(tx, app_slug, id)? else { continue };
            let vias: Vec<&String> = pending.iter().filter(|(o, _)| o == id).map(|(_, v)| v).collect();
            let mut new = data.clone();
            for spec in ctx.computed.iter().filter(|c| c.owner_table == table && vias.contains(&&c.via)) {
                let value = aggregate(tx, id, spec)?;
                if let Some(map) = new.as_object_mut() {
                    match value.and_then(serde_json::Number::from_f64) {
                        Some(n) => {
                            let whole = n.as_f64().is_some_and(|f| f.fract() == 0.0 && f.abs() < 9e15);
                            map.insert(spec.field.clone(), if whole { Value::from(n.as_f64().unwrap_or(0.0) as i64) } else { Value::Number(n) });
                        }
                        None => {
                            map.remove(&spec.field);
                        }
                    }
                }
                if spec.indexed && !ctx.calc.contains_key(&table) {
                    tx.execute("DELETE FROM record_index WHERE record_id = $1 AND field = $2", &[id, &spec.field])?;
                    if let Some(v) = value {
                        tx.execute(
                            "INSERT INTO record_index (app_slug, table_id, field, record_id, num) VALUES ($1, $2, $3, $4, $5)",
                            &[&app_slug, &table, &spec.field, id, &v],
                        )?;
                    }
                }
            }
            let calc = ctx.calc.get(&table);
            if let Some(c) = calc {
                crate::repository::apply_formula_list(&mut new, &c.formulas);
            }
            if new == data {
                continue;
            }
            crate::repository::set_record_data_in_tx(tx, app_slug, id, &new)?;
            if let Some(c) = calc {
                crate::repository_index::write_record_index(tx, app_slug, &table, id, &new, &c.index_fields)?;
            }
            // What changed may be read by the records that roll this one up.
            let changed: BTreeSet<&str> = {
                let (old, now) = (data.as_object(), new.as_object());
                let keys: BTreeSet<&str> = old.into_iter().chain(now).flat_map(|m| m.keys().map(String::as_str)).collect();
                keys.into_iter().filter(|k| old.and_then(|m| m.get(*k)) != now.and_then(|m| m.get(*k))).collect()
            };
            for (owner, via) in owners_of(tx, id)? {
                if ctx.computed.iter().any(|c| c.via == via && c.target.as_deref().is_some_and(|t| changed.contains(t))) {
                    next.push((owner, via));
                }
            }
        }
        pending = next;
    }
    Ok(())
}

/// One rollup or count of an owner, from its link rows and the indexed target values.
pub(crate) fn aggregate(tx: &mut postgres::Transaction<'_>, id: &str, spec: &ComputedSpec) -> Result<Option<f64>, postgres::Error> {
    Ok(match (spec.count, spec.target.as_deref()) {
        (true, _) | (_, None) => tx.query_one(COUNT_SQL, &[&id, &spec.via])?.get(0),
        (false, Some(target)) => {
            let sql = match spec.function.as_str() {
                "avg" => ROLLUP_AVG_SQL,
                "min" => ROLLUP_MIN_SQL,
                "max" => ROLLUP_MAX_SQL,
                "count" => ROLLUP_COUNT_SQL,
                "count_distinct" => ROLLUP_DISTINCT_SQL,
                _ => ROLLUP_SUM_SQL,
            };
            tx.query_one(sql, &[&id, &spec.via, &target])?.get(0)
        }
    })
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
    // The records that rolled it up now roll up one fewer.
    if !ctx.computed.is_empty() {
        let owners: Vec<(String, String)> = incoming.iter().map(|r| (r.get(0), r.get(1))).collect();
        refresh_owners(tx, app_slug, record_id, owners, Vec::new(), Vec::new(), ctx)?;
    }
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

/// Brings back the links a deleted record had, in the caller's transaction, now that the record is
/// live again. A link comes back only when the record on the other end is live. A record the restored
/// one names that is not live is dropped from it. Returns the number of links restored.
pub(crate) fn restore_links_in_tx(tx: &mut postgres::Transaction<'_>, slug: &str, id: &str, ctx: &WriteCtx) -> Result<u64, RepositoryError> {
    let Some((own, _, _)) = lock(tx, slug, id)? else {
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
            .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND id = ANY($2) AND deleted_at IS NULL", &[&slug, &ids])?
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
        crate::repository::set_record_data_in_tx(tx, slug, id, &data)?;
    }
    for field in &out_fields {
        tx.execute("DELETE FROM record_links WHERE from_record = $1 AND field = $2", &[&id, field])?;
        insert_links(tx, slug, id, &data, std::slice::from_ref(field))?;
    }
    // Links in: put the restored record back into the field of each record that still exists.
    for row in rows.iter().filter(|r| r.get::<_, String>(2) == "in") {
        let (field, other): (String, String) = (row.get(0), row.get(1));
        let Some((theirs, _, _)) = lock(tx, slug, &other)? else { continue };
        let one = !matches!(theirs.get(&field), Some(Value::Array(_)));
        let occupied = one && !linked_ids(theirs.get(&field)).is_empty();
        if occupied {
            continue;
        }
        if set_membership(tx, slug, Membership { record_id: &other, field: &field, one, other: id, present: true }, ctx)? {
            restored += 1;
        }
    }
    tx.execute("DELETE FROM trash_links WHERE record_id = $1", &[&id])?;
    Ok(restored)
}

/// What a link picker asks of a table.
pub struct LinkOptionQuery<'a> {
    pub app_slug: &'a str,
    pub table_id: &'a str,
    pub fields: &'a [String],
    pub needle: &'a str,
    pub after: Option<&'a crate::repository::RecordCursor>,
    pub limit: usize,
    pub scope: Option<&'a crate::repository::ScopePlan>,
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
                 (SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL)",
                &[&slug, &table],
            )?;
            let mut written = 0u64;
            let mut after: Option<String> = None;
            loop {
                let rows = match &after {
                    None => tx.query("SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL ORDER BY id LIMIT 5000", &[&slug, &table])?,
                    Some(id) => tx.query("SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND id > $3 ORDER BY id LIMIT 5000", &[&slug, &table, id])?,
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

    /// `restore_links_in_tx` in a transaction of its own, for a record that is already live.
    pub fn restore_record_links(&self, app_slug: &str, record_id: &str) -> Result<u64, RepositoryError> {
        let (slug, id) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let restored = restore_links_in_tx(&mut tx, &slug, &id, &WriteCtx::system("links.restore"))?;
            tx.commit()?;
            Ok(restored)
        })
    }

    /// The value of `target` on the records each owner links to through `via`, for a whole page, in
    /// one statement. An owner with no links is absent from the result.
    pub fn lookup_values(&self, owners: &[String], via: &str, target: &str) -> Result<Vec<(String, Value)>, RepositoryError> {
        let (owners, via, target) = (owners.to_vec(), via.to_string(), target.to_string());
        self.with_client(move |client| {
            LOOKUP_STATEMENTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(client
                .query(LOOKUP_SQL, &[&owners, &via, &target])?
                .iter()
                .map(|r| (r.get::<_, String>(0), r.get::<_, Option<Value>>(1).unwrap_or(Value::Null)))
                .collect())
        })
    }

    /// A page of live records of a table whose `fields` contain `needle`, ignoring case. With no
    /// needle, the first page. `%`, `_` and `\\` in the needle are text, not wildcards.
    pub fn link_option_rows(&self, q: &LinkOptionQuery) -> Result<Vec<crate::state::DatasetRecord>, RepositoryError> {
        let LinkOptionQuery { app_slug, table_id, fields, needle, after, limit, scope } = q;
        let (limit, needle) = (*limit, *needle);
        use postgres::types::ToSql;
        let mut params: Vec<Box<dyn ToSql + Sync + Send>> = vec![Box::new(app_slug.to_string()), Box::new(table_id.to_string())];
        let mut sql = String::from(
            "SELECT id, app_slug, data, ceds_mapping, is_ferpa_sensitive, created_at::text AS created_at_text, created_by, version \
             FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL",
        );
        if let Some(c) = after {
            params.push(Box::new(c.created_at.clone()));
            params.push(Box::new(c.id.clone()));
            sql.push_str(&format!(" AND (created_at, id) > (${}::text::timestamptz, ${}::text)", params.len() - 1, params.len()));
        }
        if !needle.is_empty() && !fields.is_empty() {
            let escaped = needle.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            params.push(Box::new(format!("%{escaped}%")));
            let pattern = params.len();
            let mut any = Vec::new();
            for f in fields.iter() {
                params.push(Box::new(f.clone()));
                any.push(format!("data->>${} ILIKE ${pattern} ESCAPE '\\'", params.len()));
            }
            sql.push_str(&format!(" AND ({})", any.join(" OR ")));
        }
        if let Some(plan) = scope {
            let (clause, added) = plan.clause_at("dataset_records.id", app_slug, table_id, params.len() + 1);
            sql.push_str(&clause);
            for p in added {
                params.push(match p {
                    crate::repository::SqlParam::Text(t) => Box::new(t),
                    crate::repository::SqlParam::Texts(t) => Box::new(t),
                    crate::repository::SqlParam::Float(f) => Box::new(f),
                    crate::repository::SqlParam::Floats(f) => Box::new(f),
                    crate::repository::SqlParam::Int(i) => Box::new(i),
                });
            }
        }
        params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));
        sql.push_str(&format!(" ORDER BY created_at, id LIMIT ${}", params.len()));
        self.with_client(move |client| {
            crate::repository::count_page_statement();
            let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|b| &**b as &(dyn ToSql + Sync)).collect();
            Ok(client.query(&sql, &refs)?.iter().map(crate::repository::record_from_row).collect())
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
                .query("SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND id = ANY($3) AND deleted_at IS NULL", &[&slug, &table, &ids])?
                .iter()
                .map(|r| r.get(0))
                .collect())
        })
    }
}
