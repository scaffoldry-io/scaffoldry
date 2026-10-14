//! Access rules phase 1: a workspace is shared with a person, a group, a unit, or a position.
//!
//! Each test uses fresh people and ids, and deletes the collaborator rows it added.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_core::ledger::DecisionType;
use scaffoldry_engine::{AppManifest, AppTable, FieldSpec, FieldType};
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{OrganizationNode, PositionType, RoleRow, ScimGroup, ServerState};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const OWNER: &str = "prof.curie@science.state.edu";
const WORKSPACE: &str = "ws-bio-lab";

fn db_url() -> String {
    scaffoldry_server::testing::database_url()
}

/// Removes the non-person collaborator rows and the people this test added.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let (tag, url) = (self.0.clone(), db_url());
        let _ = std::thread::spawn(move || {
            if let Ok(mut c) = postgres::Client::connect(&url, postgres::NoTls) {
                let like = format!("%{tag}%");
                let _ = c.execute("DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2", &[&WORKSPACE, &like]);
                let _ = c.execute("DELETE FROM dataset_records WHERE app_slug LIKE $1", &[&format!("%{tag}%")]);
            }
        })
        .join();
    }
}

fn manifest(slug: &str) -> AppManifest {
    AppManifest {
        slug: slug.to_string(),
        title: "Principals".to_string(),
        description: String::new(),
        organization_code: "BIO".to_string(),
        department: "biology".to_string(),
        workspace_id: Some(WORKSPACE.to_string()),
        herm_capability_id: None,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t".to_string(),
            name: "t".to_string(),
            slug: "t".to_string(),
            description: None,
            icon: None,
            fields: vec![FieldSpec::simple("name", "name", FieldType::Text, false, false)],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
        }],
        relationships: vec![],
        views: vec![],
        ceds_mappings: Default::default(),
        pages: vec![],
        access: Default::default(),
    }
}

struct World {
    app: Router,
    state: Arc<ServerState>,
    owner: String,
    slug: String,
    tag: String,
    _cleanup: Cleanup,
}

async fn world() -> World {
    let state = Arc::new(ServerState::new().expect("state"));
    state.seed_demo().expect("seed");
    let tag = Uuid::new_v4().to_string()[..8].to_string();
    let slug = format!("mp-{tag}");
    state.engine.write().unwrap().register_manifest(manifest(&slug)).unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        owner: issue_test_token_and_user(OWNER),
        _cleanup: Cleanup(tag.clone()),
        slug,
        tag,
    }
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

impl World {
    fn person(&self, name: &str) -> (String, String) {
        let eppn = format!("{name}-{}@state.edu", self.tag);
        let token = issue_test_token_and_user(&eppn);
        (eppn, token)
    }

    async fn share(&self, principal_type: &str, principal_id: &str, role: &str) -> (StatusCode, Value) {
        send(
            &self.app,
            "POST",
            &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
            &self.owner,
            Some(json!({ "principal_type": principal_type, "principal_id": principal_id, "role": role })),
        )
        .await
    }

    /// What a person can do in the workspace's app: `none`, `read`, or `write`.
    async fn level(&self, token: &str) -> &'static str {
        let (read, _) = send(&self.app, "GET", &format!("/api/v1/apps/{}/records", self.slug), token, None).await;
        if !read.is_success() {
            return "none";
        }
        let (write, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/apps/{}/records", self.slug),
            token,
            Some(json!({ "data": { "_table_id": "t", "name": "x" } })),
        )
        .await;
        if write.is_success() { "write" } else { "read" }
    }

    fn group(&self, id: &str, members: &[&str]) {
        self.state
            .repository
            .upsert_scim_group(&ScimGroup {
                id: id.to_string(),
                display_name: id.to_string(),
                members: members.iter().map(|m| json!({ "value": m })).collect(),
            })
            .unwrap();
    }

    fn unit(&self, parent: Option<Uuid>, name: &str) -> Uuid {
        let id = Uuid::new_v4();
        let node = OrganizationNode { id, parent_id: parent, name: format!("{name}-{}", self.tag), code: format!("{}-{}", name.to_uppercase(), self.tag), org_type: "Department".to_string() };
        self.state.persist_organization(&node).unwrap();
        id
    }

    fn role_at(&self, eppn: &str, unit: Uuid, position: Option<&str>) {
        // A held position has a foreign key to its type, so the type is stored first.
        if let Some(key) = position {
            self.state
                .persist_position_type(&PositionType {
                    key: key.to_string(),
                    name: key.to_string(),
                    description: String::new(),
                    org_types: vec![],
                    max_holders: 50,
                    retired_at: None,
                    created_by: "test".to_string(),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .unwrap();
        }
        self.state.persist_role(&RoleRow {
            id: Uuid::new_v4(),
            person_id: Uuid::new_v4(),
            eppn: eppn.to_string(),
            organization_id: unit,
            role_title: position.unwrap_or("Faculty").to_string(),
            scoped_affiliation: "faculty".to_string(),
            is_primary: true,
            source: "manual".to_string(),
            position_key: position.map(str::to_string),
        })
        .unwrap();
    }

    fn ledger_count(&self, decision: DecisionType) -> usize {
        self.state.ledger.read().unwrap().iter().filter(|e| e.decision_type == decision).count()
    }
}

// 1. A group gives each current member the group's role, and follows the group.
#[tokio::test]
async fn a_group_gives_each_current_member_its_role_and_follows_the_group() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (ann, ann_token) = w.person("ann");
    let (bob, bob_token) = w.person("bob");
    let gid = format!("grp-{}", w.tag);
    w.group(&gid, &[&ann]);
    let (s, body) = w.share("group", &gid, "Editor").await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(w.level(&ann_token).await, "write");
    assert_eq!(w.level(&bob_token).await, "none");

    w.group(&gid, &[&ann, &bob]);
    assert_eq!(w.level(&bob_token).await, "write", "added to the group, access on the next request");
    w.group(&gid, &[&bob]);
    assert_eq!(w.level(&ann_token).await, "none", "removed from the group, access gone");
}

