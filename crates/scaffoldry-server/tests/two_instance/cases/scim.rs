//! The SCIM provisioning routes. They use the provisioning token every database starts with.

use super::*;

const SCIM: &str = "test-scim-token";

pub fn add(m: &mut Entries) {
    m.insert("POST /scim/v2/Users".into(), case!(user_create));
    m.insert("PUT /scim/v2/Users/{id}".into(), case!(user_replace));
    m.insert("DELETE /scim/v2/Users/{id}".into(), case!(user_delete));
    m.insert("POST /scim/v2/Groups".into(), case!(group_create));
    m.insert("PUT /scim/v2/Groups/{id}".into(), case!(group_replace));
    m.insert("DELETE /scim/v2/Groups/{id}".into(), case!(group_delete));
}

async fn scim_user(w: &World, active: bool) -> Result<(String, String), String> {
    let who = person("p6scim");
    let made = status(World::call_with(&w.a, "POST", "/scim/v2/Users", SCIM, Some(json!({ "userName": who, "active": active }))).await, StatusCode::CREATED, "provision")?;
    Ok((made["id"].as_str().ok_or("an id")?.to_string(), who))
}

async fn user_create(w: World) -> Outcome {
    let (id, who) = scim_user(&w, true).await?;
    let seen = status(World::call_with(&w.b, "GET", &format!("/scim/v2/Users/{id}"), SCIM, None).await, StatusCode::OK, "B reads the user")?;
    ensure(has(&seen, &who), format!("B shows the user A provisioned: {seen}"))
}

async fn user_replace(w: World) -> Outcome {
    let (id, who) = scim_user(&w, true).await?;
    status(World::call_with(&w.a, "PUT", &format!("/scim/v2/Users/{id}"), SCIM, Some(json!({ "userName": who, "active": false }))).await, StatusCode::OK, "replace")?;
    let seen = status(World::call_with(&w.b, "GET", &format!("/scim/v2/Users/{id}"), SCIM, None).await, StatusCode::OK, "B reads the user")?;
    ensure(seen["active"] == false, format!("B shows the user deactivated: {seen}"))
}

async fn user_delete(w: World) -> Outcome {
    let (id, _) = scim_user(&w, true).await?;
    let (s, body) = World::call_with(&w.a, "DELETE", &format!("/scim/v2/Users/{id}"), SCIM, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", &format!("/scim/v2/Users/{id}"), SCIM, None).await;
    ensure(s == StatusCode::NOT_FOUND, format!("B no longer finds the user: {s} {body}"))
}

async fn scim_group(w: &World) -> Result<(String, String), String> {
    let name = unique("P6 group");
    let made = status(World::call_with(&w.a, "POST", "/scim/v2/Groups", SCIM, Some(json!({ "displayName": name }))).await, StatusCode::CREATED, "provision a group")?;
    Ok((made["id"].as_str().ok_or("an id")?.to_string(), name))
}

async fn group_create(w: World) -> Outcome {
    let (id, name) = scim_group(&w).await?;
    let seen = status(World::call_with(&w.b, "GET", &format!("/scim/v2/Groups/{id}"), SCIM, None).await, StatusCode::OK, "B reads the group")?;
    ensure(has(&seen, &name), format!("B shows the group A provisioned: {seen}"))
}

async fn group_replace(w: World) -> Outcome {
    let (id, _) = scim_group(&w).await?;
    let renamed = unique("P6 renamed group");
    status(World::call_with(&w.a, "PUT", &format!("/scim/v2/Groups/{id}"), SCIM, Some(json!({ "displayName": renamed }))).await, StatusCode::OK, "replace")?;
    let seen = status(World::call_with(&w.b, "GET", &format!("/scim/v2/Groups/{id}"), SCIM, None).await, StatusCode::OK, "B reads the group")?;
    ensure(has(&seen, &renamed), format!("B shows the new name: {seen}"))
}

async fn group_delete(w: World) -> Outcome {
    let (id, _) = scim_group(&w).await?;
    let (s, body) = World::call_with(&w.a, "DELETE", &format!("/scim/v2/Groups/{id}"), SCIM, None).await;
    ensure(s.is_success(), format!("delete: {s} {body}"))?;
    let (s, body) = World::call_with(&w.b, "GET", &format!("/scim/v2/Groups/{id}"), SCIM, None).await;
    ensure(s == StatusCode::NOT_FOUND, format!("B no longer finds the group: {s} {body}"))
}
