//! The write tools in `TOOLS`. Each is called through A's `/api/mcp` and read through B.

use super::platform::{library_fixture, LIBRARY_PASSAGE};
use super::apps::{enqueue_job, job_state, read_record, waiting_instance};
use super::*;

pub fn add(m: &mut Entries) {
    m.insert("tool:update_workspace".into(), case!(update_workspace));
    m.insert("tool:manage_workspace_member".into(), case!(manage_workspace_member));
    m.insert("tool:create_record".into(), case!(create_record));
    m.insert("tool:update_record".into(), case!(update_record));
    m.insert("tool:page_call".into(), case!(page_call));
    m.insert("tool:delete_record".into(), case!(delete_record));
    m.insert("tool:record_governance_decision".into(), case!(record_governance_decision));
    m.insert("tool:propose_app_change".into(), case!(propose_app_change));
    m.insert("tool:decide_proposal".into(), case!(decide_proposal));
    m.insert("tool:preview_proposal".into(), case!(preview_proposal));
    m.insert("tool:decide_process".into(), case!(decide_process));
    m.insert("tool:save_view".into(), case!(save_view));
    m.insert("tool:delete_view".into(), case!(delete_view));
    m.insert("tool:cancel_job".into(), case!(cancel_job));
    m.insert("tool:propose_library_rules".into(), case!(propose_library_rules));
    m.insert("tool:export_records".into(), files_case!(export_records));
}

async fn update_workspace(w: World) -> Outcome {
    let id = make_workspace(&w, &unique("P6TW"), "restricted").await?;
    let name = unique("Tool-renamed");
    w.tool_a(ADMIN, "update_workspace", json!({ "workspace_id": id, "name": name })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}"), ADMIN, None).await, StatusCode::OK, "B reads it")?;
    ensure(has(&seen, &name), format!("B shows the name the tool set: {seen}"))
}

async fn manage_workspace_member(w: World) -> Outcome {
    let id = make_workspace(&w, &unique("P6TM"), "restricted").await?;
    let who = person("p6tool");
    w.tool_a(ADMIN, "manage_workspace_member", json!({ "workspace_id": id, "action": "add", "eppn": who, "role": "viewer", "name": "Tool Person" })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    ensure(has(&seen, &who), format!("B lists the member the tool added: {seen}"))
}

async fn create_record(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let made = w.tool_a(OWNER, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "apps", "name": "By tool" } })).await?;
    let id = made["id"].as_str().or_else(|| made["record"]["id"].as_str()).ok_or_else(|| format!("the new record's id: {made}"))?.to_string();
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "By tool"), format!("B shows the record: {seen}"))
}

async fn update_record(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Before").await?;
    w.tool_a(OWNER, "update_record", json!({ "app_slug": slug, "record_id": id, "version": 1, "data": { "name": "By tool" } })).await?;
    let seen = read_record(&w, &slug, &id).await?.ok_or("B does not find the record")?;
    ensure(has(&seen, "By tool"), format!("B shows the change: {seen}"))
}

async fn delete_record(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let id = make_record(&w, &slug, "Gone").await?;
    w.tool_a(OWNER, "delete_record", json!({ "app_slug": slug, "record_id": id, "version": 1 })).await?;
    ensure(read_record(&w, &slug, &id).await?.is_none(), "B no longer finds the record")
}

async fn record_governance_decision(w: World) -> Outcome {
    let rationale = unique("p6 tool decision");
    w.tool_a(ADMIN, "record_governance_decision", json!({ "organization_code": "INST", "decision_type": "PolicyRevision", "oscal_control_id": "AU-02", "rationale": rationale })).await?;
    let seen = status(w.rb("GET", "/api/v1/governance/ledger?limit=5", ADMIN, None).await, StatusCode::OK, "B reads the ledger")?;
    ensure(has(&seen, &rationale), format!("B shows the entry the tool wrote: {seen}"))
}

async fn propose_app_change(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "toolfield", "label": "toolfield", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "By tool" })).await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    let seen = status(w.rb("GET", &format!("/api/v1/proposals/{id}"), OWNER, None).await, StatusCode::OK, "B reads the proposal")?;
    ensure(has(&seen, &id), format!("B shows the proposal the tool made: {seen}"))
}

async fn preview_proposal(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "shown", "label": "shown", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "To preview" })).await?;
    let shown = w.tool_a(OWNER, "preview_proposal", json!({ "proposal_id": proposed["proposal_id"] })).await?;
    let (token, shadow) = (shown["token"].as_str().ok_or("a token")?.to_string(), shown["app_slug"].as_str().ok_or("an app slug")?.to_string());
    let seen = status(World::call_with(&w.b, "GET", &format!("/api/v1/apps/{shadow}/schema"), &token, None).await, StatusCode::OK, "B serves the preview token")?;
    ensure(has(&seen, "shown"), format!("B shows the field the proposal adds: {seen}"))
}

async fn decide_proposal(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let mut changed = manifest(&slug);
    changed["tables"][0]["fields"].as_array_mut().ok_or("fields")?.push(json!({ "name": "decided", "label": "decided", "field_type": "Text", "required": false, "ferpa_sensitive": false }));
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": changed, "summary": "To decide" })).await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    w.tool_a(OWNER, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/schema"), OWNER, None).await, StatusCode::OK, "B reads the schema")?;
    ensure(has(&seen, "decided"), format!("B shows the field the approval added: {seen}"))
}