// 2. The highest role among all memberships wins.
#[tokio::test]
async fn a_person_in_two_memberships_has_the_higher_role() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (ann, ann_token) = w.person("ann");
    let dept = w.unit(None, "dept");
    w.role_at(&ann, dept, None);
    assert_eq!(w.share("unit", &dept.to_string(), "Viewer").await.0, StatusCode::CREATED);
    assert_eq!(w.level(&ann_token).await, "read");
    let (s, body) = send(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
        &w.owner,
        Some(json!({ "eppn": ann, "role": "Editor" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    assert_eq!(w.level(&ann_token).await, "write");
}

// 3. A unit covers a person whose role is at a unit below it, and not a sibling's.
#[tokio::test]
async fn a_unit_covers_units_below_it_and_not_a_sibling() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (below, below_token) = w.person("below");
    let (sibling, sibling_token) = w.person("sibling");
    let college = w.unit(None, "college");
    let dept = w.unit(Some(college), "dept");
    let other = w.unit(Some(college), "other");
    let lab = w.unit(Some(dept), "lab");
    w.role_at(&below, lab, None);
    w.role_at(&sibling, other, None);
    assert_eq!(w.share("unit", &dept.to_string(), "Viewer").await.0, StatusCode::CREATED);
    assert_eq!(w.level(&below_token).await, "read");
    assert_eq!(w.level(&sibling_token).await, "none");
}

// 4. A position follows the holder when it changes hands.
#[tokio::test]
async fn a_position_follows_the_holder() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (ann, ann_token) = w.person("ann");
    let (bob, bob_token) = w.person("bob");
    let dept = w.unit(None, "dept");
    w.role_at(&ann, dept, Some("chair"));
    assert_eq!(w.share("position", &format!("chair@{dept}"), "Editor").await.0, StatusCode::CREATED);
    assert_eq!(w.level(&ann_token).await, "write");
    assert_eq!(w.level(&bob_token).await, "none");
    for r in w.state.roles_of(&ann) {
        w.state.delete_role(r.id).unwrap();
    }
    w.role_at(&bob, dept, Some("chair"));
    assert_eq!(w.level(&ann_token).await, "none");
    assert_eq!(w.level(&bob_token).await, "write");
}

// 5. A group cannot be an owner. The last user owner cannot be removed.
#[tokio::test]
async fn a_group_cannot_own_and_the_last_user_owner_stays() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (s, body) = w.share("group", &format!("grp-{}", w.tag), "Owner").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");

    let owners: Vec<String> = {
        let (_, list) = send(&w.app, "GET", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"), &w.owner, None).await;
        list.as_array().unwrap().iter().filter(|c| c["role"].as_str().is_some_and(|r| r.eq_ignore_ascii_case("owner"))).map(|c| c["eppn"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(owners.len(), 1, "the demo workspace has one owner: {owners:?}");
    let (s, body) = send(&w.app, "DELETE", &format!("/api/v1/workspaces/{WORKSPACE}/collaborators/{}", owners[0]), &w.owner, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    let (s, body) = send(
        &w.app,
        "PUT",
        &format!("/api/v1/workspaces/{WORKSPACE}/collaborators/{}", owners[0]),
        &w.owner,
        Some(json!({ "role": "Viewer" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "demoting the last owner: {body}");
}

// 6. No code outside the member module and the member routes reads the collaborator table directly.
#[test]
fn only_the_member_module_and_the_member_routes_read_collaborators() {
    let _db = common::TestDb::new();
    let allowed = [
        "state.rs",
        "repository.rs",
        "service/members.rs",
        "service/workspaces.rs",
        "routes/workspaces.rs",
        "routes/admin_workspaces.rs",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    let mut offenders = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if !rel.ends_with(".rs") || allowed.contains(&rel.as_str()) {
                continue;
            }
            let squeezed: String = std::fs::read_to_string(&path).unwrap().split_whitespace().collect();
            if squeezed.contains(".collaborators.read(") || squeezed.contains(".collaborators.write(") {
                offenders.push(rel);
            }
        }
    }
    assert!(offenders.is_empty(), "these read collaborators directly: {offenders:?}");
}

// 7. Every grant to a group, unit, or position writes one ledger entry.
#[tokio::test]
async fn every_grant_writes_one_ledger_entry() {
    let _db = common::TestDb::new();
    let w = world().await;
    let before = w.ledger_count(DecisionType::AccessRoleGranted);
    let dept = w.unit(None, "dept");
    for (t, id) in [("group", format!("grp-{}", w.tag)), ("unit", dept.to_string()), ("position", format!("chair@{dept}"))] {
        assert_eq!(w.share(t, &id, "Viewer").await.0, StatusCode::CREATED, "{t}");
    }
    assert_eq!(w.ledger_count(DecisionType::AccessRoleGranted), before + 3);
}
