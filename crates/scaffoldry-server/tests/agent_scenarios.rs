//! Agent coverage phase 1: a scripted set of real requests, run through the real MCP endpoint.
//!
//! A scenario is a recorded sequence of tool calls with expected results. It tests the server, not an
//! agent's judgment, and no language model is called. Each scenario starts with the faculty request in
//! plain words, so the files double as the product's examples.
//!
//! Scenarios live in `tests/agent_scenarios/*.json`. A run writes `target/agent-gaps.md`, the list of
//! scenarios marked unsupported with their asks and reasons. That file is generated and never committed.
//!
//! The runner talks to `/api/mcp` over the router with a persona's bearer token. It does not call
//! service functions. A test below reads this file's source to hold it to that.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};
use tower::ServiceExt;
use uuid::Uuid;

/// An Internal workspace the personas share.
const WORKSPACE: &str = "ws-physics-optics";

const PERSONAS: &[(&str, &str)] = &[
    ("faculty-owner", "einstein@physics.state.edu"),
    ("editor", "editor.test@physics.state.edu"),
    ("viewer", "viewer.test@physics.state.edu"),
    ("student", "student.test@physics.state.edu"),
    ("compliance", "compliance.officer@state.edu"),
    ("platform-admin", "jordan.lee@state.edu"),
];

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://scaffoldry:scaffoldry_dev_password@127.0.0.1:5433/scaffoldry".to_string())
}

fn db<R: Send + 'static>(f: impl FnOnce(&mut postgres::Client) -> R + Send + 'static) -> R {
    let url = db_url();
    std::thread::spawn(move || {
        let mut c = postgres::Client::connect(&url, postgres::NoTls).expect("database");
        f(&mut c)
    })
    .join()
    .expect("database thread")
}

// ---------------------------------------------------------------------------------------------
// The harness: a booted server and a token for each persona.
// ---------------------------------------------------------------------------------------------

pub struct Harness {
    app: Router,
    tokens: BTreeMap<String, String>,
}

impl Harness {
    async fn boot() -> Harness {
        // Personas first, so the server loads them at boot.
        let mut tokens = BTreeMap::new();
        for (name, eppn) in PERSONAS {
            tokens.insert((*name).to_string(), issue_test_token_and_user(eppn));
        }
        // A compliance officer's affiliation is compliance.
        db(|c| {
            c.execute(
                "UPDATE scim_users SET payload = jsonb_set(payload, '{roles}', '[{\"type\":\"eduPersonScopedAffiliation\",\"value\":\"compliance@state.edu\"}]'::jsonb) \
                 WHERE user_name = 'compliance.officer@state.edu'",
                &[],
            )
            .unwrap();
        });
        // A scoped agent token: an agent token minted for the faculty owner.
        let raw = format!("sct_agent_{}", Uuid::new_v4().simple());
        let (hash, owner) = (hash_token(&raw), PERSONAS[0].1.to_string());
        db(move |c| {
            let expires = chrono::Utc::now() + chrono::Duration::days(1);
            c.execute(
                "INSERT INTO api_tokens (token_hash, id, kind, eppn, label, created_at, expires_at) VALUES ($1, $2, 'agent', $3, 'Scenario agent', NOW(), $4)",
                &[&hash, &Uuid::new_v4(), &owner, &expires],
            )
            .unwrap();
        });
        tokens.insert("agent".to_string(), raw);

        let state = Arc::new(ServerState::new().expect("state"));
        state.seed_demo().expect("seed");
        {
            let mut all = state.collaborators.write().unwrap();
            let list = all.entry(WORKSPACE.to_string()).or_default();
            for (name, role) in [("editor", "editor"), ("viewer", "viewer")] {
                let eppn = PERSONAS.iter().find(|(n, _)| *n == name).unwrap().1;
                list.push(CollaboratorRecord {
                    id: format!("collab-{}", Uuid::new_v4()),
                    workspace_id: WORKSPACE.to_string(),
                    eppn: eppn.to_string(),
                    name: name.to_string(),
                    role: role.to_string(),
                    scoped_affiliation: "faculty".to_string(),
                    department: "physics".to_string(),
                    added_at: String::new(),
                });
            }
        }
        Harness { app: build_app_with_state(state).expect("router"), tokens }
    }

    fn token(&self, persona: &str) -> &str {
        self.tokens.get(persona).unwrap_or_else(|| panic!("no persona '{persona}'"))
    }

