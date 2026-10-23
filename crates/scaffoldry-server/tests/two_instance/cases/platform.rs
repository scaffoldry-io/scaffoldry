//! Workspaces, units, positions, people, datasets, settings, sessions and the ledger.

use super::*;
use scaffoldry_server::service::identity::issue_test_token_and_user;

pub fn add(m: &mut Entries) {
    // Workspaces are mounted at the root and under /api/v1: two paths, one handler, a case for each.
    for (root, prefix) in [("/api/v1", "/api/v1"), ("", "")] {
        let k = |method: &str, path: &str| format!("{method} {prefix}{path}");
        m.insert(k("POST", "/workspaces"), case!(ws_create, root));
        m.insert(k("PUT", "/workspaces/{id}"), case!(ws_update, root));
        m.insert(k("POST", "/workspaces/{id}/collaborators"), case!(collab_add, root));
        m.insert(k("PUT", "/workspaces/{id}/collaborators/{eppn}"), case!(collab_change, root));
        m.insert(k("DELETE", "/workspaces/{id}/collaborators/{eppn}"), case!(collab_remove, root));
        m.insert(k("PUT", "/workspaces/{id}/guards"), case!(guards_put, root));
        m.insert(k("PUT", "/workspaces/{id}/coordinates"), case!(coordinates_put, root));
        m.insert(k("POST", "/workspaces/{id}/coordinates/changes/{change}/decide"), case!(coordinates_decide, root));
        exempt(m, &k("POST", "/workspaces/{id}/guards/impact"), "evaluates a guard against the workspace and stores nothing");
        exempt(m, &k("POST", "/workspaces/{id}/guards/test"), "evaluates a guard against a request and stores nothing");
    }
    for (root, prefix) in [("/api/v1/admin", "/api/v1/admin"), ("/admin", "/admin")] {
        let k = |method: &str, path: &str| format!("{method} {prefix}{path}");
        m.insert(k("POST", "/access/grants"), case!(grant_propose, root));
        m.insert(k("POST", "/access/grants/{id}/revoke"), case!(grant_revoke, root));
        m.insert(k("POST", "/access/changes/{id}/decide"), case!(grant_decide, root));
        m.insert(k("PATCH", "/workspaces/{id}"), case!(admin_ws_move, root));
        m.insert(k("POST", "/workspaces/{id}/transfer-ownership"), case!(admin_ws_transfer, root));
    }

    m.insert("POST /api/v1/orgs".into(), case!(org_create));
    m.insert("PATCH /api/v1/orgs/{id}".into(), case!(org_patch));
    m.insert("POST /api/v1/orgs/{id}/appointments".into(), case!(appoint));
    m.insert("DELETE /api/v1/orgs/{id}/appointments/{eppn}/{affiliation}".into(), case!(revoke_appointment));
    m.insert("POST /api/v1/admin/positions".into(), case!(position_create));
    m.insert("PATCH /api/v1/admin/positions/{key}".into(), case!(position_patch));
    m.insert("POST /api/v1/orgs/{id}/positions/{key}/holders".into(), case!(holder_assign));
    m.insert("DELETE /api/v1/orgs/{id}/positions/{key}/holders/{eppn}".into(), case!(holder_vacate));
    m.insert("POST /api/v1/admin/users".into(), case!(user_create));
    m.insert("POST /api/v1/admin/users/{id}/hold".into(), case!(user_hold));
    m.insert("POST /api/v1/admin/users/{id}/revoke-tokens".into(), case!(user_revoke_tokens));

    m.insert("POST /api/v1/datasets".into(), case!(dataset_create));
    m.insert("POST /api/v1/datasets/{id}/relationships".into(), case!(relationship_create));
    m.insert("PUT /api/v1/settings/{key}".into(), case!(setting_put));
    m.insert("POST /api/v1/settings/sensitivity/presets/{id}".into(), case!(preset_enable));

    m.insert("POST /api/v1/clients".into(), case!(client_create));
    m.insert("POST /api/v1/clients/{id}/decide".into(), case!(client_decide));
    m.insert("POST /api/v1/clients/{id}/disable".into(), case!(client_disable));

    m.insert("POST /oauth/authorize".into(), case!(oauth_authorize));
    m.insert("POST /api/v1/oauth/authorize".into(), case!(oauth_authorize_v1));
    m.insert("POST /oauth/token".into(), case!(oauth_token));
    m.insert("POST /oauth/revoke".into(), case!(oauth_revoke));
    m.insert("DELETE /api/v1/me/connections/{client_id}".into(), case!(connection_delete));

    m.insert("POST /api/v1/auth/tokens".into(), case!(token_create));
    m.insert("DELETE /api/v1/auth/tokens/{id}".into(), case!(token_revoke));
    m.insert("POST /api/v1/auth/logout".into(), case!(logout));
    m.insert("POST /api/v1/auth/impersonate".into(), case!(impersonate));
    m.insert("POST /api/v1/auth/stop-impersonate".into(), case!(stop_impersonate));

    m.insert("POST /api/v1/governance/ledger/append".into(), case!(ledger_append));
    exempt(m, "POST /api/v1/governance/ledger/verify", "reads the chain and reports; stores nothing");
    exempt(m, "POST /api/v1/policies/simulate", "asks the policy engine a question; stores nothing");
    exempt(m, "POST /api/v1/formula/preview", "evaluates a formula; stores nothing");
    for mcp in ["/api/mcp", "/api/v1/mcp", "/api/apps/{slug}/mcp", "/api/v1/apps/{slug}/mcp"] {
        exempt(m, &format!("POST {mcp}"), "the MCP transport hands each call to a tool; every write tool has its own case");
    }
}

