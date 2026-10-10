//! Sensitive content phase 1: categories, presets and detectors are settings.
//!
//! The settings are stored in the shared database, so the tests take turns and put the two
//! sensitivity settings back before they start.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use tower::ServiceExt;

static LOCK: Mutex<()> = Mutex::const_new(());

struct Ctx {
    app: Router,
    admin: String,
    _guard: MutexGuard<'static, ()>,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn db(f: impl FnOnce(&mut postgres::Client) + Send + 'static) {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string());
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c);
    })
    .join()
    .unwrap();
}

/// A server with both sensitivity settings empty, so each test starts from nothing.
async fn ctx() -> Ctx {
    let guard = LOCK.lock().await;
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let state = Arc::new(ServerState::new().expect("state"));
    let app = build_app_with_state(state).expect("router");
    let c = Ctx { app, admin, _guard: guard };
    let (s, b) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.categories", &c.admin, Some(json!([]))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.detectors", &c.admin, Some(json!([]))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    c
}

async fn setting(c: &Ctx, key: &str) -> Value {
    let (s, all) = send(&c.app, "GET", "/api/v1/settings", &c.admin, None).await;
    assert_eq!(s, StatusCode::OK);
    all[key].clone()
}

async fn ledger_len(c: &Ctx) -> u64 {
    let (_, body) = send(&c.app, "GET", "/api/v1/governance/ledger", &c.admin, None).await;
    body["total_entries"].as_u64().unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect()
}