    /// One tool call over the real endpoint. Returns the raw response text too, for the leak check.
    async fn call(&self, persona: &str, tool: &str, args: Value) -> Called {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
        let (status, text) = self.post("/api/mcp", persona, &payload).await;
        assert_eq!(status, StatusCode::OK, "{tool}: {text}");
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(err) = v.get("error") {
            return Called { is_error: true, code: None, message: err["message"].as_str().unwrap_or_default().to_string(), result: Value::Null, raw: text };
        }
        let result = &v["result"];
        if result["isError"] == true {
            let structured = &result["structuredContent"];
            return Called {
                is_error: true,
                code: structured["code"].as_str().map(str::to_string),
                message: result["content"][0]["text"].as_str().unwrap_or_default().to_string(),
                result: Value::Null,
                raw: text,
            };
        }
        Called { is_error: false, code: None, message: String::new(), result: result["structuredContent"].clone(), raw: text }
    }

    async fn post(&self, uri: &str, persona: &str, body: &Value) -> (StatusCode, String) {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.token(persona)))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    async fn get(&self, uri: &str, persona: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.token(persona)))
            .body(Body::empty())
            .unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

pub struct Called {
    is_error: bool,
    code: Option<String>,
    message: String,
    result: Value,
    raw: String,
}

// ---------------------------------------------------------------------------------------------
// Substitution and expectations.
// ---------------------------------------------------------------------------------------------

/// Replaces `$name` with a saved value. A string that is exactly `$name` keeps the value's type.
fn substitute(v: &Value, vars: &BTreeMap<String, Value>) -> Value {
    match v {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix('$') {
                if let Some(found) = vars.get(name) {
                    return found.clone();
                }
            }
            let mut out = s.clone();
            // Longest names first, so `$app2` is not eaten by `$app`.
            let mut names: Vec<&String> = vars.keys().collect();
            names.sort_by_key(|n| std::cmp::Reverse(n.len()));
            for name in names {
                let needle = format!("${name}");
                if out.contains(&needle) {
                    let text = match &vars[name] {
                        Value::String(t) => t.clone(),
                        other => other.to_string(),
                    };
                    out = out.replace(&needle, &text);
                }
            }
            Value::String(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|i| substitute(i, vars)).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, i)| (k.clone(), substitute(i, vars))).collect()),
        other => other.clone(),
    }
}

/// Follows a dotted path. An array element is found by its `name` or by its index.
fn pick<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let mut cur = root;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(items) => match seg.parse::<usize>() {
                Ok(i) => items.get(i)?,
                Err(_) => items.iter().find(|e| e["name"] == seg)?,
            },
            _ => return None,
        };
    }
    Some(cur)
}

fn numbers_equal(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-9,
        _ => false,
    }
}

