//! Admin console phase 4, first part: a record's sensitivity comes from the effective flag, so a
//! label raises it. The label table and screens are the later part. Here the label set is
//! filled by hand, which is all the server can do with it today.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_engine::{AppManifest, AppView, DataLabel, FieldSpec, FieldType, ViewType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn create(app: &axum::Router, token: &str, slug: &str, data: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/apps/{slug}/records"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "data": data }).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[tokio::test]
async fn a_label_raises_the_flag_of_a_record_that_holds_the_field() {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("labels-{}", &Uuid::new_v4().to_string()[..8]);
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "Labels".to_string(),
            description: String::new(),
            organization_code: "PHYS".to_string(),
            department: "physics".to_string(),
            workspace_id: Some("ws-physics-optics".to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table(
                "main",
                "Main",
                ViewType::Table,
                vec![
                    FieldSpec::simple("title", "Title", FieldType::Text, false, false),
                    FieldSpec::simple("advisor_notes", "Advisor notes", FieldType::Text, false, false),
                ],
            )],
            ceds_mappings: Default::default(),
            tables: vec![],
            relationships: vec![],
        })
        .unwrap();
    let app = build_app_with_state(state.clone()).expect("router");
    let token = issue_test_token_and_user("jordan.lee@state.edu");

    // No label: nothing is sensitive.
    let plain = create(&app, &token, &slug, json!({ "title": "A", "advisor_notes": "n" })).await;
    assert_eq!(plain["is_ferpa_sensitive"], false);

    // A label on one field raises the records that hold it, and only those.
    state.labels.write().unwrap().insert(
        (slug.clone(), String::new(), "advisor_notes".to_string()),
        DataLabel { ferpa_sensitive: true, note: "Advisor notes name students".into(), set_by: "compliance@state.edu".into() },
    );
    let raised = create(&app, &token, &slug, json!({ "title": "B", "advisor_notes": "n" })).await;
    assert_eq!(raised["is_ferpa_sensitive"], true);
    let unaffected = create(&app, &token, &slug, json!({ "title": "C" })).await;
    assert_eq!(unaffected["is_ferpa_sensitive"], false, "a record without the field is not raised");
}

/// Sensitive content phase 1: a field carries categories, and the organization's settings say
/// which of them are protected. A record is flagged by a protected category and by nothing else.
#[tokio::test]
async fn a_protected_category_flags_a_record_and_an_unprotected_one_does_not() {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let slug = format!("cats-{}", &Uuid::new_v4().to_string()[..8]);
    let mut card = FieldSpec::simple("card", "Card", FieldType::Text, false, false);
    card.categories = vec!["pci".to_string()];
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(AppManifest {
            slug: slug.clone(),
            title: "Categories".to_string(),
            description: String::new(),
            organization_code: "PHYS".to_string(),
            department: "physics".to_string(),
            workspace_id: Some("ws-physics-optics".to_string()),
            herm_capability_id: None,
            custom_domain: None,
            custom_domain_verified: false,
            views: vec![AppView::table(
                "main",
                "Main",
                ViewType::Table,
                vec![FieldSpec::simple("title", "Title", FieldType::Text, false, false), card],
            )],
            ceds_mappings: Default::default(),
            tables: vec![],
            relationships: vec![],
        })
        .unwrap();
    let app = build_app_with_state(state.clone()).expect("router");
    let token = issue_test_token_and_user("jordan.lee@state.edu");
    let set = |protected: bool| {
        state.settings.write().unwrap().insert(
            "sensitivity.categories".to_string(),
            json!([{ "id": "pci", "name": "Payment card data", "protected": protected, "detectors": [] }]),
        );
    };

    set(true);
    let held = create(&app, &token, &slug, json!({ "title": "A", "card": "x" })).await;
    assert_eq!(held["is_ferpa_sensitive"], true, "pci is protected and the record holds the field");
    let absent = create(&app, &token, &slug, json!({ "title": "B" })).await;
    assert_eq!(absent["is_ferpa_sensitive"], false, "a record without the field is not flagged");

    set(false);
    let open = create(&app, &token, &slug, json!({ "title": "C", "card": "x" })).await;
    assert_eq!(open["is_ferpa_sensitive"], false, "the organization marked pci unprotected");
}
