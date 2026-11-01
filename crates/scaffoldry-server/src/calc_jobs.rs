//! The job that recomputes the records that roll up a changed record, when there are too many to do
//! inline. It works in batches of 200, each in its own transaction, and saves where it stopped.

use crate::jobs::{JobCtx, JobError};
use crate::repository::WriteCtx;
use crate::repository_links::{recompute_owners, ComputedSpec, TableCalc, INLINE_OWNERS};
use scaffoldry_engine::recalc::{convert_value, Converted};
use scaffoldry_engine::FieldType;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Owners recomputed in one transaction.
const BATCH: i64 = (INLINE_OWNERS * 5) as i64;

fn retry(e: impl std::fmt::Display) -> JobError {
    JobError::Retry(e.to_string())
}

fn text(payload: &Value, key: &str) -> Result<String, JobError> {
    payload.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| JobError::Permanent(format!("the job has no '{key}'")))
}

/// Recomputes every record that links to `record_id`, and the few that used to.
pub fn recalc_owners(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app_slug, record_id, actor) = (text(&payload, "app_slug")?, text(&payload, "record_id")?, text(&payload, "actor")?);
    let computed: Vec<ComputedSpec> = serde_json::from_value(payload["plan"]["computed"].clone()).map_err(|e| JobError::Permanent(e.to_string()))?;
    let tables: HashMap<String, TableCalc> = serde_json::from_value(payload["plan"]["tables"].clone()).map_err(|e| JobError::Permanent(e.to_string()))?;
    let dropped: Vec<(String, String)> = serde_json::from_value(payload["dropped"].clone()).unwrap_or_default();
    let repo = ctx.repository();

    let mut write = WriteCtx::system(&actor);
    write.computed = computed;
    write.calc = std::sync::Arc::new(tables);

    let total: i64 = {
        let id = record_id.clone();
        repo.with_client(move |c| Ok(c.query_one("SELECT count(DISTINCT from_record) FROM record_links WHERE to_record = $1", &[&id])?.get(0))).map_err(retry)?
    };
    let (mut after, mut done) = match ctx.load_checkpoint() {
        Some(cp) => (cp.get("after").and_then(Value::as_str).map(str::to_string), cp.get("done").and_then(Value::as_i64).unwrap_or(0)),
        None => (None, 0),
    };
    let mut first = after.is_none();
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (slug, id, from, w, extra) = (app_slug.clone(), record_id.clone(), after.clone(), write.clone(), if first { dropped.clone() } else { Vec::new() });
        let batch: Vec<(String, String)> = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                // Whole owners at a time, so no owner is split between two batches.
                let rows = tx.query(
                    "WITH page AS (SELECT DISTINCT from_record FROM record_links WHERE to_record = $1 AND ($2::text IS NULL OR from_record > $2) ORDER BY from_record LIMIT $3) \
                     SELECT l.from_record, l.field FROM record_links l JOIN page p ON p.from_record = l.from_record WHERE l.to_record = $1 ORDER BY l.from_record, l.field",
                    &[&id, &from, &BATCH],
                )?;
                let owners: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
                recompute_owners(&mut tx, &slug, owners.iter().cloned().chain(extra).collect(), &w)?;
                tx.commit()?;
                Ok(owners)
            })
            .map_err(retry)?;
        first = false;
        let mut seen: Vec<&String> = batch.iter().map(|(o, _)| o).collect();
        seen.dedup();
        done += seen.len() as i64;
        if let Some((last, _)) = batch.last() {
            after = Some(last.clone());
        }
        ctx.checkpoint(json!({ "after": after, "done": done }))?;
        ctx.progress(done, total.max(done))?;
        if (seen.len() as i64) < BATCH {
            break;
        }
    }
    ctx.result(json!({ "owners": done }))
}

// ---------------------------------------------------------------------------------------------
// A definition changed: write the stored values again.
// ---------------------------------------------------------------------------------------------