// --- workspaces ------------------------------------------------------------------------------------------------------

async fn ws_create(w: World, root: &'static str) -> Outcome {
    let code = unique("P6WS");
    let made = status(
        w.wa("POST", &format!("{root}/workspaces"), ADMIN, Some(json!({ "name": code, "code": code, "visibility": "restricted", "department": "biology" }))).await,
        StatusCode::CREATED,
        "create",
    )?;
    let id = made["id"].as_str().ok_or("an id")?.to_string();
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the workspace")?;
    ensure(has(&seen, &code), format!("B shows the workspace A made: {seen}"))
}

async fn ws_update(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6UP"), "restricted").await?;
    let name = unique("Renamed");
    status(w.wa("PUT", &format!("{root}/workspaces/{id}"), ADMIN, Some(json!({ "name": name, "reason": "Rename" }))).await, StatusCode::OK, "update")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}"), ADMIN, None).await, StatusCode::OK, "B reads it")?;
    ensure(has(&seen, &name), format!("B shows the new name: {seen}"))
}

async fn collab_add(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6CA"), "restricted").await?;
    let who = person("p6add");
    status(w.wa("POST", &format!("{root}/workspaces/{id}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "viewer" }))).await, StatusCode::CREATED, "add")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    ensure(has(&seen, &who), format!("B lists the collaborator A added: {seen}"))
}

async fn collab_change(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6CC"), "restricted").await?;
    let who = person("p6role");
    status(w.wa("POST", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "viewer" }))).await, StatusCode::CREATED, "add")?;
    status(w.wa("PUT", &format!("{root}/workspaces/{id}/collaborators/{who}"), ADMIN, Some(json!({ "role": "editor" }))).await, StatusCode::OK, "change the role")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    let row = seen.as_array().and_then(|rows| rows.iter().find(|r| has(r, &who))).cloned().unwrap_or(Value::Null);
    ensure(row["role"] == "editor", format!("B shows the new role: {row}"))
}

async fn collab_remove(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6CR"), "restricted").await?;
    let who = person("p6gone");
    status(w.wa("POST", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, Some(json!({ "eppn": who, "role": "viewer" }))).await, StatusCode::CREATED, "add")?;
    let before = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    ensure(has(&before, &who), "B lists the collaborator before the removal")?;
    let (s, body) = w.wa("DELETE", &format!("{root}/workspaces/{id}/collaborators/{who}"), ADMIN, None).await;
    ensure(s.is_success(), format!("remove: {s} {body}"))?;
    let after = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them again")?;
    ensure(!has(&after, &who), format!("B no longer lists the removed collaborator: {after}"))
}

async fn guards_put(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6GD"), "restricted").await?;
    status(
        w.wa("PUT", &format!("{root}/workspaces/{id}/guards"), ADMIN, Some(json!({ "rules": [{ "template": "deny_access_outside_units", "unit_ids": [ROOT] }], "reason": "Units only" }))).await,
        StatusCode::OK,
        "put guards",
    )?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/guards"), ADMIN, None).await, StatusCode::OK, "B reads the guards")?;
    ensure(has(&seen, "deny_access_outside_units"), format!("B shows the guard A set: {seen}"))
}

async fn coordinates_put(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6CO"), "restricted").await?;
    status(w.wa("PUT", &format!("{root}/workspaces/{id}/coordinates"), ADMIN, Some(json!({ "sensitivity": "Restricted", "reason": "Narrowing" }))).await, StatusCode::OK, "narrow")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/coordinates"), ADMIN, None).await, StatusCode::OK, "B reads them")?;
    ensure(seen["effective"]["sensitivity"] == "Restricted", format!("B shows the sensitivity A set: {seen}"))
}

