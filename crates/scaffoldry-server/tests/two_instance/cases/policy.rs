//! Admin console phase 5: policy versions, test cases and OSCAL requirements. A writes. B, built before the write, reads.

use super::*;
use scaffoldry_server::service::identity::issue_test_token_and_user;

pub fn add(m: &mut Entries) {
    m.insert("POST /api/v1/admin/policy/versions".into(), case!(draft_create));
    m.insert("POST /api/v1/admin/policy/versions/{id}/clone".into(), case!(draft_clone));
    m.insert("POST /api/v1/admin/policy/versions/{id}/activate".into(), case!(activate));
    m.insert("POST /api/v1/admin/policy/cases".into(), case!(case_add));
    m.insert("DELETE /api/v1/admin/policy/cases/{id}".into(), case!(case_remove));
    m.insert("PUT /api/v1/admin/oscal/requirements/{control_id}".into(), case!(requirement_put));
    exempt(m, "POST /api/v1/admin/policy/versions/{id}/run-cases", "runs the test cases against a stored version in a temporary engine and stores nothing");
    exempt(m, "POST /api/v1/admin/policy/validate", "checks a source and stores nothing");
    exempt(m, "POST /api/v1/admin/policy/simulate", "asks one question of an engine and stores nothing");
}

/// The running source with one more rule named `key`.
async fn source_with(w: &World, key: &str) -> Result<String, String> {
    let active = status(w.wa("GET", "/api/v1/admin/policy/active", ADMIN, None).await, StatusCode::OK, "A reads the active policy")?;
    let base = active["source"].as_str().ok_or("a source")?;
    Ok(format!("{base}\n@id(\"{key}\") @oscal(\"ac-3\")\nforbid (principal, action == Action::\"read\", resource) when {{ principal.scoped_affiliation == \"affiliate\" }};\n"))
}

async fn draft_create(w: World) -> Outcome {
    let key = unique("rule");
    let source = source_with(&w, &key).await?;
    let made = status(w.wa("POST", "/api/v1/admin/policy/versions", ADMIN, Some(json!({ "source": source, "note": "Two servers" }))).await, StatusCode::CREATED, "A stores a draft")?;
    let id = made["id"].as_str().ok_or("an id")?;
    let seen = status(w.rb("GET", &format!("/api/v1/admin/policy/versions/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the draft")?;
    ensure(seen["status"] == "Draft" && seen["source"] == source.as_str(), format!("B reads the draft A stored: {seen}"))
}

async fn draft_clone(w: World) -> Outcome {
    let active = status(w.wa("GET", "/api/v1/admin/policy/active", ADMIN, None).await, StatusCode::OK, "A reads the active policy")?;
    let id = active["version"]["id"].as_str().ok_or("an id")?;
    let made = status(w.wa("POST", &format!("/api/v1/admin/policy/versions/{id}/clone"), ADMIN, Some(json!({}))).await, StatusCode::CREATED, "A clones it")?;
    let list = status(w.rb("GET", "/api/v1/admin/policy/versions", ADMIN, None).await, StatusCode::OK, "B lists the versions")?;
    ensure(has(&list, made["id"].as_str().ok_or("an id")?), format!("B lists the clone: {list}"))
}

/// A writes a draft and a second Platform Admin switches it on through A. B has never been asked about the policy, and shows it.
async fn activate(w: World) -> Outcome {
    let key = unique("rule");
    let source = source_with(&w, &key).await?;
    let made = status(w.wa("POST", "/api/v1/admin/policy/versions", ADMIN, Some(json!({ "source": source }))).await, StatusCode::CREATED, "A stores a draft")?;
    let id = made["id"].as_str().ok_or("an id")?.to_string();
    let second = person("p5second");
    issue_test_token_and_user(&second);
    let who = second.clone();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO roles (id, person_id, organization_id, role_title, scoped_affiliation, is_primary, source) \
             SELECT gen_random_uuid(), p.id, o.id, 'Platform Admin', 'platform_admin', true, 'api' \
             FROM persons p, organizations o WHERE LOWER(p.eppn) = LOWER($1) AND o.parent_id IS NULL ON CONFLICT DO NOTHING",
            &[&who],
        )
        .unwrap();
    });
    status(w.wa("POST", &format!("/api/v1/admin/policy/versions/{id}/activate"), &second, Some(json!({ "reason": "Reviewed." }))).await, StatusCode::OK, "A switches it on")?;
    let seen = status(w.rb("GET", "/api/v1/admin/policy/active", ADMIN, None).await, StatusCode::OK, "B reads the active policy")?;
    ensure(has(&seen["policies"], &key), format!("B runs the policy A switched on: {}", seen["version"]))
}

fn case_body(name: &str) -> Value {
    let who = json!({ "type": "User", "id": "a@state.edu", "attrs": { "eppn": "a@state.edu", "name": "A", "scoped_affiliation": "faculty", "department": "biology", "units": [], "unit_ids": [], "is_platform_admin": false } });
    let system = json!({ "type": "System", "id": "console", "attrs": { "department": "central_admin", "unit": { "type": "Unit", "id": "central_admin" }, "is_ferpa_sensitive": false } });
    json!({ "name": name, "principal": who, "action": "access_admin", "resource": system, "expected": "Deny", "reason": "A case for two servers." })
}

async fn case_add(w: World) -> Outcome {
    let name = unique("case");
    status(w.wa("POST", "/api/v1/admin/policy/cases", ADMIN, Some(case_body(&name))).await, StatusCode::CREATED, "A adds a case")?;
    let list = status(w.rb("GET", "/api/v1/admin/policy/cases?limit=200", ADMIN, None).await, StatusCode::OK, "B lists the cases")?;
    ensure(has(&list, &name), format!("B lists the case: {list}"))
}

async fn case_remove(w: World) -> Outcome {
    let name = unique("case");
    let made = status(w.wa("POST", "/api/v1/admin/policy/cases", ADMIN, Some(case_body(&name))).await, StatusCode::CREATED, "A adds a case")?;
    let id = made["id"].as_str().ok_or("an id")?;
    status(w.rb("DELETE", &format!("/api/v1/admin/policy/cases/{id}"), ADMIN, Some(json!({ "reason": "No longer needed." }))).await, StatusCode::OK, "B removes it")?;
    let list = status(w.wa("GET", "/api/v1/admin/policy/cases?limit=200", ADMIN, None).await, StatusCode::OK, "A lists the cases")?;
    ensure(!has(&list, &name), format!("the case is gone on A: {list}"))
}

async fn requirement_put(w: World) -> Outcome {
    status(w.wa("PUT", "/api/v1/admin/oscal/requirements/sa-9", ADMIN, Some(json!({ "statement": "Outside services are reviewed.", "status": "planned", "reason": "Adding it." }))).await, StatusCode::OK, "A edits a requirement")?;
    let doc = status(w.rb("GET", "/api/v1/admin/oscal/component-definition", ADMIN, None).await, StatusCode::OK, "B exports")?;
    ensure(has(&doc, "Outside services are reviewed."), format!("B exports the statement A wrote: {doc}"))
}
