//! Authentication enforcement tests.
//! Every API route except health and login must reject callers that lack a valid session.
//! Identity must come from the session only, never from headers or request bodies.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn login(app: &Router, eppn: &str) -> String {
    let (status, body) = send(app, "POST", "/api/v1/auth/token", None, &[], Some(json!({ "eppn": eppn }))).await;
    assert_eq!(status, StatusCode::OK, "token issue must succeed for {eppn}");
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn protected_routes_reject_requests_without_a_session() {
    let app = build_app().expect("router");
    let routes = [
        ("GET", "/api/v1/workspaces"),
        ("POST", "/api/v1/workspaces"),
        ("GET", "/api/v1/workspaces/ws-bio-lab"),
        ("GET", "/api/v1/workspaces/ws-bio-lab/collaborators"),
        ("GET", "/api/v1/datasets"),
        ("GET", "/api/v1/policies"),
        ("GET", "/api/v1/framework/spec"),
        ("GET", "/api/v1/governance/ledger"),
        ("GET", "/api/v1/apps/physics-grants/records"),
        ("POST", "/api/v1/apps/physics-grants/records"),
        ("GET", "/api/mcp"),
        ("POST", "/api/mcp"),
    ];
    for (method, uri) in routes {
        let body = if method == "POST" { Some(json!({})) } else { None };
        let (status, _) = send(&app, method, uri, None, &[], body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri} must require a session");
    }
}

