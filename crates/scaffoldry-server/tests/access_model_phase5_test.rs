//! Access model phase 5: synthetic institution, downgrade refusal, and isolation rules.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::synthesize::{compute_world_hash, generate_world, SynthesizeOptions};
use scaffoldry_server::state::OrganizationNode;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const SYNTH_PERSON: &str = "synth_1@synthetic.invalid";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) })
}

// ---------------------------------------------------------------------------------------------
// Test 1: The same seed produces the same institution twice. Compare a hash of every row it wrote.
// ---------------------------------------------------------------------------------------------
#[test]
fn test_1_same_seed_produces_identical_world_hash() {
    let opts1 = SynthesizeOptions {
        seed: 1234567,
        people: 100,
        units: 20,
        hierarchies: vec!["academic".to_string(), "supervisory".to_string()],
        apps: 5,
        records: 50,
    };
    let opts2 = opts1.clone();

    let world1 = generate_world(&opts1);
    let world2 = generate_world(&opts2);

    let hash1 = compute_world_hash(&world1);
    let hash2 = compute_world_hash(&world2);

    assert_eq!(hash1, hash2, "Identical seed must produce identical world hash");

    // Different seed produces different hash
    let mut opts3 = opts1.clone();
    opts3.seed = 7654321;
    let world3 = generate_world(&opts3);
    let hash3 = compute_world_hash(&world3);
    assert_ne!(hash1, hash3, "Different seed must produce different hash");
}

// ---------------------------------------------------------------------------------------------
// Test 2: Changing `synthetic` from true to false is refused by the database.
// ---------------------------------------------------------------------------------------------
#[test]
fn test_2_changing_synthetic_from_true_to_false_is_refused_by_database() {
    let db = TestDb::new();
    let root = Uuid::parse_str(ROOT).unwrap();

    db.run(move |c| {
        // 1. Organizations
        let synth_org = Uuid::new_v4();
        c.execute(
            "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key, synthetic) \
             VALUES ($1, $2, 'Synth Org', 'SYN-TEST-1', 'College', 'academic', TRUE)",
            &[&synth_org, &root],
        ).unwrap();

        let res = c.execute(
            "UPDATE organizations SET synthetic = FALSE WHERE id = $1",
            &[&synth_org],
        );
        assert!(res.is_err(), "Database trigger refuse_synthetic_downgrade must reject synthetic = FALSE on organizations");
        let db_err = res.unwrap_err();
        let err_msg = db_err.as_db_error().map(|e| e.message().to_string()).unwrap_or_else(|| db_err.to_string());
        assert!(err_msg.contains("Cannot change synthetic from true to false"), "Error message should mention refuse_synthetic_downgrade: {err_msg}");

        // 2. Persons
        let synth_person = Uuid::new_v4();
        c.execute(
            "INSERT INTO persons (id, first_name, last_name, email, eppn, synthetic) \
             VALUES ($1, 'Synth', 'User', 'test@synthetic.invalid', 'test@synthetic.invalid', TRUE)",
            &[&synth_person],
        ).unwrap();

        let res_p = c.execute(
            "UPDATE persons SET synthetic = FALSE WHERE id = $1",
            &[&synth_person],
        );
        assert!(res_p.is_err(), "Database trigger must reject synthetic = FALSE on persons");

        // 3. SCIM users
        c.execute(
            "INSERT INTO scim_users (id, user_name, active, payload, synthetic, updated_at) \
             VALUES ($1, 'test@synthetic.invalid', TRUE, '{}'::jsonb, TRUE, NOW())",
            &[&synth_person.to_string()],
        ).unwrap();

        let res_s = c.execute(
            "UPDATE scim_users SET synthetic = FALSE WHERE id = $1",
            &[&synth_person.to_string()],
        );
        assert!(res_s.is_err(), "Database trigger must reject synthetic = FALSE on scim_users");

        // 4. Workspaces
        let synth_ws = "ws-synth-test";
        c.execute(
            "INSERT INTO workspaces (id, name, code, organization, department, description, icon, lead, visibility, data_classification, organization_id, synthetic) \
             VALUES ($1, 'Synth WS', 'WS-SYN-TEST', 'INST', 'biology', 'desc', '🔬', 'lead', 'restricted', 'Internal', $2, TRUE)",
            &[&synth_ws, &root],
        ).unwrap();

        let res_w = c.execute(
            "UPDATE workspaces SET synthetic = FALSE WHERE id = $1",
            &[&synth_ws],
        );
        assert!(res_w.is_err(), "Database trigger must reject synthetic = FALSE on workspaces");
    });
}