async fn coordinates_decide(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6CD"), "restricted").await?;
    let second = person("p6second");
    make_user(&w, &second).await?;
    status(w.wa("POST", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, Some(json!({ "eppn": second, "role": "admin" }))).await, StatusCode::CREATED, "add the second person")?;
    status(w.wa("PUT", &format!("{root}/workspaces/{id}/coordinates"), ADMIN, Some(json!({ "sensitivity": "Restricted", "reason": "Narrowing" }))).await, StatusCode::OK, "narrow")?;
    let pending = status(w.wa("PUT", &format!("{root}/workspaces/{id}/coordinates"), ADMIN, Some(json!({ "sensitivity": "Directory", "reason": "Widening" }))).await, StatusCode::ACCEPTED, "widen")?;
    let change = pending["change_id"].as_str().unwrap_or_default().to_string();
    let listed = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/coordinates"), ADMIN, None).await, StatusCode::OK, "B lists the pending change")?;
    ensure(has(&listed["pending"], &change), format!("B lists the change A stored: {listed}"))?;
    status(w.rb("POST", &format!("{root}/workspaces/{id}/coordinates/changes/{change}/decide"), &second, Some(json!({ "decision": "confirm" }))).await, StatusCode::OK, "B confirms")?;
    let seen = status(w.wa("GET", &format!("/api/v1/workspaces/{id}/coordinates"), ADMIN, None).await, StatusCode::OK, "A reads them")?;
    ensure(seen["effective"]["sensitivity"] == "Directory", format!("A shows the change B confirmed: {seen}"))
}

/// A unit, a unit admin of it, and a grant to a new person anchored in it, proposed on A as the Platform Admin.
async fn grant_fixture(w: &World, root: &str) -> Result<(String, String, String, String), String> {
    let unit = make_unit(w, &unique("P6GU")).await?;
    let boss = person("p6boss");
    make_user(w, &boss).await?;
    status(
        w.wa("POST", &format!("/api/v1/orgs/{unit}/appointments"), ADMIN, Some(json!({ "eppn": boss, "scoped_affiliation": "unit_admin", "reason": "Fixture" }))).await,
        StatusCode::CREATED,
        "appoint the unit admin",
    )?;
    let holder = person("p6holder");
    let body = json!({ "holder_kind": "person", "holder_id": holder, "role": "viewer", "hierarchy": "academic", "anchor_kind": "unit", "anchor_id": unit, "source": "request", "reason": "Fixture grant" });
    let pending = status(w.wa("POST", &format!("{root}/access/grants"), ADMIN, Some(body)).await, StatusCode::ACCEPTED, "propose the grant")?;
    let change = pending["change_id"].as_str().ok_or("a change id")?.to_string();
    Ok((unit, boss, holder, change))
}

async fn grant_propose(w: World, root: &'static str) -> Outcome {
    let (_, _, holder, change) = grant_fixture(&w, root).await?;
    let seen = status(w.rb("GET", "/api/v1/admin/access/grants?state=pending", ADMIN, None).await, StatusCode::OK, "B lists the pending changes")?;
    ensure(has(&seen["changes"], &change) && has(&seen["changes"], &holder), format!("B shows the change A stored: {seen}"))
}

async fn grant_decide(w: World, root: &'static str) -> Outcome {
    let (_, boss, holder, change) = grant_fixture(&w, root).await?;
    status(w.rb("POST", &format!("{root}/access/changes/{change}/decide"), &boss, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await, StatusCode::OK, "B confirms")?;
    let seen = status(w.wa("GET", &format!("/api/v1/admin/access/grants?state=active&holder={holder}"), ADMIN, None).await, StatusCode::OK, "A lists the active grants")?;
    ensure(seen["grants"].as_array().is_some_and(|g| g.len() == 1), format!("A shows the grant B confirmed: {seen}"))
}

async fn grant_revoke(w: World, root: &'static str) -> Outcome {
    let (_, boss, holder, change) = grant_fixture(&w, root).await?;
    let done = status(w.rb("POST", &format!("{root}/access/changes/{change}/decide"), &boss, Some(json!({ "decision": "confirm", "reason": "Reviewed" }))).await, StatusCode::OK, "B confirms")?;
    let id = done["grant_id"].as_i64().ok_or("a grant id")?;
    status(w.wa("POST", &format!("{root}/access/grants/{id}/revoke"), ADMIN, Some(json!({ "reason": "Done" }))).await, StatusCode::OK, "A revokes")?;
    let seen = status(w.rb("GET", &format!("/api/v1/admin/access/grants?state=revoked&holder={holder}"), ADMIN, None).await, StatusCode::OK, "B lists the revoked grants")?;
    ensure(seen["grants"].as_array().is_some_and(|g| g.len() == 1), format!("B shows the grant A revoked: {seen}"))
}

async fn admin_ws_move(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6MV"), "restricted").await?;
    let unit = make_unit(&w, &unique("P6MU")).await?;
    status(w.wa("PATCH", &format!("{root}/workspaces/{id}"), ADMIN, Some(json!({ "organization_id": unit, "reason": "Moving" }))).await, StatusCode::OK, "move")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}"), ADMIN, None).await, StatusCode::OK, "B reads it")?;
    ensure(has(&seen, &unit), format!("B shows the unit the workspace moved to: {seen}"))
}

