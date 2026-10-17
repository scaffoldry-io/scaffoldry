//! Background jobs for import phase 1: analyze, dry-run validate, and run.

use calamine::DataType;
use crate::jobs::{JobCtx, JobError};
use crate::repository::{ActorKind, AppendDecisionParams, PostgresRepository, WriteCtx};
use crate::repository_files::NewRef;
use crate::state::DatasetRecord;
use chrono::Utc;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::infer::{infer_column_type, AnalyzeOptions};
use scaffoldry_core::access::{row_scope, Person, RowOp, RowRule, RowScope};
use scaffoldry_engine::FieldType;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use uuid::Uuid;

pub const DEFAULT_MAX_ROWS: u64 = 500_000;
pub const DEFAULT_MAX_MB: u64 = 100;

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

pub fn detect_delimiter(first_chunk: &str) -> char {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for ch in [',', '\t', ';', '|'] {
        counts.insert(ch, 0);
    }
    for line in first_chunk.lines().take(10) {
        let mut in_quotes = false;
        for c in line.chars() {
            if c == '"' {
                in_quotes = !in_quotes;
            } else if !in_quotes {
                if let Some(count) = counts.get_mut(&c) {
                    *count += 1;
                }
            }
        }
    }
    let mut best_delim = ',';
    let mut max_count = 0;
    for (d, cnt) in counts {
        if cnt > max_count {
            max_count = cnt;
            best_delim = d;
        }
    }
    best_delim
}

pub fn parse_csv_stream<R: Read>(
    reader: R,
    delimiter: char,
    limit_rows: Option<usize>,
) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;

    let mut buf_reader = std::io::BufReader::new(reader);
    let mut byte_buf = Vec::new();
    buf_reader.read_to_end(&mut byte_buf).map_err(|e| e.to_string())?;

    let slice = if byte_buf.starts_with(b"\xef\xbb\xbf") {
        &byte_buf[3..]
    } else {
        &byte_buf[..]
    };

    let text = match std::str::from_utf8(slice) {
        Ok(t) => t,
        Err(_) => return Err("Invalid file encoding: only UTF-8 is supported.".to_string()),
    };

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(ch);
            }
        } else {
            if ch == '"' {
                in_quotes = true;
            } else if ch == delimiter {
                current_row.push(current_field.trim().to_string());
                current_field.clear();
            } else if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                current_row.push(current_field.trim().to_string());
                current_field.clear();
                rows.push(std::mem::take(&mut current_row));
                if let Some(limit) = limit_rows {
                    if rows.len() > limit {
                        break;
                    }
                }
            } else if ch == '\n' {
                current_row.push(current_field.trim().to_string());
                current_field.clear();
                rows.push(std::mem::take(&mut current_row));
                if let Some(limit) = limit_rows {
                    if rows.len() > limit {
                        break;
                    }
                }
            } else {
                current_field.push(ch);
            }
        }
    }

    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field.trim().to_string());
        rows.push(current_row);
    }

    if rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let headers = rows.remove(0);
    Ok((headers, rows))
}

pub fn read_file_raw(repo: &PostgresRepository, file_ref_id: Uuid) -> Result<(Vec<u8>, String), JobError> {
    let dir = crate::files::dir().ok_or_else(|| JobError::Permanent("File storage is not configured.".to_string()))?;
    let (sha256, _name) = repo
        .with_client(move |c| {
            let row = c.query_opt(
                "SELECT sha256, name FROM file_refs WHERE id = $1",
                &[&file_ref_id],
            )?;
            Ok(row.map(|r| (r.get::<_, String>(0).trim().to_string(), r.get::<_, String>(1))))
        })
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("File reference {} not found.", file_ref_id)))?;

    let path = crate::files::blob_path(&dir, &sha256);
    let bytes = std::fs::read(&path)
        .map_err(|e| JobError::Permanent(format!("Could not read blob {}: {}", path.display(), e)))?;
    Ok((bytes, sha256))
}