/// Records read and written in one transaction.
const FIELD_BATCH: i64 = 1_000;
/// The examples a preview keeps of each kind.
const EXAMPLES: usize = 50;

fn parse<T: serde::de::DeserializeOwned>(payload: &Value, key: &str) -> Result<T, JobError> {
    serde_json::from_value(payload[key].clone()).map_err(|e| JobError::Permanent(format!("the job's '{key}' is not usable: {e}")))
}

fn count_rows(ctx: &JobCtx, app: &str, table: &str) -> Result<i64, JobError> {
    let (app, table) = (app.to_string(), table.to_string());
    ctx.repository()
        .with_client(move |c| Ok(c.query_one("SELECT count(*) FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL", &[&app, &table])?.get(0)))
        .map_err(retry)
}

fn resume(ctx: &JobCtx) -> (Option<String>, i64) {
    match ctx.load_checkpoint() {
        Some(cp) => (cp.get("after").and_then(Value::as_str).map(str::to_string), cp.get("done").and_then(Value::as_i64).unwrap_or(0)),
        None => (None, 0),
    }
}

/// Writes the computed fields of `payload.fields` again for every record of a table, in run order.
/// Each batch locks its records, evaluates the definition in force now, and writes only what changed, so
/// a record written during the job (which already used the new definition) is left as it is.
pub fn recalc_field(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app, table) = (text(&payload, "app_slug")?, text(&payload, "table_id")?);
    let fields: Vec<String> = parse(&payload, "fields")?;
    let formulas: Vec<(String, String)> = parse(&payload, "formulas")?;
    let rollups: Vec<ComputedSpec> = parse(&payload, "rollups")?;
    let index_fields: Vec<scaffoldry_engine::FieldSpec> = parse(&payload, "index_fields")?;
    let repo = ctx.repository();
    let total = count_rows(ctx, &app, &table)?;
    let (mut after, mut done) = resume(ctx);
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (a, t, from) = (app.clone(), table.clone(), after.clone());
        let (fields, formulas, rollups, index_fields) = (fields.clone(), formulas.clone(), rollups.clone(), index_fields.clone());
        let (last, read) = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let rows = tx.query(
                    "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($3::text IS NULL OR id > $3) \
                     ORDER BY id LIMIT $4 FOR UPDATE",
                    &[&a, &t, &from, &FIELD_BATCH],
                )?;
                let mut last: Option<String> = None;
                for row in &rows {
                    let (id, data): (String, Value) = (row.get(0), row.get(1));
                    let mut new = data.clone();
                    for field in &fields {
                        if let Some((_, expression)) = formulas.iter().find(|(f, _)| f == field) {
                            crate::repository::apply_formula_list(&mut new, std::slice::from_ref(&(field.clone(), expression.clone())));
                        } else if let Some(spec) = rollups.iter().find(|s| &s.field == field) {
                            let value = crate::repository_links::aggregate(&mut tx, &id, spec)?;
                            if let Some(map) = new.as_object_mut() {
                                match value.and_then(serde_json::Number::from_f64) {
                                    Some(n) => {
                                        let whole = n.as_f64().is_some_and(|f| f.fract() == 0.0 && f.abs() < 9e15);
                                        map.insert(field.clone(), if whole { Value::from(n.as_f64().unwrap_or(0.0) as i64) } else { Value::Number(n) });
                                    }
                                    None => {
                                        map.remove(field);
                                    }
                                }
                            }
                        }
                    }
                    if new != data {
                        crate::repository::set_record_data_in_tx(&mut tx, &a, &id, &new)?;
                        crate::repository_index::write_record_index(&mut tx, &a, &t, &id, &new, &index_fields)?;
                    }
                    last = Some(id);
                }
                tx.commit()?;
                Ok((last, rows.len() as i64))
            })
            .map_err(retry)?;
        done += read;
        if last.is_some() {
            after = last;
        }
        ctx.checkpoint(json!({ "after": after, "done": done }))?;
        ctx.progress(done, total.max(done))?;
        if read < FIELD_BATCH {
            break;
        }
    }
    // Only the status rows this job set: a later approval has its own job and its own rows.
    let (a, t, fields, job) = (app.clone(), table.clone(), fields.clone(), ctx.job_id());
    repo.with_client(move |c| {
        c.execute(
            "UPDATE calc_status SET state = 'ready', updated_at = NOW() WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3) AND job_id = $4",
            &[&a, &t, &fields, &job],
        )?;
        Ok(())
    })
    .map_err(retry)?;
    ctx.result(json!({ "records": done }))
}

