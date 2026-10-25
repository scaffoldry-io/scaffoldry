//! Policy library phase 1: policies, sources, snapshots, rules and citations.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::TestDb;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use serde_json::{json, Value};
use tower::ServiceExt;

const ADMIN: &str = "jordan.lee@state.edu";
const OWNER: &str = "owner.library@state.edu";
const FACULTY: &str = "prof.curie@science.state.edu";
const ROOT: &str = "00000000-0000-0000-0000-000000000001";

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header(
        "authorization",
        format!("Bearer {}", issue_test_token_and_user(who)),
    );
    let body = match body {
        Some(v) => {
            req = req.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        },
    )
}

const RETENTION: &str = "Student records are kept for seven years after the student leaves. After that period they are destroyed unless a legal hold applies. Directory information may be disclosed without consent.";

struct World {
    db: TestDb,
    app: axum::Router,
    policy: String,
    source: String,
    snapshot: String,
}

fn policy_body(key: &str) -> Value {
    json!({ "key": key, "title": "Records retention", "summary": "How long records are kept", "organization_id": ROOT, "owner": { "Person": { "eppn": OWNER } }, "review_every_days": 365, "reason": "The registrar owns it" })
}

async fn world() -> World {
    let db = TestDb::new();
    let app = build_app_with_state(db.state()).expect("router");
    issue_test_token_and_user(OWNER);
    issue_test_token_and_user(FACULTY);
    let (s, p) = send(
        &app,
        "POST",
        "/api/v1/admin/library/policies",
        ADMIN,
        Some(policy_body("records-retention")),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let policy = p["policy"]["id"].as_str().unwrap().to_string();
    let (s, src) = send(&app, "POST", &format!("/api/v1/admin/library/policies/{policy}/sources"), ADMIN, Some(json!({ "title": "Retention schedule", "kind": "text", "reason": "The text of the schedule" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{src}");
    let source = src["source"]["id"].as_str().unwrap().to_string();
    let (s, snap) = send(
        &app,
        "POST",
        &format!("/api/v1/admin/library/sources/{source}/snapshots"),
        ADMIN,
        Some(json!({ "text": RETENTION, "reason": "Pasted from the schedule" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{snap}");
    let snapshot = snap["snapshot"]["id"].as_str().unwrap().to_string();
    World {
        db,
        app,
        policy,
        source,
        snapshot,
    }
}

fn rule(w: &World, key: &str, passage: &str) -> Value {
    json!({ "key": key, "statement": "Student records are kept for seven years.", "applies_to": "student records", "citations": [{ "snapshot_id": w.snapshot, "passage": passage, "locator": "section 1" }], "reason": "Read from the schedule" })
}

async fn entries(w: &World, kind: &str) -> usize {
    let (_, body) = send(
        &w.app,
        "GET",
        "/api/v1/governance/ledger?limit=200",
        ADMIN,
        None,
    )
    .await;
    body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["decision_type"] == kind)
        .count()
}

// 1 and 2 are the pure functions in scaffoldry-core.

// 3. A rule needs a citation that is in the snapshot, in this policy.
#[tokio::test]
async fn a_rule_whose_citation_is_not_in_the_snapshot_is_refused_and_naming_the_passage() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/rules", w.policy);
    let wrong = "Student records are kept for eight years after the student leaves.";
    let (s, body) = send(
        &w.app,
        "POST",
        &uri,
        ADMIN,
        Some(rule(&w, "retention", wrong)),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "citation_not_found", "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("eight years"),
        "the refusal names the passage: {body}"
    );

    // No citation at all.
    let mut none = rule(&w, "retention", wrong);
    none["citations"] = json!([]);
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(none)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");

    // A snapshot of another policy.
    let (_, other) = send(
        &w.app,
        "POST",
        "/api/v1/admin/library/policies",
        ADMIN,
        Some(policy_body("other-policy")),
    )
    .await;
    let other_id = other["policy"]["id"].as_str().unwrap().to_string();
    let (_, src) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/policies/{other_id}/sources"),
        ADMIN,
        Some(json!({ "title": "Other", "kind": "text", "reason": "r" })),
    )
    .await;
    let other_source = src["source"]["id"].as_str().unwrap().to_string();
    let (_, snap) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/sources/{other_source}/snapshots"),
        ADMIN,
        Some(json!({ "text": RETENTION, "reason": "r" })),
    )
    .await;
    let foreign = snap["snapshot"]["id"].as_str().unwrap().to_string();
    let mut cites_other = rule(
        &w,
        "retention",
        "Student records are kept for seven years after the student leaves.",
    );
    cites_other["citations"][0]["snapshot_id"] = json!(foreign);
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(cites_other)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");

    // None of them stored a rule.
    let (_, got) = send(
        &w.app,
        "GET",
        &format!("/api/v1/admin/library/policies/{}", w.policy),
        ADMIN,
        None,
    )
    .await;
    assert_eq!(
        got["rule_counts"]
            .as_object()
            .map(|m| m.values().filter_map(|v| v.as_i64()).sum::<i64>())
            .unwrap_or(0),
        0,
        "{got}"
    );
}

#[tokio::test]
async fn a_cited_rule_is_stored_as_a_draft_and_a_reworded_one_is_the_next_version() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/rules", w.policy);
    // The passage may break its lines differently from the source.
    let (s, body) = send(
        &w.app,
        "POST",
        &uri,
        ADMIN,
        Some(rule(
            &w,
            "retention",
            "Student records are kept for\nseven years after the student leaves.",
        )),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(
        (
            body["rule"]["version"].as_i64(),
            body["rule"]["status"].as_str(),
            body["rule"]["submitted_via"].as_str()
        ),
        (Some(1), Some("Draft"), Some("person")),
        "{body}"
    );
    let (s, again) = send(
        &w.app,
        "POST",
        &uri,
        ADMIN,
        Some(rule(
            &w,
            "retention",
            "records are kept for seven years after the student leaves",
        )),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{again}");
    assert_eq!(again["rule"]["version"], 2);
    let (_, got) = send(
        &w.app,
        "GET",
        &format!("/api/v1/admin/library/policies/{}", w.policy),
        ADMIN,
        None,
    )
    .await;
    assert_eq!(got["rule_counts"]["Draft"], 2, "{got}");
}

// 4. A second snapshot with the same text is not stored.
#[tokio::test]
async fn a_snapshot_with_the_same_text_is_not_stored_again() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/sources/{}/snapshots", w.source);
    // Spacing is not a change: the text is compared as the words it holds.
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "text": format!("  {}\n", RETENTION.replace(' ', "  ")), "reason": "Pasted again" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["stored"], false, "{body}");
    assert_eq!(
        body["snapshot"]["id"],
        w.snapshot.as_str(),
        "it names the snapshot that holds this text"
    );
    let n: i64 = w.db.run(|c| {
        c.query_one("SELECT COUNT(*) FROM library_snapshots", &[])
            .unwrap()
            .get(0)
    });
    assert_eq!(n, 1);
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "text": format!("{RETENTION} A new sentence."), "reason": "The schedule changed" }))).await;
    assert_eq!(
        (s, body["stored"].clone()),
        (StatusCode::CREATED, json!(true)),
        "{body}"
    );
    assert_eq!(body["snapshot"]["text_origin"], "supplied");
    // The snapshot can be read back.
    let id = body["snapshot"]["id"].as_str().unwrap();
    let (s, read) = send(
        &w.app,
        "GET",
        &format!("/api/v1/admin/library/snapshots/{id}"),
        ADMIN,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert!(read["snapshot"]["text"]
        .as_str()
        .unwrap()
        .ends_with("A new sentence."));
}

#[tokio::test]
async fn a_url_source_takes_no_pasted_snapshot_and_must_be_https() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/sources", w.policy);
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "title": "Web", "kind": "url", "href": "http://example.edu/policy", "reason": "r" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("href"), "{body}");
    let (s, body) = send(&w.app, "POST", &uri, ADMIN, Some(json!({ "title": "Web", "kind": "url", "href": "https://example.edu/policy", "reason": "r" }))).await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let id = body["source"]["id"].as_str().unwrap();
    let (s, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/sources/{id}/snapshots"),
        ADMIN,
        Some(
            json!({ "text": "Pasted text that an agent supplied for a web page.", "reason": "r" }),
        ),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an agent or a person never supplies what a URL says"
    );
    let (_, got) = send(
        &w.app,
        "GET",
        &format!("/api/v1/admin/library/policies/{}", w.policy),
        ADMIN,
        None,
    )
    .await;
    let web = got["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == id)
        .unwrap();
    assert_eq!(web["state"], "Not yet fetched", "{got}");
}

