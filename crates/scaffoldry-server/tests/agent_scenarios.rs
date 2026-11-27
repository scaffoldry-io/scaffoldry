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

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::Engine;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::{hash_token, issue_test_token_and_user};
use scaffoldry_server::state::{CollaboratorRecord, ServerState};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// An Internal workspace the personas share.
const WORKSPACE: &str = "ws-physics-optics";

/// An AI tool's sign-in, as `mcp_guidance_phase6_test.rs` does it.
const AGENT_CALLBACK: &str = "https://tool.example.edu/callback";
const AGENT_ISSUER: &str = "https://api.test.edu";
const AGENT_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

const PERSONAS: &[(&str, &str)] = &[
    ("faculty-owner", "einstein@physics.state.edu"),
    ("editor", "editor.test@physics.state.edu"),
    ("viewer", "viewer.test@physics.state.edu"),
    ("student", "student.test@physics.state.edu"),
    ("compliance", "compliance.officer@state.edu"),
    ("platform-admin", "jordan.lee@state.edu"),
];

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
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
        // No job runner works inside this suite, so the jobs an approval queues stay queued. The
        // job limits have their own tests in jobs_limits_test.rs.
        std::env::set_var("SCAFFOLDRY_JOB_LIMIT_PER_USER", "100000");
        std::env::set_var("SCAFFOLDRY_JOB_LIMIT_QUEUED", "100000");
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
                    app_slug: String::new(),
                });
            }
        }
        let mut h = Harness { app: build_app_with_state(state).expect("router"), tokens };
        // An AI tool signed in as a person: `agent-viewer`, `agent-student`, `agent-platform-admin`.
        h.sign_in_agents(&[("viewer", &["access"]), ("student", &["access"]), ("platform-admin", &["build", "governance", "data"])]).await;
        h
    }

    /// The sign-in an AI tool really goes through: a client an administrator approves, a person who allows it, and a
    /// token with a session. A request such an agent files waits for the person (`mcp-guidance.md` phase 6). The token
    /// is read-only, and the tool says what it is doing with `begin_task`.
    async fn sign_in_agents(&mut self, personas: &[(&str, &[&str])]) {
        let admin = self.token("platform-admin").to_string();
        let made = self.post_as(&admin, "/api/v1/clients", &json!({ "name": "Scenario assistant", "operator": "Academic Computing", "kind": "agent", "redirect_uris": [AGENT_CALLBACK] })).await;
        let client = made["client_id"].as_str().unwrap_or_else(|| panic!("{made}")).to_string();
        let decided = self.post_as(&admin, &format!("/api/v1/clients/{client}/decide"), &json!({ "decision": "approve", "rationale": "A scenario agent" })).await;
        assert!(decided.get("error").is_none(), "{decided}");
        for (persona, toolsets) in personas {
            let person = self.token(persona).to_string();
            let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(AGENT_VERIFIER.as_bytes()));
            let allow = json!({ "client_id": client, "redirect_uri": AGENT_CALLBACK, "response_type": "code", "state": "s", "code_challenge": challenge, "code_challenge_method": "S256", "resource": AGENT_ISSUER, "decision": "allow", "access": "read", "toolsets": toolsets });
            let allowed = self.post_as(&person, "/oauth/authorize", &allow).await;
            let code = allowed["redirect_to"].as_str().and_then(|u| u.split_once('?')).and_then(|(_, q)| q.split('&').find_map(|p| p.strip_prefix("code="))).unwrap_or_else(|| panic!("{allowed}")).to_string();
            let form = format!("grant_type=authorization_code&code={code}&redirect_uri={AGENT_CALLBACK}&client_id={client}&code_verifier={AGENT_VERIFIER}");
            let req = Request::builder().method("POST").uri("/oauth/token").header("host", "api.test.edu").header("content-type", "application/x-www-form-urlencoded").body(Body::from(form)).unwrap();
            let resp = self.app.clone().oneshot(req).await.unwrap();
            let reply: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null);
            let token = reply["access_token"].as_str().unwrap_or_else(|| panic!("{reply}")).to_string();
            self.tokens.insert(format!("agent-{persona}"), token);
            let begun = self.call(&format!("agent-{persona}"), "begin_task", json!({ "task": "A scenario" })).await;
            assert!(!begun.is_error, "{}", begun.message);
        }
    }

    async fn post_as(&self, token: &str, uri: &str, body: &Value) -> Value {
        let req = Request::builder().method("POST").uri(uri).header("host", "api.test.edu").header("authorization", format!("Bearer {token}")).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap_or(Value::Null)
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
            return Called { is_error: true, code: None, message: err["message"].as_str().unwrap_or_default().to_string(), result: Value::Null, shown: Vec::new(), raw: text };
        }
        let result = &v["result"];
        if result["isError"] == true {
            let structured = &result["structuredContent"];
            return Called {
                is_error: true,
                code: structured["code"].as_str().map(str::to_string),
                message: result["content"][0]["text"].as_str().unwrap_or_default().to_string(),
                result: Value::Null,
                shown: vec![structured.clone()],
                raw: text,
            };
        }
        // The plain result is the text content. structuredContent wraps a list or a lone value in an object, as the
        // spec requires, so a scenario reads the text to see what the tool returned.
        let plain = result["content"][0]["text"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_else(|| result["structuredContent"].clone());
        let shown = vec![result["structuredContent"].clone(), plain.clone()];
        Called { is_error: false, code: None, message: String::new(), result: plain, shown, raw: text }
    }

    /// `prompts/get` over the real endpoint. Returns the prompt text and the raw response.
    async fn prompt(&self, persona: &str, name: &str, args: Value) -> Result<(String, String), String> {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "prompts/get", "params": { "name": name, "arguments": args } });
        let (status, raw) = self.post("/api/mcp", persona, &payload).await;
        let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        match v["result"]["messages"][0]["content"]["text"].as_str() {
            Some(t) if status == StatusCode::OK => Ok((t.to_string(), raw)),
            _ => Err(format!("prompts/get {name} gave no text: {status} {raw}")),
        }
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
    /// Every form of the result a client reads: `structuredContent` and the parsed text copy.
    shown: Vec<Value>,
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
// Prompts: what a generated prompt allows, where it stops, and the shape of its answer.
// ---------------------------------------------------------------------------------------------