/// Counts what a conversion would do, and keeps the first 50 of each kind. Nothing is written.
pub fn convert_preview(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app, table, field) = (text(&payload, "app_slug")?, text(&payload, "table_id")?, text(&payload, "field")?);
    let (from, to): (FieldType, FieldType) = (parse(&payload, "from")?, parse(&payload, "to")?);
    let repo = ctx.repository();
    let total = count_rows(ctx, &app, &table)?;
    let (mut after, mut done) = resume(ctx);
    let (mut converts, mut blanks) = (0i64, 0i64);
    let (mut convert_examples, mut blank_examples): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    if let Some(cp) = ctx.load_checkpoint() {
        converts = cp.get("converts").and_then(Value::as_i64).unwrap_or(0);
        blanks = cp.get("blanks").and_then(Value::as_i64).unwrap_or(0);
        convert_examples = cp.get("convert_examples").and_then(Value::as_array).cloned().unwrap_or_default();
        blank_examples = cp.get("blank_examples").and_then(Value::as_array).cloned().unwrap_or_default();
    }
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (a, t, f, from_id) = (app.clone(), table.clone(), field.clone(), after.clone());
        let rows: Vec<(String, Option<Value>)> = repo
            .with_client(move |c| {
                Ok(c.query(
                    "SELECT id, data -> $3::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($4::text IS NULL OR id > $4) ORDER BY id LIMIT $5",
                    &[&a, &t, &f, &from_id, &(FIELD_BATCH * 2)],
                )?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect())
            })
            .map_err(retry)?;
        for (id, value) in &rows {
            let Some(value) = value else { continue };
            match convert_value(value, from, to) {
                Converted::Keep => {}
                Converted::To(new) => {
                    converts += 1;
                    if convert_examples.len() < EXAMPLES {
                        convert_examples.push(json!({ "id": id, "old": value, "new": new }));
                    }
                }
                Converted::Blank => {
                    blanks += 1;
                    if blank_examples.len() < EXAMPLES {
                        blank_examples.push(json!({ "id": id, "old": value }));
                    }
                }
            }
        }
        done += rows.len() as i64;
        if let Some((last, _)) = rows.last() {
            after = Some(last.clone());
        }
        ctx.checkpoint(json!({ "after": after, "done": done, "converts": converts, "blanks": blanks, "convert_examples": convert_examples, "blank_examples": blank_examples }))?;
        ctx.progress(done, total.max(done))?;
        if (rows.len() as i64) < FIELD_BATCH * 2 {
            break;
        }
    }
    ctx.result(json!({
        "field": field,
        "from": from,
        "to": to,
        "scanned": done,
        "converts": converts,
        "blanks": blanks,
        "examples": { "convert": convert_examples, "blank": blank_examples },
    }))
}