/// Compares one named path. Returns the difference when the expectation does not hold.
fn check_expectation(result: &Value, path: &str, expected: &Value) -> Result<(), String> {
    let actual = pick(result, path);
    let diff = |why: &str| -> String {
        format!(
            "  {path}: {why}\n    expected: {expected}\n    actual:   {}",
            actual.map(|a| a.to_string()).unwrap_or_else(|| "(missing)".to_string())
        )
    };
    if let Some(ops) = expected.as_object() {
        if ops.len() == 1 {
            if let Some(want_absent) = ops.get("absent") {
                let absent = actual.is_none_or(Value::is_null);
                return if absent == want_absent.as_bool().unwrap_or(true) { Ok(()) } else { Err(diff("presence differs")) };
            }
            let Some(actual) = actual else { return Err(diff("missing")) };
            if let Some(n) = ops.get("len") {
                let len = actual.as_array().map(|a| a.len()).or_else(|| actual.as_str().map(|s| s.len()));
                return if len.map(|l| l as u64) == n.as_u64() { Ok(()) } else { Err(diff("length differs")) };
            }
            if let Some(needle) = ops.get("contains") {
                let hay = match actual {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                return if hay.contains(needle.as_str().unwrap_or_default()) { Ok(()) } else { Err(diff("text not found")) };
            }
            if let Some(want) = ops.get("has").and_then(|w| w.as_object()) {
                let found = actual.as_array().is_some_and(|items| {
                    items.iter().any(|item| want.iter().all(|(k, v)| item.get(k) == Some(v) || item.get(k).is_some_and(|a| numbers_equal(a, v))))
                });
                return if found { Ok(()) } else { Err(diff("no element has all of these")) };
            }
            if let Some(n) = ops.get("gt") {
                return if actual.as_f64() > n.as_f64() { Ok(()) } else { Err(diff("not greater")) };
            }
        }
    }
    let Some(actual) = actual else { return Err(diff("missing")) };
    // A named check stands for its outcome.
    if let (Some(obj), Some(_)) = (actual.as_object(), expected.as_str()) {
        if let Some(outcome) = obj.get("outcome") {
            return if outcome == expected { Ok(()) } else { Err(diff("outcome differs")) };
        }
    }
    if actual == expected || numbers_equal(actual, expected) {
        Ok(())
    } else {
        Err(diff("value differs"))
    }
}

// ---------------------------------------------------------------------------------------------
// The after assertions.
// ---------------------------------------------------------------------------------------------

/// A response must not hold a stack trace or a secret.
pub fn check_no_leaks(responses: &[String], secrets: &[String]) -> Result<(), String> {
    for r in responses {
        for marker in ["panicked at", "stack backtrace", "RUST_BACKTRACE", "thread '", "src/", ".rs:"] {
            if r.contains(marker) {
                return Err(format!("a response holds '{marker}': {}", r.chars().take(200).collect::<String>()));
            }
        }
        for s in secrets {
            if !s.is_empty() && r.contains(s.as_str()) {
                return Err("a response holds a secret".to_string());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Scenarios.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Pass,
    Fail(String),
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub id: String,
    pub ask: String,
    pub persona: String,
    pub raw: Value,
}

fn load_scenarios() -> Vec<Scenario> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent_scenarios");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let raw: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            Scenario {
                id: raw["id"].as_str().unwrap_or_else(|| panic!("{} has no id", path.display())).to_string(),
                ask: raw["ask"].as_str().unwrap_or_else(|| panic!("{} has no ask", path.display())).to_string(),
                persona: raw["persona"].as_str().unwrap_or("faculty-owner").to_string(),
                raw,
            }
        })
        .collect()
}

async fn run_scenario(h: &Harness, s: &Scenario) -> Outcome {
    if s.raw["status"] == "unsupported" {
        let why = s.raw["unsupported_reason"].as_str().unwrap_or("").to_string();
        assert!(!why.is_empty(), "{}: an unsupported scenario says why", s.id);
        return Outcome::Unsupported(why);
    }

    let mut vars: BTreeMap<String, Value> = BTreeMap::new();
    let unique = Uuid::new_v4().simple().to_string()[..8].to_string();
    vars.insert("ws".to_string(), json!(WORKSPACE));
    vars.insert("app".to_string(), json!(format!("sc-{unique}")));
    vars.insert("suffix".to_string(), json!(unique));

    let mut responses: Vec<String> = Vec::new();
    let mut by_persona: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let steps = s.raw["steps"].as_array().cloned().unwrap_or_default();
    for (n, step) in steps.iter().enumerate() {
        let persona = step["as"].as_str().unwrap_or(&s.persona).to_string();
        let tool = step["tool"].as_str().unwrap_or_default().to_string();
        let repeat = step["repeat"].as_u64().unwrap_or(1);
        for i in 0..repeat {
            let mut local = vars.clone();
            local.insert("i".to_string(), json!(i));
            let args = substitute(&step["args"], &local);
            let called = h.call(&persona, &tool, args).await;
            responses.push(called.raw.clone());
            by_persona.entry(persona.clone()).or_default().push(called.raw.clone());
            let label = format!("{}: step {} ({tool} as {persona})", s.id, n + 1);

            if let Some(err) = step.get("error") {
                if !called.is_error {
                    return Outcome::Fail(format!("{label}\n  expected a tool error {err}\n  actual:   {}", called.result));
                }
                if let Some(code) = err["code"].as_str() {
                    if called.code.as_deref() != Some(code) {
                        return Outcome::Fail(format!("{label}\n  error code\n    expected: {code}\n    actual:   {:?}", called.code));
                    }
                }
                if let Some(needle) = err["contains"].as_str() {
                    let needle = substitute(&json!(needle), &local);
                    if !called.message.contains(needle.as_str().unwrap_or_default()) {
                        return Outcome::Fail(format!("{label}\n  error text\n    expected to contain: {needle}\n    actual:   {}", called.message));
                    }
                }
            } else if called.is_error {
                return Outcome::Fail(format!("{label}\n  the tool returned an error: {} ({:?})", called.message, called.code));
            }

            if let Some(expect) = step["expect"].as_object() {
                for (path, want) in expect {
                    let want = substitute(want, &local);
                    if let Err(d) = check_expectation(&called.result, path, &want) {
                        return Outcome::Fail(format!("{label}\n{d}"));
                    }
                }
            }
            if let Some(save) = step["save"].as_object() {
                for (name, path) in save {
                    match pick(&called.result, path.as_str().unwrap_or_default()) {
                        Some(v) => {
                            vars.insert(name.clone(), v.clone());
                        }
                        None => return Outcome::Fail(format!("{label}\n  could not save '{name}': no value at {path}")),
                    }
                }
            }
        }
    }

    let secrets: Vec<String> = h.tokens.values().cloned().collect();
    for check in s.raw["after"].as_array().cloned().unwrap_or_default() {
        if let Some(kind) = check["ledger_has"].as_str() {
            let (status, ledger) = h.get("/api/v1/governance/ledger", "platform-admin").await;
            let app = vars["app"].as_str().unwrap_or_default();
            let found = status == StatusCode::OK
                && ledger["entries"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|e| e["decision_type"] == kind && e["app_slug"] == app));
            if !found {
                return Outcome::Fail(format!("{}: after: the ledger has no {kind} entry for {app}", s.id));
            }
        }
        if check["no_error_leaks"] == true {
            if let Err(e) = check_no_leaks(&responses, &secrets) {
                return Outcome::Fail(format!("{}: after: {e}", s.id));
            }
        }
        if let Some(spec) = check.get("never_seen_by") {
            let who = spec["persona"].as_str().unwrap_or_default();
            let text = substitute(&spec["text"], &vars);
            let text = text.as_str().unwrap_or_default();
            if by_persona.get(who).is_some_and(|rs| rs.iter().any(|r| r.contains(text))) {
                return Outcome::Fail(format!("{}: after: {who} saw '{text}'", s.id));
            }
        }
    }
    Outcome::Pass
}

