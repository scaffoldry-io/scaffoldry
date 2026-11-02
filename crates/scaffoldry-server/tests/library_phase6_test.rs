//! Policy library phase 6: the state of the library.
//!
//! `GET /admin/library/health` answers four questions with counts and links: which policies have no published rule,
//! which published rules are only guidance, which sources are unreachable, changed or need text, and which policies are
//! past their review date or have had a question open for more than 30 days. Each list is one SQL statement. An Org
//! Unit Admin sees the policies of their own units, and no others.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN: &str = "jordan.lee@state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";
const PASSAGE: &str = "Student records are kept for seven years after the student leaves.";

async fn send(app: &axum::Router, method: &str, uri: &str, who: &str, body: Option<Value>) -> (StatusCode, Value) {
    send_as(app, method, uri, &issue_test_token_and_user(who), body).await
}

/// A request with a token minted beforehand, so a timed call times the route and not the minting.
async fn send_as(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn root() -> Uuid {
    Uuid::parse_str(ROOT).unwrap()
}

/// A policy written straight to the table. `reviewed_days_ago` None means never reviewed.
fn policy(c: &mut postgres::Client, key: &str, unit: Uuid, reviewed_days_ago: Option<i32>) -> Uuid {
    let id = Uuid::new_v4();
    let owner = json!({ "Person": { "eppn": ADMIN } });
    c.execute(
        "INSERT INTO library_policies (id, key, title, organization_id, owner, review_every_days, last_reviewed_at, created_by) \
         VALUES ($1, $2, $2, $3, $4, 365, CASE WHEN $5::int IS NULL THEN NULL ELSE NOW() - make_interval(days => $5::int) END, 'seed')",
        &[&id, &key, &unit, &owner, &reviewed_days_ago],
    )
    .unwrap();
    id
}

fn rule(c: &mut postgres::Client, policy: Uuid, key: &str, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    c.execute(
        "INSERT INTO library_rules (id, policy_id, key, version, statement, status, submitted_by, submitted_via) VALUES ($1, $2, $3, 1, 'A rule.', $4, 'seed', 'person')",
        &[&id, &policy, &key, &status],
    )
    .unwrap();
    id
}

fn source(c: &mut postgres::Client, policy: Uuid, kind: &str, last_status: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    c.execute(
        "INSERT INTO library_sources (id, policy_id, title, kind, href, created_by, last_status) VALUES ($1, $2, 'Source', $3::varchar, CASE WHEN $3::varchar = 'url' THEN 'https://policy.example.edu/' END, 'seed', $4)",
        &[&id, &policy, &kind, &last_status],
    )
    .unwrap();
    id
}

fn snapshot(c: &mut postgres::Client, source: Uuid, text: &str, origin: &str, days_ago: i32) -> Uuid {
    let id = Uuid::new_v4();
    c.execute(
        "INSERT INTO library_snapshots (id, source_id, taken_at, media_type, content_sha256, text, text_sha256, text_origin, taken_by) \
         VALUES ($1, $2, NOW() - make_interval(days => $5), 'text/plain', md5($3), $3, md5($3), $4, 'seed')",
        &[&id, &source, &text, &origin, &days_ago],
    )
    .unwrap();
    id
}

fn cite(c: &mut postgres::Client, rule: Uuid, snapshot: Uuid, passage: &str) {
    c.execute("INSERT INTO library_citations (id, rule_id, snapshot_id, passage) VALUES ($1, $2, $3, $4)", &[&Uuid::new_v4(), &rule, &snapshot, &passage]).unwrap();
}

fn question(c: &mut postgres::Client, policy: Uuid, days_ago: i32) {
    c.execute(
        "INSERT INTO library_questions (id, policy_id, question, asked_by, asked_via, asked_at) VALUES ($1, $2, 'Does a hold pause it?', 'seed', 'person', NOW() - make_interval(days => $3))",
        &[&Uuid::new_v4(), &policy, &days_ago],
    )
    .unwrap();
}

/// A binding to an authority row that exists, so the rule is enforced.
fn bind_live(c: &mut postgres::Client, policy: Uuid, rule_key: &str) {
    // One authority row per unit and tier. Every live binding here points at the root's.
    c.execute("INSERT INTO authority_rows (id, organization_id, tier, path) VALUES ($1, $2, 'Moderate', 'second_person') ON CONFLICT DO NOTHING", &[&Uuid::new_v4(), &root()]).unwrap();
    let row: Uuid = c.query_one("SELECT id FROM authority_rows WHERE organization_id = $1 AND tier = 'Moderate'", &[&root()]).unwrap().get(0);
    c.execute(
        "INSERT INTO library_bindings (id, rule_key_policy_id, rule_key, target_kind, target_ref, created_by) VALUES ($1, $2, $3, 'authority_row', $4, 'seed')",
        &[&Uuid::new_v4(), &policy, &rule_key, &row.to_string()],
    )
    .unwrap();
}

fn keys(list: &Value, field: &str) -> Vec<String> {
    list["items"].as_array().map(|l| l.iter().filter_map(|i| i[field].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

// 1. Each of the four lists contains its seeded case and not its neighbor.
#[tokio::test]
async fn each_list_holds_its_case_and_not_its_neighbor() {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    let t = &Uuid::new_v4().simple().to_string()[..6];
    let k = |s: &str| format!("{s}-{t}");
    let (no_rule, has_rule, guidance, enforced, bad_src, good_src, overdue, fresh, old_q, new_q) =
        (k("norule"), k("hasrule"), k("guide"), k("enforce"), k("badsrc"), k("goodsrc"), k("overdue"), k("fresh"), k("oldq"), k("newq"));
    let (nr, hr, g, e, bs, gs, od, fr, oq, nq) =
        (no_rule.clone(), has_rule.clone(), guidance.clone(), enforced.clone(), bad_src.clone(), good_src.clone(), overdue.clone(), fresh.clone(), old_q.clone(), new_q.clone());
    let changed_rule_policy = k("changed");
    let cr = changed_rule_policy.clone();
    let needs = k("needs");
    let nd = needs.clone();
    db.run(move |c| {
        // 1. No published rule: a draft only. Its neighbor has one published rule.
        let p = policy(c, &nr, root(), Some(1));
        rule(c, p, "draft-only", "Draft");
        let p = policy(c, &hr, root(), Some(1));
        let r = rule(c, p, "published", "Published");
        bind_live(c, p, "published");
        let _ = r;
        // 2. Guidance only: published and unbound. Its neighbor is bound to a row that exists.
        let p = policy(c, &g, root(), Some(1));
        rule(c, p, "unbound", "Published");
        let p = policy(c, &e, root(), Some(1));
        rule(c, p, "bound", "Published");
        bind_live(c, p, "bound");
        // 3. Sources: unreachable, needs text, and changed are listed. A source that is fine is not.
        let p = policy(c, &bs, root(), Some(1));
        let r = rule(c, p, "r", "Published");
        bind_live(c, p, "r");
        let _ = r;
        source(c, p, "url", Some("unreachable"));
        let p = policy(c, &nd, root(), Some(1));
        let r = rule(c, p, "r", "Published");
        bind_live(c, p, "r");
        let _ = r;
        source(c, p, "url", Some("needs_text"));
        let p = policy(c, &cr, root(), Some(1));
        let r = rule(c, p, "r", "Published");
        bind_live(c, p, "r");
        let s = source(c, p, "url", Some("ok"));
        let n = snapshot(c, s, PASSAGE, "fetched", 2);
        cite(c, r, n, PASSAGE);
        c.execute("UPDATE library_rules SET source_changed = TRUE, source_changed_at = NOW() WHERE id = $1", &[&r]).unwrap();
        let p = policy(c, &gs, root(), Some(1));
        let r = rule(c, p, "r", "Published");
        bind_live(c, p, "r");
        let _ = r;
        source(c, p, "url", Some("ok"));
        // 4. Past review, or a question open over 30 days. The neighbors were reviewed lately, or asked lately.
        for (key, reviewed, q) in [(&od, Some(400), None), (&fr, Some(10), None), (&oq, Some(10), Some(45)), (&nq, Some(10), Some(5))] {
            let p = policy(c, key, root(), reviewed);
            let r = rule(c, p, "r", "Published");
            bind_live(c, p, "r");
            let _ = r;
            if let Some(days) = q {
                question(c, p, days);
            }
        }
    });
    let (s, h) = send(&app, "GET", "/api/v1/admin/library/health", ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{h}");

    let none = keys(&h["no_published_rule"], "policy_key");
    assert!(none.contains(&no_rule) && !none.contains(&has_rule), "{none:?}");

    let guide = keys(&h["guidance_only"], "policy_key");
    assert!(guide.contains(&guidance) && !guide.contains(&enforced), "{guide:?}");

    let sources: Vec<(String, String)> = h["sources"]["items"].as_array().unwrap().iter().map(|i| (i["policy_key"].as_str().unwrap().to_string(), i["problem"].as_str().unwrap().to_string())).collect();
    assert!(sources.contains(&(bad_src.clone(), "unreachable".to_string())), "{sources:?}");
    assert!(sources.contains(&(needs.clone(), "needs_text".to_string())), "{sources:?}");
    assert!(sources.contains(&(changed_rule_policy.clone(), "changed".to_string())), "{sources:?}");
    assert!(!sources.iter().any(|(k, _)| *k == good_src), "{sources:?}");

    let review = keys(&h["review"], "policy_key");
    assert!(review.contains(&overdue) && !review.contains(&fresh), "{review:?}");
    assert!(review.contains(&old_q) && !review.contains(&new_q), "{review:?}");

    // Each list has a count, and each item a link a person can follow.
    for list in ["no_published_rule", "guidance_only", "sources", "review"] {
        assert!(h[list]["count"].as_u64().unwrap() >= 1, "{list}");
        assert!(h[list]["items"][0]["link"].as_str().is_some_and(|l| l.starts_with('/')), "{list}: {}", h[list]);
    }
}

// 2. The health route's time with 2,000 policies and 50,000 rules is recorded and under 200 ms.
#[tokio::test]
async fn the_health_route_answers_in_under_200_ms_at_2000_policies_and_50000_rules() {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    db.run(|c| {
        c.batch_execute(
            "INSERT INTO library_policies (id, key, title, organization_id, owner, review_every_days, last_reviewed_at, created_by) \
             SELECT gen_random_uuid(), 'scale-' || g, 'Policy ' || g, '00000000-0000-0000-0000-000000000001', '{\"Person\":{\"eppn\":\"jordan.lee@state.edu\"}}'::jsonb, 365, \
                    NOW() - make_interval(days => (g % 500)), 'seed' FROM generate_series(1, 2000) g;
             INSERT INTO library_rules (id, policy_id, key, version, statement, status, submitted_by, submitted_via) \
             SELECT gen_random_uuid(), p.id, 'rule-' || r, 1, 'A rule.', CASE WHEN r % 5 = 0 THEN 'Draft' ELSE 'Published' END, 'seed', 'person' \
             FROM library_policies p CROSS JOIN generate_series(1, 25) r WHERE p.key LIKE 'scale-%';
             INSERT INTO library_sources (id, policy_id, title, kind, href, created_by, last_status) \
             SELECT gen_random_uuid(), p.id, 'Source', 'url', 'https://policy.example.edu/' || p.key, 'seed', \
                    (ARRAY['ok', 'ok', 'ok', 'unreachable', 'needs_text', 'failed'])[1 + (abs(hashtext(p.key)) % 6)] FROM library_policies p WHERE p.key LIKE 'scale-%';
             INSERT INTO library_questions (id, policy_id, question, asked_by, asked_via, asked_at) \
             SELECT gen_random_uuid(), p.id, 'Q?', 'seed', 'person', NOW() - make_interval(days => abs(hashtext(p.key)) % 60) FROM library_policies p WHERE p.key LIKE 'scale-%';
             ANALYZE library_policies; ANALYZE library_rules; ANALYZE library_sources; ANALYZE library_questions; ANALYZE library_bindings; ANALYZE library_citations;",
        )
        .unwrap();
        let rules: i64 = c.query_one("SELECT count(*) FROM library_rules", &[]).unwrap().get(0);
        assert!(rules >= 50_000, "{rules} rules");
    });
    // The token is minted once, before the clock starts: the timing is the route's alone. One call warms the
    // connection and plans, then the median of five.
    let token = issue_test_token_and_user(ADMIN);
    let (s, h) = send_as(&app, "GET", "/api/v1/admin/library/health", &token, None).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let (s, _) = send_as(&app, "GET", "/api/v1/admin/library/health", &token, None).await;
        times.push(start.elapsed());
        assert_eq!(s, StatusCode::OK);
    }
    times.sort();
    let median = times[2];
    println!("library health at 2,000 policies and 50,000 rules: median {} ms, all {:?}", median.as_millis(), times.iter().map(|t| t.as_millis()).collect::<Vec<_>>());
    assert!(median < Duration::from_millis(200), "median {median:?}");
    // The policy list, with its counts, at the same size. Recorded, not held to the health screen's limit.
    let start = Instant::now();
    let (s, list) = send_as(&app, "GET", "/api/v1/admin/library/policies", &token, None).await;
    println!("policy list at the same size: {} ms", start.elapsed().as_millis());
    assert_eq!(s, StatusCode::OK);
    assert!(list["policies"].as_array().unwrap().len() >= 2000);
    // The lists are capped. Counts say how many there are.
    assert!(h["guidance_only"]["count"].as_u64().unwrap() >= 40_000);
    assert!(h["guidance_only"]["items"].as_array().unwrap().len() <= 50);
}

// 4. An Org Unit Admin sees the policies of their units and cannot open another unit's.
#[tokio::test]
async fn an_org_unit_admin_sees_their_units_policies_and_cannot_open_another() {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    let mut units = Vec::new();
    for _ in 0..2 {
        let code = format!("U{}", &Uuid::new_v4().simple().to_string()[..8]);
        let (s, u) = send(&app, "POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": code, "code": code, "org_type": "Department", "parent_id": ROOT }))).await;
        assert_eq!(s, StatusCode::CREATED, "{u}");
        units.push(Uuid::parse_str(u["id"].as_str().unwrap()).unwrap());
    }
    let (mine, theirs) = (units[0], units[1]);
    let unit_admin = "unit.admin.p6@state.edu";
    issue_test_token_and_user(unit_admin);
    db.run(move |c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, $2, 'Unit admin', 'unit_admin', true, 'api' FROM persons p WHERE lower(p.eppn) = lower($1)",
            &[&unit_admin, &mine],
        )
        .unwrap();
    });
    let t = &Uuid::new_v4().simple().to_string()[..6];
    let (my_key, their_key) = (format!("mine-{t}"), format!("theirs-{t}"));
    let mut ids = Vec::new();
    for (key, unit) in [(&my_key, mine), (&their_key, theirs)] {
        let body = json!({ "key": key, "title": key, "organization_id": unit, "owner": { "Person": { "eppn": ADMIN } }, "reason": "Seed" });
        let (s, p) = send(&app, "POST", "/api/v1/admin/library/policies", ADMIN, Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{p}");
        ids.push(p["policy"]["id"].as_str().unwrap().to_string());
    }
    let (s, list) = send(&app, "GET", "/api/v1/admin/library/policies", unit_admin, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let listed: Vec<&str> = list["policies"].as_array().unwrap().iter().filter_map(|p| p["key"].as_str()).collect();
    assert!(listed.contains(&my_key.as_str()), "{listed:?}");
    assert!(!listed.contains(&their_key.as_str()), "{listed:?}");
    let (s, _) = send(&app, "GET", &format!("/api/v1/admin/library/policies/{}", ids[0]), unit_admin, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = send(&app, "GET", &format!("/api/v1/admin/library/policies/{}", ids[1]), unit_admin, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Reading is not writing: the unit admin cannot change the policy.
    let (s, _) = send(&app, "PATCH", &format!("/api/v1/admin/library/policies/{}", ids[0]), unit_admin, Some(json!({ "title": "Renamed", "reason": "Mine" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // The health screen holds only their units.
    let (s, h) = send(&app, "GET", "/api/v1/admin/library/health", unit_admin, None).await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let none = keys(&h["no_published_rule"], "policy_key");
    assert!(none.contains(&my_key) && !none.contains(&their_key), "{none:?}");
    // Someone who administers nothing sees nothing.
    let (s, _) = send(&app, "GET", "/api/v1/admin/library/health", "prof.curie@science.state.edu", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// The side-by-side view reads the previous and the latest snapshot of a changed source, with the passages its
// published rules cite in the old one. Text is returned only as untrusted_text.
#[tokio::test]
async fn compare_returns_the_old_and_new_text_and_the_cited_passages() {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    let key = format!("cmp-{}", &Uuid::new_v4().simple().to_string()[..6]);
    let (src, policy_id): (Uuid, Uuid) = db.run(move |c| {
        let p = policy(c, &key, root(), Some(1));
        let r = rule(c, p, "r", "Published");
        let s = source(c, p, "url", Some("ok"));
        let old = snapshot(c, s, &format!("Intro. {PASSAGE} Outro."), "fetched", 3);
        cite(c, r, old, PASSAGE);
        snapshot(c, s, "Intro. Student records are kept for ten years after the student leaves. Outro.", "supplied", 0);
        (s, p)
    });
    let _ = policy_id;
    let (s, b) = send(&app, "GET", &format!("/api/v1/admin/library/sources/{src}/compare"), ADMIN, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["old"]["untrusted_text"].as_str().unwrap().contains(PASSAGE));
    assert!(b["new"]["untrusted_text"].as_str().unwrap().contains("ten years"));
    assert_eq!(b["new"]["text_origin"], "supplied");
    assert_eq!(b["cited"], json!([PASSAGE]));
    let (s, _) = send(&app, "GET", &format!("/api/v1/admin/library/sources/{src}/compare"), "prof.curie@science.state.edu", None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