/// Converts every value of a field to its new type. Each changed record is a normal write by the
/// system, in one batch, so its old value is in `record_history` and the batch can be undone.
pub fn convert_field(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app, table, field, actor) = (text(&payload, "app_slug")?, text(&payload, "table_id")?, text(&payload, "field")?, text(&payload, "actor")?);
    let (from, to): (FieldType, FieldType) = (parse(&payload, "from")?, parse(&payload, "to")?);
    let batch_id = uuid::Uuid::parse_str(&text(&payload, "batch_id")?).map_err(|e| JobError::Permanent(e.to_string()))?;
    let mut write = WriteCtx::system(&actor);
    write.batch_id = batch_id;
    write.index_fields = parse(&payload, "index_fields")?;
    write.links = parse(&payload, "links")?;
    let repo = ctx.repository();
    let total = count_rows(ctx, &app, &table)?;
    let (mut after, mut done) = resume(ctx);
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (a, t, f, from_id, w) = (app.clone(), table.clone(), field.clone(), after.clone(), write.clone());
        let (last, read) = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let rows = tx.query(
                    "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($3::text IS NULL OR id > $3) \
                     ORDER BY id LIMIT $4 FOR UPDATE",
                    &[&a, &t, &from_id, &(FIELD_BATCH / 2)],
                )?;
                let mut last: Option<String> = None;
                for row in &rows {
                    let (id, data): (String, Value) = (row.get(0), row.get(1));
                    last = Some(id.clone());
                    let Some(old) = data.get(&f) else { continue };
                    let mut new = data.clone();
                    match convert_value(old, from, to) {
                        Converted::Keep => continue,
                        Converted::To(v) => {
                            if let Some(m) = new.as_object_mut() {
                                m.insert(f.clone(), v);
                            }
                        }
                        Converted::Blank => {
                            if let Some(m) = new.as_object_mut() {
                                m.remove(&f);
                            }
                        }
                    }
                    crate::repository::update_record_in_tx(&mut tx, &a, &id, &new, None, &w)?;
                }
                tx.commit()?;
                Ok((last, rows.len() as i64))
            })
            .map_err(retry)?;
        done += read;
        if last.is_some() {
            after = last;
        }
        ctx.checkpoint(json!({ "after": after, "done": done }))?;
        ctx.progress(done, total.max(done))?;
        if read < FIELD_BATCH / 2 {
            break;
        }
    }
    let (a, t, f, job) = (app.clone(), table.clone(), field.clone(), ctx.job_id());
    repo.with_client(move |c| {
        c.execute(
            "UPDATE calc_status SET state = 'ready', updated_at = NOW() WHERE app_slug = $1 AND table_id = $2 AND field = $3 AND job_id = $4",
            &[&a, &t, &f, &job],
        )?;
        Ok(())
    })
    .map_err(retry)?;
    ctx.result(json!({ "records": done, "batch_id": batch_id }))
}


