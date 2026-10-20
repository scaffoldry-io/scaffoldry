//! The cases for `two_instance_test.rs`, one function for each write route or tool.
//!
//! A case writes through A, then reads the result through B. B is a second server on the same database that was built
//! before the write, so what it shows is what it learned from the database on the request that read it.

use crate::two_instance::{ensure, status, unique, Entry, Outcome, World, ADMIN, EDITOR, OWNER, ROOT, WORKSPACE};
use axum::http::StatusCode;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// One case. `case!(f)` is `f(world)`. `case!(f, root)` is `f(world, root)`, for a route that is mounted both at the
/// root and under `/api/v1`: the same handler, reached by two paths, each with its own case.
macro_rules! case {
    ($f:ident) => {
        crate::two_instance::Entry::Case(Box::new(|w| Box::pin($f(w))))
    };
    ($f:ident, $root:expr) => {
        crate::two_instance::Entry::Case(Box::new(|w| Box::pin($f(w, $root))))
    };
}

/// A case that needs `SCAFFOLDRY_FILES_DIR`. See `two_instance_files_test.rs`.
macro_rules! files_case {
    ($f:ident) => {
        crate::two_instance::Entry::FilesCase(Box::new(|w| Box::pin($f(w))))
    };
}

mod apps;
mod files;
mod platform;
mod scim;
mod tools;

pub type Entries = BTreeMap<String, Entry>;

pub fn entries() -> Entries {
    let mut m = Entries::new();
    platform::add(&mut m);
    apps::add(&mut m);
    files::add(&mut m);
    scim::add(&mut m);
    tools::add(&mut m);
    m
}

/// A route that changes nothing that is stored.
pub fn exempt(m: &mut Entries, key: &str, why: &'static str) {
    m.insert(key.to_string(), Entry::Exempt(why));
}

/// The fixtures every case may use: a collaborator on the demo workspace.
pub async fn prepare(w: &World) -> Outcome {
    status(
        w.wa("POST", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), OWNER, Some(json!({ "eppn": EDITOR, "role": "editor" }))).await,
        StatusCode::CREATED,
        "add the shared editor",
    )?;
    Ok(())
}

/// Whether `needle` appears anywhere in `v`.
pub fn has(v: &Value, needle: &str) -> bool {
    v.to_string().contains(needle)
}

/// A new unit under the root, through A. Returns its id.
pub async fn make_unit(w: &World, name: &str) -> Result<String, String> {
    let body = status(
        w.wa("POST", "/api/v1/orgs", ADMIN, Some(json!({ "name": name, "code": name, "org_type": "Department", "parent_id": ROOT }))).await,
        StatusCode::CREATED,
        "make a unit",
    )?;
    Ok(body["id"].as_str().ok_or("a unit id")?.to_string())
}

/// A new workspace, through A. Returns its id.
pub async fn make_workspace(w: &World, code: &str, visibility: &str) -> Result<String, String> {
    let body = status(
        w.wa("POST", "/api/v1/workspaces", ADMIN, Some(json!({ "name": code, "code": code, "visibility": visibility, "department": "biology" }))).await,
        StatusCode::CREATED,
        "make a workspace",
    )?;
    Ok(body["id"].as_str().ok_or("a workspace id")?.to_string())
}

/// A new person, through A. Returns the user object.
pub async fn make_user(w: &World, user_name: &str) -> Result<Value, String> {
    let body = status(
        w.wa(
            "POST",
            "/api/v1/admin/users",
            ADMIN,
            Some(json!({ "userName": user_name, "name": "Test Person", "email": user_name, "affiliation": "faculty", "department": "Biology", "title": "Lecturer", "reason": "Fixture" })),
        )
        .await,
        StatusCode::CREATED,
        "make a user",
    )?;
    Ok(body["user"].clone())
}

pub fn person(prefix: &str) -> String {
    format!("{}@state.edu", unique(prefix))
}

/// The manifest every app case uses: a name, a gpa and a status.
pub fn manifest(slug: &str) -> Value {
    json!({
        "slug": slug, "title": "Applications", "description": "d", "organization_code": "PHYS", "department": "physics", "views": [],
        "tables": [{ "id": "apps", "name": "Applications", "slug": "apps", "fields": [
            { "name": "name", "label": "name", "field_type": "Text", "required": false, "ferpa_sensitive": false },
            { "name": "gpa", "label": "gpa", "field_type": "Number", "required": false, "ferpa_sensitive": false },
            { "name": "status", "label": "status", "field_type": "Text", "required": false, "ferpa_sensitive": false }
        ]}]
    })
}

/// A new app in the demo workspace, through A.
pub async fn make_app(w: &World) -> Result<String, String> {
    let slug = unique("p6app");
    status(w.wa("POST", &format!("/api/v1/workspaces/{WORKSPACE}/apps"), OWNER, Some(manifest(&slug))).await, StatusCode::CREATED, "make an app")?;
    Ok(slug)
}

/// A new record in `slug`, through A. Returns its id.
pub async fn make_record(w: &World, slug: &str, name: &str) -> Result<String, String> {
    let body = status(
        w.wa("POST", &format!("/api/v1/apps/{slug}/records"), OWNER, Some(json!({ "data": { "_table_id": "apps", "name": name, "gpa": 3.1 } }))).await,
        StatusCode::CREATED,
        "make a record",
    )?;
    Ok(body["id"].as_str().ok_or("a record id")?.to_string())
}
