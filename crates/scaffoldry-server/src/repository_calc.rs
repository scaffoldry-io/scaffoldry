//! Recalculation status: which computed fields are being written again, and how far along.
//!
//! An approval that changes a definition calls `sync_calcs_in_tx` in its own transaction. It copies
//! nothing, so the approval stays fast on a table of any size. A job does the work.

use crate::repository::{PostgresRepository, RepositoryError};
use crate::repository_links::ComputedSpec;
use scaffoldry_engine::recalc::{changed_calculations, conversion_supported, type_changes};
use scaffoldry_engine::{AppManifest, FieldType};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mark(tx: &mut postgres::Transaction<'_>, app_slug: &str, table_id: &str, fields: &[String], job: uuid::Uuid) -> Result<(), postgres::Error> {
    for field in fields {
        tx.execute(
            "INSERT INTO calc_status (app_slug, table_id, field, state, job_id) VALUES ($1, $2, $3, 'calculating', $4) \
             ON CONFLICT (app_slug, table_id, field) DO UPDATE SET state = 'calculating', job_id = $4, updated_at = NOW()",
            &[&app_slug, &table_id, field, &job],
        )?;
    }
    Ok(())
}

/// The approved manifest changes a computed field's definition, or a field's type. Mark the fields
/// `calculating` and queue the jobs, in the approval's transaction. A repeat of the same change joins
/// the job already queued.
pub(crate) fn sync_calcs_in_tx(tx: &mut postgres::Transaction<'_>, old: Option<&AppManifest>, new: &AppManifest, requested_by: &str) -> Result<(), RepositoryError> {
    for change in changed_calculations(old, new) {
        let Some(table) = new.tables.iter().find(|t| t.id == change.table_id) else { continue };
        let indexed = scaffoldry_engine::indexed_fields(new, &table.id);
        let by_name = |name: &String| table.fields.iter().find(|f| &f.name == name);
        let formulas: Vec<(String, String)> = change
            .fields
            .iter()
            .filter_map(|n| by_name(n).filter(|f| f.field_type == FieldType::Formula).and_then(|f| f.formula_expression.clone().map(|e| (f.name.clone(), e))))
            .collect();
        let rollups: Vec<ComputedSpec> = change
            .fields
            .iter()
            .filter_map(by_name)
            .filter(|f| matches!(f.field_type, FieldType::Rollup | FieldType::Count))
            .filter_map(|f| {
                Some(ComputedSpec {
                    owner_table: table.id.clone(),
                    field: f.name.clone(),
                    via: f.computed_via.clone()?,
                    count: f.field_type == FieldType::Count,
                    function: f.rollup_function.clone().unwrap_or_else(|| "sum".to_string()),
                    target: f.target_display_field.clone(),
                    indexed: indexed.contains(&f.name),
                })
            })
            .collect();
        let index_fields: Vec<_> = table.fields.iter().filter(|f| indexed.contains(&f.name)).cloned().collect();
        let payload = json!({
            "app_slug": new.slug,
            "table_id": table.id,
            "fields": change.fields,
            "formulas": formulas,
            "rollups": rollups,
            "index_fields": index_fields,
        });
        let key = format!("recalc_field:{}", hex(&Sha256::digest(payload.to_string().as_bytes())));
        let job = crate::jobs::enqueue_in_tx(tx, "recalc_field", &payload, requested_by, Some(&key))?;
        mark(tx, &new.slug, &table.id, &change.fields, job)?;
    }
    if let Some(old) = old {
        for change in type_changes(old, new).into_iter().filter(|c| conversion_supported(c.from, c.to)) {
            let Some(table) = new.tables.iter().find(|t| t.id == change.table_id) else { continue };
            let indexed = scaffoldry_engine::indexed_fields(new, &table.id);
            let index_fields: Vec<_> = table.fields.iter().filter(|f| indexed.contains(&f.name)).cloned().collect();
            let payload = json!({
                "app_slug": new.slug,
                "table_id": table.id,
                "field": change.field,
                "from": change.from,
                "to": change.to,
                "batch_id": uuid::Uuid::new_v4(),
                "actor": requested_by,
                "index_fields": index_fields,
                "links": crate::repository_links::link_specs_in(new, &table.id),
            });
            // The batch id is part of the payload, so two approvals never share a job.
            let key = format!("convert_field:{}", hex(&Sha256::digest(payload.to_string().as_bytes())));
            let job = crate::jobs::enqueue_in_tx(tx, "convert_field", &payload, requested_by, Some(&key))?;
            mark(tx, &new.slug, &table.id, std::slice::from_ref(&change.field), job)?;
        }
    }
    Ok(())
}

impl PostgresRepository {
    /// The fields of a table that are being calculated, each with the percent its job has done.
    pub fn calc_states(&self, app_slug: &str, table_id: &str) -> Result<Vec<(String, i64)>, RepositoryError> {
        let (slug, table) = (app_slug.to_string(), table_id.to_string());
        self.with_client(move |client| {
            let rows = client.query(
                "SELECT s.field, COALESCE(j.progress_done, 0), COALESCE(j.progress_total, 0) \
                 FROM calc_status s LEFT JOIN jobs j ON j.id = s.job_id \
                 WHERE s.app_slug = $1 AND s.table_id = $2 AND s.state = 'calculating' ORDER BY s.field",
                &[&slug, &table],
            )?;
            Ok(rows
                .iter()
                .map(|r| {
                    let (done, total): (i64, i64) = (r.get(1), r.get(2));
                    (r.get::<_, String>(0), if total > 0 { (done * 100 / total).clamp(0, 99) } else { 0 })
                })
                .collect())
        })
    }

    /// The result of the newest finished preview of this conversion, when there is one.
    pub fn latest_convert_preview(&self, app_slug: &str, table_id: &str, field: &str, to: FieldType) -> Result<Option<Value>, RepositoryError> {
        let (slug, table, field) = (app_slug.to_string(), table_id.to_string(), field.to_string());
        let to = serde_json::to_value(to)?;
        self.with_client(move |client| {
            let row = client.query_opt(
                "SELECT result FROM jobs WHERE kind = 'convert_preview' AND state = 'done' \
                   AND payload->>'app_slug' = $1 AND payload->>'table_id' = $2 AND payload->>'field' = $3 AND payload->'to' = $4 \
                 ORDER BY finished_at DESC NULLS LAST LIMIT 1",
                &[&slug, &table, &field, &to],
            )?;
            Ok(row.and_then(|r| r.get::<_, Option<Value>>(0)))
        })
    }
}
