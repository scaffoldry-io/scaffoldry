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
        exempt(m, &k("POST", "/workspaces/{id}/guards/impact"), "evaluates a guard against the workspace and stores nothing");
        exempt(m, &k("POST", "/workspaces/{id}/guards/test"), "evaluates a guard against a request and stores nothing");
    }
    for (root, prefix) in [("/api/v1/admin", "/api/v1/admin"), ("/admin", "/admin")] {
        let k = |method: &str, path: &str| format!("{method} {prefix}{path}");
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