pub fn check_spreadsheet_safety(name: &str, bytes: &[u8]) -> Result<(), String> {
    let lower_name = name.to_lowercase();
    if lower_name.ends_with(".xlsm") {
        return Err("Spreadsheets containing macros (.xlsm) cannot be imported.".to_string());
    }

    if bytes.starts_with(b"PK") {
        let cursor = std::io::Cursor::new(bytes);
        if let Ok(mut archive) = zip::ZipArchive::new(cursor) {
            let mut total_uncompressed: u64 = 0;
            let mut total_compressed: u64 = 0;

            for i in 0..archive.len() {
                if let Ok(file) = archive.by_index(i) {
                    let entry_name = file.name().to_lowercase();
                    if entry_name.contains("vbaproject.bin") || entry_name.starts_with("xl/vba") {
                        return Err("Spreadsheets containing macros cannot be imported.".to_string());
                    }
                    total_uncompressed += file.size();
                    total_compressed += file.compressed_size();
                }
            }

            let archive_size = bytes.len() as u64;
            let baseline = if total_compressed > 0 { total_compressed } else { archive_size };
            if total_uncompressed > 20 * baseline && total_uncompressed > 50_000 {
                return Err("Decompression bomb detected: uncompressed size exceeds 20 times the archive size.".to_string());
            }
        }
    }

    Ok(())
}

pub fn format_excel_cell(cell: &calamine::Data) -> String {
    if cell.is_datetime() {
        if let Some(dt) = cell.as_datetime() {
            if dt.time() == chrono::NaiveTime::MIN {
                return dt.format("%Y-%m-%d").to_string();
            } else {
                return dt.format("%Y-%m-%dT%H:%M:%S").to_string();
            }
        }
    }
    {
        match cell {
            calamine::Data::Empty => String::new(),
            calamine::Data::String(s) => s.clone(),
            calamine::Data::Float(f) => {
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                    (*f as i64).to_string()
                } else {
                    f.to_string()
                }
            }
            calamine::Data::Int(i) => i.to_string(),
            calamine::Data::Bool(b) => b.to_string(),
            calamine::Data::DateTimeIso(s) => s.clone(),
            calamine::Data::DurationIso(s) => s.clone(),
            calamine::Data::Error(e) => format!("{:?}", e),
            calamine::Data::DateTime(d) => {
                if let Some(dt) = d.as_datetime() {
                    if dt.time() == chrono::NaiveTime::MIN {
                        dt.format("%Y-%m-%d").to_string()
                    } else {
                        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
                    }
                } else {
                    d.to_string()
                }
            }
        }
    }
}

pub type ParsedSpreadsheet = (Vec<String>, Vec<Vec<String>>, Vec<String>);

pub fn parse_rows_from_bytes(
    bytes: &[u8],
    sheet: Option<&str>,
    delimiter: Option<char>,
    max_rows: Option<usize>,
) -> Result<ParsedSpreadsheet, String> {
    if bytes.starts_with(b"PK") {
        use calamine::{Reader, Xlsx};
        let mut workbook = Xlsx::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("Failed to open Excel spreadsheet: {e}"))?;
        let sheet_names = workbook.sheet_names().to_vec();
        if sheet_names.is_empty() {
            return Err("Excel workbook contains no sheets.".to_string());
        }
        let target_sheet = sheet.unwrap_or(&sheet_names[0]);
        let range = workbook.worksheet_range(target_sheet)
            .map_err(|e| format!("Failed to read sheet '{target_sheet}': {e}"))?;

        let mut rows_iter = range.rows();
        let headers: Vec<String> = match rows_iter.next() {
            Some(row) => row.iter().map(|c| c.to_string().trim().to_string()).collect(),
            None => return Ok((vec![], vec![], sheet_names)),
        };

        let mut data_rows = Vec::new();
        for row in rows_iter {
            if let Some(max) = max_rows {
                if data_rows.len() >= max {
                    break;
                }
            }
            let row_vec: Vec<String> = row.iter().map(format_excel_cell).collect();
            data_rows.push(row_vec);
        }
        Ok((headers, data_rows, sheet_names))
    } else {
        let slice = if bytes.starts_with(b"\xEF\xBB\xBF") { &bytes[3..] } else { bytes };
        let delim = delimiter.unwrap_or(',');
        let (headers, data_rows) = parse_csv_stream(slice, delim, max_rows)?;
        Ok((headers, data_rows, vec![]))
    }
}