/// The parts of a generated prompt that a scenario holds the agent to.
#[derive(Debug, Clone)]
pub struct PromptParts {
    pub name: String,
    pub allowed: Vec<String>,
    pub avoid: Vec<String>,
    pub stops: Vec<String>,
    pub shape: Value,
}

fn backticked(line: &str) -> Vec<String> {
    line.split('`').skip(1).step_by(2).map(str::to_string).collect()
}

/// Reads the five parts of a prompt's text. The tools come from the `Tools you may use` list, the stops from the
/// `Stop and ask the person` list, and the shape from the `Output shape` block, which must be JSON.
pub fn parse_prompt(name: &str, text: &str) -> Result<PromptParts, String> {
    for part in ["Task: ", "\nContext:\n", "\nTools you may use:\n", "\nStop and ask the person:\n", "\nOutput shape:\n"] {
        if !text.contains(part) {
            return Err(format!("the prompt {name} has no '{}' part", part.trim()));
        }
    }
    let between = |from: &str, to: &str| -> String {
        let after = text.split(from).nth(1).unwrap_or_default();
        after.split(to).next().unwrap_or_default().to_string()
    };
    let tools_block = between("\nTools you may use:\n", "\nStop and ask the person:\n");
    let mut allowed = Vec::new();
    let mut avoid = Vec::new();
    for line in tools_block.lines() {
        if line.starts_with("- `") {
            allowed.extend(backticked(line).into_iter().take(1));
        } else if line.starts_with("Do not use:") {
            avoid.extend(backticked(line));
        }
    }
    let stops: Vec<String> = between("\nStop and ask the person:\n", "\nOutput shape:\n").lines().filter_map(|l| l.strip_prefix("- ")).map(str::to_string).collect();
    let shape_text = between("\nOutput shape:\n", "\n\nYou work for");
    let shape: Value = serde_json::from_str(shape_text.trim()).map_err(|e| format!("the output shape of {name} is not JSON ({e}): {shape_text}"))?;
    if allowed.is_empty() || stops.is_empty() {
        return Err(format!("the prompt {name} lists no tools or no stopping points"));
    }
    Ok(PromptParts { name: name.to_string(), allowed, avoid, stops, shape })
}