fn preset_path(id: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../governance/sensitivity-presets").join(format!("{id}.json"))
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn a_new_installation_has_pii_and_pci_on_with_flag_detectors() {
    let _turn = LOCK.lock().await;
    db(|c| {
        c.execute("DELETE FROM platform_settings WHERE key LIKE 'sensitivity.%'", &[]).unwrap();
    });
    let state = Arc::new(ServerState::new().expect("state"));
    let categories = state.settings.read().unwrap()["sensitivity.categories"].clone();
    let detectors = state.settings.read().unwrap()["sensitivity.detectors"].clone();
    assert_eq!(ids(&categories), ["pii", "pci"]);
    for c in categories.as_array().unwrap() {
        assert_eq!(c["protected"], true);
    }
    let by_id = |id: &str| detectors.as_array().unwrap().iter().find(|d| d["id"] == id).cloned().unwrap_or_else(|| panic!("detector {id}"));
    assert_eq!(by_id("us_ssn")["action"], "flag");
    assert_eq!(by_id("us_ssn")["enabled"], true);
    assert_eq!(by_id("payment_card")["action"], "flag");
    assert_eq!(by_id("payment_card")["enabled"], true);
    assert_eq!(by_id("email")["enabled"], false, "everything else is off");
}

// 6. Switching a preset on.
#[tokio::test]
async fn switching_on_a_preset_copies_it_into_settings_with_one_ledger_entry() {
    let c = ctx().await;
    let before = ledger_len(&c).await;
    let (s, body) = send(&c.app, "POST", "/api/v1/settings/sensitivity/presets/pci", &c.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(ledger_len(&c).await, before + 1, "one ledger entry");

    assert_eq!(ids(&setting(&c, "sensitivity.categories").await), ["pci"]);
    assert_eq!(ids(&setting(&c, "sensitivity.detectors").await), ["payment_card"]);

    // Switching it on twice is 409 and changes nothing.
    let (s, _) = send(&c.app, "POST", "/api/v1/settings/sensitivity/presets/pci", &c.admin, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(ledger_len(&c).await, before + 1);
    assert_eq!(ids(&setting(&c, "sensitivity.categories").await).len(), 1);

    let (s, _) = send(&c.app, "POST", "/api/v1/settings/sensitivity/presets/no-such-preset", &c.admin, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn every_shipped_preset_can_be_switched_on() {
    let c = ctx().await;
    for id in ["pii", "pci", "phi", "ferpa"] {
        let (s, body) = send(&c.app, "POST", &format!("/api/v1/settings/sensitivity/presets/{id}"), &c.admin, None).await;
        assert_eq!(s, StatusCode::OK, "{id}: {body}");
    }
    let categories = setting(&c, "sensitivity.categories").await;
    assert_eq!(ids(&categories), ["pii", "pci", "phi", "ferpa"]);
    let detectors = setting(&c, "sensitivity.detectors").await;
    // phi and ferpa carry a disabled example, because the shape is the institution's own.
    let phi_example = detectors.as_array().unwrap().iter().find(|d| d["kind"] == "shape" && d["id"].as_str().unwrap().starts_with("phi")).unwrap();
    assert_eq!(phi_example["enabled"], false);
}

#[tokio::test]
async fn editing_the_copied_category_does_not_touch_the_preset_file() {
    let c = ctx().await;
    let file = std::fs::read(preset_path("pci")).expect("the preset file exists");
    let before = sha(&file);

    send(&c.app, "POST", "/api/v1/settings/sensitivity/presets/pci", &c.admin, None).await;
    let mut cats = setting(&c, "sensitivity.categories").await;
    cats[0]["name"] = json!("Cardholder data, our wording");
    cats[0]["protected"] = json!(false);
    let (s, body) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.categories", &c.admin, Some(cats)).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    assert_eq!(sha(&std::fs::read(preset_path("pci")).unwrap()), before, "the preset file is unchanged");
    assert_eq!(setting(&c, "sensitivity.categories").await[0]["name"], "Cardholder data, our wording");
}

#[tokio::test]
async fn only_a_platform_admin_switches_a_preset_on() {
    let c = ctx().await;
    let faculty = issue_test_token_and_user("faculty.curie@state.edu");
    let (s, _) = send(&c.app, "POST", "/api/v1/settings/sensitivity/presets/pci", &faculty, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.detectors", &faculty, Some(json!([]))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(setting(&c, "sensitivity.categories").await, json!([]));
}

// 4 and 8. A bad save is 400 with the path of the error, and nothing is saved.
#[tokio::test]
async fn a_bad_detector_or_category_is_400_with_its_path_and_saves_nothing() {
    let c = ctx().await;
    let put = |key: &'static str, v: Value| {
        let (app, admin) = (c.app.clone(), c.admin.clone());
        async move { send(&app, "PUT", &format!("/api/v1/settings/{key}"), &admin, Some(v)).await }
    };

    // A shape over 64 characters.
    let (s, body) = put("sensitivity.detectors", json!([{ "id": "long", "kind": "shape", "shape": "#".repeat(65) }])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["fields"].get("sensitivity.detectors[0].shape").is_some(), "the path is named: {body}");

    // A duplicate detector id.
    let (s, body) = put("sensitivity.detectors", json!([{ "id": "a", "kind": "email" }, { "id": "a", "kind": "us_ssn" }])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body["fields"].get("sensitivity.detectors[1].id").is_some(), "{body}");

    // A good detector, then a category that names one that does not exist.
    let (s, _) = put("sensitivity.detectors", json!([{ "id": "ssn", "kind": "us_ssn", "action": "flag", "enabled": true }])).await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = put("sensitivity.categories", json!([{ "id": "pii", "name": "PII", "protected": true, "detectors": ["ssn", "ghost"] }])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["fields"].get("sensitivity.categories[0].detectors[1]").is_some(), "{body}");
    assert_eq!(setting(&c, "sensitivity.categories").await, json!([]), "nothing was saved");

    // A category that is fine, then removing the detector it names.
    let (s, _) = put("sensitivity.categories", json!([{ "id": "pii", "name": "PII", "protected": true, "detectors": ["ssn"] }])).await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = put("sensitivity.detectors", json!([])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["fields"].get("sensitivity.categories[0].detectors[0]").is_some(), "{body}");
    assert_eq!(ids(&setting(&c, "sensitivity.detectors").await), ["ssn"], "the detector is still there");

    // The old keys still work and the closed list still holds.
    let (s, _) = put("sensitivity.nonsense", json!([])).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_detector_action_is_flag_warn_or_block_and_all_three_save() {
    let c = ctx().await;
    for action in ["flag", "warn", "block"] {
        let (s, body) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.detectors", &c.admin,
            Some(json!([{ "id": "ssn", "kind": "us_ssn", "action": action, "enabled": true }]))).await;
        assert_eq!(s, StatusCode::OK, "{action}: {body}");
    }
    let (s, _) = send(&c.app, "PUT", "/api/v1/settings/sensitivity.detectors", &c.admin,
        Some(json!([{ "id": "ssn", "kind": "us_ssn", "action": "delete", "enabled": true }]))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_published_schema_and_every_preset_file_exist_and_agree() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../governance");
    let schema: Value = serde_json::from_slice(&std::fs::read(root.join("schema/sensitivity-settings.schema.json")).expect("the schema is published")).unwrap();
    assert!(schema["$id"].as_str().unwrap().ends_with("sensitivity-settings.schema.json"));
    for id in ["ferpa", "phi", "pci", "pii"] {
        let preset: Value = serde_json::from_slice(&std::fs::read(preset_path(id)).unwrap_or_else(|_| panic!("{id}.json"))).unwrap();
        assert_eq!(preset["category"]["id"], id);
        let parsed = scaffoldry_engine::sensitivity::validate_preset(&preset);
        assert!(parsed.is_ok(), "{id}: {:?}", parsed.err());
    }
}