/// Job: import_analyze
pub fn import_analyze(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let import_id = payload.get("import_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("Missing or invalid import_id".to_string()))?;
    let app_slug = payload.get("app_slug")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let repo = ctx.repository();
    let import = repo
        .get_import(app_slug, import_id)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("Import {} not found", import_id)))?;

    let (bytes, _sha) = read_file_raw(repo, import.file_ref_id)?;

    let (delimiter_str, encoding_str, headers, sample_rows, sheets) = if bytes.starts_with(b"PK") {
        let (headers, sample_rows, sheet_names) = parse_rows_from_bytes(&bytes, None, None, Some(5000))
            .map_err(JobError::Permanent)?;
        ("xlsx".to_string(), "binary".to_string(), headers, sample_rows, sheet_names)
    } else {
        let slice = if bytes.starts_with(b"\xEF\xBB\xBF") {
            &bytes[3..]
        } else {
            &bytes[..]
        };

        let text_sample = match std::str::from_utf8(slice) {
            Ok(t) => t,
            Err(_) => {
                let err_msg = "Invalid file encoding: only UTF-8 is supported.";
                let _ = repo.update_import_analysis(
                    import_id,
                    "failed",
                    ",",
                    "unknown",
                    &json!([]),
                    &json!([]),
                    &json!({ "failure": err_msg }),
                );
                return Err(JobError::Permanent(err_msg.to_string()));
            }
        };

        let delimiter = detect_delimiter(&text_sample[..text_sample.len().min(4096)]);
        let (headers, sample_rows) = parse_csv_stream(slice, delimiter, Some(5000))
            .map_err(JobError::Permanent)?;
        (delimiter.to_string(), "UTF-8".to_string(), headers, sample_rows, vec![])
    };

    if headers.is_empty() {
        return Err(JobError::Permanent("File contains no headers or columns.".to_string()));
    }

    let people_set: HashSet<String> = repo.with_client(|c| {
        let rows = c.query("SELECT eppn FROM admin_people", &[])?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0).to_lowercase()).collect())
    }).unwrap_or_default();

    let is_person = |s: &str| people_set.contains(&s.trim().to_lowercase());
    let analyze_opts = AnalyzeOptions {
        is_person: Some(is_person),
    };

    let mut inferred_types = Map::new();
    let mut sample_json = Vec::new();

    for (col_idx, header) in headers.iter().enumerate() {
        let col_values: Vec<&str> = sample_rows.iter()
            .filter_map(|r| r.get(col_idx).map(|s| s.as_str()))
            .collect();
        let inf = infer_column_type(&col_values, &analyze_opts);
        inferred_types.insert(header.clone(), json!(inf));
    }

    for row in sample_rows.iter().take(50) {
        let mut obj = Map::new();
        for (col_idx, header) in headers.iter().enumerate() {
            let val = row.get(col_idx).cloned().unwrap_or_default();
            obj.insert(header.clone(), Value::String(val));
        }
        sample_json.push(Value::Object(obj));
    }

    let columns_val = json!(headers);
    let sample_val = if sheets.is_empty() {
        json!(sample_json)
    } else {
        json!({
            "rows": sample_json,
            "sheets": sheets,
        })
    };
    let inferred_val = Value::Object(inferred_types);

    repo.update_import_analysis(
        import_id,
        "analyzed",
        &delimiter_str,
        &encoding_str,
        &columns_val,
        &sample_val,
        &inferred_val,
    ).map_err(|e| JobError::Retry(e.to_string()))?;

    ctx.result(json!({
        "delimiter": delimiter_str,
        "encoding": encoding_str,
        "columns": columns_val,
        "sample": sample_val,
        "inferred_types": inferred_val,
    }))
}