// ---------------------------------------------------------------------------------------------
// Test 3: A synthetic person's grant anchored in a real unit is refused (HTTP 400).
// ---------------------------------------------------------------------------------------------
#[tokio::test]
async fn test_3_synthetic_person_grant_anchored_in_real_unit_is_refused() {
    let db = TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();

    // Create a real unit (synthetic: false)
    let real_unit = Uuid::new_v4();
    let real_org = OrganizationNode {
        id: real_unit,
        parent_id: Some(root),
        name: "Real Chemistry Department".to_string(),
        code: "REAL-CHEM".to_string(),
        org_type: "Department".to_string(),
        hierarchy_key: "academic".to_string(),
        synthetic: false,
    };
    state.organizations.write().unwrap().insert(real_unit, real_org.clone());
    state.repository.upsert_organization(&real_org).unwrap();

    // Create a synthetic unit (synthetic: true)
    let synth_unit = Uuid::new_v4();
    let synth_org = OrganizationNode {
        id: synth_unit,
        parent_id: Some(root),
        name: "Synthetic Chemistry Department".to_string(),
        code: "SYN-CHEM".to_string(),
        org_type: "Department".to_string(),
        hierarchy_key: "academic".to_string(),
        synthetic: true,
    };
    state.organizations.write().unwrap().insert(synth_unit, synth_org.clone());
    state.repository.upsert_organization(&synth_org).unwrap();

    let app = build_app_with_state(state.clone()).unwrap();

    // Attempt to propose a grant for a synthetic person anchored in a REAL unit -> 400
    let grant_in_real = json!({
        "holder_kind": "person",
        "holder_id": SYNTH_PERSON,
        "role": "viewer",
        "hierarchy": "academic",
        "anchor_kind": "unit",
        "anchor_id": real_unit.to_string(),
        "reason": "Testing synthetic grant isolation in real unit",
    });

    let (status, resp) = send(&app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(grant_in_real)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Grant held by synthetic person in real unit must be refused with 400");
    let err = resp.get("error").and_then(Value::as_str).unwrap_or("");
    assert!(
        err.contains("A grant held by a synthetic person may only anchor inside the synthetic subtree"),
        "Error message must specify isolation remedy: {err}"
    );

    // Now propose the same grant anchored in the SYNTHETIC unit -> Accepted (202)
    let grant_in_synth = json!({
        "holder_kind": "person",
        "holder_id": SYNTH_PERSON,
        "role": "viewer",
        "hierarchy": "academic",
        "anchor_kind": "unit",
        "anchor_id": synth_unit.to_string(),
        "reason": "Testing synthetic grant in synthetic unit",
    });

    let (status, _) = send(&app, "POST", "/api/v1/admin/access/grants", ADMIN, Some(grant_in_synth)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "Grant held by synthetic person in synthetic unit must succeed");

    // Also verify: synthetic unit may only sit under a synthetic unit -> creating synthetic unit under real_unit is 400
    let invalid_synth_unit = json!({
        "name": "Sub-synth under real",
        "code": "SYN-UNDER-REAL",
        "org_type": "Program",
        "parent_id": real_unit,
        "hierarchy_key": "academic",
        "synthetic": true,
    });
    let (status, resp) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(invalid_synth_unit)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "Synthetic unit under non-synthetic unit must return 400");
    let err = resp.get("error").and_then(Value::as_str).unwrap_or("");
    assert!(
        err.contains("A synthetic unit may only sit under a synthetic unit"),
        "Error should state synthetic unit rule: {err}"
    );
}
