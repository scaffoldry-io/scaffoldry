//! Phase 5: Access rules authoring, approval, problem checks, and preview endpoint.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use scaffoldry_core::access::{Op, Operand, PersonAttr, Pred, RowOp, RowRule};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;


const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const ADMIN: &str = "jordan.lee@state.edu";

struct World {
    app: axum::Router,
    owner: String,
    admin: String,
    slug: String,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    let slug = format!("app-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    World {
        app,
        owner: issue_test_token_and_user(OWNER),
        admin: issue_test_token_and_user(ADMIN),
        slug,
    }
}

async fn rest(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn tool(app: &axum::Router, token: &str, name: &str, args: Value) -> (bool, Value) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(app, "POST", "/api/mcp", token, Some(payload)).await;
    if let Some(err) = v.get("error") {
        return (true, err["message"].clone());
    }
    let result = &v["result"];
    if result["isError"] == true {
        return (true, result["content"][0]["text"].clone());
    }
    (false, result["structuredContent"].clone())
}

fn sample_manifest(slug: &str, rows: Vec<RowRule>) -> Value {
    json!({
        "slug": slug,
        "title": "Advising App",
        "description": "Student advising records",
        "organization_code": "PHYS",
        "department": "Physics",
        "workspace_id": WORKSPACE,
        "tables": [{
            "id": "students",
            "name": "Students",
            "slug": "students",
            "fields": [
                { "name": "name", "label": "Name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
                { "name": "advisor", "label": "Advisor", "field_type": "Text", "required": false, "ferpa_sensitive": false, "indexed": true },
                { "name": "gpa", "label": "GPA", "field_type": "Number", "required": false, "ferpa_sensitive": false }
            ],
            "sample_records": []
        }],
        "views": [],
        "pages": [],
        "access": {
            "rows": rows,
            "columns": []
        }
    })
}

// 2. Removing a rule raises access_loosened. The proposer cannot approve. Another admin can. Adding a rule does not raise it.
#[tokio::test]
async fn removing_a_rule_raises_access_loosened_requiring_second_person() {
    let _db = common::TestDb::new();
    let w = world().await;

    let rule1 = RowRule {
        id: "r1".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp {
            field: "advisor".into(),
            op: Op::Eq,
            value: Operand::Person(PersonAttr::Address),
        },
        except_roles: vec!["owner".into(), "admin".into()],
        message: None,
    };

    // 1. Propose adding rule1
    let (err, p1) = tool(
        &w.app,
        &w.owner,
        "propose_app_change",
        json!({
            "workspace_id": WORKSPACE,
            "manifest": sample_manifest(&w.slug, vec![rule1.clone()]),
            "summary": "Add initial access rule"
        }),
    )
    .await;
    assert!(!err, "{p1}");
    let checks = p1["checks"].as_array().unwrap();
    let loosened = checks.iter().find(|c| c["name"] == "access_loosened").unwrap();
    assert_eq!(loosened["outcome"], "pass", "Adding a rule does not raise access_loosened");

    // Proposer can approve adding a rule
    let id1 = p1["proposal_id"].as_str().unwrap();
    let (err, done) = tool(
        &w.app,
        &w.owner,
        "decide_proposal",
        json!({
            "proposal_id": id1,
            "decision": "approve",
            "rationale": "Initial setup"
        }),
    )
    .await;
    assert!(!err, "Owner can approve initial rule addition: {done}");

    // 2. Now propose removing the rule
    let (err, p2) = tool(
        &w.app,
        &w.owner,
        "propose_app_change",
        json!({
            "workspace_id": WORKSPACE,
            "manifest": sample_manifest(&w.slug, vec![]),
            "summary": "Remove access rule"
        }),
    )
    .await;
    assert!(!err, "{p2}");
    let checks2 = p2["checks"].as_array().unwrap();
    let loosened2 = checks2.iter().find(|c| c["name"] == "access_loosened").unwrap();
    assert_eq!(loosened2["outcome"], "fail", "Removing a rule raises access_loosened");

    // Proposer CANNOT approve because access_loosened is a flagged failure
    let id2 = p2["proposal_id"].as_str().unwrap();
    let (err, err_msg) = tool(
        &w.app,
        &w.owner,
        "decide_proposal",
        json!({
            "proposal_id": id2,
            "decision": "approve",
            "rationale": "Self-approval attempt"
        }),
    )
    .await;
    assert!(err, "Proposer should fail to approve flagged failure");
    let msg = err_msg.as_str().unwrap_or("");
    assert!(
        msg.contains("second person") || msg.contains("proposer cannot"),
        "Expected second person error: {msg}"
    );

    // Another admin CAN approve
    let (err, approved) = tool(
        &w.app,
        &w.admin,
        "decide_proposal",
        json!({
            "proposal_id": id2,
            "decision": "approve",
            "rationale": "Approved by second admin"
        }),
    )
    .await;
    assert!(!err, "Second admin can approve: {approved}");
    assert_eq!(approved["status"], "Approved");
}

// 4. The preview returns counts matching a direct row_scope count and never returns row data.
#[tokio::test]
async fn access_preview_returns_counts_matching_row_scope_and_never_rows() {
    let _db = common::TestDb::new();
    let w = world().await;

    // Create app and seed some records
    let (s, body) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/apps"),
        &w.owner,
        Some(sample_manifest(&w.slug, vec![])),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "res: {body}");

    // Insert 3 records: 2 advised by einstein, 1 by someone else
    let records = vec![
        json!({ "_table_id": "students", "name": "Alice", "advisor": OWNER, "gpa": 3.8 }),
        json!({ "_table_id": "students", "name": "Bob", "advisor": OWNER, "gpa": 3.4 }),
        json!({ "_table_id": "students", "name": "Charlie", "advisor": "other@state.edu", "gpa": 3.1 }),
    ];
    for r in records {
        let (s, body) = rest(
            &w.app,
            "POST",
            &format!("/api/v1/apps/{}/tables/students/records", w.slug),
            &w.owner,
            Some(r),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "res: {body}");
    }

    // Call access preview with proposed rule: Advisor == person.address
    let rule = RowRule {
        id: "r_adv".into(),
        table_id: "students".into(),
        ops: vec![RowOp::Read],
        visible_when: Pred::Cmp {
            field: "advisor".into(),
            op: Op::Eq,
            value: Operand::Person(PersonAttr::Address),
        },
        except_roles: vec![],
        message: None,
    };

    let (s, preview) = rest(
        &w.app,
        "POST",
        &format!("/api/v1/apps/{}/access/preview", w.slug),
        &w.owner,
        Some(json!({
            "table_id": "students",
            "eppn": OWNER,
            "rules": [rule]
        })),
    )
    .await;

    assert_eq!(s, StatusCode::OK, "{preview}");
    assert_eq!(preview["total_rows"], 3);
    assert_eq!(preview["can_see"], 2);
    assert_eq!(preview["can_add"], 3);
    assert_eq!(preview["can_change"], 3);
    assert_eq!(preview["can_remove"], 3);
    // Verify it NEVER returns row data
    assert!(preview.get("records").is_none());
    assert!(preview.get("rows").is_none());
    assert!(preview.get("data").is_none());
}