/// For each record of a table, how many records of another table hold the same text in the joined field. Nothing is written.
/// A record whose field is blank is counted apart: it has nothing to link.
pub fn relation_preview(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app, table, field, relation) = (text(&payload, "app_slug")?, text(&payload, "table_id")?, text(&payload, "field")?, text(&payload, "relation_id")?);
    let (target_table, target_field) = (text(&payload, "target_table_id")?, text(&payload, "target_field")?);
    let repo = ctx.repository();
    let total = count_rows(ctx, &app, &table)?;
    let (mut after, mut done) = resume(ctx);
    let (mut matched, mut unmatched, mut ambiguous, mut blank) = (0i64, 0i64, 0i64, 0i64);
    let (mut unmatched_examples, mut ambiguous_examples): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    if let Some(cp) = ctx.load_checkpoint() {
        let n = |k: &str| cp.get(k).and_then(Value::as_i64).unwrap_or(0);
        (matched, unmatched, ambiguous, blank) = (n("matched"), n("unmatched"), n("ambiguous"), n("blank"));
        unmatched_examples = cp.get("unmatched_examples").and_then(Value::as_array).cloned().unwrap_or_default();
        ambiguous_examples = cp.get("ambiguous_examples").and_then(Value::as_array).cloned().unwrap_or_default();
    }
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (a, t, f, from_id, tt, tf) = (app.clone(), table.clone(), field.clone(), after.clone(), target_table.clone(), target_field.clone());
        let (rows, found): (Vec<(String, Option<String>)>, HashMap<String, Vec<String>>) = repo
            .with_client(move |c| {
                let rows: Vec<(String, Option<String>)> = c
                    .query(
                        "SELECT id, data ->> $3::text FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($4::text IS NULL OR id > $4) ORDER BY id LIMIT $5",
                        &[&a, &t, &f, &from_id, &(FIELD_BATCH * 2)],
                    )?
                    .iter()
                    .map(|r| (r.get(0), r.get(1)))
                    .collect();
                let values: Vec<String> = rows.iter().filter_map(|(_, v)| v.clone()).filter(|v| !v.is_empty()).collect();
                let found = c
                    .query(
                        "SELECT data ->> $3::text, array_agg(id ORDER BY id) FROM dataset_records \
                         WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND data ->> $3::text = ANY($4) GROUP BY 1",
                        &[&a, &tt, &tf, &values],
                    )?
                    .iter()
                    .map(|r| (r.get::<_, String>(0), r.get::<_, Vec<String>>(1)))
                    .collect();
                Ok((rows, found))
            })
            .map_err(retry)?;
        for (id, value) in &rows {
            let Some(value) = value.as_deref().filter(|v| !v.is_empty()) else {
                blank += 1;
                continue;
            };
            match found.get(value).map(Vec::as_slice) {
                Some([_]) => matched += 1,
                Some(many) => {
                    ambiguous += 1;
                    if ambiguous_examples.len() < EXAMPLES {
                        ambiguous_examples.push(json!({ "id": id, "value": value, "matches": many }));
                    }
                }
                None => {
                    unmatched += 1;
                    if unmatched_examples.len() < EXAMPLES {
                        unmatched_examples.push(json!({ "id": id, "value": value }));
                    }
                }
            }
        }
        done += rows.len() as i64;
        if let Some((last, _)) = rows.last() {
            after = Some(last.clone());
        }
        ctx.checkpoint(json!({
            "after": after, "done": done, "matched": matched, "unmatched": unmatched, "ambiguous": ambiguous, "blank": blank,
            "unmatched_examples": unmatched_examples, "ambiguous_examples": ambiguous_examples,
        }))?;
        ctx.progress(done, total.max(done))?;
        if (rows.len() as i64) < FIELD_BATCH * 2 {
            break;
        }
    }
    ctx.result(json!({
        "relation_id": relation,
        "field": field,
        "scanned": done,
        "matched": matched,
        "unmatched": unmatched,
        "ambiguous": ambiguous,
        "blank": blank,
        "examples": { "unmatched": unmatched_examples, "ambiguous": ambiguous_examples },
    }))
}

