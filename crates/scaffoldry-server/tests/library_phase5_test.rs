//! Policy library phase 5: fetching web sources and noticing change.
//!
//! A `url` source is fetched through `net::egress` and the outbound allowlist, at most once a day. A snapshot is stored
//! only when the words change. When one is, every published rule that cites a passage the new text no longer holds is
//! marked `source_changed` and goes to its reviewers. Its status does not change. The tests reach a page server on this
//! machine through an injected resolver and an allowlist entry, as `integrations.md` phase 1 does.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::library_fetch::{self, Checked};
use scaffoldry_server::net::egress::{Allow, Client, Resolver};
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::ServerState;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const R1: &str = "reviewer.one@state.edu";
const R2: &str = "reviewer.two@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const P1: &str = "Student records are kept for seven years after the student leaves.";
const P2: &str = "Financial aid files are kept for five years after the award year ends.";
const HOST: &str = "policy.invalid";

/// What the page server answers: status, content type, body, and a `Location` for a redirect.
#[derive(Clone)]
struct Page {
    status: u16,
    content_type: String,
    body: Vec<u8>,
    location: Option<String>,
}

fn html(body: &str) -> Page {
    Page { status: 200, content_type: "text/html; charset=utf-8".into(), body: body.as_bytes().to_vec(), location: None }
}

/// A page server on this machine. It answers every request with the current page and counts connections.
struct PageServer {
    port: u16,
    page: Arc<Mutex<Page>>,
    connections: Arc<AtomicUsize>,
}

impl PageServer {
    fn start(page: Page) -> PageServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let page = Arc::new(Mutex::new(page));
        let connections = Arc::new(AtomicUsize::new(0));
        let (p, n) = (page.clone(), connections.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                n.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf);
                let page = p.lock().unwrap().clone();
                let reason = if page.status == 200 { "OK" } else { "Other" };
                let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n", page.status, page.content_type, page.body.len());
                if let Some(l) = &page.location {
                    head.push_str(&format!("Location: {l}\r\n"));
                }
                head.push_str("\r\n");
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&page.body);
            }
        });
        PageServer { port, page, connections }
    }
    fn set(&self, page: Page) {
        *self.page.lock().unwrap() = page;
    }
    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// Every name is this machine.