/// Where an answer differs from the output shape. A shape string stands for a string or a number. A shape string that
/// starts with `optional` may be left out. An array shape holds one example element that every element must follow.
pub fn shape_problems(shape: &Value, got: &Value, path: &str) -> Vec<String> {
    let here = if path.is_empty() { "the answer" } else { path };
    match (shape, got) {
        (Value::Object(want), Value::Object(have)) => {
            let mut out = Vec::new();
            for (k, w) in want {
                match have.get(k) {
                    Some(g) => out.extend(shape_problems(w, g, &format!("{path}.{k}"))),
                    None if w.as_str().is_some_and(|s| s.starts_with("optional")) => {}
                    None => out.push(format!("{here} has no '{k}'")),
                }
            }
            for k in have.keys() {
                if !want.contains_key(k) {
                    out.push(format!("{here} has '{k}', which the shape does not name"));
                }
            }
            out
        }
        (Value::Array(want), Value::Array(have)) => {
            let Some(example) = want.first() else { return Vec::new() };
            have.iter().enumerate().flat_map(|(i, g)| shape_problems(example, g, &format!("{path}[{i}]"))).collect()
        }
        (Value::String(_), Value::String(_) | Value::Number(_)) => Vec::new(),
        (w, g) => {
            let kind = |v: &Value| match v {
                Value::Null => "null",
                Value::Bool(_) => "a boolean",
                Value::Number(_) => "a number",
                Value::String(_) => "a string",
                Value::Array(_) => "an array",
                Value::Object(_) => "an object",
            };
            vec![format!("{here} is {}, and the shape wants {}", kind(g), kind(w))]
        }
    }
}