#[tokio::test]
async fn unknown_bearer_token_is_rejected() {
    let app = build_app().expect("router");
    let (status, _) = send(&app, "GET", "/api/v1/workspaces", Some("sct_not_a_real_token"), &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_and_login_stay_public() {
    let app = build_app().expect("router");
    let (status, _) = send(&app, "GET", "/healthz", None, &[], None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&app, "POST", "/api/v1/auth/token", None, &[], Some(json!({ "eppn": "jordan.lee@state.edu" }))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn caller_header_cannot_forge_an_identity() {
    let app = build_app().expect("router");

    // Without a session the header alone must not authenticate anyone.
    let (status, _) = send(&app, "GET", "/api/v1/workspaces", None, &[("x-caller-eppn", "jordan.lee@state.edu")], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // An invented admin-looking name must not become central_admin.
    let (status, _) = send(&app, "GET", "/api/v1/workspaces", None, &[("x-caller-eppn", "root.admin@evil.example")], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With a real low-privilege session, the header must not escalate it.
    let token = login(&app, "einstein@physics.state.edu").await;
    let (status, body) = send(
        &app,
        "GET",
        "/api/v1/workspaces",
        Some(&token),
        &[("x-caller-eppn", "jordan.lee@state.edu")],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body.as_array().unwrap().iter().map(|w| w["id"].as_str().unwrap()).collect();
    assert!(!ids.contains(&"ws-bio-lab"), "forged admin header must not reveal restricted workspaces: {ids:?}");
}

#[tokio::test]
async fn record_identity_comes_from_the_session_not_the_body() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let (status, ws) = send(&app, "POST", "/api/v1/workspaces", Some(&admin), &[], Some(json!({ "name": "Physics", "code": "PHYS", "organization": "Sciences" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    let ws_id = ws["id"].as_str().unwrap();

    let app_payload = json!({
        "slug": "physics-grants",
        "title": "Physics Research Grants",
        "description": "Grant tracking",
        "organization_code": "PHYS",
        "department": "physics",
        "herm_capability_id": "RES-01-GRANTS",
        "views": [{
            "id": "main", "title": "All", "view_type": "Table",
            "fields": [{ "name": "title", "label": "Title", "field_type": "Text", "required": true, "ferpa_sensitive": false }]
        }],
        "ceds_mappings": {}
    });
    let (status, _) = send(&app, "POST", &format!("/api/v1/workspaces/{ws_id}/apps"), Some(&admin), &[], Some(app_payload)).await;
    assert_eq!(status, StatusCode::CREATED);

    // A biology student claims in the body to be a physics faculty member.
    let student = login(&app, "student.smith@science.state.edu").await;
    let forged = json!({
        "caller_eppn": "dr.smith@physics.university.edu",
        "caller_affiliation": "faculty",
        "data": { "title": "Forged submission" }
    });
    let (status, _) = send(&app, "POST", "/api/v1/apps/physics-grants/records", Some(&student), &[], Some(forged)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body claims must not grant write access");
}

#[tokio::test]
async fn scim_requires_the_provisioning_credential_and_fails_closed() {
    use scaffoldry_server::build_app_with_state;
    use scaffoldry_server::state::ServerState;
    use std::sync::Arc;

    let uri = "/scim/v2/ServiceProviderConfig";

    // Configured credential: missing and wrong tokens are rejected, the right one is accepted.
    let mut state = ServerState::new().expect("state");
    state.scim_token = Some("scim-provisioning-secret".to_string());
    let app = build_app_with_state(Arc::new(state)).expect("router");
    let (status, _) = send(&app, "GET", uri, None, &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&app, "GET", uri, Some("wrong"), &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&app, "GET", uri, Some("scim-provisioning-secret"), &[], None).await;
    assert_eq!(status, StatusCode::OK);

    // No credential configured: SCIM is closed, not open.
    let mut state = ServerState::new().expect("state");
    state.scim_token = None;
    let app = build_app_with_state(Arc::new(state)).expect("router");
    let (status, _) = send(&app, "GET", uri, Some("anything"), &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_issuance_cannot_forge_central_admin_or_arbitrary_privileges() {
    let app = build_app().expect("router");

    // 1. Mallory with "admin" in eppn must NOT be given central_admin affiliation
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/auth/token",
        None,
        &[],
        Some(json!({ "eppn": "mallory@evil.example.admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        body["user"]["affiliation"].as_str(),
        Some("central_admin"),
        "Arbitrary eppn containing 'admin' must not become central_admin"
    );

    // 2. Caller attempting to pass affiliation: central_admin explicitly must be refused or downgraded
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/auth/token",
        None,
        &[],
        Some(json!({
            "eppn": "mallory@evil.example",
            "affiliation": "central_admin"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        body["user"]["affiliation"].as_str(),
        Some("central_admin"),
        "Unauthenticated caller cannot escalate affiliation to central_admin"
    );

    // 3. Known central admin jordan.lee@state.edu is valid
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/auth/token",
        None,
        &[],
        Some(json!({ "eppn": "jordan.lee@state.edu" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["affiliation"].as_str(), Some("central_admin"));
}

#[tokio::test]
async fn app_and_dataset_routes_enforce_cedar_policy_authorization() {
    let app = build_app().expect("router");
    let student = login(&app, "student.smith@science.state.edu").await;
    let faculty = login(&app, "prof.curie@science.state.edu").await;

    // A student in biology attempting to update or publish an app must be forbidden
    let app_manifest = json!({
        "slug": "bio-lab-inventory",
        "title": "Bio Lab Equipment",
        "description": "Lab inventory",
        "organization_code": "DEPT-BIO",
        "department": "biology",
        "herm_capability_id": "RES-01",
        "views": [],
        "ceds_mappings": {}
    });

    // 1. Faculty member in biology registers bio-lab-inventory
    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/apps/bio-lab-inventory",
        Some(&faculty),
        &[],
        Some(app_manifest.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Department faculty can create app");

    // 2. A student in biology attempting to update or publish the app must be forbidden
    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/apps/bio-lab-inventory",
        Some(&student),
        &[],
        Some(app_manifest.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Student must not update app manifest");

    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/apps/bio-lab-inventory/publish",
        Some(&student),
        &[],
        Some(json!({ "custom_domain": "bio.science.state.edu" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Student must not publish app");

    // 3. Student publishing a dataset must be forbidden
    let dataset_payload = json!({
        "name": "Unauthorized Dataset",
        "department": "biology",
        "organization": "science.state.edu",
        "fields": []
    });
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/datasets",
        Some(&student),
        &[],
        Some(dataset_payload),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Student must not publish dataset");

    // 4. Faculty member in biology can publish app
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/apps/bio-lab-inventory/publish",
        Some(&faculty),
        &[],
        Some(json!({ "custom_domain": "bio.science.state.edu" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Department faculty can publish app");
}

#[tokio::test]
async fn governance_ledger_principal_comes_strictly_from_session_never_request_body() {
    let app = build_app().expect("router");
    let student = login(&app, "student.smith@science.state.edu").await;
    let admin = login(&app, "jordan.lee@state.edu").await;

    let forged_payload = json!({
        "principal": "provost@state.edu",
        "organization_code": "UNIV",
        "decision_type": "PolicyRevision",
        "oscal_control_id": "CM-03",
        "rationale": "Forged by student"
    });

    // 1. Non-admin student cannot append to governance ledger
    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/governance/ledger/append",
        Some(&student),
        &[],
        Some(forged_payload.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Student must not append to governance ledger");

    // 2. Admin appends, but server MUST override payload.principal with admin session identity
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/governance/ledger/append",
        Some(&admin),
        &[],
        Some(forged_payload),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "Admin can append to governance ledger");
    assert_eq!(
        body["entry"]["principal"].as_str(),
        Some("jordan.lee@state.edu"),
        "Ledger entry principal must come from session identity, not request body"
    );
}

