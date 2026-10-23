//! Agent coverage phase 4: an AI tool is set up in one step.
//!
//! `get_agent_kit` returns an AGENTS.md section between markers, a skill folder, and the server version and the date of
//! the guides in every file. The kit writes nothing and holds no token, address or record value.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const OWNER: &str = "einstein@physics.state.edu";
const BEGIN: &str = "<!-- scaffoldry:begin -->";
const END: &str = "<!-- scaffoldry:end -->";

struct World {
    _db: common::TestDb,
    app: Router,
    owner: String,
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let app = build_app_with_state(state).expect("router");
    World { _db: db, app, owner: issue_test_token_and_user(OWNER) }
}

async fn send(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

async fn kit(w: &World) -> Value {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "get_agent_kit", "arguments": {} } });
    let (_, v) = send(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    assert!(v["result"]["isError"] != true, "{v}");
    v["result"]["structuredContent"].clone()
}

/// What an agent does with the kit's section: replace the text between the markers, or add the section at the end when
/// the file has none. Text outside the markers is kept exactly.
fn apply(existing: &str, section: &str) -> String {
    match (existing.find(BEGIN), existing.find(END)) {
        (Some(start), Some(end)) if start < end => format!("{}{}{}", &existing[..start], section.trim_end_matches('\n'), &existing[end + END.len()..]),
        _ => format!("{}{}{}\n", existing, if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" }, section.trim_end_matches('\n')),
    }
}

// 1. The three parts are present and each states the server version.
#[tokio::test]
async fn the_kit_has_three_parts_and_each_states_the_version() {
    let w = world().await;
    let kit = kit(&w).await;
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(kit["version"], version);
    assert!(kit["guides_date"].as_str().is_some_and(|d| d.len() == 10), "a date: {}", kit["guides_date"]);

    let section = kit["agents_md"].as_str().expect("an AGENTS.md section");
    assert!(section.starts_with(BEGIN) && section.trim_end().ends_with(END), "{section}");
    assert_eq!(section.matches(BEGIN).count(), 1);
    assert_eq!(section.matches(END).count(), 1);
    assert!(section.len() <= 2048, "at most 2 KB: {}", section.len());
    for needle in ["propose", "approval", "list_workspaces", "scaffoldry://guide/building-apps", "sensitive"] {
        assert!(section.contains(needle), "the section says {needle}");
    }
    assert!(section.contains(&format!("scaffoldry-version: {version}")) && section.contains("guides-date: "), "{section}");

    let skill = &kit["skill"];
    let paths: Vec<&str> = skill["paths"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(paths, [".claude/skills/scaffoldry/", ".agents/skills/scaffoldry/"]);
    let files = skill["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().filter_map(|f| f["path"].as_str()).collect();
    assert!(names.contains(&"SKILL.md") && names.contains(&"references/building-apps.md"), "{names:?}");
    for f in files {
        let content = f["content"].as_str().unwrap();
        assert!(content.contains(&format!("scaffoldry-version: {version}")) && content.contains("guides-date: "), "{} has no header", f["path"]);
    }
    let skill_md = files.iter().find(|f| f["path"] == "SKILL.md").unwrap()["content"].as_str().unwrap();
    assert!(skill_md.starts_with("---\nname: scaffoldry\ndescription: "), "{skill_md}");
    let description = skill_md.lines().nth(2).unwrap();
    assert!(description.len() < 300 && skill_md.lines().nth(3) == Some("---"), "a one-line description: {description}");

    // The guide in the kit is the guide the server serves.
    let guide = files.iter().find(|f| f["path"] == "references/building-apps.md").unwrap()["content"].as_str().unwrap();
    assert!(guide.contains("# Building an app in Scaffoldry"));

    // The files are listed before anything is written, so the person can be shown them.
    let plan: Vec<&str> = kit["files"].as_array().unwrap().iter().filter_map(|f| f["path"].as_str()).collect();
    assert_eq!(plan[0], "AGENTS.md");
    assert!(plan.contains(&".claude/skills/scaffoldry/SKILL.md") && plan.contains(&".agents/skills/scaffoldry/SKILL.md"), "{plan:?}");

    let (status, over_rest) = send(&w.app, "GET", "/api/v1/agent-kit", &w.owner, None).await;
    assert_eq!(status, StatusCode::OK, "{over_rest}");
    assert_eq!(over_rest, kit, "the route returns what the tool returns");
}

// 2. Applying the section changes only the text between the markers, and twice changes nothing more.
#[tokio::test]
async fn applying_the_section_changes_only_what_is_between_the_markers() {
    let w = world().await;
    let section = kit(&w).await["agents_md"].as_str().unwrap().to_string();
    let above = "# My project\n\nRules of this repo.\n\n";
    let below = "\n\n## Other notes\nKeep these exactly.\n";
    let old = format!("{above}{BEGIN}\nold text from an earlier version\n{END}{below}");
    let once = apply(&old, &section);
    assert!(once.starts_with(above) && once.ends_with(below), "text outside the markers is kept exactly");
    assert!(once.contains(&section.trim_end().to_string()) && !once.contains("old text from an earlier version"));
    assert_eq!(apply(&once, &section), once, "a second application changes nothing");

    let fresh = apply("# My project\n", &section);
    assert!(fresh.starts_with("# My project\n") && fresh.contains(BEGIN));
    assert_eq!(apply(&fresh, &section), fresh);
}

// 3. The kit holds no token, no address, and no record value.
#[tokio::test]
async fn the_kit_holds_no_token_no_address_and_no_record_value() {
    let w = world().await;
    let record = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_workspaces", "arguments": {} } });
    let _ = send(&w.app, "POST", "/api/mcp", &w.owner, Some(record)).await;
    let text = kit(&w).await.to_string();
    assert!(!text.contains("scf_"), "no token prefix");
    assert!(!text.to_lowercase().contains(&OWNER.to_lowercase()) && !text.contains("einstein"), "no address of the caller");
    assert!(!text.contains(&w.owner), "no bearer token");
    assert!(!text.contains("localhost") && !text.contains("http://") && !text.contains("https://"), "no server address");
}

// The prompt tells the agent to call the tool, show the files, and ask before writing.
#[tokio::test]
async fn the_set_up_prompt_asks_before_it_writes() {
    let w = world().await;
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": "prompts/list", "params": {} });
    let (_, listed) = send(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    let names: Vec<&str> = listed["result"]["prompts"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    assert!(names.contains(&"set_up_this_project"), "{names:?}");
    let payload = json!({ "jsonrpc": "2.0", "id": 2, "method": "prompts/get", "params": { "name": "set_up_this_project", "arguments": {} } });
    let (_, got) = send(&w.app, "POST", "/api/mcp", &w.owner, Some(payload)).await;
    let text = got["result"]["messages"][0]["content"]["text"].as_str().expect("a prompt");
    for needle in ["get_agent_kit", "ask", "between the markers", "<!-- scaffoldry:begin -->"] {
        assert!(text.contains(needle), "the prompt says {needle}: {text}");
    }
    let ask = text.find("ask").unwrap();
    let write = text.find("Write").or_else(|| text.find("write")).unwrap();
    assert!(text.find("get_agent_kit").unwrap() < ask && ask <= write + 40, "it calls the tool, then asks, then writes");
}
