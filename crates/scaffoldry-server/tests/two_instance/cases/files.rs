//! Uploads, imports and exports. They store files under `SCAFFOLDRY_FILES_DIR`, so they run in
//! `two_instance_files_test.rs`.

use super::apps::job_state;
use super::*;

pub fn add(m: &mut Entries) {
    m.insert("POST /api/v1/apps/{slug}/files".into(), files_case!(file_upload));
    m.insert("POST /api/v1/apps/{slug}/exports".into(), files_case!(export_start));
    m.insert("POST /api/v1/apps/{slug}/imports".into(), files_case!(import_upload));
    m.insert("POST /api/v1/apps/{slug}/imports/{id}/mapping".into(), files_case!(import_mapping));
    m.insert("POST /api/v1/apps/{slug}/imports/{id}/analyze".into(), files_case!(import_analyze));
    m.insert("POST /api/v1/apps/{slug}/imports/{id}/dry-run".into(), files_case!(import_dry_run));
    m.insert("POST /api/v1/apps/{slug}/imports/{id}/run".into(), files_case!(import_run));
}

/// A raw upload through A, as the clients send it: the bytes as the body and the name in a header.
async fn send_bytes(w: &World, uri: &str, who: &str, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {}", scaffoldry_server::service::identity::issue_test_token_and_user(who)))
        .header("content-length", bytes.len().to_string())
        .header("x-file-name", name)
        .body(Body::from(bytes))
        .unwrap();
    let resp = w.a.clone().oneshot(req).await.unwrap();
    let code = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (code, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn png(tag: u8) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend(std::iter::repeat_n(tag, 200));
    b
}

async fn file_upload(w: World) -> Outcome {
    // An app with an attachment field, so the upload can be attached to a record and read back.
    let slug = unique("p6files");
    let mut m = manifest(&slug);
    m["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "doc", "label": "Documents", "field_type": { "Attachment": { "allow_multiple": true } }, "required": false, "ferpa_sensitive": false }));
    status(w.wa("POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(m)).await, StatusCode::CREATED, "make the app")?;
    let up = send_bytes(&w, &format!("/api/v1/apps/{slug}/files"), OWNER, "p.png", png(7)).await;
    let up = status(up, StatusCode::CREATED, "upload")?;
    let ref_id = up["ref_id"].as_str().ok_or("a ref id")?.to_string();
    let made = status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "apps", "doc": [{ "ref_id": ref_id, "name": up["name"] }] } }))).await,
        StatusCode::CREATED,
        "attach it to a record",
    )?;
    let record = made["id"].as_str().ok_or("a record id")?.to_string();
    let listed = status(w.rb("GET", &format!("/api/v1/apps/{slug}/records/{record}/files"), OWNER, None).await, StatusCode::OK, "B lists the files")?;
    ensure(has(&listed, &ref_id), format!("B lists the file A stored: {listed}"))
}

async fn export_start(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    make_record(&w, &slug, "Exported").await?;
    let (code, started) = w.wa("POST", &format!("/api/v1/apps/{slug}/exports"), OWNER, Some(json!({ "table_id": "apps", "format": "csv" }))).await;
    ensure(code.is_success(), format!("start the export: {code} {started}"))?;
    let job = started["job_id"].as_str().ok_or_else(|| format!("a job id: {started}"))?.to_string();
    ensure(!job_state(&w, &job).await?.is_empty(), format!("B sees the export job: {started}"))
}

async fn uploaded_import(w: &World, slug: &str) -> Result<String, String> {
    let csv = "name,gpa\nAda,3.9\nBo,3.1\n".as_bytes().to_vec();
    let up = status(send_bytes(w, &format!("/api/v1/apps/{slug}/imports"), OWNER, "people.csv", csv).await, StatusCode::CREATED, "upload the file")?;
    Ok(up["import_id"].as_str().ok_or("an import id")?.to_string())
}

async fn import_upload(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = uploaded_import(&w, &slug).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/imports/{id}"), OWNER, None).await, StatusCode::OK, "B reads the import")?;
    ensure(has(&seen, &id), format!("B shows the import A started: {seen}"))
}

async fn import_mapping(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = uploaded_import(&w, &slug).await?;
    let before = status(w.rb("GET", &format!("/api/v1/apps/{slug}/imports/{id}"), OWNER, None).await, StatusCode::OK, "B reads the import")?;
    status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/imports/{id}/mapping"), OWNER, Some(json!({ "target_table": "apps", "column_mappings": { "name": "name", "gpa": "gpa" } }))).await,
        StatusCode::OK,
        "set the mapping",
    )?;
    let after = status(w.rb("GET", &format!("/api/v1/apps/{slug}/imports/{id}"), OWNER, None).await, StatusCode::OK, "B reads the import again")?;
    ensure(before != after, format!("B shows the mapping A set: {after}"))
}

async fn import_job(w: &World, step: &str) -> Outcome {
    let slug = make_app(w).await?;
    let id = uploaded_import(w, &slug).await?;
    let started = status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/imports/{id}/{step}"), OWNER, Some(json!({ "target_table": "apps", "column_mappings": { "name": "name", "gpa": "gpa" } }))).await,
        StatusCode::OK,
        step,
    )?;
    let job = started["job_id"].as_str().ok_or_else(|| format!("a job id: {started}"))?.to_string();
    ensure(!job_state(w, &job).await?.is_empty(), format!("B sees the {step} job: {started}"))
}

async fn import_analyze(w: World) -> Outcome {
    import_job(&w, "analyze").await
}

async fn import_dry_run(w: World) -> Outcome {
    import_job(&w, "dry-run").await
}

async fn import_run(w: World) -> Outcome {
    import_job(&w, "run").await
}
