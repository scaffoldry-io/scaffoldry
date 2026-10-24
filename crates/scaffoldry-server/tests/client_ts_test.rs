//! External apps phase 4: a TypeScript client generated for each app.
//!
//! `GET /api/v1/apps/{slug}/client.ts` returns one module built from the app's API description for the caller. With a
//! client token it holds only what the ceiling allows. The module compiles under `tsc --strict`, uses `fetch` and
//! nothing else, and runs against the real server.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::CollaboratorRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const WORKSPACE: &str = "ws-physics-optics";
const OWNER: &str = "einstein@physics.state.edu";
const HOST: &str = "api.test.edu";
const CALLBACK: &str = "https://tracker.example.edu/callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
    second: String,
    editor: String,
    slug: String,
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, content_type: Option<&str>, body: Body) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut b = Request::builder().method(method).uri(uri).header("host", HOST).header("authorization", format!("Bearer {token}"));
    if let Some(ct) = content_type {
        b = b.header("content-type", ct);
    }
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (status, headers, resp.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (status, _, bytes) = match body {
        Some(v) => send(app, method, uri, token, Some("application/json"), Body::from(v.to_string())).await,
        None => send(app, method, uri, token, None, Body::empty()).await,
    };
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn tool(w: &World, token: &str, name: &str, args: Value) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let (_, v) = rest(&w.app, "POST", "/api/mcp", token, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{name}: {v}");
    v["result"]["structuredContent"].clone()
}

fn field(name: &str, kind: &str) -> Value {
    json!({ "name": name, "label": name, "field_type": kind, "required": false, "ferpa_sensitive": false })
}

fn member(state: &scaffoldry_server::state::ServerState, eppn: &str, role: &str) {
    state.collaborators.write().unwrap().entry(WORKSPACE.to_string()).or_default().push(CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: WORKSPACE.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    });
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let (second, editor) = (format!("second-{tag}@state.edu"), format!("editor-{tag}@state.edu"));
    member(&state, &second, "admin");
    member(&state, &editor, "editor");
    let app = build_app_with_state(state).expect("router");
    let w = World { _db: db, app, owner: issue_test_token_and_user(OWNER), second: issue_test_token_and_user(&second), editor: issue_test_token_and_user(&editor), slug: format!("ts-{tag}") };
    let mut title = field("title", "Text");
    title["required"] = json!(true);
    let mut status = field("status", "Select");
    status["select_options"] = json!([{ "id": "open", "label": "Open" }, { "id": "closed", "label": "Closed" }]);
    let mut tags = field("tags", "MultiSelect");
    tags["select_options"] = json!([{ "id": "a", "label": "Alpha" }, { "id": "b", "label": "Beta" }]);
    let manifest = json!({
        "slug": w.slug, "title": "Lab equipment", "description": "d", "organization_code": "PHYS", "department": "physics",
        "workspace_id": WORKSPACE, "views": [], "pages": [],
        "tables": [
            { "id": "equipment", "name": "Equipment", "slug": "equipment", "fields": [title, status, tags, field("cost", "Number"), field("bought", "Date"), field("working", "Boolean"), field("notes", "Text")] },
            { "id": "loans", "name": "Loans", "slug": "loans", "fields": [field("who", "Text")] }
        ]
    });
    let p = tool(&w, &w.owner, "propose_app_change", json!({ "workspace_id": WORKSPACE, "manifest": manifest, "summary": "An app" })).await;
    tool(&w, &w.owner, "decide_proposal", json!({ "proposal_id": p["proposal_id"], "decision": "approve", "rationale": "Set up", "digest": p["digest"] })).await;
    w
}