/// Turns the text of a joined field into links. A record whose text matches exactly one record of the target table gets a
/// link to it. A record whose text matches none, or several, is left blank. Each changed record is a normal write by the
/// system in one batch, so the old text is in `record_history` and the batch can be undone. A record that is already a link
/// is left alone, so the job can run again.
pub fn convert_relation_to_link(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let (app, table, field, actor) = (text(&payload, "app_slug")?, text(&payload, "table_id")?, text(&payload, "field")?, text(&payload, "actor")?);
    let (target_table, target_field) = (text(&payload, "target_table_id")?, text(&payload, "target_field")?);
    let batch_id = uuid::Uuid::parse_str(&text(&payload, "batch_id")?).map_err(|e| JobError::Permanent(e.to_string()))?;
    let mut write = WriteCtx::system(&actor);
    write.batch_id = batch_id;
    write.index_fields = parse(&payload, "index_fields")?;
    write.links = parse(&payload, "links")?;
    let repo = ctx.repository();
    let total = count_rows(ctx, &app, &table)?;
    let (mut after, mut done) = resume(ctx);
    let (mut linked, mut blanked) = match ctx.load_checkpoint() {
        Some(cp) => (cp.get("linked").and_then(Value::as_i64).unwrap_or(0), cp.get("blanked").and_then(Value::as_i64).unwrap_or(0)),
        None => (0, 0),
    };
    loop {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }
        let (a, t, f, from_id, w) = (app.clone(), table.clone(), field.clone(), after.clone(), write.clone());
        let (tt, tf) = (target_table.clone(), target_field.clone());
        let (last, read, l, b) = repo
            .with_client(move |c| {
                let mut tx = c.transaction()?;
                let rows = tx.query(
                    "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND ($3::text IS NULL OR id > $3) \
                     ORDER BY id LIMIT $4 FOR UPDATE",
                    &[&a, &t, &from_id, &(FIELD_BATCH / 2)],
                )?;
                let page: Vec<(String, Value)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
                let values: Vec<String> = page.iter().filter_map(|(_, d)| d.get(&f).and_then(Value::as_str).map(str::to_string)).filter(|v| !v.is_empty()).collect();
                let found: HashMap<String, Vec<String>> = tx
                    .query(
                        "SELECT data ->> $3::text, array_agg(id ORDER BY id) FROM dataset_records \
                         WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND data ->> $3::text = ANY($4) GROUP BY 1",
                        &[&a, &tt, &tf, &values],
                    )?
                    .iter()
                    .map(|r| (r.get::<_, String>(0), r.get::<_, Vec<String>>(1)))
                    .collect();
                let (mut linked, mut blanked) = (0i64, 0i64);
                for (id, data) in &page {
                    // Only text is converted. A link already there, or no value at all, stays as it is.
                    let Some(old) = data.get(&f).and_then(Value::as_str).filter(|v| !v.is_empty()) else { continue };
                    let mut new = data.clone();
                    let Some(m) = new.as_object_mut() else { continue };
                    match found.get(old).map(Vec::as_slice) {
                        Some([only]) => {
                            m.insert(f.clone(), json!([only]));
                            linked += 1;
                        }
                        _ => {
                            m.remove(&f);
                            blanked += 1;
                        }
                    }
                    crate::repository::update_record_in_tx(&mut tx, &a, id, &new, None, &w)?;
                }
                tx.commit()?;
                Ok((page.last().map(|(id, _)| id.clone()), page.len() as i64, linked, blanked))
            })
            .map_err(retry)?;
        done += read;
        linked += l;
        blanked += b;
        if last.is_some() {
            after = last;
        }
        ctx.checkpoint(json!({ "after": after, "done": done, "linked": linked, "blanked": blanked }))?;
        ctx.progress(done, total.max(done))?;
        if read < FIELD_BATCH / 2 {
            break;
        }
    }
    let (a, t, f, job) = (app.clone(), table.clone(), field.clone(), ctx.job_id());
    repo.with_client(move |c| {
        c.execute(
            "UPDATE calc_status SET state = 'ready', updated_at = NOW() WHERE app_slug = $1 AND table_id = $2 AND field = $3 AND job_id = $4",
            &[&a, &t, &f, &job],
        )?;
        Ok(())
    })
    .map_err(retry)?;
    ctx.result(json!({ "records": done, "linked": linked, "blanked": blanked, "batch_id": batch_id }))
}

