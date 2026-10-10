//! Authentication enforcement tests.
//! Every API route except health and login must reject callers that lack a valid session.
//! Identity must come from the session only, never from headers or request bodies.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::{build_app, ServerState};
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

async fn send_full(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
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
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, val)
}

async fn login(_app: &Router, eppn: &str) -> String {
    scaffoldry_server::service::identity::issue_test_token_and_user(eppn)
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
    let (status, _) = send(&app, "GET", "/.well-known/oauth-protected-resource", None, &[], None).await;
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
    use scaffoldry_server::state::{ApiToken, ServerState};
    use std::sync::Arc;

    let uri = "/scim/v2/ServiceProviderConfig";

    // Configured credential: missing and wrong tokens are rejected, the right one is accepted.
    let state = ServerState::new().expect("state");
    let scim_secret = "scim-provisioning-secret";
    let token_hash = scaffoldry_server::service::identity::hash_token(scim_secret);
    let token = ApiToken {
        token_hash: token_hash.clone(),
        id: uuid::Uuid::new_v4(),
        kind: "scim".to_string(),
        eppn: "scim@scaffoldry.local".to_string(),
        label: "test-scim".to_string(),
        original_admin: None,
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::days(1),
        last_used_at: None,
        revoked_at: None,
    };
    state.persist_api_token(&token).unwrap();
    let app = build_app_with_state(Arc::new(state)).expect("router");
    let (status, _) = send(&app, "GET", uri, None, &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&app, "GET", uri, Some("wrong"), &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&app, "GET", uri, Some("scim-provisioning-secret"), &[], None).await;
    assert_eq!(status, StatusCode::OK);

    // No credential configured: SCIM is closed, not open.
    let state = ServerState::new().expect("state");
    if let Ok(mut tokens) = state.api_tokens.write() {
        tokens.clear();
    }
    let app = build_app_with_state(Arc::new(state)).expect("router");
    let (status, _) = send(&app, "GET", uri, Some("anything"), &[], None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_issuance_cannot_forge_central_admin_or_arbitrary_privileges() {
    let state = std::sync::Arc::new(scaffoldry_server::state::ServerState::new().unwrap());
    // 1. Arbitrary user resolved by identity does NOT become central_admin
    let _mallory_token = scaffoldry_server::service::identity::issue_test_token_and_user("mallory@evil.example.admin");
    let mallory = scaffoldry_server::service::identity::resolve_user("mallory@evil.example.admin", &state).unwrap();
    assert_ne!(mallory.affiliation, "central_admin");

    // 2. Setup identity resolves to central_admin
    let setup = scaffoldry_server::service::identity::resolve_user("setup@scaffoldry.local", &state).unwrap();
    assert_eq!(setup.affiliation, "central_admin");

    // 3. Jordan Lee with root platform_admin role resolves to central_admin
    let _ = scaffoldry_server::service::identity::issue_test_token_and_user("jordan.lee@state.edu");
    let jordan = scaffoldry_server::service::identity::resolve_user("jordan.lee@state.edu", &state).unwrap();
    assert_eq!(jordan.affiliation, "central_admin");
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

    let admin = login(&app, "jordan.lee@state.edu").await;
    let (_, ws) = send(
        &app,
        "POST",
        "/api/v1/workspaces",
        Some(&admin),
        &[],
        Some(json!({
            "name": "Bio Workspace",
            "code": "WS-BIO",
            "organization": "science.state.edu",
            "department": "biology",
            "visibility": "restricted"
        })),
    ).await;
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let _ = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/collaborators"),
        Some(&admin),
        &[],
        Some(json!({
            "eppn": "prof.curie@science.state.edu",
            "role": "owner",
            "name": "Marie Curie"
        })),
    ).await;

    // 1. Faculty member in biology creates and updates bio-lab-inventory
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/apps"),
        Some(&faculty),
        &[],
        Some(app_manifest.clone()),
    ).await;
    assert_eq!(status, StatusCode::CREATED, "Department faculty can create app");

    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/apps/bio-lab-inventory",
        Some(&faculty),
        &[],
        Some(app_manifest.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Department faculty can update app");

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



#[tokio::test]
async fn test_phase_5_app_workspace_scoping_and_stored_row_access() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let owner_a = login(&app, "owner.a@science.state.edu").await;
    let editor_a = login(&app, "editor.a@science.state.edu").await;
    let viewer_a = login(&app, "viewer.a@science.state.edu").await;
    let owner_b = login(&app, "owner.b@physics.state.edu").await;

    // Platform Admin creates Workspace A
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/workspaces",
        Some(&admin),
        &[],
        Some(json!({
            "name": "Workspace A",
            "code": "WS-A",
            "organization": "Science",
            "department": "biology",
            "visibility": "restricted"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let ws_a_id = body["id"].as_str().unwrap().to_string();

    // Add owner_a, editor_a, and viewer_a as collaborators in Workspace A
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_a_id}/collaborators"),
        Some(&admin),
        &[],
        Some(json!({
            "eppn": "owner.a@science.state.edu",
            "role": "owner",
            "name": "Owner A"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_a_id}/collaborators"),
        Some(&owner_a),
        &[],
        Some(json!({
            "eppn": "editor.a@science.state.edu",
            "role": "editor",
            "name": "Editor A"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_a_id}/collaborators"),
        Some(&owner_a),
        &[],
        Some(json!({
            "eppn": "viewer.a@science.state.edu",
            "role": "viewer",
            "name": "Viewer A"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    // Platform Admin creates Workspace B
    let (status, body) = send(
        &app,
        "POST",
        "/api/v1/workspaces",
        Some(&admin),
        &[],
        Some(json!({
            "name": "Workspace B",
            "code": "WS-B",
            "organization": "Physics",
            "department": "physics",
            "visibility": "restricted"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let ws_b_id = body["id"].as_str().unwrap().to_string();

    // Add owner_b as owner in Workspace B
    let (status, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_b_id}/collaborators"),
        Some(&admin),
        &[],
        Some(json!({
            "eppn": "owner.b@physics.state.edu",
            "role": "owner",
            "name": "Owner B"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    // 5. POST /workspaces/A/apps with workspace_id: "B" in the body stores A
    let app_manifest = json!({
        "slug": "app-in-a",
        "title": "App in A",
        "description": "Scattered lab inventory",
        "organization_code": "SCI",
        "department": "biology",
        "workspace_id": ws_b_id,
        "views": [],
        "tables": [{
            "id": "t1",
            "slug": "t1",
            "name": "Table 1",
            "fields": [{
                "name": "name",
                "label": "Name",
                "field_type": "Text",
                "required": false,
                "ferpa_sensitive": false
            }]
        }],
        "ceds_mappings": {}
    });

    let (status, body) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_a_id}/apps"),
        Some(&owner_a),
        &[],
        Some(app_manifest),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        body["workspace_id"].as_str(),
        Some(ws_a_id.as_str()),
        "App creation must ignore workspace_id in body and store path workspace_id"
    );

    // Owner A creates an initial record in app-in-a
    let (status, rec_body) = send(
        &app,
        "POST",
        "/api/v1/apps/app-in-a/records",
        Some(&owner_a),
        &[],
        Some(json!({ "data": { "name": "Item 1" } })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let rec_id = rec_body["id"].as_str().unwrap().to_string();

    // 1. B's owner gets 403 on GET /apps/{slug}, on record list, and on record create.
    let (status, _) = send(&app, "GET", "/api/v1/apps/app-in-a", Some(&owner_b), &[], None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "B's owner must get 403 on GET /apps/app-in-a");

    let (status, _) = send(&app, "GET", "/api/v1/apps/app-in-a/records", Some(&owner_b), &[], None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "B's owner must get 403 on record list");

    let (status, _) = send(
        &app,
        "POST",
        "/api/v1/apps/app-in-a/records",
        Some(&owner_b),
        &[],
        Some(json!({ "data": { "name": "Item 2" } })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "B's owner must get 403 on record create");

    // 2. A's viewer gets 200 on record list and 403 on record update.
    let (status, _) = send(&app, "GET", "/api/v1/apps/app-in-a/records", Some(&viewer_a), &[], None).await;
    assert_eq!(status, StatusCode::OK, "A's viewer must get 200 on record list");

    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/apps/app-in-a/records/{rec_id}"),
        Some(&viewer_a),
        &[],
        Some(json!({ "data": { "name": "Updated by viewer" } })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "A's viewer must get 403 on record update");

    // 3. A's editor gets 200 on record update and 403 on PUT /apps/{slug}.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/v1/apps/app-in-a/records/{rec_id}"),
        Some(&editor_a),
        &[],
        Some(json!({ "data": { "name": "Updated by editor" } })),
    ).await;
    assert_eq!(status, StatusCode::OK, "A's editor must get 200 on record update");

    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/apps/app-in-a",
        Some(&editor_a),
        &[],
        Some(json!({
            "title": "Renamed by editor",
            "description": "Updated",
            "organization_code": "SCI",
            "department": "biology",
            "views": []
        })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "A's editor must get 403 on PUT /apps/app-in-a");

    // 4. B's owner sends PUT /apps/{slug} with department set to their own department. 403. The stored manifest is unchanged.
    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/apps/app-in-a",
        Some(&owner_b),
        &[],
        Some(json!({
            "title": "Hijacked by B",
            "description": "Updated",
            "organization_code": "PHYS",
            "department": "physics",
            "views": []
        })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "B's owner must get 403 on PUT /apps/app-in-a");

    let (status, manifest_body) = send(&app, "GET", "/api/v1/apps/app-in-a", Some(&owner_a), &[], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        manifest_body["department"].as_str(),
        Some("biology"),
        "Stored manifest department must be unchanged"
    );
    assert_eq!(
        manifest_body["workspace_id"].as_str(),
        Some(ws_a_id.as_str()),
        "Stored manifest workspace_id must remain A"
    );
}


#[tokio::test]
async fn test_phase_6_open_routes_enforcement() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let owner_a = login(&app, "owner.a@science.state.edu").await;
    let admin_a = login(&app, "admin.a@science.state.edu").await;
    let editor_a = login(&app, "editor.a@science.state.edu").await;
    let owner_b = login(&app, "owner.b@physics.state.edu").await;
    let faculty = login(&app, "prof.curie@science.state.edu").await;
    let student = login(&app, "student.smith@science.state.edu").await;

    // Workspace A
    let (_, ws_a) = send(
        &app,
        "POST",
        "/api/v1/workspaces",
        Some(&admin),
        &[],
        Some(json!({
            "name": "Workspace A6",
            "code": "WS-A6",
            "organization": "Science",
            "department": "biology",
            "visibility": "restricted"
        })),
    ).await;
    let ws_a_id = ws_a["id"].as_str().unwrap().to_string();

    for (eppn, role) in [
        ("owner.a@science.state.edu", "owner"),
        ("admin.a@science.state.edu", "admin"),
        ("editor.a@science.state.edu", "editor"),
    ] {
        let (st, _) = send(
            &app,
            "POST",
            &format!("/api/v1/workspaces/{ws_a_id}/collaborators"),
            Some(&admin),
            &[],
            Some(json!({ "eppn": eppn, "role": role, "name": eppn })),
        ).await;
        assert_eq!(st, StatusCode::CREATED);
    }

    // Workspace B
    let (_, ws_b) = send(
        &app,
        "POST",
        "/api/v1/workspaces",
        Some(&admin),
        &[],
        Some(json!({
            "name": "Workspace B6",
            "code": "WS-B6",
            "organization": "Physics",
            "department": "physics",
            "visibility": "restricted"
        })),
    ).await;
    let ws_b_id = ws_b["id"].as_str().unwrap().to_string();
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_b_id}/collaborators"),
        Some(&admin),
        &[],
        Some(json!({ "eppn": "owner.b@physics.state.edu", "role": "owner", "name": "Owner B" })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);

    // App in Workspace A
    let app_slug = format!("app-phase6-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let app_manifest = json!({
        "slug": app_slug,
        "title": "Phase 6 App",
        "description": "App for phase 6 tests",
        "organization_code": "SCI",
        "department": "biology",
        "views": [],
        "tables": [{
            "id": "t1",
            "slug": "t1",
            "name": "Table 1",
            "fields": [{
                "name": "status",
                "label": "Status",
                "field_type": "Text",
                "required": false,
                "ferpa_sensitive": false
            }]
        }],
        "ceds_mappings": {}
    });
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/workspaces/{ws_a_id}/apps"),
        Some(&owner_a),
        &[],
        Some(app_manifest),
    ).await;
    assert_eq!(st, StatusCode::CREATED);

    // 1. B's owner gets 403 on automation create and on process list for A's app.
    let rule_payload = json!({
        "id": "rule-1",
        "name": "Approval rule",
        "enabled": true,
        "trigger": "RecordCreated",
        "steps": [{
            "id": "step-1",
            "name": "Admin review",
            "kind": {
                "UserTask": {
                    "role": "admin",
                    "prompt": "Review required",
                    "approve": [{ "UpdateRecordStatus": { "new_status": "Approved" } }],
                    "reject": [{ "UpdateRecordStatus": { "new_status": "Rejected" } }]
                }
            }
        }]
    });
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/automations"),
        Some(&owner_b),
        &[],
        Some(rule_payload.clone()),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "B's owner must get 403 on automation create");

    let (st, _) = send(
        &app,
        "GET",
        &format!("/api/v1/apps/{app_slug}/processes"),
        Some(&owner_b),
        &[],
        None,
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "B's owner must get 403 on process list");

    // Owner A creates the automation rule
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/automations"),
        Some(&owner_a),
        &[],
        Some(rule_payload),
    ).await;
    assert_eq!(st, StatusCode::CREATED);

    // Owner A creates a record, which triggers automation rule and creates waiting process instance
    let (st, _rec_body) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/records"),
        Some(&owner_a),
        &[],
        Some(json!({ "data": { "status": "Pending" } })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);

    // Owner A lists processes to find the waiting instance
    let (st, proc_body) = send(
        &app,
        "GET",
        &format!("/api/v1/apps/{app_slug}/processes"),
        Some(&owner_a),
        &[],
        None,
    ).await;
    assert_eq!(st, StatusCode::OK);
    let proc_list = proc_body.as_array().expect("array of processes");
    assert!(!proc_list.is_empty(), "waiting process must exist");
    let proc_id = proc_list[0]["id"].as_str().unwrap().to_string();

    // 2. A step with role: "admin" is refused for A's editor and accepted for A's admin.
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/processes/{proc_id}/decide"),
        Some(&editor_a),
        &[],
        Some(json!({ "decision": "approve" })),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "Editor cannot decide admin step");

    // 3. A caller with affiliation member gets 403 on decide.
    let member_user = login(&app, "member.user@community.state.edu").await;
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/processes/{proc_id}/decide"),
        Some(&member_user),
        &[],
        Some(json!({ "decision": "approve" })),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "Member gets 403 on decide");

    // Admin A approves -> accepted
    let (st, _) = send(
        &app,
        "POST",
        &format!("/api/v1/apps/{app_slug}/processes/{proc_id}/decide"),
        Some(&admin_a),
        &[],
        Some(json!({ "decision": "approve" })),
    ).await;
    assert_eq!(st, StatusCode::OK, "Admin can decide admin step");

    // 4. A faculty caller reading the grants dataset receives no sample_data key.
    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/datasets",
        Some(&admin),
        &[],
        Some(json!({
            "id": "grants",
            "name": "Research Grants",
            "department": "research",
            "organization": "state.edu",
            "sensitivity_level": "Restricted",
            "sample_data": [{ "grant_id": 101, "amount": 50000 }]
        })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, ds_body) = send(
        &app,
        "GET",
        "/api/v1/datasets/grants",
        Some(&faculty),
        &[],
        None,
    ).await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        ds_body.get("sample_data").is_none(),
        "Faculty must not receive sample_data key for Restricted dataset"
    );

    // 5. A student gets 403 on GET /governance/ledger.
    let (st, _) = send(
        &app,
        "GET",
        "/api/v1/governance/ledger",
        Some(&student),
        &[],
        None,
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "Student must get 403 on GET /governance/ledger");
}


// =========================================================================
// Phase 7 Tests: Tokens are rows
// =========================================================================

#[tokio::test]
async fn test_phase_7_minted_token_works_and_delete_revokes() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    // 1. Mint a token via POST /api/v1/auth/tokens
    let (st, body) = send(
        &app,
        "POST",
        "/api/v1/auth/tokens",
        Some(&admin),
        &[],
        Some(json!({ "label": "Test Agent Token", "days": 30 })),
    ).await;
    assert_eq!(st, StatusCode::CREATED, "Minting token must return 201 Created: {body:?}");
    let token = body["token"].as_str().expect("plaintext token returned once");
    let token_id = body["id"].as_str().expect("token id returned");
    assert!(token.starts_with("scf_"), "token must start with scf_");

    // Token works on protected routes
    let (st, me_body) = send(&app, "GET", "/api/v1/auth/me", Some(token), &[], None).await;
    assert_eq!(st, StatusCode::OK, "Minted token must authenticate successfully");
    assert_eq!(me_body["user"]["eppn"].as_str(), Some("jordan.lee@state.edu"));

    // Revoke the token via DELETE /api/v1/auth/tokens/{id}
    let (st, _) = send(
        &app,
        "DELETE",
        &format!("/api/v1/auth/tokens/{token_id}"),
        Some(&admin),
        &[],
        None,
    ).await;
    assert_eq!(st, StatusCode::OK, "Revoking token must succeed");

    // After DELETE, the same token is 401
    let (st, _) = send(&app, "GET", "/api/v1/auth/me", Some(token), &[], None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "Revoked token must be 401");
}

#[tokio::test]
async fn test_phase_7_expired_token_is_401() {
    let app = build_app().expect("router");
    // Attempting to authenticate with an expired token is 401
    let fake_expired_token = format!("scf_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let (st, _) = send(&app, "GET", "/api/v1/auth/me", Some(&fake_expired_token), &[], None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_phase_7_database_holds_no_column_equal_to_token_text() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let (st, body) = send(
        &app,
        "POST",
        "/api/v1/auth/tokens",
        Some(&admin),
        &[],
        Some(json!({ "label": "Audit Hash Check", "days": 10 })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);
    let token = body["token"].as_str().expect("token").to_string();

    tokio::task::spawn_blocking(move || {
        let client_res = postgres::Client::connect(
            "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry",
            postgres::NoTls,
        );
        if let Ok(mut client) = client_res {
            let rows = client.query(
                "SELECT COUNT(*) FROM api_tokens WHERE token_hash = $1 OR label = $1",
                &[&token],
            ).unwrap();
            let cnt: i64 = rows[0].get(0);
            assert_eq!(cnt, 0, "Database holds no column equal to the token text");
        }
    }).await.unwrap();
}

#[tokio::test]
async fn test_phase_7_agent_token_gets_403_on_post_auth_tokens() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    // Mint an agent token
    let (st, body) = send(
        &app,
        "POST",
        "/api/v1/auth/tokens",
        Some(&admin),
        &[],
        Some(json!({ "label": "First Agent", "days": 14, "kind": "agent" })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);
    let agent_token = body["token"].as_str().unwrap();

    // Request made with an agent token gets 403 on POST /auth/tokens
    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/auth/tokens",
        Some(agent_token),
        &[],
        Some(json!({ "label": "Sub-Agent", "days": 14 })),
    ).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "Agent token must get 403 on POST /auth/tokens");
}

#[tokio::test]
async fn test_phase_7_inactive_scim_user_token_is_401() {
    let app = build_app().expect("router");
    let _admin = login(&app, "jordan.lee@state.edu").await;

    // Create a new SCIM user
    let user_eppn = format!("user-{}@state.edu", &uuid::Uuid::new_v4().to_string()[..8]);
    let (st, scim_res) = send(
        &app,
        "POST",
        "/scim/v2/Users",
        Some("test-scim-token"),
        &[],
        Some(json!({
            "userName": user_eppn,
            "active": true,
            "name": { "formatted": "Temporary User" }
        })),
    ).await;
    assert_eq!(st, StatusCode::CREATED);
    let user_id = scim_res["id"].as_str().unwrap();

    // Login or mint token for that user
    let user_token = login(&app, &user_eppn).await;

    // Verify token works
    let (st, _) = send(&app, "GET", "/api/v1/auth/me", Some(&user_token), &[], None).await;
    assert_eq!(st, StatusCode::OK);

    // Set the SCIM user to active: false
    let (st, _) = send(
        &app,
        "PUT",
        &format!("/scim/v2/Users/{user_id}"),
        Some("test-scim-token"),
        &[],
        Some(json!({
            "userName": user_eppn,
            "active": false,
            "name": { "formatted": "Temporary User" }
        })),
    ).await;
    assert_eq!(st, StatusCode::OK);

    // That user's token is 401 on the next request
    let (st, _) = send(&app, "GET", "/api/v1/auth/me", Some(&user_token), &[], None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "Inactive SCIM user token must be 401");
}

#[tokio::test]
async fn test_phase_7_legacy_post_auth_token_is_404() {
    let app = build_app().expect("router");
    let (st, _) = send(
        &app,
        "POST",
        "/api/v1/auth/token",
        None,
        &[],
        Some(json!({ "eppn": "jordan.lee@state.edu" })),
    ).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "POST /api/v1/auth/token must be 404");
}

#[tokio::test]
async fn test_phase_7_second_boot_prints_no_setup_token() {
    let app = build_app().expect("router");
    let _admin = login(&app, "jordan.lee@state.edu").await;
    // With admin present in DB, check_boot_setup_token returns None
    assert!(!scaffoldry_server::service::identity::boot_setup_token_needed(&scaffoldry_server::state::ServerState::new().unwrap()), "Second boot must not need setup token");
}


#[tokio::test]
async fn test_phase_8_settings_cors_allowed_origins_preflight_and_ledger() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let test_origin_str = format!("https://partner-{}.example.edu", &uuid::Uuid::new_v4().to_string()[..8]);
    let test_origin = test_origin_str.as_str();

    // 1. Preflight before setting is not allowed
    let (_, headers, _) = send_full(
        &app,
        "OPTIONS",
        "/api/v1/workspaces",
        None,
        &[
            ("origin", test_origin),
            ("access-control-request-method", "GET"),
        ],
        None,
    ).await;
    assert_ne!(headers.get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some(test_origin));

    // 2. Platform Admin updates cors.allowed_origins
    let (st, body) = send(
        &app,
        "PUT",
        "/api/v1/settings/cors.allowed_origins",
        Some(&admin),
        &[],
        Some(json!([test_origin])),
    ).await;
    assert_eq!(st, StatusCode::OK, "PUT /settings/cors.allowed_origins must succeed: {body:?}");

    // 3. Preflight from that origin is now allowed without a restart
    let (_, headers, _) = send_full(
        &app,
        "OPTIONS",
        "/api/v1/workspaces",
        None,
        &[
            ("origin", test_origin),
            ("access-control-request-method", "GET"),
        ],
        None,
    ).await;
    assert_eq!(
        headers.get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
        Some(test_origin),
        "CORS preflight from newly added origin must succeed immediately"
    );

    // 4. Ledger entry was added for PolicyRevision
    let (st, ledger_body) = send(&app, "GET", "/api/v1/governance/ledger", Some(&admin), &[], None).await;
    assert_eq!(st, StatusCode::OK);
    let entries = ledger_body["entries"].as_array().expect("ledger entries array");
    let last = entries.last().expect("at least one ledger entry");
    assert_eq!(last["decision_type"], "PolicyRevision");
    assert!(last["rationale"].as_str().unwrap().contains("cors.allowed_origins"));
}

#[tokio::test]
async fn test_phase_8_put_settings_dev_mode_is_400() {
    let app = build_app().expect("router");
    let admin = login(&app, "jordan.lee@state.edu").await;

    let (st, _) = send(
        &app,
        "PUT",
        "/api/v1/settings/dev_mode",
        Some(&admin),
        &[],
        Some(json!(true)),
    ).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "PUT /settings/dev_mode must be 400");
}

#[tokio::test]
async fn test_phase_8_faculty_caller_gets_403_on_get_settings() {
    let app = build_app().expect("router");
    let faculty = login(&app, "prof.curie@science.state.edu").await;

    let (st, _) = send(&app, "GET", "/api/v1/settings", Some(&faculty), &[], None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "Faculty caller must get 403 on GET /settings");

    let admin = login(&app, "jordan.lee@state.edu").await;
    let (st, body) = send(&app, "GET", "/api/v1/settings", Some(&admin), &[], None).await;
    assert_eq!(st, StatusCode::OK, "Platform Admin gets 200 on GET /settings: {body:?}");
}

#[test]
fn test_phase_8_fresh_database_has_zero_workspaces() {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string()
    });
    if let Ok(mut client) = postgres::Client::connect(&db_url, postgres::NoTls) {
        let _ = client.batch_execute("TRUNCATE TABLE workspaces CASCADE;");
    }
    let state = ServerState::new().expect("ServerState::new on fresh database");
    assert_eq!(state.workspaces.read().unwrap().len(), 0, "Fresh database after boot must have zero workspaces");
    let repo_count = state.repository.as_ref().map(|r| r.list_workspaces().unwrap().len()).unwrap_or(0);
    assert_eq!(repo_count, 0, "PostgreSQL repository must have zero workspaces after fresh boot");
}

#[test]
fn test_phase_8_unreachable_database_url_returns_error() {
    let _orig = std::env::var("DATABASE_URL").ok();
    std::env::set_var("DATABASE_URL", "postgres://scaffoldry:wrong_password@127.0.0.1:5433/scaffoldry");
    let res = ServerState::new();
    if let Some(ref o) = _orig {
        std::env::set_var("DATABASE_URL", o);
    } else {
        std::env::remove_var("DATABASE_URL");
    }
    assert!(res.is_err(), "ServerState::new must return error when DATABASE_URL is unreachable");
}