async fn admin_ws_transfer(w: World, root: &'static str) -> Outcome {
    let id = make_workspace(&w, &unique("P6TR"), "restricted").await?;
    let heir = person("p6heir");
    make_user(&w, &heir).await?;
    status(w.wa("POST", &format!("{root}/workspaces/{id}/transfer-ownership"), ADMIN, Some(json!({ "eppn": heir, "reason": "Handover" }))).await, StatusCode::OK, "transfer")?;
    let seen = status(w.rb("GET", &format!("/api/v1/workspaces/{id}/collaborators"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    let row = seen.as_array().and_then(|rows| rows.iter().find(|r| has(r, &heir))).cloned().unwrap_or(Value::Null);
    ensure(row["role"] == "owner", format!("B shows the new owner: {seen}"))
}

// --- units, positions, people ------------------------------------------------------------------------------------------

async fn org_create(w: World) -> Outcome {
    let code = unique("P6ORG");
    make_unit(&w, &code).await?;
    let seen = status(w.rb("GET", "/api/v1/orgs", ADMIN, None).await, StatusCode::OK, "B lists units")?;
    ensure(has(&seen, &code), format!("B lists the unit A made: {seen}"))
}

async fn org_patch(w: World) -> Outcome {
    let unit = make_unit(&w, &unique("P6OP")).await?;
    let name = unique("Renamed-unit");
    status(w.wa("PATCH", &format!("/api/v1/orgs/{unit}"), ADMIN, Some(json!({ "name": name }))).await, StatusCode::OK, "rename")?;
    let seen = status(w.rb("GET", "/api/v1/orgs", ADMIN, None).await, StatusCode::OK, "B lists units")?;
    ensure(has(&seen, &name), format!("B shows the new name: {seen}"))
}

async fn appoint(w: World) -> Outcome {
    let unit = make_unit(&w, &unique("P6AP")).await?;
    let who = person("p6appt");
    let user = make_user(&w, &who).await?;
    let id = user["id"].as_str().ok_or("a user id")?.to_string();
    status(
        w.wa("POST", &format!("/api/v1/orgs/{unit}/appointments"), ADMIN, Some(json!({ "eppn": who, "scoped_affiliation": "unit_admin", "reason": "Fixture" }))).await,
        StatusCode::CREATED,
        "appoint",
    )?;
    let seen = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person")?;
    ensure(has(&seen, &unit), format!("B shows the appointment A made: {seen}"))
}

async fn revoke_appointment(w: World) -> Outcome {
    let unit = make_unit(&w, &unique("P6RV")).await?;
    let who = person("p6revoke");
    let user = make_user(&w, &who).await?;
    let id = user["id"].as_str().ok_or("a user id")?.to_string();
    status(
        w.wa("POST", &format!("/api/v1/orgs/{unit}/appointments"), ADMIN, Some(json!({ "eppn": who, "scoped_affiliation": "unit_admin", "reason": "Fixture" }))).await,
        StatusCode::CREATED,
        "appoint",
    )?;
    let before = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person")?;
    ensure(has(&before, &unit), "B shows the appointment before it is revoked")?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/orgs/{unit}/appointments/{who}/unit_admin"), ADMIN, None).await;
    ensure(s.is_success(), format!("revoke: {s} {body}"))?;
    let after = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person again")?;
    ensure(!has(&after, &unit), format!("B no longer shows the appointment: {after}"))
}

async fn position_create(w: World) -> Outcome {
    let key = unique("p6pos").replace('-', "");
    status(
        w.wa("POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Dean", "org_types": ["Department"], "max_holders": 1, "reason": "A unit has a dean" }))).await,
        StatusCode::CREATED,
        "create the position type",
    )?;
    let seen = status(w.rb("GET", "/api/v1/admin/positions", ADMIN, None).await, StatusCode::OK, "B lists position types")?;
    ensure(has(&seen, &key), format!("B lists the position type A made: {seen}"))
}

async fn position_patch(w: World) -> Outcome {
    let key = unique("p6pat").replace('-', "");
    status(w.wa("POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Chair", "org_types": ["Department"], "max_holders": 1, "reason": "Define" }))).await, StatusCode::CREATED, "create")?;
    let name = unique("Renamed-position");
    status(w.wa("PATCH", &format!("/api/v1/admin/positions/{key}"), ADMIN, Some(json!({ "name": name, "reason": "Rename" }))).await, StatusCode::OK, "patch")?;
    let seen = status(w.rb("GET", "/api/v1/admin/positions", ADMIN, None).await, StatusCode::OK, "B lists position types")?;
    ensure(has(&seen, &name), format!("B shows the new name: {seen}"))
}