/// Converts every string value of a select field to its option ID.
/// Values matching no option become blank and are counted in the result.
/// Old values are written into record_history with system actor.
pub fn convert_select_options(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let app_slug = text(&payload, "app_slug")?;
    let repo = ctx.repository();
    let manifest = repo
        .get_app_manifest(&app_slug)
        .map_err(retry)?
        .ok_or_else(|| JobError::Permanent(format!("App manifest for '{app_slug}' not found")))?;

    let batch_id = uuid::Uuid::new_v4();
    let mut total_converted: i64 = 0;
    let mut total_blanked: i64 = 0;

    for table in &manifest.tables {
        let select_fields: Vec<scaffoldry_engine::FieldSpec> = table
            .fields
            .iter()
            .filter(|f| matches!(f.field_type, FieldType::Select | FieldType::MultiSelect))
            .cloned()
            .collect();
        if select_fields.is_empty() {
            continue;
        }

        let mut write = WriteCtx::system("system");
        write.batch_id = batch_id;
        let indexed_names = scaffoldry_engine::indexed_fields(&manifest, &table.id);
        write.index_fields = table.fields.iter().filter(|f| indexed_names.contains(&f.name)).cloned().collect();

        let mut after: Option<String> = None;
        loop {
            if ctx.cancelled()? {
                return Err(JobError::Cancelled);
            }
            let (a, t, from_id, w, s_fields) = (
                app_slug.clone(),
                table.id.clone(),
                after.clone(),
                write.clone(),
                select_fields.clone(),
            );

            let (last, read, conv, blank) = repo
                .with_client(move |c| {
                    let mut tx = c.transaction()?;
                    let rows = tx.query(
                        "SELECT id, data FROM dataset_records                          WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL                            AND ($3::text IS NULL OR id > $3)                          ORDER BY id LIMIT $4 FOR UPDATE",
                        &[&a, &t, &from_id, &100i64],
                    )?;
                    let mut last: Option<String> = None;
                    let mut batch_conv = 0i64;
                    let mut batch_blank = 0i64;

                    for row in &rows {
                        let (id, data): (String, Value) = (row.get(0), row.get(1));
                        last = Some(id.clone());
                        let mut new_data = data.clone();
                        let mut changed = false;

                        for f in &s_fields {
                            let Some(val) = data.get(&f.name) else { continue };
                            if val.is_null() { continue };

                            match f.field_type {
                                FieldType::Select => {
                                    if let Some(s) = val.as_str() {
                                        if f.select_options.iter().any(|o| o.id == s) {
                                            // Already an ID
                                            continue;
                                        }
                                        if let Some(opt) = f.select_options.iter().find(|o| o.label.eq_ignore_ascii_case(s) || o.label == s) {
                                            if let Some(m) = new_data.as_object_mut() {
                                                m.insert(f.name.clone(), Value::String(opt.id.clone()));
                                                changed = true;
                                                batch_conv += 1;
                                            }
                                        } else {
                                            // Unknown string -> blank
                                            if let Some(m) = new_data.as_object_mut() {
                                                m.remove(&f.name);
                                                changed = true;
                                                batch_blank += 1;
                                            }
                                        }
                                    }
                                }
                                FieldType::MultiSelect => {
                                    if let Some(arr) = val.as_array() {
                                        let mut new_arr = Vec::new();
                                        let mut arr_changed = false;
                                        for item in arr {
                                            if let Some(s) = item.as_str() {
                                                if f.select_options.iter().any(|o| o.id == s) {
                                                    new_arr.push(Value::String(s.to_string()));
                                                } else if let Some(opt) = f.select_options.iter().find(|o| o.label.eq_ignore_ascii_case(s) || o.label == s) {
                                                    new_arr.push(Value::String(opt.id.clone()));
                                                    arr_changed = true;
                                                    batch_conv += 1;
                                                } else {
                                                    arr_changed = true;
                                                    batch_blank += 1;
                                                }
                                            }
                                        }
                                        if arr_changed {
                                            if let Some(m) = new_data.as_object_mut() {
                                                if new_arr.is_empty() {
                                                    m.remove(&f.name);
                                                } else {
                                                    m.insert(f.name.clone(), Value::Array(new_arr));
                                                }
                                                changed = true;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        if changed {
                            crate::repository::update_record_in_tx(&mut tx, &a, &id, &new_data, None, &w)?;
                        }
                    }
                    tx.commit()?;
                    Ok((last, rows.len() as i64, batch_conv, batch_blank))
                })
                .map_err(retry)?;

            total_converted += conv;
            total_blanked += blank;
            if last.is_some() {
                after = last;
            }
            if read < 100 {
                break;
            }
        }
    }

    ctx.result(json!({
        "converted": total_converted,
        "blanked": total_blanked,
        "batch_id": batch_id.to_string(),
    }))
}

#[allow(clippy::type_complexity)]
pub fn recalc_volatile(ctx: &JobCtx, _payload: Value) -> Result<Value, JobError> {
    let repo = ctx.repository();
    let tables: Vec<(String, String)> = repo
        .with_client(|c| {
            let rows = c.query("SELECT DISTINCT app_slug, table_id FROM dataset_records WHERE deleted_at IS NULL", &[])?;
            Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
        .map_err(retry)?;

    let mut total_done = 0;
    for (app, table) in tables {
        let manifest_info: Option<(Vec<(String, String)>, Vec<scaffoldry_engine::FieldSpec>)> = repo.with_client({
            let (app, table) = (app.clone(), table.clone());
            move |c| {
                let row = c.query_opt(
                    "SELECT manifest FROM app_manifests WHERE slug = $1 ORDER BY version DESC LIMIT 1",
                    &[&app],
                )?;
                let Some(row) = row else { return Ok(None) };
                let val: Value = row.get(0);
                let Ok(manifest): Result<scaffoldry_engine::AppManifest, _> = serde_json::from_value(val) else { return Ok(None) };
                let Some(t) = manifest.tables.iter().find(|t| t.id == table) else { return Ok(None) };
                let volatile: Vec<(String, String)> = t.fields.iter()
                    .filter(|f| f.field_type == FieldType::Formula && f.formula_expression.as_deref().map(scaffoldry_engine::is_formula_volatile).unwrap_or(false))
                    .filter_map(|f| f.formula_expression.as_ref().map(|e| (f.name.clone(), e.clone())))
                    .collect();
                let indexed: Vec<scaffoldry_engine::FieldSpec> = t.fields.iter().filter(|f| f.indexed).cloned().collect();
                Ok(Some((volatile, indexed)))
            }
        }).map_err(retry)?;

        let Some((formulas, indexed_fields)) = manifest_info else { continue };
        if formulas.is_empty() {
            continue;
        }

        let mut after: Option<String> = None;
        loop {
            if ctx.cancelled()? {
                return Err(JobError::Cancelled);
            }
            let (a, t, from, f_list, idx) = (app.clone(), table.clone(), after.clone(), formulas.clone(), indexed_fields.clone());
            let (last, read_count) = repo
                .with_client(move |c| {
                    let mut tx = c.transaction()?;
                    let rows = match from {
                        None => tx.query(
                            "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL ORDER BY id LIMIT $3 FOR UPDATE",
                            &[&a, &t, &FIELD_BATCH],
                        )?,
                        Some(ref last_id) => tx.query(
                            "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND id > $3 ORDER BY id LIMIT $4 FOR UPDATE",
                            &[&a, &t, last_id, &FIELD_BATCH],
                        )?,
                    };
                    let mut last = None;
                    for row in &rows {
                        let id: String = row.get(0);
                        let mut data: Value = row.get(1);
                        last = Some(id.clone());
                        let mut changed = false;
                        for (field, expr) in &f_list {
                            let new_val = scaffoldry_engine::evaluate_formula(expr, &data);
                            if data.get(field) != Some(&new_val) {
                                if let Some(map) = data.as_object_mut() {
                                    map.insert(field.clone(), new_val);
                                    changed = true;
                                }
                            }
                        }
                        if changed {
                            crate::repository::set_record_data_in_tx(&mut tx, &a, &id, &data)?;
                            crate::repository_index::write_record_index(&mut tx, &a, &t, &id, &data, &idx)?;
                        }
                    }
                    tx.commit()?;
                    Ok((last, rows.len() as i64))
                })
                .map_err(retry)?;

            total_done += read_count;
            if last.is_some() {
                after = last;
            }
            if read_count < FIELD_BATCH {
                break;
            }
        }
    }
    ctx.result(json!({ "recomputed": total_done }))
}