/// The gap register: unsupported scenarios with their asks and reasons.
fn write_gap_register(rows: &[(Scenario, Outcome)], file: &str) -> std::path::PathBuf {
    let mut text = String::from("# Agent gaps\n\nScenarios marked unsupported, with the ask and why. Generated by `tests/agent_scenarios.rs`. Do not commit.\n\n");
    for (s, o) in rows {
        if let Outcome::Unsupported(why) = o {
            text.push_str(&format!("## {}\n\n- Ask: {}\n- Why: {}\n\n", s.id, s.ask, why));
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

/// A supported scenario that fails fails the suite. An unsupported one does not.
fn failures(rows: &[(Scenario, Outcome)]) -> Vec<String> {
    rows.iter()
        .filter_map(|(s, o)| match o {
            Outcome::Fail(d) => Some(format!("{}\n{d}", s.id)),
            _ => None,
        })
        .collect()
}

static HARNESS: OnceCell<Harness> = OnceCell::const_new();
static SUITE: Mutex<()> = Mutex::const_new(());

async fn harness() -> &'static Harness {
    HARNESS.get_or_init(Harness::boot).await
}

// ---------------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------------

// 1. The runner executes the twenty scenarios and all pass.
#[tokio::test]
async fn the_scenarios_all_pass() {
    let _turn = SUITE.lock().await;
    let h = harness().await;
    let scenarios = load_scenarios();
    assert!(scenarios.len() >= 20, "twenty scenarios: found {}", scenarios.len());

    let mut rows = Vec::new();
    for s in scenarios {
        let outcome = run_scenario(h, &s).await;
        rows.push((s, outcome));
    }
    println!("\n{:<44} result", "scenario");
    for (s, o) in &rows {
        let word = match o {
            Outcome::Pass => "pass".to_string(),
            Outcome::Fail(_) => "FAIL".to_string(),
            Outcome::Unsupported(_) => "unsupported".to_string(),
        };
        println!("{:<44} {word}", s.id);
    }
    let gaps = write_gap_register(&rows, "agent-gaps.md");
    println!("gap register: {}", gaps.display());
    let failed = failures(&rows);
    assert!(failed.is_empty(), "scenarios failed:\n{}", failed.join("\n\n"));
}

// 2. A wrong expectation fails with a diff of expected and actual.
#[tokio::test]
async fn a_wrong_expectation_fails_with_a_diff() {
    let _turn = SUITE.lock().await;
    let h = harness().await;
    let mut s = load_scenarios().into_iter().find(|s| s.id == "enter-records").expect("the enter-records scenario");
    s.raw["steps"].as_array_mut().unwrap().last_mut().unwrap()["expect"] = json!({ "records": { "len": 99 } });
    match run_scenario(h, &s).await {
        Outcome::Fail(d) => {
            assert!(d.contains("expected") && d.contains("actual"), "{d}");
            assert!(d.contains("99"), "{d}");
        }
        other => panic!("it should fail: {other:?}"),
    }
}

// 3. Unsupported is listed and does not fail. Supported and failing does.
#[tokio::test]
async fn unsupported_scenarios_are_listed_and_do_not_fail_the_suite() {
    let _turn = SUITE.lock().await;
    let h = harness().await;
    let fixture = Scenario {
        id: "fixture-unsupported".to_string(),
        ask: "Total the amounts of every application.".to_string(),
        persona: "faculty-owner".to_string(),
        raw: json!({ "id": "fixture-unsupported", "status": "unsupported", "unsupported_reason": "No aggregate tool exists.", "steps": [] }),
    };
    let outcome = run_scenario(h, &fixture).await;
    assert_eq!(outcome, Outcome::Unsupported("No aggregate tool exists.".to_string()));

    let broken = Scenario {
        id: "fixture-broken".to_string(),
        ask: "A supported scenario that fails.".to_string(),
        persona: "faculty-owner".to_string(),
        raw: json!({ "id": "fixture-broken", "status": "supported", "steps": [ { "tool": "no_such_tool", "args": {} } ] }),
    };
    let rows = vec![(fixture, outcome), (broken.clone(), run_scenario(h, &broken).await)];
    let path = write_gap_register(&rows, "agent-gaps-fixture.md");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("fixture-unsupported") && text.contains("No aggregate tool exists."), "{text}");
    assert!(!text.contains("fixture-broken"), "a failing scenario is not a gap: {text}");
    let failed = failures(&rows);
    assert_eq!(failed.len(), 1, "only the supported failure counts: {failed:?}");
    assert!(failed[0].contains("fixture-broken"));
}

// 4. Two scenarios run at once without sharing state.
#[tokio::test]
async fn two_scenarios_run_in_parallel_without_sharing_state() {
    let h = harness().await;
    let s = load_scenarios().into_iter().find(|s| s.id == "enter-records").expect("the enter-records scenario");
    // Each run lists its own app and expects exactly its own three records.
    let (a, b) = tokio::join!(run_scenario(h, &s), run_scenario(h, &s));
    assert_eq!(a, Outcome::Pass, "{a:?}");
    assert_eq!(b, Outcome::Pass, "{b:?}");
}

// 5. `no_error_leaks` fails on a panic or a stack frame, and on a secret.
#[test]
fn no_error_leaks_fails_on_a_panic_a_stack_frame_or_a_secret() {
    assert!(check_no_leaks(&["{\"result\":\"fine\"}".to_string()], &["sct_secret".to_string()]).is_ok());
    for leak in [
        "thread 'tokio-runtime-worker' panicked at crates/scaffoldry-server/src/x.rs:10:5",
        "stack backtrace:\n   0: rust_begin_unwind",
        "error at crates/scaffoldry-server/src/service/records.rs:44",
    ] {
        assert!(check_no_leaks(&[leak.to_string()], &[]).is_err(), "should fail: {leak}");
    }
    assert!(check_no_leaks(&["token sct_secret leaked".to_string()], &["sct_secret".to_string()]).is_err());
}

// 6. The runner uses the real endpoint and does not call service functions.
#[test]
fn the_runner_calls_the_http_endpoint_and_not_the_service_layer() {
    let source = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent_scenarios.rs")).unwrap();
    // Only code before this test counts. This test names the forbidden paths itself.
    let code = source.split("// 6. The runner uses").next().unwrap();
    assert!(code.contains("\"/api/mcp\""), "the runner posts to the MCP endpoint");
    for forbidden in ["tools::call", "service::records", "service::proposals", "service::processes", "service::workspaces", ".state.repository"] {
        assert!(!code.contains(forbidden), "the runner reaches into the server: {forbidden}");
    }
    // The one service helper it may use is the one that mints a test token.
    let service_uses: Vec<&str> = code.lines().filter(|l| l.contains("scaffoldry_server::service::")).collect();
    assert!(service_uses.iter().all(|l| l.contains("identity::{hash_token, issue_test_token_and_user}")), "{service_uses:?}");
    let _ = Map::<String, Value>::new();
}