/// Job: import_validate (dry-run)
pub fn import_validate(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let import_id = payload.get("import_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("Missing or invalid import_id".to_string()))?;
    let app_slug = payload.get("app_slug")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let _caller_eppn = payload.get("caller_eppn")
        .and_then(|v| v.as_str())
        .unwrap_or("user@university.edu");

    let repo = ctx.repository();
    let import = repo
        .get_import(app_slug, import_id)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("Import {} not found", import_id)))?;

    let mapping = payload.get("mapping")
        .or(import.mapping.as_ref())
        .ok_or_else(|| JobError::Permanent("Import mapping is required".to_string()))?;

    let target_table_id = mapping.get("target_table")
        .or_else(|| mapping.get("target_table_id"))
        .and_then(|v| v.as_str())
        .or(import.target_table_id.as_deref())
        .ok_or_else(|| JobError::Permanent("Mapping must specify target_table".to_string()))?;

    let (bytes, _sha) = read_file_raw(repo, import.file_ref_id)?;
    let delimiter = import.delimiter.chars().next().unwrap_or(',');
    let sheet = payload.get("sheet")
        .and_then(|v| v.as_str())
        .or_else(|| mapping.get("sheet").and_then(|v| v.as_str()));
    let (headers, rows, _) = parse_rows_from_bytes(&bytes, sheet, Some(delimiter), None)
        .map_err(JobError::Permanent)?;

    let rules: Vec<RowRule> = payload.get("row_rules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let person: Option<Person> = payload.get("caller_person")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let max_rows = payload.get("max_rows").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_MAX_ROWS) as usize;
    if rows.len() > max_rows {
        let err = format!("File exceeds the maximum row limit of {} rows.", max_rows);
        return Err(JobError::Permanent(err));
    }

    let manifest = repo.get_app_manifest(app_slug)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("App {} not found", app_slug)))?;
    let table = manifest.tables.iter().find(|t| t.id == target_table_id || t.slug == target_table_id)
        .ok_or_else(|| JobError::Permanent(format!("Table {} not found in app", target_table_id)))?;

    let upsert_key = mapping.get("upsert_key").and_then(|v| v.as_str());

    let mut field_map: HashMap<String, String> = HashMap::new();
    if let Some(col_map) = mapping.get("fields").or_else(|| mapping.get("column_mappings")).or_else(|| mapping.get("columns")).and_then(|v| v.as_object()) {
        for (k, v) in col_map {
            if let Some(target) = v.as_str() {
                if !target.is_empty() {
                    field_map.insert(k.clone(), target.to_string());
                }
            }
        }
    } else if let Some(fields_map) = mapping.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields_map {
            if let Some(target) = v.get("target_field").and_then(|t| t.as_str()) {
                if !target.is_empty() {
                    field_map.insert(k.clone(), target.to_string());
                }
            }
        }
    }

    let mut select_strategies: HashMap<String, String> = HashMap::new();
    if let Some(strat_map) = mapping.get("select_strategies").and_then(|v| v.as_object()) {
        for (k, v) in strat_map {
            if let Some(s) = v.as_str() {
                select_strategies.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(fields_map) = mapping.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields_map {
            if let Some(s) = v.get("unknown_select").and_then(|t| t.as_str()) {
                select_strategies.insert(k.clone(), s.to_string());
            }
        }
    }

    let mut valid_count: usize = 0;
    let mut invalid_count: usize = 0;
    let mut create_count: usize = 0;
    let mut update_count: usize = 0;
    let mut errors: Vec<Value> = Vec::new();
    let mut proposed_options: HashMap<String, HashSet<String>> = HashMap::new();

    for (row_idx, row) in rows.iter().enumerate() {
        let row_num = row_idx + 1;
        let mut row_has_error = false;
        let mut row_data = Map::new();

        for (col_idx, header) in headers.iter().enumerate() {
            let Some(target_field_name) = field_map.get(header) else {
                continue;
            };
            let cell_value = row.get(col_idx).map(|s| s.as_str()).unwrap_or("").trim();
            let target_field = table.fields.iter().find(|f| &f.name == target_field_name);

            if let Some(f) = target_field {
                match &f.field_type {
                    FieldType::Number => {
                        if !cell_value.is_empty() {
                            if cell_value.parse::<f64>().is_ok() {
                                row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                            } else {
                                row_has_error = true;
                                if errors.len() < 100 {
                                    errors.push(json!({
                                        "row": row_num,
                                        "column": header,
                                        "message": format!("Row {}: '{}' is not a valid number for '{}'", row_num, cell_value, target_field_name),
                                    }));
                                }
                            }
                        }
                    }
                    FieldType::Select => {
                        if !cell_value.is_empty() {
                            let allowed: Vec<String> = f.select_options.iter().map(|o| o.label.clone()).collect();

                            if allowed.contains(&cell_value.to_string()) {
                                row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                            } else {
                                let strategy = select_strategies.get(target_field_name).map(|s| s.as_str()).unwrap_or("reject");
                                match strategy {
                                    "blank" => {
                                        row_data.insert(target_field_name.clone(), Value::Null);
                                    }
                                    "propose_option" => {
                                        proposed_options.entry(target_field_name.clone()).or_default().insert(cell_value.to_string());
                                        row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                                    }
                                    _ => {
                                        row_has_error = true;
                                        if errors.len() < 100 {
                                            errors.push(json!({
                                                "row": row_num,
                                                "column": header,
                                                "message": format!("Row {}: Unknown option '{}' for select field '{}'", row_num, cell_value, target_field_name),
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    FieldType::Link => {
                        if !cell_value.is_empty() {
                            if let Some(ref rel_table_id) = f.target_table_id {
                                let (q_app, q_rel, q_field, q_val) = (
                                    app_slug.to_string(),
                                    rel_table_id.to_string(),
                                    f.target_display_field.clone().unwrap_or_else(|| "name".to_string()),
                                    cell_value.to_string(),
                                );
                                let matching_ids: Vec<String> = repo.with_client(move |c| {
                                    let q = "SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND (data->>$3 = $4 OR data->>'name' = $4 OR data->>'title' = $4)";
                                    let query_rows = c.query(q, &[&q_app, &q_rel, &q_field, &q_val])?;
                                    Ok(query_rows.iter().map(|r| r.get(0)).collect())
                                }).unwrap_or_default();

                                if matching_ids.is_empty() {
                                    row_has_error = true;
                                    if errors.len() < 100 {
                                        errors.push(json!({
                                            "row": row_num,
                                            "column": header,
                                            "message": format!("Row {}: Link target '{}' was not found in table '{}'", row_num, cell_value, rel_table_id),
                                        }));
                                    }
                                } else if matching_ids.len() > 1 {
                                    row_has_error = true;
                                    if errors.len() < 100 {
                                        errors.push(json!({
                                            "row": row_num,
                                            "column": header,
                                            "message": format!("Row {}: Link target '{}' matched multiple records in table '{}' and is ambiguous", row_num, cell_value, rel_table_id),
                                        }));
                                    }
                                } else {
                                    row_data.insert(target_field_name.clone(), Value::String(matching_ids[0].clone()));
                                }
                            }
                        }
                    }
                    _ => {
                        row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                    }
                }
            } else {
                row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
            }
        }

        if !row_has_error && !rules.is_empty() {
            if let Some(ref p) = person {
                let scope = row_scope(p, None, false, &rules, &table.id, RowOp::Create);
                let allowed = match scope {
                    RowScope::AllowAll => true,
                    RowScope::DenyAll => false,
                    RowScope::Mixed(pred) => {
                        scaffoldry_engine::access::eval(&pred, &Value::Object(row_data.clone()), &table.fields)
                    }
                };
                if !allowed {
                    row_has_error = true;
                    if errors.len() < 100 {
                        errors.push(json!({
                            "row": row_num,
                            "column": "_row_",
                            "message": format!("Row {}: Access denied by row rules: row is outside caller's create scope", row_num),
                        }));
                    }
                }
            }
        }

        if row_has_error {
            invalid_count += 1;
        } else {
            valid_count += 1;
            if let Some(key) = upsert_key {
                if let Some(key_val) = row_data.get(key).and_then(|v| v.as_str()) {
                    let (q_app, q_tbl, q_key, q_val) = (
                        app_slug.to_string(),
                        table.id.clone(),
                        key.to_string(),
                        key_val.to_string(),
                    );
                    let exists: bool = repo.with_client(move |c| {
                        let row = c.query_opt(
                            "SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND data->>$3 = $4 LIMIT 1",
                            &[&q_app, &q_tbl, &q_key, &q_val],
                        )?;
                        Ok(row.is_some())
                    }).unwrap_or(false);

                    if exists {
                        update_count += 1;
                    } else {
                        create_count += 1;
                    }
                } else {
                    create_count += 1;
                }
            } else {
                create_count += 1;
            }
        }
    }

    let status = if invalid_count == 0 { "dry_run_passed" } else { "dry_run" };
    let result = json!({
        "valid_count": valid_count,
        "invalid_count": invalid_count,
        "create_count": create_count,
        "update_count": update_count,
        "errors": errors,
        "proposed_options": proposed_options,
    });

    repo.update_import_dry_run(import_id, status, &result)
        .map_err(|e| JobError::Retry(e.to_string()))?;

    ctx.result(result)
}

/// Job: import_run
pub fn import_run(ctx: &JobCtx, payload: Value) -> Result<Value, JobError> {
    let import_id = payload.get("import_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| JobError::Permanent("Missing or invalid import_id".to_string()))?;
    let app_slug = payload.get("app_slug")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let caller_eppn = payload.get("caller_eppn")
        .and_then(|v| v.as_str())
        .unwrap_or("user@university.edu");
    let accept_errors = payload.get("accept_errors").and_then(|v| v.as_bool()).unwrap_or(false);

    let repo = ctx.repository();
    let import = repo
        .get_import(app_slug, import_id)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("Import {} not found", import_id)))?;

    let mapping = payload.get("mapping")
        .or(import.mapping.as_ref())
        .ok_or_else(|| JobError::Permanent("Import mapping is required".to_string()))?;

    let target_table_id = mapping.get("target_table")
        .or_else(|| mapping.get("target_table_id"))
        .and_then(|v| v.as_str())
        .or(import.target_table_id.as_deref())
        .ok_or_else(|| JobError::Permanent("Mapping must specify target_table".to_string()))?;

    let (bytes, sha256) = read_file_raw(repo, import.file_ref_id)?;
    let delimiter = import.delimiter.chars().next().unwrap_or(',');
    let sheet = payload.get("sheet")
        .and_then(|v| v.as_str())
        .or_else(|| mapping.get("sheet").and_then(|v| v.as_str()));
    let (headers, rows, _) = parse_rows_from_bytes(&bytes, sheet, Some(delimiter), None)
        .map_err(JobError::Permanent)?;

    let total_rows = rows.len();
    let rules: Vec<RowRule> = payload.get("row_rules")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let person: Option<Person> = payload.get("caller_person")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let max_rows = payload.get("max_rows").and_then(|v| v.as_u64()).unwrap_or(DEFAULT_MAX_ROWS) as usize;
    if rows.len() > max_rows {
        return Err(JobError::Permanent(format!("File exceeds the maximum row limit of {} rows.", max_rows)));
    }
    let manifest = repo.get_app_manifest(app_slug)
        .map_err(|e| JobError::Retry(e.to_string()))?
        .ok_or_else(|| JobError::Permanent(format!("App {} not found", app_slug)))?;
    let table = manifest.tables.iter().find(|t| t.id == target_table_id || t.slug == target_table_id)
        .ok_or_else(|| JobError::Permanent(format!("Table {} not found in app", target_table_id)))?;

    let upsert_key = mapping.get("upsert_key").and_then(|v| v.as_str()).map(str::to_string);

    let mut field_map: HashMap<String, String> = HashMap::new();
    if let Some(col_map) = mapping.get("fields").or_else(|| mapping.get("column_mappings")).or_else(|| mapping.get("columns")).and_then(|v| v.as_object()) {
        for (k, v) in col_map {
            if let Some(target) = v.as_str() {
                if !target.is_empty() {
                    field_map.insert(k.clone(), target.to_string());
                }
            }
        }
    } else if let Some(fields_map) = mapping.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields_map {
            if let Some(target) = v.get("target_field").and_then(|t| t.as_str()) {
                if !target.is_empty() {
                    field_map.insert(k.clone(), target.to_string());
                }
            }
        }
    }

    let mut select_strategies: HashMap<String, String> = HashMap::new();
    if let Some(strat_map) = mapping.get("select_strategies").and_then(|v| v.as_object()) {
        for (k, v) in strat_map {
            if let Some(s) = v.as_str() {
                select_strategies.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(fields_map) = mapping.get("fields").and_then(|v| v.as_object()) {
        for (k, v) in fields_map {
            if let Some(s) = v.get("unknown_select").and_then(|t| t.as_str()) {
                select_strategies.insert(k.clone(), s.to_string());
            }
        }
    }

    let checkpoint = ctx.load_checkpoint();
    let mut processed_rows: usize = checkpoint.as_ref()
        .and_then(|cp| cp.get("processed_rows"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let mut created_count: usize = checkpoint.as_ref()
        .and_then(|cp| cp.get("created_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let mut updated_count: usize = checkpoint.as_ref()
        .and_then(|cp| cp.get("updated_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let mut rejected_rows: Vec<(Vec<String>, String)> = Vec::new();

    const BATCH_SIZE: usize = 1000;

    while processed_rows < total_rows {
        if ctx.cancelled()? {
            return Err(JobError::Cancelled);
        }

        let batch_end = (processed_rows + BATCH_SIZE).min(total_rows);
        let batch_slice = rows[processed_rows..batch_end].to_vec();
        let batch_id = Uuid::new_v4();

        let mut write_ctx = WriteCtx::new(caller_eppn, ActorKind::Import);
        write_ctx.batch_id = batch_id;

        let b_headers = headers.clone();
        let b_field_map = field_map.clone();
        let b_table = table.clone();
        let b_app_slug = app_slug.to_string();
        let b_caller_eppn = caller_eppn.to_string();
        let b_upsert_key = upsert_key.clone();
        let b_select_strategies = select_strategies.clone();
        let b_accept_errors = accept_errors;
        let b_processed_offset = processed_rows;
        let b_rules = rules.clone();
        let b_person = person.clone();

        let (batch_created, batch_updated, batch_rejected) = repo.with_client(move |c| {
            let mut tx = c.transaction()?;
            let mut created = 0;
            let mut updated = 0;
            let mut rejected = Vec::new();

            for (offset, row) in batch_slice.iter().enumerate() {
                let row_num = b_processed_offset + offset + 1;
                let mut row_has_error: Option<String> = None;
                let mut row_data = Map::new();

                for (col_idx, header) in b_headers.iter().enumerate() {
                    let Some(target_field_name) = b_field_map.get(header) else {
                        continue;
                    };
                    let cell_value = row.get(col_idx).map(|s| s.as_str()).unwrap_or("").trim();
                    let target_field = b_table.fields.iter().find(|f| &f.name == target_field_name);

                    if let Some(f) = target_field {
                        match &f.field_type {
                            FieldType::Number => {
                                if !cell_value.is_empty() {
                                    if let Ok(num) = cell_value.parse::<f64>() {
                                        row_data.insert(target_field_name.clone(), json!(num));
                                    } else {
                                        row_has_error = Some(format!("'{}' is not a valid number for '{}'", cell_value, target_field_name));
                                        break;
                                    }
                                }
                            }
                            FieldType::Select => {
                                if !cell_value.is_empty() {
                                    let allowed: Vec<String> = f.select_options.iter().map(|o| o.label.clone()).collect();

                                    if allowed.contains(&cell_value.to_string()) {
                                        row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                                    } else {
                                        let strategy = b_select_strategies.get(target_field_name).map(|s| s.as_str()).unwrap_or("reject");
                                        match strategy {
                                            "blank" => {
                                                row_data.insert(target_field_name.clone(), Value::Null);
                                            }
                                            "propose_option" => {
                                                row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                                            }
                                            _ => {
                                                row_has_error = Some(format!("Unknown option '{}' for select field '{}'", cell_value, target_field_name));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            FieldType::Link => {
                                if !cell_value.is_empty() {
                                    if let Some(ref rel_table_id) = f.target_table_id {
                                        let primary_field = f.target_display_field.as_deref().unwrap_or("name");
                                        let q = "SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND (data->>$3 = $4 OR data->>'name' = $4 OR data->>'title' = $4)";
                                        let matching_ids: Vec<String> = tx.query(q, &[&b_app_slug, &rel_table_id, &primary_field, &cell_value])?
                                            .iter()
                                            .map(|r| r.get(0))
                                            .collect();

                                        if matching_ids.is_empty() {
                                            row_has_error = Some(format!("Link target '{}' was not found in table '{}'", cell_value, rel_table_id));
                                            break;
                                        } else if matching_ids.len() > 1 {
                                            row_has_error = Some(format!("Link target '{}' matched multiple records in table '{}' and is ambiguous", cell_value, rel_table_id));
                                            break;
                                        } else {
                                            row_data.insert(target_field_name.clone(), Value::String(matching_ids[0].clone()));
                                        }
                                    }
                                }
                            }
                            _ => {
                                row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                            }
                        }
                    } else {
                        row_data.insert(target_field_name.clone(), Value::String(cell_value.to_string()));
                    }
                }

                if row_has_error.is_none() && !b_rules.is_empty() {
                    if let Some(ref p) = b_person {
                        let scope = row_scope(p, None, false, &b_rules, &b_table.id, RowOp::Create);
                        let allowed = match scope {
                            RowScope::AllowAll => true,
                            RowScope::DenyAll => false,
                            RowScope::Mixed(pred) => {
                                scaffoldry_engine::access::eval(&pred, &Value::Object(row_data.clone()), &b_table.fields)
                            }
                        };
                        if !allowed {
                            row_has_error = Some("Access denied by row rules: row is outside caller's create scope".to_string());
                        }
                    }
                }

                if let Some(err_msg) = row_has_error {
                    if !b_accept_errors {
                        return Err(crate::repository::RepositoryError::Conflict(format!("Row {}: {}", row_num, err_msg)));
                    }
                    rejected.push((row.clone(), err_msg));
                    continue;
                }

                row_data.insert("_table_id".to_string(), Value::String(b_table.id.clone()));
                row_data.insert("table_id".to_string(), Value::String(b_table.id.clone()));
                let final_data = Value::Object(row_data);

                let mut existing_rec_id: Option<String> = None;
                if let Some(ref key) = b_upsert_key {
                    if let Some(key_val) = final_data.get(key).and_then(|v| v.as_str()) {
                        let row_opt = tx.query_opt(
                            "SELECT id FROM dataset_records WHERE app_slug = $1 AND table_id = $2 AND deleted_at IS NULL AND data->>$3 = $4 LIMIT 1",
                            &[&b_app_slug, &b_table.id, &key, &key_val],
                        )?;
                        if let Some(r) = row_opt {
                            existing_rec_id = Some(r.get(0));
                        }
                    }
                }

                if let Some(rec_id) = existing_rec_id {
                    crate::repository::update_record_in_tx(&mut tx, &b_app_slug, &rec_id, &final_data, None, &write_ctx)?;
                    updated += 1;
                } else {
                    let rec = DatasetRecord {
                        id: format!("rec_{}", &Uuid::new_v4().to_string().replace('-', "")[..16]),
                        app_slug: b_app_slug.clone(),
                        data: final_data,
                        ceds_mapping: json!({}),
                        is_ferpa_sensitive: false,
                        created_by: Some(b_caller_eppn.clone()),
                        version: 1,
                        created_at: Utc::now().to_rfc3339(),
                    };
                    crate::repository::insert_record_in_tx(&mut tx, &rec, &write_ctx)?;
                    created += 1;
                }
            }

            tx.commit()?;
            Ok((created, updated, rejected))
        }).map_err(|e| JobError::Retry(e.to_string()))?;

        created_count += batch_created;
        updated_count += batch_updated;
        rejected_rows.extend(batch_rejected);
        processed_rows = batch_end;

        let cp = json!({
            "processed_rows": processed_rows,
            "created_count": created_count,
            "updated_count": updated_count,
            "rejected_count": rejected_rows.len(),
        });
        ctx.checkpoint(cp.clone())?;
        ctx.progress(processed_rows as i64, total_rows as i64)?;
        let _ = repo.update_import_checkpoint(import_id, &cp);
    }

    let mut rejected_file_ref_id: Option<Uuid> = None;
    if !rejected_rows.is_empty() {
        let mut csv_out = String::new();
        let mut rej_headers = headers.clone();
        rej_headers.push("_import_error".to_string());
        csv_out.push_str(&scaffoldry_engine::export_records_to_csv(
            &rej_headers.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &rejected_rows.iter().map(|(row, err)| {
                let mut m = Map::new();
                for (i, h) in headers.iter().enumerate() {
                    m.insert(h.clone(), Value::String(row.get(i).cloned().unwrap_or_default()));
                }
                m.insert("_import_error".to_string(), Value::String(err.clone()));
                Value::Object(m)
            }).collect::<Vec<_>>(),
        ));

        let dir = crate::files::dir().ok_or_else(|| JobError::Permanent("File dir not set".to_string()))?;
        let rej_bytes = csv_out.into_bytes();
        let rej_sha = hex_digest(&rej_bytes);
        let rej_id = Uuid::new_v4();
        let blob_dest = crate::files::blob_path(&dir, &rej_sha);
        if let Some(parent) = blob_dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&blob_dest, &rej_bytes);

        let new_ref = NewRef {
            id: rej_id,
            sha256: &rej_sha,
            size: rej_bytes.len() as i64,
            media_type: "text/csv",
            app_slug,
            name: &format!("rejected_rows_{}.csv", import_id),
            uploaded_by: caller_eppn,
            scan_state: "skipped",
        };
        if repo.insert_file_ref(new_ref).is_ok() {
            rejected_file_ref_id = Some(rej_id);
        }
    }

    let total_written = created_count + updated_count;
    if total_written > 1000 {
        let _ = repo.append_ledger_decision(
            AppendDecisionParams {
                principal: caller_eppn.to_string(),
                organization_code: "INST".to_string(),
                app_slug: Some(app_slug.to_string()),
                decision_type: DecisionType::DataImported,
                oscal_control_id: "SI-10".to_string(),
                rationale: format!("Imported {} records into {}", total_written, target_table_id),
                payload: json!({
                    "app_slug": app_slug,
                    "table_id": target_table_id,
                    "created_count": created_count,
                    "updated_count": updated_count,
                    "rejected_count": rejected_rows.len(),
                    "file_sha256": sha256,
                }),
            },
        );
    }

    let run_res = json!({
        "created_count": created_count,
        "updated_count": updated_count,
        "rejected_count": rejected_rows.len(),
        "rejected_file_ref_id": rejected_file_ref_id,
    });

    let _ = repo.update_import_completed(import_id, "completed", &run_res, rejected_file_ref_id);

    ctx.result(run_res)
}