async fn holder_assign(w: World) -> Outcome {
    let unit = make_unit(&w, &unique("P6HA")).await?;
    let key = unique("p6hold").replace('-', "");
    status(w.wa("POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Head", "org_types": ["Department"], "max_holders": 1, "reason": "Define" }))).await, StatusCode::CREATED, "create")?;
    let who = person("p6head");
    make_user(&w, &who).await?;
    status(w.wa("POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": who, "reason": "Appointed" }))).await, StatusCode::CREATED, "assign")?;
    let seen = status(w.rb("GET", &format!("/api/v1/orgs/{unit}/positions"), ADMIN, None).await, StatusCode::OK, "B lists the unit's positions")?;
    ensure(has(&seen, &who), format!("B shows the holder A assigned: {seen}"))
}

async fn holder_vacate(w: World) -> Outcome {
    let unit = make_unit(&w, &unique("P6HV")).await?;
    let key = unique("p6vac").replace('-', "");
    status(w.wa("POST", "/api/v1/admin/positions", ADMIN, Some(json!({ "key": key, "name": "Head", "org_types": ["Department"], "max_holders": 1, "reason": "Define" }))).await, StatusCode::CREATED, "create")?;
    let who = person("p6leave");
    make_user(&w, &who).await?;
    status(w.wa("POST", &format!("/api/v1/orgs/{unit}/positions/{key}/holders"), ADMIN, Some(json!({ "eppn": who, "reason": "Appointed" }))).await, StatusCode::CREATED, "assign")?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/orgs/{unit}/positions/{key}/holders/{who}"), ADMIN, Some(json!({ "reason": "Left" }))).await;
    ensure(s.is_success(), format!("vacate: {s} {body}"))?;
    let seen = status(w.rb("GET", &format!("/api/v1/orgs/{unit}/positions"), ADMIN, None).await, StatusCode::OK, "B lists the unit's positions")?;
    ensure(!has(&seen, &who), format!("B no longer shows the holder: {seen}"))
}

async fn user_create(w: World) -> Outcome {
    let who = person("p6user");
    let user = make_user(&w, &who).await?;
    let id = user["id"].as_str().ok_or("a user id")?.to_string();
    let seen = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person")?;
    ensure(has(&seen, &who), format!("B shows the person A created: {seen}"))
}

async fn user_hold(w: World) -> Outcome {
    let who = person("p6hold");
    let user = make_user(&w, &who).await?;
    let id = user["id"].as_str().ok_or("a user id")?.to_string();
    let before = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person")?;
    status(w.wa("POST", &format!("/api/v1/admin/users/{id}/hold"), ADMIN, Some(json!({ "hold": true, "reason": "Pending review" }))).await, StatusCode::OK, "hold")?;
    let after = status(w.rb("GET", &format!("/api/v1/admin/users/{id}"), ADMIN, None).await, StatusCode::OK, "B reads the person again")?;
    ensure(before["user"]["hold"] == false && after["user"]["hold"] == true, format!("B shows the hold A placed: {after}"))
}

async fn user_revoke_tokens(w: World) -> Outcome {
    let who = person("p6tok");
    let user = make_user(&w, &who).await?;
    let id = user["id"].as_str().ok_or("a user id")?.to_string();
    // A session of this person. It is minted on the test's own thread, which is where the test database is named.
    let token = issue_test_token_and_user(&who);
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/workspaces", &token, None).await;
    ensure(s == StatusCode::OK, format!("B accepts the token before the revocation: {s} {body}"))?;
    status(w.wa("POST", &format!("/api/v1/admin/users/{id}/revoke-tokens"), ADMIN, Some(json!({ "reason": "Lost laptop" }))).await, StatusCode::OK, "revoke")?;
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/workspaces", &token, None).await;
    ensure(s == StatusCode::UNAUTHORIZED, format!("B refuses the token A revoked: {s} {body}"))
}

// --- datasets, settings ------------------------------------------------------------------------------------------------

async fn dataset_create(w: World) -> Outcome {
    let id = unique("p6ds").replace('-', "_");
    status(w.wa("POST", "/api/v1/datasets", ADMIN, Some(json!({ "id": id, "name": id, "department": "biology" }))).await, StatusCode::CREATED, "publish")?;
    let seen = status(w.rb("GET", &format!("/api/v1/datasets/{id}"), ADMIN, None).await, StatusCode::OK, "B reads it")?;
    ensure(has(&seen, &id), format!("B shows the dataset A published: {seen}"))
}

