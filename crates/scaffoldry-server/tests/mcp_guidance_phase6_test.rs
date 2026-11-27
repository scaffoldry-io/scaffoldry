//! MCP guidance phase 6: typed text is marked untrusted in every tool result, and a tool that would change access,
//! ownership or labels drafts a change and a desk link, and changes nothing. The ledger names the agent as the proposer and
//! the person who confirms as the decider.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::service::tools::TOOLS;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const SECOND: &str = "second.admin@state.edu";
const TARGET: &str = "target.person@state.edu";
const FACULTY: &str = "sarah.connor@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const CALLBACK: &str = "https://tool.example.edu/callback";
const ISSUER: &str = "https://api.test.edu";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
/// Every text a person typed in the seeded world starts with this. It never appears outside `untrusted_text`.
const TYPED: &str = "ZZ-TYPED-";
const PAGE: &str = "export default function Page() { return <p>ZZ-TYPED-PAGE</p>; }";

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

async fn raw(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("host", "api.test.edu");
    if !token.is_empty() {
        b = b.header("authorization", format!("Bearer {token}"));
    }
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The whole MCP result of a tool call.
async fn result(app: &Router, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    raw(app, "POST", "/api/mcp", token, Some(payload)).await.1["result"].clone()
}

/// A tool call. `Err` holds the code and the sentence.
async fn tool(app: &Router, token: &str, name: &str, args: Value) -> Result<Value, (String, String)> {
    let r = result(app, token, name, args).await;
    if r["isError"] == true {
        return Err((r["structuredContent"]["code"].as_str().unwrap_or("").to_string(), r["content"][0]["text"].as_str().unwrap_or("").to_string()));
    }
    Ok(r["structuredContent"].clone())
}

struct World {
    db: TestDb,
    app: Router,
    ws: String,
    slug: String,
    record: String,
    view: String,
    kanban: String,
    snapshot: String,
    policy: String,
    job: String,
    proposal: String,
}

impl World {
    async fn rest(&self, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        raw(&self.app, method, uri, &issue_test_token_and_user(who), body).await
    }

    fn count(&self, statement: &'static str) -> i64 {
        self.db.run(move |c| c.query_one(statement, &[]).unwrap().get(0))
    }

    /// A person's token as an AI tool session at `access`, with the task it names.
    async fn agent(&self, user: &str, access: &str, task: &str) -> (String, String) {
        let admin = issue_test_token_and_user(ADMIN);
        let (s, made) = raw(&self.app, "POST", "/api/v1/clients", &admin, Some(json!({ "name": "Desk assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [CALLBACK] }))).await;
        assert_eq!(s, StatusCode::CREATED, "{made}");
        let id = made["client_id"].as_str().unwrap().to_string();
        let (s, d) = raw(&self.app, "POST", &format!("/api/v1/clients/{id}/decide"), &admin, Some(json!({ "decision": "approve", "rationale": "An allowed tool" }))).await;
        assert_eq!(s, StatusCode::OK, "{d}");
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
        let body = json!({ "client_id": id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge, "code_challenge_method": "S256", "resource": ISSUER, "decision": "allow", "access": access, "toolsets": ["data", "build", "access"] });
        let (s, allowed) = raw(&self.app, "POST", "/oauth/authorize", &issue_test_token_and_user(user), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{allowed}");
        let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
        let form = format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={id}&code_verifier={VERIFIER}");
        let req = Request::builder().method("POST").uri("/oauth/token").header("host", "api.test.edu").header("content-type", "application/x-www-form-urlencoded").body(Body::from(form)).unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let reply: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let token = reply["access_token"].as_str().unwrap_or_else(|| panic!("{reply}")).to_string();
        tool(&self.app, &token, "begin_task", json!({ "task": task })).await.unwrap();
        (token, id)
    }
}

async fn world() -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    for who in [ADMIN, SECOND, TARGET, FACULTY] {
        issue_test_token_and_user(who);
    }
    let admin = issue_test_token_and_user(ADMIN);
    let second = issue_test_token_and_user(SECOND);
    let code = format!("D{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (s, unit) = raw(&app, "POST", "/api/v1/orgs", &admin, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let (s, b) = raw(&app, "POST", &format!("/api/v1/orgs/{ROOT}/appointments"), &admin, Some(json!({ "eppn": SECOND, "scoped_affiliation": "platform_admin", "reason": "A second administrator" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // A workspace whose name, description and collaborator were typed by a person.
    let wcode = format!("W{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (s, ws) = raw(&app, "POST", "/api/v1/workspaces", &admin, Some(json!({ "name": format!("{TYPED}WS-NAME {wcode}"), "code": wcode, "visibility": "restricted", "department": "physics", "organization_id": unit, "description": format!("{TYPED}WS-DESC") }))).await;
    assert_eq!(s, StatusCode::CREATED, "{ws}");
    let ws = ws["id"].as_str().unwrap().to_string();
    let (s, b) = raw(&app, "POST", &format!("/api/v1/workspaces/{ws}/collaborators"), &admin, Some(json!({ "eppn": FACULTY, "role": "viewer", "name": format!("{TYPED}COLLABORATOR") }))).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    // An app whose title, description, labels and option labels were typed by a person.
    let slug = format!("p6-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let manifest = json!({ "slug": slug, "title": format!("{TYPED}APP-TITLE"), "description": format!("{TYPED}APP-DESC"), "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [{ "id": "t", "name": format!("{TYPED}TABLE"), "slug": "t", "fields": [
        { "name": "title", "label": format!("{TYPED}FIELD-LABEL"), "field_type": "Text", "required": false, "ferpa_sensitive": false },
        { "name": "notes", "label": "Notes", "field_type": "LongText", "required": false, "ferpa_sensitive": false },
        { "name": "topic", "label": "Topic", "field_type": "Text", "required": false, "ferpa_sensitive": false, "indexed": true },
        { "name": "stage", "label": "Stage", "field_type": "Select", "required": false, "ferpa_sensitive": false, "select_options": [{ "id": "open", "label": format!("{TYPED}OPTION"), "color": "neutral", "order": 0 }] },
    ] }], "__PAGES__": 0 });
    let mut manifest = manifest;
    manifest.as_object_mut().unwrap().remove("__PAGES__");
    manifest["pages"] = json!([{ "id": "board", "title": "Board", "kind": { "Custom": { "source_sha256": sha(PAGE), "grants": [{ "table_id": "t", "read": ["title"], "write": [], "create": false, "delete": false }] } } }]);
    let p = tool(&app, &admin, "propose_app_change", json!({ "workspace_id": ws, "manifest": manifest, "summary": format!("{TYPED}SUMMARY-1"), "page_sources": [PAGE] })).await.unwrap();
    tool(&app, &second, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": format!("{TYPED}RATIONALE"), "digest": p["digest"] })).await.unwrap();
    let rec = tool(&app, &admin, "create_record", json!({ "app_slug": slug, "data": { "_table_id": "t", "title": format!("{TYPED}RECORD-TITLE"), "notes": format!("{TYPED}RECORD-NOTES"), "topic": format!("{TYPED}TOPIC"), "stage": "open" } })).await.unwrap();
    let record = rec["id"].as_str().unwrap().to_string();
    let v = tool(&app, &admin, "save_view", json!({ "app_slug": slug, "name": format!("{TYPED}VIEW"), "kind": "grid", "table_id": "t" })).await.unwrap();
    let view = v["id"].as_str().or(v["view"]["id"].as_str()).unwrap_or_default().to_string();
    let kanban = tool(&app, &admin, "save_view", json!({ "app_slug": slug, "name": "Board", "kind": "kanban", "table_id": "t", "config": { "kanban_column_field": "stage" } })).await.unwrap();
    let kanban = kanban["id"].as_str().unwrap_or_default().to_string();
    let exported = tool(&app, &admin, "export_records", json!({ "app_slug": slug, "table_id": "t" })).await.unwrap();
    let job = exported["job_id"].as_str().or(exported["id"].as_str()).unwrap_or_default().to_string();
    // A second, pending proposal.
    let pending = tool(&app, &admin, "propose_app_change", json!({ "workspace_id": ws, "summary": format!("{TYPED}SUMMARY-2"), "manifest": { "slug": format!("{slug}b"), "title": format!("{TYPED}APP2"), "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [] } })).await.unwrap();
    let proposal = pending["proposal_id"].as_str().unwrap_or_default().to_string();
    // A policy with a source snapshot.
    let (s, p) = raw(&app, "POST", "/api/v1/admin/library/policies", &admin, Some(json!({ "key": "records-retention", "title": format!("{TYPED}POLICY"), "organization_id": ROOT, "owner": { "Person": { "eppn": ADMIN } }, "reason": "The registrar owns it" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let policy = p["policy"]["id"].as_str().unwrap().to_string();
    let (_, src) = raw(&app, "POST", &format!("/api/v1/admin/library/policies/{policy}/sources"), &admin, Some(json!({ "title": "Schedule", "kind": "text", "reason": "r" }))).await;
    let source = src["source"]["id"].as_str().unwrap().to_string();
    let (s, snap) = raw(&app, "POST", &format!("/api/v1/admin/library/sources/{source}/snapshots"), &admin, Some(json!({ "text": format!("Student records are kept for seven years. {TYPED}SNAPSHOT"), "reason": "r" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{snap}");
    let snapshot = snap["snapshot"]["id"].as_str().unwrap().to_string();
    World { db, app, ws, slug, record, view, kanban, snapshot, policy, job, proposal }
}


/// One call in the table, and whether the seeded world gives its result text a person typed.
struct Case {
    tool: &'static str,
    args: Value,
    carries: bool,
    /// Who calls. Empty is the administrator with a plain token. Otherwise the person, with a plain token.
    who: &'static str,
    /// Called with an AI tool session of the administrator.
    agent: bool,
    /// The tool may refuse in this world. What it says is still scanned.
    may_refuse: bool,
}

fn case(tool: &'static str, args: Value, carries: bool) -> Case {
    Case { tool, args, carries, who: "", agent: false, may_refuse: false }
}

/// One case for every tool, in an order that leaves the world usable for the cases after it.
async fn cases(w: &World) -> Vec<Case> {
    let admin = issue_test_token_and_user(ADMIN);
    let (ws, slug) = (w.ws.clone(), w.slug.clone());
    let made = |t: &str| json!({ "app_slug": slug, "data": { "_table_id": "t", "title": format!("{TYPED}{t}") } });
    let spare = tool(&w.app, &admin, "create_record", made("SPARE")).await.unwrap()["id"].as_str().unwrap().to_string();
    let gone = tool(&w.app, &admin, "create_record", made("GONE")).await.unwrap()["id"].as_str().unwrap().to_string();
    let grant = json!({ "holder_kind": "person", "holder_id": TARGET, "role": "viewer", "anchor_kind": "container", "anchor_id": format!("app:{slug}"), "source": "task", "reason": "Needed" });
    let rule = json!({ "key": "typed-rule", "statement": format!("{TYPED}STATEMENT"), "applies_to": format!("{TYPED}APPLIES"), "citations": [{ "snapshot_id": w.snapshot, "passage": "Student records are kept for seven years." }] });
    vec![
        case("list_workspaces", json!({}), true),
        case("get_workspace", json!({ "workspace_id": ws }), true),
        case("list_apps", json!({ "workspace_id": ws }), true),
        case("describe_app", json!({ "app_slug": slug }), true),
        case("list_records", json!({ "app_slug": slug, "table_id": "t" }), true),
        case("get_record", json!({ "app_slug": slug, "record_id": w.record }), true),
        case("get_record", json!({ "app_slug": slug, "record_id": w.record, "history": true }), true),
        case("get_view_data", json!({ "app_slug": slug, "view_id": w.kanban }), true),
        // The index of a new field is built by a job, so the call is refused here. The group paths are held by a test below.
        Case { may_refuse: true, ..case("aggregate_records", json!({ "app_slug": slug, "table_id": "t", "function": "count", "group_by": "topic" }), false) },
        case("list_views", json!({ "app_slug": slug }), true),
        case("list_proposals", json!({ "workspace_id": ws }), true),
        case("get_proposal", json!({ "proposal_id": w.proposal }), true),
        case("get_job", json!({ "job_id": w.job }), false),
        case("list_datasets", json!({}), true),
        case("governance_report", json!({ "report": "ledger" }), false),
        case("governance_report", json!({ "report": "oscal" }), false),
        case("governance_report", json!({ "report": "framework" }), false),
        case("simulate_cedar_policy", json!({ "eppn": FACULTY, "action": "read" }), false),
        case("explain_access", json!({ "object": format!("app:{slug}") }), true),
        case("list_my_work", json!({}), false),
        case("access_suggestions", json!({}), false),
        case("list_library_policies", json!({}), true),
        case("read_library_snapshot", json!({ "snapshot_id": w.snapshot }), true),
        case("find_library_rules", json!({}), false),
        case("list_library_questions", json!({}), false),
        case("admin_inventory", json!({ "kind": "workspaces" }), true),
        case("admin_inventory", json!({ "kind": "apps" }), true),
        case("admin_inventory", json!({ "kind": "ledger" }), true),
        case("get_page", json!({ "app_slug": slug }), false),
        case("get_page", json!({ "app_slug": slug, "page_id": "board" }), true),
        Case { may_refuse: true, ..case("page_call", json!({ "app_slug": slug, "page_id": "board", "op": "list", "table_id": "t" }), true) },
        case("get_agent_kit", json!({}), false),
        case("create_record", made("CREATED"), true),
        case("update_record", json!({ "app_slug": slug, "record_id": spare, "version": 1, "data": { "title": format!("{TYPED}UPDATED") } }), true),
        case("delete_record", json!({ "app_slug": slug, "record_id": gone, "version": 1 }), false),
        case("save_view", json!({ "app_slug": slug, "name": format!("{TYPED}VIEW-2"), "kind": "grid", "table_id": "t" }), true),
        case("delete_view", json!({ "app_slug": slug, "view_id": w.view }), false),
        case("export_records", json!({ "app_slug": slug, "table_id": "t" }), false),
        case("record_governance_decision", json!({ "organization_code": "PHYS", "decision_type": "PolicyRevision", "oscal_control_id": "AC-03", "rationale": format!("{TYPED}LEDGER") }), false),
        case("update_workspace", json!({ "workspace_id": ws, "description": format!("{TYPED}WS-DESC-2") }), true),
        case("manage_workspace_member", json!({ "workspace_id": ws, "action": "add", "eppn": TARGET, "role": "viewer", "name": format!("{TYPED}NEW-PERSON") }), false),
        case("draft_access_change", json!({ "kind": "grant", "change": grant, "reason": format!("{TYPED}DRAFT") }), true),
        Case { who: FACULTY, ..case("request_access", json!({ "object_kind": "app", "object_id": slug, "role": "editor", "reason": format!("{TYPED}REASON"), "days": 7 }), true) },
        case("propose_library_rules", json!({ "policy_id": w.policy, "rules": [rule], "questions": [{ "question": format!("{TYPED}QUESTION"), "context_passage": "Student records are kept for seven years.", "snapshot_id": w.snapshot }] }), false),
        Case { may_refuse: true, ..case("complete_task", json!({ "work_id": Uuid::new_v4().to_string(), "decision": "decline", "reason": format!("{TYPED}DECLINE") }), false) },
        Case { may_refuse: true, ..case("decide_process", json!({ "app_slug": slug, "instance_id": Uuid::new_v4().to_string(), "decision": "approve", "comment": format!("{TYPED}COMMENT") }), false) },
        Case { agent: true, ..case("begin_task", json!({ "task": format!("{TYPED}TASK") }), true) },
        case("preview_proposal", json!({ "proposal_id": w.proposal }), true),
        case("propose_app_change", json!({ "workspace_id": ws, "summary": format!("{TYPED}SUMMARY-3"), "manifest": { "slug": format!("{slug}c"), "title": format!("{TYPED}APP3"), "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [] } }), true),
        case("decide_proposal", json!({ "proposal_id": w.proposal, "decision": "reject", "rationale": format!("{TYPED}REJECTED") }), true),
        case("cancel_job", json!({ "job_id": w.job }), false),
    ]
}

/// Every path to a string that holds the planted text, and whether `untrusted_text` is on the way to it.
fn find_typed(v: &Value, path: &str, marked: bool, out: &mut Vec<(String, bool)>) {
    match v {
        Value::String(s) if s.contains(TYPED) => out.push((path.to_string(), marked)),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| find_typed(x, &format!("{path}[{i}]"), marked, out)),
        Value::Object(m) => m.iter().for_each(|(k, x)| find_typed(x, &format!("{path}.{k}"), marked || k == "untrusted_text", out)),
        _ => {}
    }
}

// 1. A new tool fails this until it has a case. A case fails until the text it returns is under `untrusted_text`.
#[tokio::test]
async fn every_tool_puts_the_text_a_person_typed_under_untrusted_text() {
    let w = world().await;
    let list = cases(&w).await;
    let named: std::collections::BTreeSet<&str> = list.iter().map(|c| c.tool).collect();
    for t in TOOLS {
        assert!(named.contains(t.name), "the tool '{}' has no case. Add one that seeds typed text its result would carry, with carries set", t.name);
    }
    let agent = w.agent(ADMIN, "propose", "Read the world").await.0;
    let mut leaks = Vec::new();
    let mut misses = Vec::new();
    for c in &list {
        let token = if c.agent { agent.clone() } else { issue_test_token_and_user(if c.who.is_empty() { ADMIN } else { c.who }) };
        let r = result(&w.app, &token, c.tool, c.args.clone()).await;
        if r["isError"] == true && !c.may_refuse {
            panic!("{} was refused: {r}", c.tool);
        }
        // The text of the result and its structured content both count.
        let mut seen = Vec::new();
        find_typed(&r["structuredContent"], "structuredContent", false, &mut seen);
        let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap_or("null")).unwrap_or(Value::Null);
        find_typed(&text, "content", false, &mut seen);
        for (path, marked) in &seen {
            if !marked {
                leaks.push(format!("{} {path}", c.tool));
            }
        }
        if c.carries && !seen.iter().any(|(_, m)| *m) {
            misses.push(format!("{} {}", c.tool, c.args));
        }
    }
    assert!(misses.is_empty(), "these carry typed text in this world, and none was found under untrusted_text:\n{}", misses.join("\n"));
    assert!(leaks.is_empty(), "typed text outside untrusted_text:\n{}", leaks.join("\n"));
}

// 1b. Marking loses nothing: put back, the result has the text where it was.
#[tokio::test]
async fn marking_moves_text_and_loses_none() {
    let w = world().await;
    let admin = issue_test_token_and_user(ADMIN);
    let r = tool(&w.app, &admin, "get_record", json!({ "app_slug": w.slug, "record_id": w.record })).await.unwrap();
    assert_eq!(r["data"]["title"], Value::Null, "{r}");
    assert_eq!(r["untrusted_text"]["data"]["title"], format!("{TYPED}RECORD-TITLE"), "{r}");
    assert_eq!(r["data"]["stage"], "open", "a value that is not typed text stays: {r}");
    let mut back = r.clone();
    scaffoldry_server::service::untrusted::restore(&mut back);
    assert_eq!(back["data"]["title"], format!("{TYPED}RECORD-TITLE"));
    assert!(back.get("untrusted_text").is_none());
    // The server's own words are not marked.
    let g = tool(&w.app, &admin, "list_records", json!({ "app_slug": w.slug, "table_id": "t" })).await.unwrap();
    assert!(g["guidance"]["next"].is_string(), "{g}");
    // The text of a source page keeps the shape it had: the member is the text.
    let page = tool(&w.app, &admin, "read_library_snapshot", json!({ "snapshot_id": w.snapshot })).await.unwrap();
    assert!(page["untrusted_text"].is_string(), "{page}");
}

#[tokio::test]
async fn instructions_name_the_field() {
    let w = world().await;
    let admin = issue_test_token_and_user(ADMIN);
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } });
    let text = raw(&w.app, "POST", "/api/mcp", &admin, Some(payload)).await.1["result"]["instructions"].as_str().unwrap().to_string();
    assert!(text.contains("untrusted_text"), "{text}");
    assert!(text.len() <= 1800, "{} bytes", text.len());
}

// 2a. A new tool that writes fails this until it is sorted: it drafts, or it is not about access, ownership or labels.
#[test]
fn every_writing_tool_is_sorted() {
    const DRAFTS: &[&str] = &["update_workspace", "manage_workspace_member", "draft_access_change", "propose_app_change", "request_access", "propose_library_rules"];
    // Writes of records, views, jobs, tasks and the ledger by the person's own rights. None changes access, ownership or labels.
    const OTHER: &[&str] = &[
        "create_record", "update_record", "delete_record", "save_view", "delete_view", "export_records", "cancel_job", "begin_task", "complete_task",
        "decide_process", "page_call", "preview_proposal", "decide_proposal", "record_governance_decision",
    ];
    for t in TOOLS.iter().filter(|t| !t.read_only) {
        assert!(DRAFTS.contains(&t.name) || OTHER.contains(&t.name), "the writing tool '{}' is not sorted. Does it change access, ownership or labels? Then it drafts, and goes in DRAFTS", t.name);
    }
}

/// What a workspace and its access look like now.
async fn picture(w: &World) -> Value {
    let (_, ws) = w.rest("GET", &format!("/api/v1/workspaces/{}", w.ws), ADMIN, None).await;
    json!({ "ws": ws, "grants": w.count("SELECT COUNT(*) FROM access_grants"), "collaborators": w.count("SELECT COUNT(*) FROM workspace_collaborators") })
}

// 2b. Each tool that would change access, ownership or labels drafts it, links the desk, and changes nothing.
#[tokio::test]
async fn a_tool_that_would_change_access_drafts_it_and_changes_nothing() {
    let w = world().await;
    let admin = issue_test_token_and_user(ADMIN);
    let agent = w.agent(ADMIN, "propose", "Tidy access").await.0;
    let ws = w.ws.clone();
    let grant = json!({ "holder_kind": "person", "holder_id": TARGET, "role": "viewer", "anchor_kind": "container", "anchor_id": format!("app:{}", w.slug), "source": "task", "reason": "Needed" });
    let calls: Vec<(&str, Value, &str)> = vec![
        ("update_workspace", json!({ "workspace_id": ws, "visibility": "institutional" }), "workspace_settings"),
        ("update_workspace", json!({ "workspace_id": ws, "data_classification": "Public" }), "workspace_settings"),
        ("update_workspace", json!({ "workspace_id": ws, "department": "chemistry" }), "workspace_settings"),
        ("manage_workspace_member", json!({ "workspace_id": ws, "action": "add", "eppn": TARGET, "role": "editor" }), "workspace_member"),
        ("manage_workspace_member", json!({ "workspace_id": ws, "action": "update", "eppn": FACULTY, "role": "editor" }), "workspace_member"),
        ("manage_workspace_member", json!({ "workspace_id": ws, "action": "remove", "eppn": FACULTY }), "workspace_member"),
    ];
    for (name, args, kind) in calls {
        let before = picture(&w).await;
        let drafts = w.count("SELECT COUNT(*) FROM access_changes");
        let r = tool(&w.app, &admin, name, args.clone()).await.unwrap_or_else(|e| panic!("{name} {args}: {e:?}"));
        assert_eq!(r["status"], "Pending", "{r}");
        assert_eq!(r["kind"], kind, "{r}");
        assert!(r["desk_link"].as_str().is_some_and(|l| l.starts_with('/') && l.contains(r["change_id"].as_str().unwrap())), "{r}");
        assert_eq!(picture(&w).await, before, "{name} {args} changed state");
        assert_eq!(w.count("SELECT COUNT(*) FROM access_changes"), drafts + 1);
    }
    // An agent's grant draft.
    let before = picture(&w).await;
    let d = tool(&w.app, &agent, "draft_access_change", json!({ "kind": "grant", "change": grant, "reason": "Needed" })).await.unwrap();
    assert!(d["desk_link"].as_str().is_some_and(|l| l.starts_with("/admin/access?change=")), "{d}");
    assert_eq!(picture(&w).await, before);
    // A proposal links the desk too, and the app is as it was.
    let version = tool(&w.app, &admin, "describe_app", json!({ "app_slug": w.slug })).await.unwrap()["version"].clone();
    let p = tool(&w.app, &admin, "propose_app_change", json!({ "workspace_id": ws, "summary": "Again", "manifest": { "slug": format!("{}z", w.slug), "title": "Z", "description": "d", "organization_code": "PHYS", "department": "physics", "workspace_id": ws, "views": [], "pages": [], "tables": [] } })).await.unwrap();
    assert!(p["desk_link"].as_str().is_some_and(|l| l.starts_with("/proposals?id=")), "{p}");
    assert_eq!(tool(&w.app, &admin, "describe_app", json!({ "app_slug": w.slug })).await.unwrap()["version"], version);
    // A change to the title alone is not access. It is applied.
    let r = tool(&w.app, &admin, "update_workspace", json!({ "workspace_id": ws, "icon": "X" })).await.unwrap();
    assert!(r.get("change_id").is_none(), "{r}");
}

// 2c. A confirmed workspace draft is applied, by the person who confirms it and not by the person it was drafted for.
#[tokio::test]
async fn a_person_confirms_a_workspace_draft_in_the_desk() {
    let w = world().await;
    let admin = issue_test_token_and_user(ADMIN);
    let d = tool(&w.app, &admin, "update_workspace", json!({ "workspace_id": w.ws, "visibility": "institutional" })).await.unwrap();
    let id = d["change_id"].as_str().unwrap();
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), ADMIN, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the person whose tool drafted a widening change cannot confirm it: {b}");
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(picture(&w).await["ws"]["visibility"], "institutional");
    // A member draft: add TARGET.
    let d = tool(&w.app, &admin, "manage_workspace_member", json!({ "workspace_id": w.ws, "action": "add", "eppn": TARGET, "role": "viewer" })).await.unwrap();
    let id = d["change_id"].as_str().unwrap();
    let before = w.count("SELECT COUNT(*) FROM workspace_collaborators");
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.count("SELECT COUNT(*) FROM workspace_collaborators"), before + 1);
}

// 3. The ledger names the agent as the proposer and the confirming person as the decider.
#[tokio::test]
async fn the_ledger_names_the_agent_as_proposer_and_the_person_as_decider() {
    let w = world().await;
    let (agent, client) = w.agent(ADMIN, "propose", "Give the audit access").await;
    let grant = json!({ "holder_kind": "person", "holder_id": TARGET, "role": "viewer", "anchor_kind": "container", "anchor_id": format!("app:{}", w.slug), "source": "task", "reason": "The audit needs it" });
    let d = tool(&w.app, &agent, "draft_access_change", json!({ "kind": "grant", "change": grant, "reason": "The audit needs it" })).await.unwrap();
    let id = d["change_id"].as_str().unwrap().to_string();
    // The person whose tool drafted it cannot confirm it, in the desk.
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), ADMIN, Some(json!({ "decision": "confirm", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(w.count("SELECT COUNT(*) FROM access_grants WHERE holder_id = 'target.person@state.edu'"), 0);
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (principal, payload): (String, Value) = w.db.run(|c| {
        let r = c.query_one("SELECT principal, payload FROM governance_ledger WHERE payload->>'action' = 'grant_confirmed' ORDER BY sequence DESC LIMIT 1", &[]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(principal, SECOND, "the person who confirmed is the decider");
    assert_eq!(payload["proposer"]["via"], "agent", "{payload}");
    assert_eq!(payload["proposer"]["client"], client.as_str(), "{payload}");
    assert_eq!(payload["proposer"]["tool"], "draft_access_change", "{payload}");
    assert_eq!(payload["proposer"]["task"], "Give the audit access", "{payload}");
    assert_eq!(payload["proposer"]["for"], ADMIN, "{payload}");
    // A workspace draft is named the same way.
    let admin = issue_test_token_and_user(ADMIN);
    let d = tool(&w.app, &admin, "manage_workspace_member", json!({ "workspace_id": w.ws, "action": "add", "eppn": TARGET, "role": "viewer" })).await.unwrap();
    let id = d["change_id"].as_str().unwrap();
    let (s, b) = w.rest("POST", &format!("/api/v1/admin/access/changes/{id}/decide"), SECOND, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (principal, payload): (String, Value) = w.db.run(|c| {
        let r = c.query_one("SELECT principal, payload FROM governance_ledger WHERE payload->>'action' = 'workspace_change_confirmed' ORDER BY sequence DESC LIMIT 1", &[]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!(principal, SECOND);
    assert_eq!(payload["proposer"]["tool"], "manage_workspace_member", "{payload}");
}

// 4. A request filed by a tool waits for the person to confirm it.
#[tokio::test]
async fn a_request_filed_by_a_tool_waits_for_the_person() {
    let w = world().await;
    let (agent, _) = w.agent(FACULTY, "read", "Ask for the audit app").await;
    let grants = w.count("SELECT COUNT(*) FROM access_grants");
    let r = tool(&w.app, &agent, "request_access", json!({ "object_kind": "app", "object_id": w.slug, "role": "editor", "reason": "I need to edit", "days": 7 })).await.unwrap();
    assert_eq!(r["status"], "Waiting for person", "{r}");
    assert!(r["desk_link"].as_str().is_some_and(|l| l.starts_with("/me/access/requests")), "{r}");
    let id = r["id"].as_str().unwrap().to_string();
    assert_eq!(w.count("SELECT COUNT(*) FROM access_grants"), grants);
    // The tool cannot confirm it. The person's own session can.
    let (s, b) = raw(&w.app, "POST", &format!("/api/v1/access/requests/{id}/confirm"), &agent, None).await;
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED, "{s} {b}");
    let (s, b) = w.rest("POST", &format!("/api/v1/access/requests/{id}/confirm"), FACULTY, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_ne!(b["status"], "Waiting for person", "{b}");
}

// The groups of an aggregate are values of a field a person typed. The index is built by a job, so this holds the paths directly.
#[tokio::test]
async fn the_groups_of_an_aggregate_are_marked() {
    let w = world().await;
    let mut r = json!({ "function": "count", "group_by": "topic", "groups": [{ "key": format!("{TYPED}GROUP"), "group": format!("{TYPED}GROUP"), "value": 1.0 }] });
    scaffoldry_server::service::untrusted::mark("aggregate_records", &json!({ "app_slug": w.slug }), &w.db.state(), &mut r);
    let mut seen = Vec::new();
    find_typed(&r, "", false, &mut seen);
    assert!(!seen.is_empty() && seen.iter().all(|(_, m)| *m), "{r}");
}
