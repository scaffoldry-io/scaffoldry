//! Export jobs: stream records to CSV, XLSX, JSON, or JSONL files.

use crate::files;
use crate::jobs::{JobCtx, JobError};
use crate::repository::AppendDecisionParams;
use crate::repository_disclosure::NewDisclosure;
use crate::repository_files::NewRef;
use scaffoldry_core::access::{row_scope, Person, RowOp, RowRule, RowScope};
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Map, Value};
use uuid::Uuid;

pub const DEFAULT_MAX_EXPORT_ROWS: usize = 100_000;

/// Prepends a leading apostrophe if string begins with =, +, -, or @
/// to prevent CSV and spreadsheet formula injection / DDE execution.
pub fn neutralize_cell_value(val: &str) -> String {
    if val.starts_with('=') || val.starts_with('+') || val.starts_with('-') || val.starts_with('@') {
        format!("'{val}")
    } else {
        val.to_string()
    }
}

fn escape_csv_field(val: &str) -> String {
    if val.contains(',') || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

pub fn export_records_to_csv(
    headers: &[String],
    records: &[Map<String, Value>],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let header_line = headers.iter().map(|h| escape_csv_field(h)).collect::<Vec<_>>().join(",");
    out.extend_from_slice(header_line.as_bytes());
    out.extend_from_slice(b"
");

    for rec in records {
        let mut row_fields = Vec::with_capacity(headers.len());
        for h in headers {
            let val_str = match rec.get(h) {
                Some(Value::String(s)) => neutralize_cell_value(s),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => neutralize_cell_value(&other.to_string()),
            };
            row_fields.push(escape_csv_field(&val_str));
        }
        let line = row_fields.join(",");
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"
");
    }
    Ok(out)
}

pub fn export_records_to_xlsx(
    headers: &[String],
    records: &[Map<String, Value>],
) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::Workbook;
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Export").map_err(|e| e.to_string())?;

    for (col, h) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, h).map_err(|e| e.to_string())?;
    }

    for (row_idx, rec) in records.iter().enumerate() {
        let r = (row_idx + 1) as u32;
        for (col, h) in headers.iter().enumerate() {
            let c = col as u16;
            match rec.get(h) {
                Some(Value::String(s)) => {
                    let neutralized = neutralize_cell_value(s);
                    worksheet.write_string(r, c, &neutralized).map_err(|e| e.to_string())?;
                }
                Some(Value::Number(n)) => {
                    if let Some(f) = n.as_f64() {
                        worksheet.write_number(r, c, f).map_err(|e| e.to_string())?;
                    } else if let Some(i) = n.as_i64() {
                        worksheet.write_number(r, c, i as f64).map_err(|e| e.to_string())?;
                    }
                }
                Some(Value::Bool(b)) => {
                    worksheet.write_boolean(r, c, *b).map_err(|e| e.to_string())?;
                }
                Some(Value::Null) | None => {}
                Some(other) => {
                    let s = other.to_string();
                    let neutralized = neutralize_cell_value(&s);
                    worksheet.write_string(r, c, &neutralized).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    workbook.save_to_buffer().map_err(|e| e.to_string())
}

pub fn export_records_to_json(
    records: &[Map<String, Value>],
) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(records).map_err(|e| e.to_string())
}

pub fn export_records_to_jsonl(
    records: &[Map<String, Value>],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for rec in records {
        let line = serde_json::to_string(rec).map_err(|e| e.to_string())?;
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

/// Job: export_run
pub fn export_run(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let export_id = payload.get("export_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("Missing or invalid export_id".to_string()))?;

    let app_slug = payload.get("app_slug")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JobError::Permanent("Missing app_slug".to_string()))?
        .to_string();

    let table_id_opt = payload.get("table_id").and_then(|v| v.as_str());
    let format = payload.get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("csv")
        .to_lowercase();

    let caller_eppn = payload.get("caller_eppn")
        .and_then(|v| v.as_str())
        .ok_or_else(|| JobError::Permanent("Missing caller_eppn: an export is made for a person".to_string()))?
        .to_string();

    let mock_row_count = payload.get("mock_row_count").and_then(Value::as_u64);
    if let Some(count) = mock_row_count {
        if count > DEFAULT_MAX_EXPORT_ROWS as u64 {
            return Err(JobError::Permanent("Export exceeds limit of 100,000 records. Please narrow your query using a filter.".to_string()));
        }
    }

    let repo = ctx.repository();
    let manifest = repo.get_app_manifest(&app_slug)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("App manifest '{app_slug}' not found")))?;

    let table = match table_id_opt {
        Some(t_id) => manifest.tables.iter().find(|t| t.id == t_id || t.slug == t_id)
            .ok_or_else(|| JobError::Permanent(format!("Table '{t_id}' not found in app '{app_slug}'")))?,
        None => manifest.tables.first()
            .ok_or_else(|| JobError::Permanent(format!("No tables defined in app '{app_slug}'")))?,
    };

    let requested_fields: Vec<String> = payload.get("fields")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_else(|| table.fields.iter().map(|f| f.name.clone()).collect());

    let disallowed: Vec<String> = payload.get("disallowed_fields")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    let mut allowed_fields = Vec::new();
    let mut left_out_fields = Vec::new();

    for f_name in requested_fields {
        if disallowed.contains(&f_name) {
            left_out_fields.push(f_name);
        } else {
            allowed_fields.push(f_name);
        }
    }

    // What the reader sees of a field they are not cleared for: its mask, from the same function a list uses.
    let masks: std::collections::BTreeMap<String, scaffoldry_core::mask::Mask> = payload
        .get("masks")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let row_rules: Vec<RowRule> = payload.get("row_rules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let caller_person: Option<Person> = payload.get("caller_person")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let (slug_q, table_q) = (app_slug.clone(), table.id.clone());
    let raw_records: Vec<(String, Value)> = repo.with_client(move |c| {
        let sql = "SELECT id, data FROM dataset_records WHERE app_slug = $1 AND (table_id = $2 OR $2 = '' OR table_id IS NULL) AND deleted_at IS NULL ORDER BY created_at ASC, id ASC LIMIT $3";
        let max_fetch = (DEFAULT_MAX_EXPORT_ROWS + 1) as i64;
        let rows = c.query(sql, &[&slug_q, &table_q, &max_fetch])?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }).map_err(|e| JobError::Permanent(e.to_string()))?;

    if raw_records.len() > DEFAULT_MAX_EXPORT_ROWS {
        return Err(JobError::Permanent("Export exceeds limit of 100,000 records. Please narrow your query using a filter.".to_string()));
    }

    let mut filtered_records: Vec<(String, Map<String, Value>)> = Vec::new();
    let row_scope_opt = if !row_rules.is_empty() {
        caller_person.as_ref().map(|p| row_scope(p, None, false, &row_rules, &table.id, RowOp::Read))
    } else {
        None
    };

    for (rec_id, data_val) in raw_records {
        let allowed = match &row_scope_opt {
            Some(RowScope::AllowAll) | None => true,
            Some(RowScope::DenyAll) => false,
            Some(RowScope::Mixed(pred)) => scaffoldry_engine::access::eval(pred, &data_val, &table.fields),
        };
        if allowed {
            if let Value::Object(map) = data_val {
                filtered_records.push((rec_id, map));
            }
        }
    }

    let mut export_maps: Vec<Map<String, Value>> = Vec::new();
    let mut exported_ids: Vec<String> = Vec::new();

    for (rec_id, map) in filtered_records {
        exported_ids.push(rec_id);
        let mut row_map = Map::new();
        for field in &allowed_fields {
            if let Some(v) = map.get(field) {
                let shown = match masks.get(field) {
                    Some(m) => scaffoldry_core::mask::mask(v, *m),
                    None => v.clone(),
                };
                row_map.insert(field.clone(), shown);
            } else {
                row_map.insert(field.clone(), Value::Null);
            }
        }
        export_maps.push(row_map);
    }

    let (output_bytes, media_type, ext) = match format.as_str() {
        "xlsx" => (
            export_records_to_xlsx(&allowed_fields, &export_maps)
                .map_err(JobError::Permanent)?,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        ),
        "json" => (
            export_records_to_json(&export_maps)
                .map_err(JobError::Permanent)?,
            "application/json",
            "json",
        ),
        "jsonl" => (
            export_records_to_jsonl(&export_maps)
                .map_err(JobError::Permanent)?,
            "application/x-ndjson",
            "jsonl",
        ),
        _ => (
            export_records_to_csv(&allowed_fields, &export_maps)
                .map_err(JobError::Permanent)?,
            "text/csv",
            "csv",
        ),
    };

    let dir = files::dir().ok_or_else(|| JobError::Permanent("File storage not configured".to_string()))?;
    let sha256 = files::store_bytes(&dir, &output_bytes)
        .map_err(|e| JobError::Permanent(format!("Failed to store export file: {e}")))?;

    let filename = format!("{}_export_{}.{}", table.slug, &export_id.to_string()[..8], ext);

    repo.insert_file_ref(NewRef {
        id: export_id,
        sha256: &sha256,
        size: output_bytes.len() as i64,
        media_type,
        app_slug: &app_slug,
        name: &filename,
        uploaded_by: &caller_eppn,
        scan_state: "skipped",
    }).map_err(|e| JobError::Permanent(format!("Failed to record file ref: {e}")))?;

    // The request decided which fields are sensitive, through labels and categories as well as the manifest.
    let sensitive: Vec<String> = payload.get("sensitive_fields")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let sensitive_exported: Vec<String> = allowed_fields.iter().filter(|f| sensitive.contains(f)).cloned().collect();

    if !sensitive_exported.is_empty() {
        repo.insert_disclosure(NewDisclosure {
            kind: "export".to_string(),
            actor: caller_eppn.clone(),
            app_slug: app_slug.clone(),
            table_id: Some(table.id.clone()),
            record_ids: exported_ids.clone(),
            fields: sensitive_exported.clone(),
            recipient: None,
            note: format!("Exported {} records from table '{}'", export_maps.len(), table.id),
        }).map_err(|e| JobError::Permanent(format!("Failed to write disclosure log: {e}")))?;
        // Each disclosure also writes its receipt.
        let receipt = crate::service::receipts::disclosure_receipt(&caller_eppn, "export", &app_slug, Some(&table.id));
        repo.insert_access_receipt(&receipt).map_err(|e| JobError::Retry(format!("Failed to write the receipt: {e}")))?;

        let org_code = if manifest.organization_code.is_empty() {
            "SCI".to_string()
        } else {
            manifest.organization_code.clone()
        };

        repo.append_ledger_decision(AppendDecisionParams {
            principal: caller_eppn.clone(),
            organization_code: org_code,
            app_slug: Some(app_slug.clone()),
            decision_type: DecisionType::DataExported,
            oscal_control_id: "AU-2".to_string(),
            rationale: "Export of dataset records".to_string(),
            payload: json!({
                "app_slug": app_slug,
                "table_id": table.id,
                "record_count": export_maps.len(),
                "fields": allowed_fields,
                "sensitive_fields": sensitive_exported,
                "format": format,
            }),
        }).map_err(|e| JobError::Permanent(format!("Failed to write governance ledger: {e}")))?;
    }

    let left_out_msg = if left_out_fields.is_empty() {
        String::new()
    } else {
        format!("Left out: {}.", left_out_fields.join(", "))
    };

    ctx.result(json!({
        "export_id": export_id.to_string(),
        "job_id": ctx.job_id().to_string(),
        "download_url": format!("/api/v1/apps/{}/exports/{}/download", app_slug, export_id),
        "format": format,
        "record_count": export_maps.len(),
        "left_out": left_out_fields,
        "message": left_out_msg,
    }))
}