/// An approved client. `reads` and `writes` are fields of `equipment`.
async fn client(w: &World, reads: &[&str], writes: &[&str], create: bool) -> String {
    let body = json!({
        "name": "Lab tracker", "operator": "Physics Department", "kind": "public", "redirect_uris": [CALLBACK],
        "ceiling": { "apps": [{ "app_slug": w.slug, "tables": [{ "table_id": "equipment", "read": reads, "write": writes, "create": create, "delete": false }] }] }
    });
    let (status, made) = rest(&w.app, "POST", "/api/v1/clients", &w.owner, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let id = made["client_id"].as_str().unwrap().to_string();
    let (status, decided) = rest(&w.app, "POST", &format!("/api/v1/clients/{id}/decide"), &w.second, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    id
}

async fn sign_in(w: &World, user: &str, client_id: &str) -> String {
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let body = json!({
        "client_id": client_id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge,
        "code_challenge_method": "S256", "resource": "https://api.test.edu", "decision": "allow"
    });
    let (status, allowed) = rest(&w.app, "POST", "/oauth/authorize", user, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{allowed}");
    let code = allowed["redirect_to"].as_str().unwrap().split_once('?').unwrap().1.split('&').find_map(|p| p.strip_prefix("code=")).unwrap().to_string();
    let form = format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={client_id}&code_verifier={VERIFIER}");
    let (status, _, bytes) = send(&w.app, "POST", "/oauth/token", "", Some("application/x-www-form-urlencoded"), Body::from(form)).await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_slice::<Value>(&bytes).unwrap()["access_token"].as_str().unwrap().to_string()
}

async fn module(w: &World, token: &str) -> String {
    let (status, headers, bytes) = send(&w.app, "GET", &format!("/api/v1/apps/{}/client.ts", w.slug), token, None, Body::empty()).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
    assert!(headers["content-type"].to_str().unwrap().starts_with("text/typescript"));
    String::from_utf8(bytes).unwrap()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Type-checks `files` under `tsc --strict`. The compiler comes from `npm ci`. CI installs it; a missing compiler fails
/// the test there, and says how to get it anywhere else.
fn tsc(dir: &Path, files: &[&str]) -> Result<(), String> {
    let bin = repo_root().join("node_modules/.bin/tsc");
    assert!(bin.exists(), "tsc is missing: run `npm ci` at the repository root");
    let out = std::process::Command::new(bin)
        .current_dir(dir)
        .args(["--strict", "--noEmit", "--target", "es2022", "--lib", "es2022,dom", "--module", "es2022", "--moduleResolution", "bundler", "--skipLibCheck", "false"])
        .args(files)
        .output()
        .expect("run tsc");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// 1. The module compiles with tsc --strict, and its types match the manifest.
#[tokio::test(flavor = "multi_thread")]
async fn the_module_compiles_under_strict_and_its_types_match_the_manifest() {
    let w = world().await;
    let source = module(&w, &w.owner).await;
    for expected in [
        "\"title\": string;",
        "\"status\"?: \"open\" | \"closed\";",
        "\"tags\"?: Array<\"a\" | \"b\">;",
        "\"cost\"?: number;",
        "\"working\"?: boolean;",
        "export function createClient(",
    ] {
        assert!(source.contains(expected), "the module has `{expected}`:\n{source}");
    }
    assert!(!source.contains("require(") && !source.contains("import "), "no dependency");
    let dir = temp_dir("client-ts");
    std::fs::write(dir.join("client.ts"), &source).unwrap();
    std::fs::write(
        dir.join("use.ts"),
        r#"import { createClient, ScaffoldryError } from "./client";
const c = createClient({ baseUrl: "https://s.example.edu", token: () => "t" });
export async function use(): Promise<void> {
  const page = await c.tables["equipment"].list({ limit: 5 });
  const title: string | undefined = page.records[0]?.data["title"];
  const cost: number | undefined = page.records[0]?.data["cost"];
  await c.tables["equipment"].create({ title: "Oscilloscope", cost: 12, status: "open", tags: ["a"] });
  // @ts-expect-error cost is a number
  await c.tables["equipment"].create({ title: "x", cost: "twelve" });
  // @ts-expect-error title is required
  await c.tables["equipment"].create({});
  // @ts-expect-error status is one of the options
  await c.tables["equipment"].create({ title: "x", status: "lost" });
  for await (const row of c.tables["equipment"].all({ limit: 2 })) console.log(row.id, row.version);
  await c.tables["loans"].remove("id", 1);
  try { await c.tables["equipment"].get("missing"); } catch (e) { if (e instanceof ScaffoldryError) console.log(e.code, e.status); }
  console.log(title, cost);
}
"#,
    )
    .unwrap();
    tsc(&dir, &["client.ts", "use.ts"]).unwrap_or_else(|e| panic!("tsc --strict failed:\n{e}"));
}

// 2. Called with a client token, the module has no method or field outside the ceiling.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_token_gets_a_module_with_nothing_outside_the_ceiling() {
    let w = world().await;
    let id = client(&w, &["title", "status"], &["status"], false).await;
    let token = sign_in(&w, &w.editor, &id).await;
    let source = module(&w, &token).await;
    assert!(source.contains("\"title\""), "{source}");
    assert!(source.contains("\"status\""), "{source}");
    for outside in ["\"notes\"", "\"cost\"", "\"tags\"", "loans", "create: createOf", "remove: removeOf"] {
        assert!(!source.contains(outside), "`{outside}` is outside the ceiling:\n{source}");
    }
    assert!(source.contains("update: updateOf"), "it may write status");

    let dir = temp_dir("client-ts-ceiling");
    std::fs::write(dir.join("client.ts"), &source).unwrap();
    std::fs::write(
        dir.join("use.ts"),
        r#"import { createClient } from "./client";
const c = createClient({ baseUrl: "https://s.example.edu", token: () => "t" });
export async function use(): Promise<void> {
  const page = await c.tables["equipment"].list();
  const status: "open" | "closed" | undefined = page.records[0]?.data["status"];
  await c.tables["equipment"].update("id", 1, { status: "closed" });
  // @ts-expect-error title is readable but the ceiling does not let the app change it
  await c.tables["equipment"].update("id", 1, { title: "x" });
  // @ts-expect-error notes is outside the ceiling
  const notes = page.records[0]?.data["notes"];
  // @ts-expect-error the ceiling does not allow create
  await c.tables["equipment"].create({});
  // @ts-expect-error the ceiling does not allow delete
  await c.tables["equipment"].remove("id", 1);
  // @ts-expect-error loans is outside the ceiling
  await c.tables["loans"].list();
  console.log(status, notes);
}
"#,
    )
    .unwrap();
    tsc(&dir, &["client.ts", "use.ts"]).unwrap_or_else(|e| panic!("tsc --strict failed:\n{e}"));

    // An app the ceiling does not name is refused.
    let (status, _, bytes) = send(&w.app, "GET", "/api/v1/apps/some-other-app/client.ts", &token, None, Body::empty()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{}", String::from_utf8_lossy(&bytes));
    assert!(String::from_utf8_lossy(&bytes).contains("client_scope"));
}

// 3. "Build me a page outside Scaffoldry that lists my lab's equipment." The client is registered and approved, a person
// signs in, and the generated module lists rows against a real server, run by node.
#[tokio::test(flavor = "multi_thread")]
async fn the_generated_module_lists_rows_against_the_real_server() {
    let w = world().await;
    for (title, status) in [("Oscilloscope", "open"), ("Laser", "closed"), ("Spectrometer", "open")] {
        let (code, row) = rest(&w.app, "POST", &format!("/api/v1/apps/{}/tables/equipment/records", w.slug), &w.owner, Some(json!({ "data": { "title": title, "status": status, "notes": "private" } }))).await;
        assert_eq!(code, StatusCode::CREATED, "{row}");
    }
    let id = client(&w, &["title", "status"], &[], false).await;
    let token = sign_in(&w, &w.editor, &id).await;
    let source = module(&w, &token).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = w.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = temp_dir("client-ts-run");
    std::fs::write(dir.join("client.ts"), &source).unwrap();
    std::fs::write(
        dir.join("page.ts"),
        r#"import { createClient, ScaffoldryError } from "./client.ts";
const c = createClient({ baseUrl: process.env.BASE_URL as string, token: () => process.env.TOKEN as string });
const titles: string[] = [];
for await (const row of c.tables["equipment"].all({ limit: 2 })) titles.push(String(row.data["title"]) + ":" + String(row.data["notes"]));
let refused = "";
try { await (c.tables["equipment"] as any).get("no-such-record"); } catch (e) { refused = e instanceof ScaffoldryError ? e.code : "other"; }
console.log(JSON.stringify({ titles, refused }));
"#,
    )
    .unwrap();
    let (base, token_value, run_dir) = (format!("http://{address}"), token.clone(), dir.clone());
    let out = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node").current_dir(run_dir).args(["--experimental-strip-types", "--no-warnings", "page.ts"]).env("BASE_URL", base).env("TOKEN", token_value).output().expect("run node")
    })
    .await
    .unwrap();
    assert!(out.status.success(), "node failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let result: Value = serde_json::from_slice(&out.stdout).expect("the page printed JSON");
    let mut titles: Vec<String> = result["titles"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect();
    titles.sort();
    assert_eq!(titles, ["Laser:undefined", "Oscilloscope:undefined", "Spectrometer:undefined"], "three rows over two pages, and no notes field: {result}");
    assert!(!result["refused"].as_str().unwrap().is_empty(), "an error carries its code: {result}");
}

// 4. The guide for builders is an MCP resource of at most 8 KB, and the agent kit carries it.
#[tokio::test(flavor = "multi_thread")]
async fn the_guide_for_outside_apps_is_a_small_resource_and_part_of_the_kit() {
    let w = world().await;
    let rpc = |method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let (_, listed) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(rpc("resources/list", json!({})))).await;
    assert!(listed["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == "scaffoldry://guide/outside-apps"), "{listed}");
    let (_, read) = rest(&w.app, "POST", "/api/mcp", &w.owner, Some(rpc("resources/read", json!({ "uri": "scaffoldry://guide/outside-apps" })))).await;
    let text = read["result"]["contents"][0]["text"].as_str().expect("the guide text");
    assert!(text.len() <= 8 * 1024, "the guide is {} bytes", text.len());
    for needed in ["PKCE", "client.ts", "S256", "sensitive"] {
        assert!(text.to_lowercase().contains(&needed.to_lowercase()), "the guide covers {needed}");
    }
    let kit = tool(&w, &w.owner, "get_agent_kit", json!({})).await;
    let files = kit["skill"]["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "references/outside-apps.md"), "{files:?}");
}