async fn decide_process(w: World) -> Outcome {
    let (slug, instance) = waiting_instance(&w).await?;
    w.tool_a(OWNER, "decide_process", json!({ "app_slug": slug, "instance_id": instance, "decision": "reject" })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/processes"), OWNER, None).await, StatusCode::OK, "B lists instances")?;
    ensure(has(&seen, "Rejected") && !has(&seen, "Waiting"), format!("B shows the instance rejected: {seen}"))
}

async fn save_view(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let name = unique("p6toolview");
    w.tool_a(OWNER, "save_view", json!({ "app_slug": slug, "name": name, "kind": "grid", "table_id": "apps", "config": {} })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    ensure(has(&seen, &name), format!("B lists the view the tool saved: {seen}"))
}

async fn delete_view(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    let name = unique("p6doomed");
    let saved = w.tool_a(OWNER, "save_view", json!({ "app_slug": slug, "name": name, "kind": "grid", "table_id": "apps", "config": {} })).await?;
    let id = saved["id"].as_str().or_else(|| saved["view"]["id"].as_str()).ok_or_else(|| format!("the view id: {saved}"))?.to_string();
    w.tool_a(OWNER, "delete_view", json!({ "app_slug": slug, "view_id": id })).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/apps/{slug}/views"), OWNER, None).await, StatusCode::OK, "B lists views")?;
    ensure(!has(&seen, &name), format!("B no longer lists the view: {seen}"))
}

async fn cancel_job(w: World) -> Outcome {
    let id = enqueue_job(&w, ADMIN)?;
    w.tool_a(ADMIN, "cancel_job", json!({ "job_id": id })).await?;
    let state = job_state(&w, &id).await?;
    ensure(state == "cancelled", format!("B sees the job cancelled, not {state}"))
}

async fn export_records(w: World) -> Outcome {
    let slug = make_app(&w).await?;
    make_record(&w, &slug, "Exported").await?;
    let started = w.tool_a(OWNER, "export_records", json!({ "app_slug": slug, "table_id": "apps", "format": "csv" })).await?;
    let job = started["job_id"].as_str().ok_or_else(|| format!("a job id: {started}"))?.to_string();
    let state = job_state(&w, &job).await?;
    ensure(!state.is_empty(), format!("B sees the export job: {started}"))
}

const PAGE_SOURCE: &str = "export default function Page() { return <p>two instances</p>; }";

/// A new app with a custom page `board` that may read and write `name`, published through A.
pub async fn make_page_app(w: &World) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let slug = unique("p6page");
    let sha: String = Sha256::digest(PAGE_SOURCE.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let mut m = manifest(&slug);
    m["pages"] = json!([{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha, "grants": [
        { "table_id": "apps", "read": ["name"], "write": ["name"], "create": false, "delete": false }
    ]}}}]);
    let proposed = w.tool_a(OWNER, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": m, "summary": "A page", "page_sources": [PAGE_SOURCE] })).await?;
    let id = proposed["proposal_id"].as_str().ok_or("a proposal id")?.to_string();
    w.tool_a(OWNER, "decide_proposal", json!({ "proposal_id": id, "decision": "approve", "rationale": "Reviewed", "digest": proposed["digest"] })).await?;
    Ok(slug)
}

async fn page_call(w: World) -> Outcome {
    let slug = make_page_app(&w).await?;
    let id = make_record(&w, &slug, "Before").await?;
    w.tool_a(OWNER, "page_call", json!({ "app_slug": slug, "page_id": "board", "op": "update", "table_id": "apps", "record_id": id, "version": 1, "data": { "name": "Through a page" } })).await?;
    w.tool_a(OWNER, "page_call", json!({ "app_slug": slug, "page_id": "board", "op": "settings_set", "key": "k", "value": "kept" })).await?;
    let seen = w.tool_b(OWNER, "page_call", json!({ "app_slug": slug, "page_id": "board", "op": "get", "table_id": "apps", "record_id": id })).await?;
    ensure(has(&seen, "Through a page"), format!("B shows the change made through the page: {seen}"))?;
    let setting = w.tool_b(OWNER, "page_call", json!({ "app_slug": slug, "page_id": "board", "op": "settings_get", "key": "k" })).await?;
    ensure(has(&setting, "kept"), format!("B reads the setting A wrote: {setting}"))
}

async fn propose_library_rules(w: World) -> Outcome {
    let (policy, _, snap) = library_fixture(&w, "/api/v1").await?;
    let rule = json!({ "key": "retention", "statement": "Student records are kept for seven years.", "citations": [{ "snapshot_id": snap, "passage": LIBRARY_PASSAGE }] });
    let out = w.tool_a(ADMIN, "propose_library_rules", json!({ "policy_id": policy, "rules": [rule] })).await?;
    ensure(out["rules"][0]["stored"] == true, format!("A stores the rule: {out}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/admin/library/policies/{policy}"), ADMIN, None).await, StatusCode::OK, "B reads the policy")?;
    ensure(seen["rule_counts"]["Draft"] == 1, format!("B counts the draft the tool stored: {seen}"))
}
