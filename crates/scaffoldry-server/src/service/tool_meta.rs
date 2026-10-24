//! What `tools/list` says about each tool beyond its schema: whether it removes anything, whether repeating it is safe,
//! and one example call. The conformance test fails when a tool has no row here.

use serde_json::{json, Value};

/// `(destructive, idempotent)` for a tool that changes something. A destructive tool removes or revokes. An idempotent
/// tool is safe to repeat with the same arguments.
pub fn hints(name: &str) -> (bool, bool) {
    match name {
        "manage_workspace_member" => (true, false),
        "delete_record" | "delete_view" | "cancel_job" => (true, true),
        // A page may remove a record when its grant allows it.
        "page_call" => (true, false),
        "update_workspace" | "update_record" | "save_view" | "begin_task" => (false, true),
        _ => (false, false),
    }
}

/// One valid call, as the arguments object.
pub fn example(name: &str) -> Value {
    match name {
        "list_workspaces" | "verify_decision_ledger" | "export_oscal_compliance" | "get_framework_spec" | "list_waiting_decisions" => json!({}),
        "get_workspace" => json!({ "workspace_id": "ws-physics-optics" }),
        "update_workspace" => json!({ "workspace_id": "ws-physics-optics", "description": "Laser logs and sensor arrays" }),
        "manage_workspace_member" => json!({ "workspace_id": "ws-physics-optics", "action": "add", "eppn": "ada@example.edu", "role": "editor" }),
        "create_record" => json!({ "app_slug": "scholarships", "data": { "applicant": "Ada Lovelace", "amount": 4000 } }),
        "list_datasets" => json!({ "department": "Academic Affairs" }),
        "simulate_cedar_policy" => json!({ "eppn": "ada@example.edu", "action": "read", "ferpa_sensitive": true }),
        "record_governance_decision" => json!({ "organization_code": "PHYS", "decision_type": "PolicyRevision", "oscal_control_id": "CM-03", "rationale": "Reviewed the change with the chair" }),
        "list_apps" | "list_proposals" => json!({ "workspace_id": "ws-physics-optics" }),
        "describe_app" | "list_views" | "list_pages" => json!({ "app_slug": "scholarships" }),
        "aggregate_records" => json!({ "app_slug": "scholarships", "table_id": "applications", "function": "sum", "field": "amount" }),
        "list_records" => json!({ "app_slug": "scholarships", "table_id": "applications", "limit": 50 }),
        "get_view_data" => json!({ "app_slug": "scholarships", "view_id": "9d0a7c52-0d3e-4f55-8b53-6d2d6d9a4f10" }),
        "get_page" => json!({ "app_slug": "scholarships", "page_id": "board" }),
        "page_call" => json!({ "app_slug": "scholarships", "page_id": "board", "op": "list", "table_id": "applications", "limit": 50 }),
        "get_record" => json!({ "app_slug": "scholarships", "record_id": "rec_123" }),
        "get_record_history" => json!({ "app_slug": "scholarships", "record_id": "rec_123", "limit": 20 }),
        "update_record" => json!({ "app_slug": "scholarships", "record_id": "rec_123", "version": 3, "data": { "amount": 5000 } }),
        "delete_record" => json!({ "app_slug": "scholarships", "record_id": "rec_123", "version": 3 }),
        "propose_app_change" => json!({ "workspace_id": "ws-physics-optics", "app_slug": "scholarships", "summary": "Add an amount column" }),
        "get_agent_kit" => json!({}),
        "explain_access" => json!({ "object": "app:scholarships" }),
        "begin_task" => json!({ "task": "Enter the spring grant proposals" }),
        "preview_proposal" => json!({ "proposal_id": "3f2b8f0e-6a41-4c58-9d0b-1f6b0c7e2a11" }),
        "get_proposal" => json!({ "proposal_id": "3f2b8f0e-6a41-4c58-9d0b-1f6b0c7e2a11" }),
        "decide_proposal" => json!({ "proposal_id": "3f2b8f0e-6a41-4c58-9d0b-1f6b0c7e2a11", "decision": "approve", "rationale": "Checks passed and the chair agrees", "digest": "9f2c4b7a1e6d3058b4a9c2e7d1f0a36b5c8e9d4f7a2b1c0e3d6f8a9b4c7e2d51" }),
        "decide_process" => json!({ "app_slug": "scholarships", "instance_id": "5a4c0d6e-2b0b-4d86-9e1c-7c1c8f3b9a02", "decision": "approve" }),
        "save_view" => json!({ "app_slug": "scholarships", "name": "Over 5000", "kind": "grid", "table_id": "applications" }),
        "delete_view" => json!({ "app_slug": "scholarships", "view_id": "9d0a7c52-0d3e-4f55-8b53-6d2d6d9a4f10" }),
        "get_job" | "cancel_job" => json!({ "job_id": "c6f1c1a2-8f1e-4d44-b8a0-0e8f5f3a9b77" }),
        "export_records" => json!({ "app_slug": "scholarships", "table_id": "applications", "format": "csv" }),
        _ => Value::Null,
    }
}
