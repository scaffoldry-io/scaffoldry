//! Support for the two-instance suite: the servers, the list of what must have a case, and the check.
//! Used by `two_instance_test.rs` and `two_instance_files_test.rs`.
//!
//! Stateless server phase 6: every write is proven across two instances.
//!
//! For every route in the router that writes, and every tool in `TOOLS` that is not read-only, there is one case. A case
//! writes through instance A and reads the result through instance B, which is a second server on the same database.
//! The list of what must have a case is built from the router and from the registry. A write route or tool with no case
//! fails `every_write_route_and_tool_has_a_case` by name, and a case for something that is not there fails too.
//!
//! A route that changes nothing (a preview, a simulation, a check) has an `exempt` entry with its reason, so the list
//! of exemptions is read in review and is not silent.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use crate::common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use tower::ServiceExt;

pub const ADMIN: &str = "jordan.lee@state.edu";
pub const OWNER: &str = "einstein@physics.state.edu";
pub const EDITOR: &str = "editor.p6@physics.state.edu";
pub const WORKSPACE: &str = "ws-physics-optics";
pub const ROOT: &str = "00000000-0000-0000-0000-000000000001";

/// Two servers on one database.
pub struct Servers {
    pub db: TestDb,
    pub a: Router,
    pub b: Router,
}

/// A handle on the two servers and the helpers every case uses. It is cheap to clone, so a case owns its handle.
#[derive(Clone)]
pub struct World(std::sync::Arc<Servers>);

impl std::ops::Deref for World {
    type Target = Servers;
    fn deref(&self) -> &Servers {
        &self.0
    }
}

impl World {
    pub async fn call(app: &Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        Self::call_with(app, method, uri, &issue_test_token_and_user(who), body).await
    }

    pub async fn call_with(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
        let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
        let req = match body {
            Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A request through A.
    pub async fn wa(&self, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        Self::call(&self.a, method, uri, who, body).await
    }

    /// A request through B.
    pub async fn rb(&self, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
        Self::call(&self.b, method, uri, who, body).await
    }

    /// A tool call through `app`. `Err` is the tool's own error text.
    pub async fn tool(app: &Router, who: &str, name: &str, args: Value) -> Result<Value, String> {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
        let (_, v) = Self::call(app, "POST", "/api/mcp", who, Some(payload)).await;
        if let Some(err) = v.get("error") {
            return Err(err["message"].to_string());
        }
        let result = &v["result"];
        if result["isError"] == true {
            return Err(result["content"][0]["text"].to_string());
        }
        Ok(result["structuredContent"].clone())
    }

    pub async fn tool_a(&self, who: &str, name: &str, args: Value) -> Result<Value, String> {
        Self::tool(&self.a, who, name, args).await
    }

    pub async fn tool_b(&self, who: &str, name: &str, args: Value) -> Result<Value, String> {
        Self::tool(&self.b, who, name, args).await
    }
}

/// Two servers on a new database, with the demo data and the shared fixtures.
pub async fn world() -> World {
    let db = TestDb::new();
    let a = {
        let state = db.state();
        state.seed_demo().expect("seed");
        scaffoldry_server::build_app_with_state(state).expect("router")
    };
    let b = db.second_app();
    let world = World(std::sync::Arc::new(Servers { db, a, b }));
    cases::prepare(&world).await.expect("the shared fixtures");
    world
}

/// Runs every case `pick` selects and returns how many ran and which failed.
pub async fn run_cases(world: &World, pick: impl Fn(&Entry) -> Option<&Case>) -> (usize, Vec<String>) {
    let entries = cases::entries();
    let (mut ran, mut failures) = (0, Vec::new());
    for (key, entry) in &entries {
        if let Some(case) = pick(entry) {
            ran += 1;
            if let Err(why) = case(world.clone()).await {
                failures.push(format!("{key}: {why}"));
            }
        }
    }
    (ran, failures)
}

/// A case passes or says why not.
pub type Outcome = Result<(), String>;
pub type Case = Box<dyn Fn(World) -> Pin<Box<dyn Future<Output = Outcome>>>>;

/// What the test knows about one write route or tool.
pub enum Entry {
    /// Writes through A, reads the result through B.
    Case(Case),
    /// The same, for a route that stores a file or reads one back, so it needs `SCAFFOLDRY_FILES_DIR`. A test may not
    /// set that for the others, so these run in `two_instance_files_test.rs`, a binary with one test.
    FilesCase(Case),
    /// Changes nothing that is stored. The text says why. A person reads these.
    Exempt(&'static str),
}

pub fn ensure(condition: bool, what: impl Into<String>) -> Outcome {
    if condition { Ok(()) } else { Err(what.into()) }
}

pub fn status(got: (StatusCode, Value), want: StatusCode, what: &str) -> Result<Value, String> {
    if got.0 == want { Ok(got.1) } else { Err(format!("{what}: expected {want}, got {} {}", got.0, got.1)) }
}

pub fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

// ---------------------------------------------------------------------------------------------------------------------
// What is in the router and the registry
// ---------------------------------------------------------------------------------------------------------------------

/// Every `METHOD path` in the router that can change something: POST, PUT, PATCH and DELETE.
///
/// axum has no call that lists routes. Its `Debug` output does: each route id has the methods set on it, and the path
/// router lists the path of each id. This reads that. If axum changes the format the test fails on `found` below, not
/// silently.
pub fn write_routes_of(router: &Router) -> BTreeSet<String> {
    let text = format!("{router:?}");
    let paths_at = text.find("node: Node { paths: {").expect("the router lists its paths");
    let paths_end = paths_at + text[paths_at..].find("} }").expect("the end of the path list");
    let mut path_of = std::collections::HashMap::new();
    for part in text[paths_at..paths_end].split("RouteId(").skip(1) {
        let (id, rest) = part.split_once("): \"").expect("a route id and its path");
        path_of.insert(id.to_string(), rest.split('"').next().unwrap().to_string());
    }
    let mut found = BTreeSet::new();
    for part in text[..paths_at].split("RouteId(").skip(1) {
        let Some((id, rest)) = part.split_once("): MethodRouter(MethodRouter {") else { continue };
        let methods = rest.split("fallback:").next().unwrap();
        for (field, name) in [("post", "POST"), ("put", "PUT"), ("patch", "PATCH"), ("delete", "DELETE")] {
            // A method that is set prints as `Route`, or as `BoxedHandler` for a route added to a built router.
            if methods.contains(&format!("{field}: ")) && !methods.contains(&format!("{field}: None")) {
                found.insert(format!("{name} {}", path_of.get(id).expect("every route has a path")));
            }
        }
    }
    assert!(found.len() > 50, "the route list was not read: {} write routes found", found.len());
    found
}

/// Every tool that is not read-only, as `tool:name`.
pub fn write_tools() -> BTreeSet<String> {
    scaffoldry_server::service::tools::TOOLS.iter().filter(|t| !t.read_only).map(|t| format!("tool:{}", t.name)).collect()
}

/// What has no entry: the routes and tools in `router` and the registry that `entries` does not name, and the entries
/// that name neither.
pub fn gaps(router: &Router, entries: &std::collections::BTreeMap<String, Entry>) -> (Vec<String>, Vec<String>) {
    let mut wanted = write_routes_of(router);
    wanted.extend(write_tools());
    let missing = wanted.iter().filter(|k| !entries.contains_key(*k)).cloned().collect();
    let stale = entries.keys().filter(|k| !wanted.contains(*k)).cloned().collect();
    (missing, stale)
}

pub mod cases;