struct Here;
impl Resolver for Here {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec!["127.0.0.1".parse().unwrap()])
    }
}

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {}", issue_test_token_and_user(who)));
    let req = match body {
        Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

struct World {
    db: TestDb,
    state: Arc<ServerState>,
    app: axum::Router,
    server: PageServer,
    policy: String,
    source: uuid::Uuid,
    /// Rule `one` cites P1. Rule `two` cites P2.
    one: String,
    two: String,
}

impl World {
    /// The client a check uses: this server's allowlist, and a resolver that answers this machine.
    fn client(&self) -> Client {
        let settings = self.state.settings.read().unwrap().clone();
        Client::new(Allow::from_settings(&settings), Arc::new(Here))
    }
    async fn check(&self) -> Checked {
        let (state, source, client) = (self.state.clone(), self.source, self.client());
        tokio::task::spawn_blocking(move || library_fetch::check_source(&state, source, &client)).await.unwrap().expect("the check runs")
    }
    fn snapshots(&self) -> i64 {
        let s = self.source;
        self.db.run(move |c| c.query_one("SELECT count(*) FROM library_snapshots WHERE source_id = $1", &[&s]).unwrap().get(0))
    }
    fn rule(&self, id: &str) -> (String, bool) {
        let id = uuid::Uuid::parse_str(id).unwrap();
        self.db.run(move |c| {
            let r = c.query_one("SELECT status, source_changed FROM library_rules WHERE id = $1", &[&id]).unwrap();
            (r.get(0), r.get(1))
        })
    }
    fn source_row(&self) -> (Option<String>, i32) {
        let s = self.source;
        self.db.run(move |c| {
            let r = c.query_one("SELECT last_status, consecutive_failures FROM library_sources WHERE id = $1", &[&s]).unwrap();
            (r.get(0), r.get(1))
        })
    }
    async fn allow(&self, entries: Value) {
        let (s, b) = send(&self.app, "PUT", "/api/v1/settings/egress.allow", ADMIN, Some(entries)).await;
        assert!(s.is_success(), "{b}");
    }
}

fn page_text(extra_script: &str, p1: &str) -> String {
    format!("<html><head><script>{extra_script}</script></head><body><h1>Schedule</h1><p>{p1}</p>\n<p>{P2}</p><p>Questions go to the records office.</p></body></html>")
}

/// A policy owned by a records-officer position held by R1 and R2, with a `url` source on the page server, fetched
/// once. Two rules, drafted by R1 and published by R2, cite one passage each.
async fn world() -> World {
    let db = TestDb::new();
    let state = db.state();
    let app = build_app_with_state(state.clone()).expect("router");
    for who in [R1, R2] {
        issue_test_token_and_user(who);
    }
    let server = PageServer::start(html(&page_text("var a = 1;", P1)));
    let code = format!("L{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let (s, unit) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
    assert_eq!(s, StatusCode::CREATED, "{unit}");
    let unit = unit["id"].as_str().unwrap().to_string();
    let key = format!("records-officer-{}", code.to_lowercase());
    let (s, p) = send(&app, "POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Records officer", "description": "Test", "org_types": ["Department"], "max_holders": 5, "reason": "Define" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    for h in [R1, R2] {
        let (s, b) = send(&app, "POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": h, "replace": false, "reason": "Assign" }))).await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let body = json!({ "key": "records", "title": "Records retention", "organization_id": unit, "owner": { "Position": { "key": key, "from": "Workspace", "walk_up": false } }, "reason": "Owned by the records officers" });
    let (s, p) = send(&app, "POST", "/api/v1/admin/library/policies", ADMIN, Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let policy = p["policy"]["id"].as_str().unwrap().to_string();
    // A source is an https address. The page server here speaks plain HTTP, so the stored address is rewritten to it.
    let (s, src) = send(&app, "POST", &format!("/api/v1/admin/library/policies/{policy}/sources"), ADMIN, Some(json!({ "title": "Schedule", "kind": "url", "href": format!("https://{HOST}/schedule"), "reason": "r" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{src}");
    let source = uuid::Uuid::parse_str(src["source"]["id"].as_str().unwrap()).unwrap();
    let href = format!("http://{HOST}:{}/schedule", server.port);
    db.run(move |c| {
        c.execute("UPDATE library_sources SET href = $2 WHERE id = $1", &[&source, &href]).unwrap();
    });
    let mut w = World { db, state, app, server, policy, source, one: String::new(), two: String::new() };
    w.allow(json!([{ "host": HOST, "ports": [w.server.port] }, { "cidr": "127.0.0.0/8", "ports": [w.server.port] }])).await;
    let first = w.check().await;
    assert!(first.stored, "the first fetch stores a snapshot");
    let s = w.source;
    let snapshot: uuid::Uuid = w.db.run(move |c| c.query_one("SELECT id FROM library_snapshots WHERE source_id = $1", &[&s]).unwrap().get(0));
    for (key, passage) in [("one", P1), ("two", P2)] {
        let body = json!({ "key": key, "statement": format!("Rule {key}."), "applies_to": "records", "citations": [{ "snapshot_id": snapshot, "passage": passage }], "reason": "Read from the schedule" });
        let (s, r) = send(&w.app, "POST", &format!("/api/v1/admin/library/policies/{}/rules", w.policy), R1, Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{r}");
        let id = r["rule"]["id"].as_str().unwrap().to_string();
        let (s, r) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/submit"), R1, Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        let (s, r) = send(&w.app, "POST", &format!("/api/v1/library/rules/{id}/decide"), R2, Some(json!({ "decision": "publish", "note": "Matches" }))).await;
        assert_eq!(s, StatusCode::OK, "{r}");
        if key == "one" { w.one = id } else { w.two = id }
    }
    w
}

// 1. A page that changes only in a script block and in white space stores no new snapshot.
#[tokio::test]
async fn a_change_only_in_scripts_and_spacing_stores_no_snapshot() {
    let w = world().await;
    let before = w.snapshots();
    w.server.set(html(&page_text("var a = 2; track();", &P1.replace(' ', "  \n "))));
    let checked = w.check().await;
    assert!(!checked.stored);
    assert_eq!(checked.status, "ok");
    assert_eq!(w.snapshots(), before);
    assert_eq!(w.rule(&w.one), ("Published".to_string(), false));
}

// 2. A page that changes one cited sentence stores a snapshot, marks exactly the rules that cite it, and leaves the others.
#[tokio::test]
async fn a_changed_cited_sentence_marks_exactly_the_rules_that_cite_it() {
    let w = world().await;
    w.server.set(html(&page_text("var a = 1;", "Student records are kept for ten years after the student leaves.")));
    let checked = w.check().await;
    assert!(checked.stored);
    assert_eq!(w.snapshots(), 2);
    assert_eq!(checked.marked, vec![uuid::Uuid::parse_str(&w.one).unwrap()]);
    assert_eq!(w.rule(&w.one), ("Published".to_string(), true), "marked, and still published");
    assert_eq!(w.rule(&w.two), ("Published".to_string(), false));
    let entries: i64 = w.db.run(|c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = 'PolicyLibraryChanged' AND payload::text LIKE '%source_changed%'", &[]).unwrap().get(0));
    assert_eq!(entries, 1);
    // R1 submitted it, so R2 is its reviewer, and it is on R2's list.
    let (s, mine) = send(&w.app, "GET", "/api/v1/me/library/reviews", R2, None).await;
    assert_eq!(s, StatusCode::OK);
    let changed = mine["source_changed"].as_array().expect("a list of rules whose source changed");
    assert!(changed.iter().any(|r| r["rule_id"] == w.one.as_str()), "{mine}");
    assert!(!changed.iter().any(|r| r["rule_id"] == w.two.as_str()));
}

// 3. A host not on the allowlist is never connected to. The test server counts connections.
#[tokio::test]
async fn a_host_off_the_allowlist_is_never_connected_to() {
    let w = world().await;
    w.allow(json!([])).await;
    let before = w.server.connections();
    let checked = w.check().await;
    assert_eq!(checked.status, "not_allowed");
    assert_eq!(w.server.connections(), before, "no connection was made");
    assert_eq!(w.source_row().0.as_deref(), Some("not_allowed"));
    // The policy's page says so, and what to do.
    let (s, p) = send(&w.app, "GET", &format!("/api/v1/admin/library/policies/{}", w.policy), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK);
    let src = p["sources"].as_array().unwrap().iter().find(|s| s["id"] == w.source.to_string().as_str()).cloned().unwrap();
    assert_eq!(src["last_status"], "not_allowed");
    assert!(src["remedy"].as_str().unwrap_or_default().contains("Platform Admin"), "{src}");
}

// 4. Three failures mark the source unreachable. No rule changes status.
#[tokio::test]
async fn three_failures_mark_the_source_unreachable_and_no_rule_changes() {
    let w = world().await;
    let before = w.snapshots();
    w.server.set(Page { status: 500, content_type: "text/plain".into(), body: b"down".to_vec(), location: None });
    for n in 1..=3 {
        let checked = w.check().await;
        assert!(!checked.stored);
        assert_eq!(w.source_row().1, n);
    }
    assert_eq!(w.source_row().0.as_deref(), Some("unreachable"));
    assert_eq!(w.snapshots(), before, "the latest snapshot stays");
    assert_eq!(w.rule(&w.one), ("Published".to_string(), false));
    assert_eq!(w.rule(&w.two), ("Published".to_string(), false));
    // A fetch that works again resets the count.
    w.server.set(html(&page_text("var a = 1;", P1)));
    w.check().await;
    assert_eq!(w.source_row(), (Some("ok".to_string()), 0));
}

// 5. A response of 3 MB is cut off and recorded as a failure. A redirect to a host off the allowlist is refused.
#[tokio::test]
async fn a_response_over_the_limit_fails_and_a_redirect_off_the_list_is_refused() {
    let w = world().await;
    let before = w.snapshots();
    w.server.set(Page { status: 200, content_type: "text/html".into(), body: vec![b'a'; 3 * 1024 * 1024], location: None });
    let checked = w.check().await;
    assert!(!checked.stored);
    assert_eq!(w.source_row(), (Some("failed".to_string()), 1));

    let elsewhere = PageServer::start(html("<p>elsewhere</p>"));
    w.server.set(Page { status: 302, content_type: "text/html".into(), body: Vec::new(), location: Some(format!("http://elsewhere.invalid:{}/", elsewhere.port)) });
    let checked = w.check().await;
    assert!(!checked.stored);
    assert_eq!(checked.status, "not_allowed");
    assert_eq!(elsewhere.connections(), 0, "the redirect target was not reached");
    assert_eq!(w.snapshots(), before);
}

#[tokio::test]
async fn a_redirect_to_a_host_on_the_list_is_followed_within_three_hops() {
    let w = world().await;
    let target = PageServer::start(html(&page_text("", "Student records are kept for nine years after the student leaves.")));
    w.allow(json!([{ "host": HOST, "ports": [w.server.port] }, { "host": "moved.invalid", "ports": [target.port] }, { "cidr": "127.0.0.0/8", "ports": [w.server.port, target.port] }])).await;
    w.server.set(Page { status: 301, content_type: "text/html".into(), body: Vec::new(), location: Some(format!("http://moved.invalid:{}/schedule", target.port)) });
    let checked = w.check().await;
    assert!(checked.stored, "{:?}", checked.status);
    assert_eq!(target.connections(), 1);
}

// 6. A `source_changed` rule stays Published and stays in /library, with a note that its source changed on a date.
#[tokio::test]
async fn a_rule_whose_source_changed_stays_in_the_library_with_a_note() {
    let w = world().await;
    w.server.set(html(&page_text("", "Student records are kept for ten years after the student leaves.")));
    w.check().await;
    let (s, lib) = send(&w.app, "GET", "/api/v1/library", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK);
    let rule = lib["rules"].as_array().unwrap().iter().find(|r| r["id"] == w.one.as_str()).cloned().expect("still listed");
    assert_eq!(rule["source_changed"], true);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    assert_eq!(rule["note"], format!("Its source changed on {today}."), "{rule}");
    let other = lib["rules"].as_array().unwrap().iter().find(|r| r["id"] == w.two.as_str()).cloned().unwrap();
    assert_eq!(other["source_changed"], false);
    assert!(other["note"].is_null());
}

// 7. A second check inside a minute is 429.
#[tokio::test]
async fn a_second_check_inside_a_minute_is_refused() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/sources/{}/check", w.source);
    // The route uses the system resolver, which cannot find the test host. The check still runs, and fails.
    let (s, b) = send(&w.app, "POST", &uri, R1, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = send(&w.app, "POST", &uri, R1, Some(json!({}))).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{b}");
    assert_eq!(b["code"], "rate_limited");
    // Someone who may not write the policy may not check it.
    let (s, _) = send(&w.app, "POST", &uri, "prof.curie@science.state.edu", Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 8. A PDF response stores a snapshot with empty text and `needs_text`, and no rule is marked.
#[tokio::test]
async fn a_pdf_stores_an_empty_snapshot_that_needs_text_and_marks_nothing() {
    let w = world().await;
    w.server.set(Page { status: 200, content_type: "application/pdf".into(), body: b"%PDF-1.7 binary".to_vec(), location: None });
    let checked = w.check().await;
    assert!(checked.stored);
    assert!(checked.marked.is_empty());
    assert_eq!(w.source_row().0.as_deref(), Some("needs_text"));
    let s = w.source;
    let (media, text): (String, String) = w.db.run(move |c| {
        let r = c.query_one("SELECT media_type, text FROM library_snapshots WHERE source_id = $1 ORDER BY taken_at DESC, id DESC LIMIT 1", &[&s]).unwrap();
        (r.get(0), r.get(1))
    });
    assert_eq!((media.as_str(), text.as_str()), ("application/pdf", ""));
    assert_eq!(w.rule(&w.one), ("Published".to_string(), false));
    assert_eq!(w.rule(&w.two), ("Published".to_string(), false));
}

// The hourly job fetches only sources that are due, and a source is fetched at most once a day.
#[tokio::test]
async fn the_job_fetches_only_due_sources_and_each_at_most_once_a_day() {
    let w = world().await;
    let s = w.source;
    w.db.run(move |c| {
        c.execute("UPDATE library_sources SET next_check_at = NOW() - INTERVAL '1 minute' WHERE id = $1", &[&s]).unwrap();
    });
    let (state, client) = (w.state.clone(), w.client());
    let ran = tokio::task::spawn_blocking(move || library_fetch::check_due(&state, &client)).await.unwrap().unwrap();
    assert_eq!(ran, 1);
    let next: bool = w.db.run(move |c| c.query_one("SELECT next_check_at > NOW() + INTERVAL '23 hours' FROM library_sources WHERE id = $1", &[&s]).unwrap().get(0));
    assert!(next, "the next check is a day later");
    let (state, client) = (w.state.clone(), w.client());
    let again = tokio::task::spawn_blocking(move || library_fetch::check_due(&state, &client)).await.unwrap().unwrap();
    assert_eq!(again, 0, "not again the same day");
    assert!(scaffoldry_server::jobs::JOB_KINDS.iter().any(|k| k.name == "library_fetch"));
}

// A reviewer clears the mark by confirming with a note. A new citation is needed when the old passage is gone.
#[tokio::test]
async fn a_reviewer_clears_the_mark_by_confirming_with_a_new_citation() {
    let w = world().await;
    let changed = "Student records are kept for ten years after the student leaves.";
    w.server.set(html(&page_text("", changed)));
    w.check().await;
    let uri = format!("/api/v1/library/rules/{}/confirm", w.one);
    let (s, b) = send(&w.app, "POST", &uri, R2, Some(json!({ "note": "Still right" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "the old passage is gone, so a new citation is needed: {b}");
    let (s, _) = send(&w.app, "POST", &uri, R1, Some(json!({ "note": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the submitter is not its reviewer");
    let src = w.source;
    let latest: uuid::Uuid = w.db.run(move |c| c.query_one("SELECT id FROM library_snapshots WHERE source_id = $1 ORDER BY taken_at DESC, id DESC LIMIT 1", &[&src]).unwrap().get(0));
    let (s, b) = send(&w.app, "POST", &uri, R2, Some(json!({ "note": "The term changed; the rule is reworded next", "citations": [{ "snapshot_id": latest, "passage": changed }] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(w.rule(&w.one), ("Published".to_string(), false));
}

// Review dates: a policy is due when its review interval has passed. Marking it reviewed needs a note and a second person.
#[tokio::test]
async fn marking_a_policy_reviewed_needs_a_note_and_another_person() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/reviewed", w.policy);
    let (s, _) = send(&w.app, "POST", &uri, R1, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a note is required");
    let (s, b) = send(&w.app, "POST", &uri, R1, Some(json!({ "note": "Read it through" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _) = send(&w.app, "POST", &uri, R1, Some(json!({ "note": "Again" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the person who marked it last cannot mark it again");
    let (s, b) = send(&w.app, "POST", &uri, R2, Some(json!({ "note": "Second look" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/admin/library/policies/{}", w.policy), ADMIN, None).await;
    assert_eq!(p["policy"]["due"], false);
    let pid = uuid::Uuid::parse_str(&w.policy).unwrap();
    w.db.run(move |c| {
        c.execute("UPDATE library_policies SET last_reviewed_at = NOW() - INTERVAL '400 days', review_every_days = 365 WHERE id = $1", &[&pid]).unwrap();
    });
    let (_, p) = send(&w.app, "GET", &format!("/api/v1/admin/library/policies/{}", w.policy), ADMIN, None).await;
    assert_eq!(p["policy"]["due"], true);
}