// 5. The owner can write. A faculty member who is not the owner cannot.
#[tokio::test]
async fn the_owner_of_a_policy_can_write_and_a_faculty_member_cannot() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/sources", w.policy);
    let body = json!({ "title": "Second source", "kind": "text", "reason": "More text" });
    let (s, resp) = send(&w.app, "POST", &uri, OWNER, Some(body.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "the owner may: {resp}");
    let (s, resp) = send(&w.app, "POST", &uri, FACULTY, Some(body)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{resp}");
    let (s, _) = send(
        &w.app,
        "POST",
        "/api/v1/admin/library/policies",
        FACULTY,
        Some(policy_body("faculty-policy")),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "nobody is the owner of a policy that does not exist yet"
    );
    let (s, _) = send(
        &w.app,
        "GET",
        "/api/v1/admin/library/policies",
        FACULTY,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 6. Every write is one ledger entry, written before the row.
#[tokio::test]
async fn every_write_is_one_ledger_entry_and_a_failed_entry_stores_nothing() {
    let w = world().await;
    let before = entries(&w, "PolicyLibraryChanged").await;
    assert_eq!(before, 3, "a policy, a source and a snapshot");
    let (s, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/policies/{}/rules", w.policy),
        ADMIN,
        Some(rule(
            &w,
            "retention",
            "Student records are kept for seven years after the student leaves.",
        )),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = send(
        &w.app,
        "PATCH",
        &format!("/api/v1/admin/library/policies/{}", w.policy),
        ADMIN,
        Some(json!({ "title": "Retention of records", "reason": "Renamed" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        entries(&w, "PolicyLibraryChanged").await,
        before + 2,
        "one entry for each write"
    );

    // A ledger that refuses the entry leaves no row.
    w.db.run(|c| {
        c.batch_execute(
            "CREATE FUNCTION fail_library_ledger() RETURNS trigger AS $$ BEGIN \
               IF NEW.payload->>'policy_key' = 'doomed' THEN RAISE EXCEPTION 'ledger refused'; END IF; RETURN NEW; END; $$ LANGUAGE plpgsql; \
             CREATE TRIGGER fail_library_ledger BEFORE INSERT ON governance_ledger FOR EACH ROW EXECUTE FUNCTION fail_library_ledger();",
        )
        .unwrap();
    });
    let (s, body) = send(
        &w.app,
        "POST",
        "/api/v1/admin/library/policies",
        ADMIN,
        Some(policy_body("doomed")),
    )
    .await;
    w.db.run(|c| c.batch_execute("DROP TRIGGER fail_library_ledger ON governance_ledger; DROP FUNCTION fail_library_ledger();").unwrap());
    assert!(s.is_server_error(), "{s} {body}");
    let n: i64 = w.db.run(|c| {
        c.query_one(
            "SELECT COUNT(*) FROM library_policies WHERE key = 'doomed'",
            &[],
        )
        .unwrap()
        .get(0)
    });
    assert_eq!(n, 0, "no row without its ledger entry");
}

// 7. A body that fails the schema is 400 and names the field.
#[tokio::test]
async fn a_body_that_fails_the_schema_names_the_field() {
    let w = world().await;
    let cases: Vec<(&str, &str, Value)> = vec![
        (
            "POST",
            "/api/v1/admin/library/policies",
            json!({ "title": "No key", "organization_id": ROOT, "owner": { "Person": { "eppn": OWNER } }, "reason": "r" }),
        ),
        ("POST", "/api/v1/admin/library/policies", {
            let mut b = policy_body("bad-owner");
            b["owner"] = json!({ "Committee": { "name": "x" } });
            b
        }),
        ("POST", "/api/v1/admin/library/policies", {
            let mut b = policy_body("bad-title");
            b["title"] = json!(12);
            b
        }),
        ("POST", "/api/v1/admin/library/policies", {
            let mut b = policy_body("Bad Key");
            b["key"] = json!("Bad Key");
            b
        }),
        ("POST", "/api/v1/admin/library/policies", {
            let mut b = policy_body("no-reason");
            b.as_object_mut().unwrap().remove("reason");
            b
        }),
        ("POST", "/api/v1/admin/library/policies", {
            let mut b = policy_body("bad-days");
            b["review_every_days"] = json!(0);
            b
        }),
    ];
    let expected = [
        "key",
        "owner",
        "title",
        "key",
        "reason",
        "review_every_days",
    ];
    for ((method, uri, body), field) in cases.into_iter().zip(expected) {
        let (s, resp) = send(&w.app, method, uri, ADMIN, Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{resp}");
        assert!(
            resp["fields"].get(field).is_some(),
            "the refusal names {field}: {resp}"
        );
    }
    let (s, resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/policies/{}/rules", w.policy),
        ADMIN,
        Some(json!({ "key": "x", "statement": 5, "citations": [], "reason": "r" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{resp}");
    assert!(resp["fields"].get("statement").is_some(), "{resp}");
}

#[tokio::test]
async fn limits_hold() {
    let w = world().await;
    let uri = format!("/api/v1/admin/library/policies/{}/rules", w.policy);
    let mut long = rule(
        &w,
        "long",
        "Student records are kept for seven years after the student leaves.",
    );
    long["statement"] = json!("x".repeat(501));
    let (s, resp) = send(&w.app, "POST", &uri, ADMIN, Some(long)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{resp}");
    assert!(resp["fields"].get("statement").is_some(), "{resp}");
    let mut three = rule(
        &w,
        "three",
        "Student records are kept for seven years after the student leaves.",
    );
    three["statement"] = json!("Records are kept for seven years. Then they are destroyed. A hold stops that. Nobody may delete them early.");
    let (s, resp) = send(&w.app, "POST", &uri, ADMIN, Some(three)).await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "a statement is one sentence or two: {resp}"
    );
    assert!(resp["fields"].get("statement").is_some(), "{resp}");
    let (s, resp) = send(
        &w.app,
        "POST",
        &format!("/api/v1/admin/library/sources/{}/snapshots", w.source),
        ADMIN,
        Some(json!({ "text": "w ".repeat(1_100_000), "reason": "Too big" })),
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE, "{resp}");
}