/// Typed text may appear in a result only under `untrusted_text`. Returns the first place it appears outside.
pub fn typed_text_outside_untrusted(v: &Value, text: &str, under: bool) -> Option<String> {
    match v {
        Value::String(s) => (!under && s.contains(text)).then(|| s.chars().take(120).collect()),
        Value::Array(items) => items.iter().find_map(|i| typed_text_outside_untrusted(i, text, under)),
        Value::Object(map) => map.iter().find_map(|(k, i)| typed_text_outside_untrusted(i, text, under || k == "untrusted_text")),
        _ => None,
    }
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

/// The `scenario` code blocks of the building guide the server serves. The guide cannot drift from the server, because
/// each of its examples is run here.
fn load_guide_scenarios() -> Vec<Scenario> {
    let guide = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("guides/building-apps.md")).unwrap();
    let mut out = Vec::new();
    let mut rest = guide.as_str();
    while let Some(start) = rest.find("```scenario\n") {
        let after = &rest[start + "```scenario\n".len()..];
        let end = after.find("\n```").expect("a scenario block is closed");
        let raw: Value = serde_json::from_str(&after[..end]).unwrap_or_else(|e| panic!("a guide scenario is not JSON: {e}\n{}", &after[..end]));
        out.push(Scenario {
            id: raw["id"].as_str().expect("a guide scenario has an id").to_string(),
            ask: raw["ask"].as_str().expect("a guide scenario has an ask").to_string(),
            persona: raw["persona"].as_str().unwrap_or("faculty-owner").to_string(),
            raw,
        });
        rest = &after[end..];
    }
    out
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
    // What the last prompt allows, and the tables to hold still from the moment it was asked for.
    let mut prompt: Option<PromptParts> = None;
    let mut shown: Vec<Value> = Vec::new();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut by_persona: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let steps = s.raw["steps"].as_array().cloned().unwrap_or_default();
    for (n, step) in steps.iter().enumerate() {
        let persona = step["as"].as_str().unwrap_or(&s.persona).to_string();
        let tool = step["tool"].as_str().unwrap_or_default().to_string();
        let repeat = step["repeat"].as_u64().unwrap_or(1);
        let label = format!("{}: step {} ({persona})", s.id, n + 1);

        // A prompt step: the person's agent asks for a generated prompt, as the persona.
        if let Some(name) = step["prompt"].as_str() {
            let args = substitute(&step["args"], &vars);
            let (text, raw) = match h.prompt(&persona, name, args).await {
                Ok(got) => got,
                Err(e) => return Outcome::Fail(format!("{label}\n  {e}")),
            };
            responses.push(raw.clone());
            by_persona.entry(persona.clone()).or_default().push(raw);
            let parts = match parse_prompt(name, &text) {
                Ok(p) => p,
                Err(e) => return Outcome::Fail(format!("{label}\n  {e}")),
            };
            for needle in step["contains"].as_array().cloned().unwrap_or_default() {
                let needle = substitute(&needle, &vars);
                if !text.contains(needle.as_str().unwrap_or_default()) {
                    return Outcome::Fail(format!("{label}\n  the prompt does not say '{needle}'"));
                }
            }
            for table in s.raw["after"].as_array().into_iter().flatten().filter_map(|c| c["unchanged"].as_array()).flatten().filter_map(Value::as_str) {
                counts.insert(table.to_string(), count_rows(table));
            }
            prompt = Some(parts);
            continue;
        }

        // A person step: the person's reply, recorded. It is what lets an agent go past a stopping point.
        if step.get("person").is_some() {
            continue;
        }

        // An answer step: what the agent hands the person, held to the prompt's output shape.
        if step.get("answer").is_some() {
            let answer = substitute(&step["answer"], &vars);
            if step["matches_output_shape"] == true {
                let Some(p) = &prompt else { return Outcome::Fail(format!("{label}\n  an answer is checked against a prompt, and none was asked for")) };
                let problems = shape_problems(&p.shape, &answer, "");
                if !problems.is_empty() {
                    return Outcome::Fail(format!("{label}\n  the answer does not match the output shape of {}:\n    {}", p.name, problems.join("\n    ")));
                }
            }
            continue;
        }

        for i in 0..repeat {
            let mut local = vars.clone();
            local.insert("i".to_string(), json!(i));
            let args = substitute(&step["args"], &local);
            let called = h.call(&persona, &tool, args).await;
            responses.push(called.raw.clone());
            shown.extend(called.shown.clone());
            by_persona.entry(persona.clone()).or_default().push(called.raw.clone());
            let label = format!("{}: step {} ({tool} as {persona})", s.id, n + 1);
            if let Some(p) = &prompt {
                if !p.allowed.contains(&tool) {
                    return Outcome::Fail(format!("{label}\n  {tool} is not among the tools {} allows: {}", p.name, p.allowed.join(", ")));
                }
            }

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
        if let Some(tables) = check["unchanged"].as_array() {
            for table in tables.iter().filter_map(Value::as_str) {
                let was = counts.get(table).copied();
                if was.is_none() || was != Some(count_rows(table)) {
                    return Outcome::Fail(format!("{}: after: {table} changed after the prompt, or no prompt was asked for", s.id));
                }
            }
        }
        if let Some(texts) = check["typed_text_only_under_untrusted"].as_array() {
            for text in texts {
                let text = substitute(text, &vars);
                let text = text.as_str().unwrap_or_default();
                if let Some(at) = shown.iter().find_map(|v| typed_text_outside_untrusted(v, text, false)) {
                    return Outcome::Fail(format!("{}: after: typed text '{text}' appears outside untrusted_text: {at}", s.id));
                }
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
            if let Some(r) = by_persona.get(who).and_then(|rs| rs.iter().find(|r| r.contains(text))) {
                let at = r.find(text).unwrap_or(0);
                let near: String = r.chars().skip(at.saturating_sub(100)).take(240).collect();
                return Outcome::Fail(format!("{}: after: {who} saw '{text}', near: {near}", s.id));
            }
        }
    }
    Outcome::Pass
}

/// The rows in a table, for the checks that hold a table still.
fn count_rows(table: &str) -> i64 {
    assert!(table.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "a table name: {table}");
    let sql = format!("SELECT COUNT(*) FROM {table}");
    db(move |c| c.query_one(sql.as_str(), &[]).unwrap().get(0))
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

/// A server of its own on a database of its own, for each test.
async fn harness() -> Harness {
    Harness::boot().await
}

// ---------------------------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------------------------

// 1. The runner executes the twenty scenarios and all pass.
#[tokio::test]
async fn the_scenarios_all_pass() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let scenarios = load_scenarios();
    assert!(scenarios.len() >= 20, "twenty scenarios: found {}", scenarios.len());

    let mut rows = Vec::new();
    for s in scenarios {
        let outcome = run_scenario(&h, &s).await;
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

// The building guide's code blocks run as scenarios (agent-coverage phase 2).
#[tokio::test]
async fn every_code_block_in_the_building_guide_runs_as_a_scenario() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let blocks = load_guide_scenarios();
    assert!(blocks.len() >= 5, "the guide has its examples: found {}", blocks.len());
    let mut rows = Vec::new();
    for s in blocks {
        let outcome = run_scenario(&h, &s).await;
        rows.push((s, outcome));
    }
    for (s, o) in &rows {
        assert!(!matches!(o, Outcome::Unsupported(_)), "a guide example is never unsupported: {}", s.id);
    }
    let failed = failures(&rows);
    assert!(failed.is_empty(), "guide examples failed:\n{}", failed.join("\n\n"));
}

// 2. A wrong expectation fails with a diff of expected and actual.
#[tokio::test]
async fn a_wrong_expectation_fails_with_a_diff() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let mut s = load_scenarios().into_iter().find(|s| s.id == "enter-records").expect("the enter-records scenario");
    s.raw["steps"].as_array_mut().unwrap().last_mut().unwrap()["expect"] = json!({ "records": { "len": 99 } });
    match run_scenario(&h, &s).await {
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
    let _db = common::TestDb::new();
    let h = harness().await;
    let fixture = Scenario {
        id: "fixture-unsupported".to_string(),
        ask: "Total the amounts of every application.".to_string(),
        persona: "faculty-owner".to_string(),
        raw: json!({ "id": "fixture-unsupported", "status": "unsupported", "unsupported_reason": "No aggregate tool exists.", "steps": [] }),
    };
    let outcome = run_scenario(&h, &fixture).await;
    assert_eq!(outcome, Outcome::Unsupported("No aggregate tool exists.".to_string()));

    let broken = Scenario {
        id: "fixture-broken".to_string(),
        ask: "A supported scenario that fails.".to_string(),
        persona: "faculty-owner".to_string(),
        raw: json!({ "id": "fixture-broken", "status": "supported", "steps": [ { "tool": "no_such_tool", "args": {} } ] }),
    };
    let rows = vec![(fixture, outcome), (broken.clone(), run_scenario(&h, &broken).await)];
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
    let _db = common::TestDb::new();
    let h = harness().await;
    let s = load_scenarios().into_iter().find(|s| s.id == "enter-records").expect("the enter-records scenario");
    // Each run lists its own app and expects exactly its own three records.
    let (a, b) = tokio::join!(run_scenario(&h, &s), run_scenario(&h, &s));
    assert_eq!(a, Outcome::Pass, "{a:?}");
    assert_eq!(b, Outcome::Pass, "{b:?}");
}

// 5. `no_error_leaks` fails on a panic or a stack frame, and on a secret.
#[test]
fn no_error_leaks_fails_on_a_panic_a_stack_frame_or_a_secret() {
    let _db = common::TestDb::new();
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
    let _db = common::TestDb::new();
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

// ---------------------------------------------------------------------------------------------
// Prompt scenarios (mcp-guidance phase 7): the runner's own checks.
// ---------------------------------------------------------------------------------------------

const FIVE_PROMPTS: [&str; 5] = ["request_dataset_access", "review_a_change", "write_a_policy", "review_a_share_request", "explain_a_refusal"];

fn fixture(raw: Value) -> Scenario {
    Scenario { id: raw["id"].as_str().unwrap().to_string(), ask: "A fixture.".to_string(), persona: "agent-viewer".to_string(), raw }
}

// Every generated prompt can be parsed: five parts, its tools and stops, and an output shape that is JSON.
#[tokio::test]
async fn every_prompt_parses_into_tools_stops_and_a_json_output_shape() {
    let _db = common::TestDb::new();
    let h = harness().await;
    for name in FIVE_PROMPTS {
        let (text, _) = h.prompt("faculty-owner", name, json!({})).await.unwrap();
        let parts = parse_prompt(name, &text).unwrap_or_else(|e| panic!("{e}"));
        assert!(!parts.allowed.is_empty() && !parts.stops.is_empty() && parts.shape.is_object(), "{name}: {parts:?}");
    }
}

// A call outside the prompt's tool list fails the scenario, and says which tool and which prompt.
#[tokio::test]
async fn a_tool_outside_the_prompts_list_fails() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let s = fixture(json!({ "id": "fixture-outside", "status": "supported", "steps": [
        { "prompt": "request_dataset_access", "args": {} },
        { "tool": "list_datasets", "args": {} },
        { "tool": "list_records", "args": { "app_slug": "$app" } }
    ] }));
    match run_scenario(&h, &s).await {
        Outcome::Fail(d) => assert!(d.contains("list_records is not among the tools request_dataset_access allows"), "{d}"),
        other => panic!("it should fail: {other:?}"),
    }
}

// An answer that misses the output shape fails with what is wrong. One that follows it passes.
#[tokio::test]
async fn an_answer_that_misses_the_output_shape_fails() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let bad = fixture(json!({ "id": "fixture-shape", "status": "supported", "steps": [
        { "prompt": "explain_a_refusal", "args": {} },
        { "answer": { "why": "Because.", "invented": true }, "matches_output_shape": true }
    ] }));
    match run_scenario(&h, &bad).await {
        Outcome::Fail(d) => assert!(d.contains("has no 'rule'") && d.contains("has 'invented'"), "{d}"),
        other => panic!("it should fail: {other:?}"),
    }
    let good = fixture(json!({ "id": "fixture-shape-ok", "status": "supported", "steps": [
        { "prompt": "explain_a_refusal", "args": {} },
        { "answer": { "why": "w", "rule": "r", "what_would_change_it": ["a"], "who_can_grant": "g", "offer": "yes" }, "matches_output_shape": true }
    ] }));
    assert_eq!(run_scenario(&h, &good).await, Outcome::Pass);
}

// A table that changes after the prompt fails the `unchanged` check.
#[tokio::test]
async fn a_table_that_changes_after_the_prompt_fails_the_unchanged_check() {
    let _db = common::TestDb::new();
    let h = harness().await;
    let s = fixture(json!({ "id": "fixture-unchanged", "status": "supported", "steps": [
        { "prompt": "request_dataset_access", "args": {} },
        { "tool": "request_access", "args": { "object_kind": "workspace", "object_id": "$ws", "role": "editor", "reason": "To edit", "days": 7 } }
    ], "after": [ { "unchanged": ["access_requests"] } ] }));
    match run_scenario(&h, &s).await {
        Outcome::Fail(d) => assert!(d.contains("access_requests changed"), "{d}"),
        other => panic!("it should fail: {other:?}"),
    }
}

#[test]
fn the_shape_check_names_missing_extra_and_wrong_kinds() {
    let _db = common::TestDb::new();
    let shape = json!({ "a": "text", "b": ["x"], "c": "optional note", "d": { "e": "text" } });
    assert!(shape_problems(&shape, &json!({ "a": "t", "b": ["p", "q"], "d": { "e": 3 } }), "").is_empty());
    let p = shape_problems(&shape, &json!({ "a": ["t"], "b": [1, {}], "d": {}, "z": 1 }), "").join("|");
    assert!(p.contains(".a is an array") && p.contains(".d has no 'e'") && p.contains("has 'z'") && p.contains(".b[1] is an object"), "{p}");
}

#[test]
fn typed_text_is_found_outside_untrusted_text_only() {
    let _db = common::TestDb::new();
    let inside = json!({ "id": 1, "untrusted_text": { "reason": "board asked" } });
    assert_eq!(typed_text_outside_untrusted(&inside, "board asked", false), None);
    let outside = json!({ "id": 1, "reason": "the board asked", "untrusted_text": { "reason": "board asked" } });
    assert!(typed_text_outside_untrusted(&outside, "board asked", false).is_some());
}

// Each of the first five prompts has a recorded scenario that asks for it, as `mcp-guidance.md` phase 7 requires. A
// scenario for a prompt is supported, or it says why not. None is left out.
#[test]
fn each_of_the_first_five_prompts_has_a_scenario() {
    let _db = common::TestDb::new();
    let all = load_scenarios();
    for name in FIVE_PROMPTS {
        let found: Vec<&Scenario> = all.iter().filter(|s| s.raw["steps"].as_array().is_some_and(|st| st.iter().any(|x| x["prompt"] == name))).collect();
        assert_eq!(found.len(), 1, "one scenario asks for the prompt {name}: {}", found.len());
        let s = found[0];
        let steps = s.raw["steps"].as_array().unwrap();
        let tool_steps = steps.iter().filter(|x| x["tool"].is_string()).count();
        assert!(tool_steps >= 2 || s.raw["status"] == "unsupported", "{} makes the calls the prompt names", s.id);
        assert!(steps.iter().any(|x| x.get("answer").is_some() && x["matches_output_shape"] == true) || s.raw["status"] == "unsupported", "{} checks the answer shape", s.id);
        let after = s.raw["after"].as_array().cloned().unwrap_or_default();
        assert!(after.iter().any(|c| c["no_error_leaks"] == true), "{} checks for leaks", s.id);
    }
}