async fn relationship_create(w: World) -> Outcome {
    let (left, right) = (unique("p6l").replace('-', "_"), unique("p6r").replace('-', "_"));
    for id in [&left, &right] {
        status(w.wa("POST", "/api/v1/datasets", ADMIN, Some(json!({ "id": id, "name": id, "department": "biology" }))).await, StatusCode::CREATED, "publish")?;
    }
    let rel = unique("p6rel").replace('-', "_");
    status(
        w.wa("POST", &format!("/api/v1/datasets/{left}/relationships"), ADMIN, Some(json!({ "id": rel, "target_dataset_id": right, "source_field": "id", "target_field": "id" }))).await,
        StatusCode::CREATED,
        "relate",
    )?;
    let seen = status(w.rb("GET", &format!("/api/v1/datasets/{left}/relationships"), ADMIN, None).await, StatusCode::OK, "B lists them")?;
    ensure(has(&seen, &rel), format!("B lists the relationship A made: {seen}"))
}

async fn setting_put(w: World) -> Outcome {
    // A value the setting allows, and not the one it has.
    let before = status(w.rb("GET", "/api/v1/settings", ADMIN, None).await, StatusCode::OK, "B reads settings")?;
    let days = 100 + (uuid::Uuid::new_v4().as_u128() % 200) as i64;
    status(w.wa("PUT", "/api/v1/settings/process.stale_days", ADMIN, Some(json!(days))).await, StatusCode::OK, "put the setting")?;
    let seen = status(w.rb("GET", "/api/v1/settings", ADMIN, None).await, StatusCode::OK, "B reads settings again")?;
    ensure(before != seen && has(&seen, &days.to_string()), format!("B shows the setting A wrote ({days}): {seen}"))
}

async fn preset_enable(w: World) -> Outcome {
    let before = status(w.rb("GET", "/api/v1/settings", ADMIN, None).await, StatusCode::OK, "B reads settings")?;
    status(w.wa("POST", "/api/v1/settings/sensitivity/presets/pci", ADMIN, None).await, StatusCode::OK, "enable the preset")?;
    let after = status(w.rb("GET", "/api/v1/settings", ADMIN, None).await, StatusCode::OK, "B reads settings again")?;
    ensure(before != after, "B shows the categories the preset added")
}

// --- outside apps ----------------------------------------------------------------------------------------------------------

/// A client with a read-only ceiling over a new app, registered through A. Returns its id.
async fn register_client(w: &World) -> Result<String, String> {
    let slug = make_app(w).await?;
    let body = json!({
        "name": unique("p6client"), "operator": "Physics", "kind": "public", "redirect_uris": ["https://tracker.example.edu/callback"],
        "ceiling": { "apps": [{ "app_slug": slug, "tables": [{ "table_id": "apps", "read": ["name", "gpa"], "write": [], "create": false, "delete": false }] }] }
    });
    let made = status(w.wa("POST", "/api/v1/clients", OWNER, Some(body)).await, StatusCode::CREATED, "register a client")?;
    Ok(made["client_id"].as_str().ok_or("a client id")?.to_string())
}

async fn client_create(w: World) -> Outcome {
    let id = register_client(&w).await?;
    let seen = status(w.rb("GET", &format!("/api/v1/clients/{id}"), OWNER, None).await, StatusCode::OK, "B reads the client")?;
    ensure(seen["status"] == "Pending" && has(&seen, "Read the name and gpa of Applications"), format!("B shows the client A registered: {seen}"))?;
    let listed = status(w.rb("GET", "/api/v1/clients", OWNER, None).await, StatusCode::OK, "B lists clients")?;
    ensure(has(&listed, &id), format!("B lists it: {listed}"))
}

async fn client_decide(w: World) -> Outcome {
    let id = register_client(&w).await?;
    status(World::call(&w.b, "POST", &format!("/api/v1/clients/{id}/decide"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await, StatusCode::OK, "B approves")?;
    let seen = status(World::call(&w.a, "GET", &format!("/api/v1/clients/{id}"), OWNER, None).await, StatusCode::OK, "A reads the client")?;
    ensure(seen["status"] == "Active", format!("A shows the approval B made: {seen}"))
}

async fn client_disable(w: World) -> Outcome {
    let id = register_client(&w).await?;
    status(w.wa("POST", &format!("/api/v1/clients/{id}/decide"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await, StatusCode::OK, "A approves")?;
    status(World::call(&w.b, "POST", &format!("/api/v1/clients/{id}/disable"), OWNER, Some(json!({ "reason": "Done" }))).await, StatusCode::OK, "B disables")?;
    let seen = status(World::call(&w.a, "GET", &format!("/api/v1/clients/{id}"), OWNER, None).await, StatusCode::OK, "A reads the client")?;
    ensure(seen["status"] == "Disabled", format!("A shows the disablement B made: {seen}"))
}

// --- signing in to an outside app ------------------------------------------------------------------------------------------

const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CALLBACK: &str = "https://tracker.example.edu/callback";

/// A form post to `uri` through `app`.
async fn form_post(app: &axum::Router, uri: &str, body: String) -> (StatusCode, Value) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let req = Request::builder().method("POST").uri(uri).header("content-type", "application/x-www-form-urlencoded").body(Body::from(body)).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// An approved client, and a code for OWNER from it, made through A.
async fn client_and_code(w: &World) -> Result<(String, String), String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let id = register_client(w).await?;
    status(w.wa("POST", &format!("/api/v1/clients/{id}/decide"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await, StatusCode::OK, "approve the client")?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let allowed = status(
        w.wa("POST", "/oauth/authorize", OWNER, Some(json!({
            "client_id": id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s1", "code_challenge": challenge,
            "code_challenge_method": "S256", "resource": "https://localhost", "decision": "allow"
        }))).await,
        StatusCode::OK,
        "allow",
    )?;
    let redirect = allowed["redirect_to"].as_str().ok_or("a redirect")?;
    let code = redirect.split_once("code=").and_then(|(_, rest)| rest.split('&').next()).ok_or("a code")?.to_string();
    Ok((id, code))
}

fn token_form(id: &str, code: &str) -> String {
    format!("grant_type=authorization_code&code={code}&redirect_uri={CALLBACK}&client_id={id}&code_verifier={VERIFIER}")
}

/// An access token for OWNER: the code is made on A and traded on B.
async fn access_token(w: &World) -> Result<(String, String), String> {
    let (id, code) = client_and_code(w).await?;
    let (s, issued) = form_post(&w.b, "/oauth/token", token_form(&id, &code)).await;
    ensure(s == StatusCode::OK, format!("B trades the code A made: {s} {issued}"))?;
    Ok((id, issued["access_token"].as_str().ok_or("a token")?.to_string()))
}

async fn token_works(app: &axum::Router, token: &str) -> bool {
    World::call_with(app, "GET", "/api/v1/auth/me", token, None).await.0 == StatusCode::OK
}

async fn oauth_authorize(w: World) -> Outcome {
    let (id, code) = client_and_code(&w).await?;
    // B reads the code A stored and knows the consent A recorded.
    let (s, issued) = form_post(&w.b, "/oauth/token", token_form(&id, &code)).await;
    ensure(s == StatusCode::OK, format!("B accepts the code A made: {s} {issued}"))?;
    let shown = World::call(&w.b, "GET", &format!("/oauth/authorize?client_id={id}&redirect_uri={CALLBACK}&response_type=code&state=s&code_challenge={}&code_challenge_method=S256&resource=https://localhost", "a".repeat(43)), OWNER, None).await;
    ensure(shown.0 == StatusCode::OK && shown.1["remembered"] == true, format!("B knows the consent A recorded: {} {}", shown.0, shown.1))
}

/// The desk's copy of the consent call, under /api/v1. A consent made on B through it is remembered on A.
async fn oauth_authorize_v1(w: World) -> Outcome {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let id = register_client(&w).await?;
    status(w.wa("POST", &format!("/api/v1/clients/{id}/decide"), OWNER, Some(json!({ "decision": "approve", "rationale": "Reviewed" }))).await, StatusCode::OK, "approve the client")?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let (s, body) = World::call(&w.b, "POST", "/api/v1/oauth/authorize", OWNER, Some(json!({
        "client_id": id, "redirect_uri": CALLBACK, "response_type": "code", "state": "s1", "code_challenge": challenge,
        "code_challenge_method": "S256", "resource": "https://localhost", "decision": "allow"
    }))).await;
    ensure(s == StatusCode::OK && body["redirect_to"].is_string(), format!("B records the consent: {s} {body}"))?;
    let listed = status(World::call(&w.a, "GET", "/api/v1/me/connections", OWNER, None).await, StatusCode::OK, "A lists connections")?;
    ensure(has(&listed, &id), format!("A shows the consent B recorded: {listed}"))
}

async fn oauth_token(w: World) -> Outcome {
    let (id, token) = access_token(&w).await?;
    ensure(token_works(&w.a, &token).await && token_works(&w.b, &token).await, "both instances accept the token B issued")?;
    // The same code on A is a second use. It revokes the token B issued.
    let (_, code) = client_and_code(&w).await?;
    let (first, _) = form_post(&w.b, "/oauth/token", token_form(&id, &code)).await;
    let (second, _) = form_post(&w.a, "/oauth/token", token_form(&id, &code)).await;
    ensure(first != StatusCode::OK || second == StatusCode::BAD_REQUEST, "A refuses a code B already used")
}

async fn oauth_revoke(w: World) -> Outcome {
    let (id, token) = access_token(&w).await?;
    let (s, _) = form_post(&w.b, "/oauth/revoke", format!("token={token}&client_id={id}")).await;
    ensure(s == StatusCode::OK, "B revokes")?;
    ensure(!token_works(&w.a, &token).await, "A refuses the token B revoked")
}

async fn connection_delete(w: World) -> Outcome {
    let (id, token) = access_token(&w).await?;
    let (s, body) = World::call(&w.b, "DELETE", &format!("/api/v1/me/connections/{id}"), OWNER, None).await;
    ensure(s == StatusCode::OK, format!("B disconnects: {s} {body}"))?;
    ensure(!token_works(&w.a, &token).await, "A refuses the token after B disconnected")?;
    let listed = status(World::call(&w.a, "GET", "/api/v1/me/connections", OWNER, None).await, StatusCode::OK, "A lists connections")?;
    ensure(!has(&listed, &id), format!("A no longer lists it: {listed}"))
}

// --- sessions ------------------------------------------------------------------------------------------------------------

async fn token_create(w: World) -> Outcome {
    let label = unique("p6token");
    let made = status(w.wa("POST", "/api/v1/auth/tokens", ADMIN, Some(json!({ "label": label, "days": 1, "kind": "agent" }))).await, StatusCode::CREATED, "create a token")?;
    let token = made["token"].as_str().ok_or("a token")?.to_string();
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::OK && has(&body, ADMIN), format!("B accepts the token A made: {s} {body}"))?;
    let listed = status(w.rb("GET", "/api/v1/auth/tokens", ADMIN, None).await, StatusCode::OK, "B lists tokens")?;
    ensure(has(&listed, &label), format!("B lists it: {listed}"))
}

async fn token_revoke(w: World) -> Outcome {
    let made = status(w.wa("POST", "/api/v1/auth/tokens", ADMIN, Some(json!({ "label": unique("p6rev"), "days": 1, "kind": "agent" }))).await, StatusCode::CREATED, "create a token")?;
    let (token, id) = (made["token"].as_str().ok_or("a token")?.to_string(), made["id"].as_str().ok_or("an id")?.to_string());
    let (s, _) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::OK, "B accepts the token before it is revoked")?;
    let (s, body) = w.wa("DELETE", &format!("/api/v1/auth/tokens/{id}"), ADMIN, None).await;
    ensure(s.is_success(), format!("revoke: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::UNAUTHORIZED, format!("B refuses the revoked token: {s} {body}"))
}

async fn logout(w: World) -> Outcome {
    let made = status(w.wa("POST", "/api/v1/auth/tokens", ADMIN, Some(json!({ "label": unique("p6out"), "days": 1, "kind": "agent" }))).await, StatusCode::CREATED, "create a token")?;
    let token = made["token"].as_str().ok_or("a token")?.to_string();
    let (s, _) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::OK, "B accepts the token before the logout")?;
    let (s, body) = World::call_with(&w.a, "POST", "/api/v1/auth/logout", &token, None).await;
    ensure(s.is_success(), format!("logout: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::UNAUTHORIZED, format!("B refuses the token that logged out: {s} {body}"))
}

async fn impersonate(w: World) -> Outcome {
    let who = person("p6imp");
    make_user(&w, &who).await?;
    let made = status(w.wa("POST", "/api/v1/auth/impersonate", ADMIN, Some(json!({ "target_eppn": who }))).await, StatusCode::OK, "impersonate")?;
    let token = made["token"].as_str().ok_or("a token")?.to_string();
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::OK && has(&body, &who), format!("B accepts the impersonation token as the target: {s} {body}"))
}

async fn stop_impersonate(w: World) -> Outcome {
    let who = person("p6stop");
    make_user(&w, &who).await?;
    let made = status(w.wa("POST", "/api/v1/auth/impersonate", ADMIN, Some(json!({ "target_eppn": who }))).await, StatusCode::OK, "impersonate")?;
    let token = made["token"].as_str().ok_or("a token")?.to_string();
    let (s, body) = World::call_with(&w.a, "POST", "/api/v1/auth/stop-impersonate", &token, None).await;
    ensure(s.is_success(), format!("stop: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", "/api/v1/auth/me", &token, None).await;
    ensure(s == StatusCode::UNAUTHORIZED, format!("B refuses the impersonation token after it stopped: {s} {body}"))
}

// --- the ledger ----------------------------------------------------------------------------------------------------------

async fn ledger_append(w: World) -> Outcome {
    let before = status(w.rb("GET", "/api/v1/governance/ledger?limit=1", ADMIN, None).await, StatusCode::OK, "B reads the ledger")?;
    let rationale = unique("p6 ledger");
    status(
        w.wa(
            "POST",
            "/api/v1/governance/ledger/append",
            ADMIN,
            Some(json!({ "organization_code": "INST", "decision_type": "PolicyRevision", "oscal_control_id": "AU-02", "rationale": rationale, "payload": { "p6": true } })),
        )
        .await,
        StatusCode::CREATED,
        "append",
    )?;
    let after = status(w.rb("GET", "/api/v1/governance/ledger?limit=1", ADMIN, None).await, StatusCode::OK, "B reads the ledger again")?;
    ensure(after["total_entries"].as_i64() > before["total_entries"].as_i64() && has(&after, &rationale), format!("B shows the entry A appended: {after}"))
}
