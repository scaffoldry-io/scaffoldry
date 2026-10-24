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
                let _ = c.execute(
                    "DELETE FROM workspace_collaborators WHERE workspace_id = $1 AND eppn LIKE $2",
                    &[&WORKSPACE, &like],
                );
                let _ = c.execute(
                    "DELETE FROM dataset_records WHERE app_slug LIKE $1",
                    &[&format!("%{tag}%")],
                );
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
        coordinates: None, sealed: false,
        custom_domain: None,
        custom_domain_verified: false,
        tables: vec![AppTable {
            id: "t".to_string(),
            name: "t".to_string(),
            slug: "t".to_string(),
            description: None,
            icon: None,
            fields: vec![FieldSpec::simple(
                "name",
                "name",
                FieldType::Text,
                false,
                false,
            )],
            primary_field: Some("name".to_string()),
            sample_records: vec![],
            coordinates: None, sealed: false,
            row_place: Default::default(),
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
    state
        .engine
        .write()
        .unwrap()
        .register_manifest(manifest(&slug))
        .unwrap();
    World {
        app: build_app_with_state(state.clone()).expect("router"),
        state,
        owner: issue_test_token_and_user(OWNER),
        _cleanup: Cleanup(tag.clone()),
        slug,
        tag,
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

impl World {
    fn person(&self, name: &str) -> (String, String) {
        let eppn = format!("{name}-{}@state.edu", self.tag);
        let token = issue_test_token_and_user(&eppn);
        (eppn, token)
    }

    async fn share(
        &self,
        principal_type: &str,
        principal_id: &str,
        role: &str,
    ) -> (StatusCode, Value) {
        send(
            &self.app,
            "POST",
            &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
            &self.owner,
            Some(json!({ "principal_type": principal_type, "principal_id": principal_id, "role": role })),
        )
        .await
    }

    /// A membership limited to one app.
    async fn share_app(
        &self,
        principal_type: &str,
        principal_id: &str,
        role: &str,
        app: &str,
    ) -> (StatusCode, Value) {
        send(
            &self.app,
            "POST",
            &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
            &self.owner,
            Some(json!({ "principal_type": principal_type, "principal_id": principal_id, "role": role, "app_slug": app })),
        )
        .await
    }

    /// What a person can do in the workspace's app: `none`, `read`, or `write`.
    async fn level(&self, token: &str) -> &'static str {
        self.level_in(token, &self.slug).await
    }

    /// The same for any app in the workspace.
    async fn level_in(&self, token: &str, slug: &str) -> &'static str {
        let (read, _) = send(
            &self.app,
            "GET",
            &format!("/api/v1/apps/{slug}/records"),
            token,
            None,
        )
        .await;
        if !read.is_success() {
            return "none";
        }
        let (write, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/apps/{slug}/records"),
            token,
            Some(json!({ "data": { "_table_id": "t", "name": "x" } })),
        )
        .await;
        if write.is_success() {
            "write"
        } else {
            "read"
        }
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
        let root_id = self
            .state
            .organizations
            .read()
            .unwrap()
            .values()
            .find(|o| o.parent_id.is_none())
            .map(|o| o.id);
        let parent = parent.or(root_id);
        let id = Uuid::new_v4();
        let node = OrganizationNode {
            id,
            parent_id: parent,
            name: format!("{name}-{}", self.tag),
            code: format!("{}-{}", name.to_uppercase(), self.tag),
            org_type: "Department".to_string(),
            hierarchy_key: "academic".to_string(),
            synthetic: false,
        };
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
        self.state
            .persist_role(&RoleRow {
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
        self.state.repository.ledger_page(None, 1_000_000).unwrap().iter().filter(|e| e.decision_type == decision).count()
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
    assert_eq!(
        w.level(&bob_token).await,
        "write",
        "added to the group, access on the next request"
    );
    w.group(&gid, &[&bob]);
    assert_eq!(
        w.level(&ann_token).await,
        "none",
        "removed from the group, access gone"
    );
}

// 2. The highest role among all memberships wins.
#[tokio::test]
async fn a_person_in_two_memberships_has_the_higher_role() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (ann, ann_token) = w.person("ann");
    let dept = w.unit(None, "dept");
    w.role_at(&ann, dept, None);
    assert_eq!(
        w.share("unit", &dept.to_string(), "Viewer").await.0,
        StatusCode::CREATED
    );
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
    assert_eq!(
        w.share("unit", &dept.to_string(), "Viewer").await.0,
        StatusCode::CREATED
    );
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
    assert_eq!(
        w.share("position", &format!("chair@{dept}"), "Editor")
            .await
            .0,
        StatusCode::CREATED
    );
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
        let (_, list) = send(
            &w.app,
            "GET",
            &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
            &w.owner,
            None,
        )
        .await;
        list.as_array()
            .unwrap()
            .iter()
            .filter(|c| {
                c["role"]
                    .as_str()
                    .is_some_and(|r| r.eq_ignore_ascii_case("owner"))
            })
            .map(|c| c["eppn"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        owners.len(),
        1,
        "the demo workspace has one owner: {owners:?}"
    );
    let (s, body) = send(
        &w.app,
        "DELETE",
        &format!("/api/v1/workspaces/{WORKSPACE}/collaborators/{}", owners[0]),
        &w.owner,
        None,
    )
    .await;
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
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if !rel.ends_with(".rs") || allowed.contains(&rel.as_str()) {
                continue;
            }
            let squeezed: String = std::fs::read_to_string(&path)
                .unwrap()
                .split_whitespace()
                .collect();
            if squeezed.contains(".collaborators.read(")
                || squeezed.contains(".collaborators.write(")
            {
                offenders.push(rel);
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these read collaborators directly: {offenders:?}"
    );
}

// 7. Every grant to a group, unit, or position writes one ledger entry.
#[tokio::test]
async fn every_grant_writes_one_ledger_entry() {
    let _db = common::TestDb::new();
    let w = world().await;
    let before = w.ledger_count(DecisionType::AccessRoleGranted);
    let dept = w.unit(None, "dept");
    for (t, id) in [
        ("group", format!("grp-{}", w.tag)),
        ("unit", dept.to_string()),
        ("position", format!("chair@{dept}")),
    ] {
        assert_eq!(
            w.share(t, &id, "Viewer").await.0,
            StatusCode::CREATED,
            "{t}"
        );
    }
    assert_eq!(w.ledger_count(DecisionType::AccessRoleGranted), before + 3);
}

// 2a. A membership can be limited to one app. It reaches that app, no other app, and no workspace action.
#[tokio::test]
async fn a_membership_limited_to_one_app_reaches_only_that_app() {
    let _db = common::TestDb::new();
    let w = world().await;
    let other_app = format!("mq-{}", w.tag);
    w.state
        .engine
        .write()
        .unwrap()
        .register_manifest(manifest(&other_app))
        .unwrap();

    let chem = w.unit(None, "chem");
    let phys = w.unit(None, "phys");
    let (ann, ann_token) = w.person("ann");
    let (bob, bob_token) = w.person("bob");
    w.role_at(&ann, chem, Some("chair"));
    w.role_at(&bob, phys, Some("chair"));

    let chair = format!("chair@{chem}");
    let (status, body) = w.share_app("position", &chair, "Admin", &w.slug).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    assert_eq!(
        w.level_in(&ann_token, &w.slug).await,
        "write",
        "the chair reaches the app named"
    );
    assert_eq!(
        w.level_in(&ann_token, &other_app).await,
        "none",
        "and not another app in the same workspace"
    );
    assert_eq!(
        w.level_in(&bob_token, &w.slug).await,
        "none",
        "and a chair elsewhere reaches nothing"
    );

    // An Admin on one app does not manage the workspace.
    let (status, _) = send(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
        &ann_token,
        Some(json!({ "principal_type": "user", "principal_id": bob, "role": "Viewer" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an app grant is not a workspace grant"
    );
}

// 2a. The rules around an app membership: the app must be in the workspace, an app membership cannot be an owner,
// a principal may hold both kinds, and removing one kind leaves the other.
#[tokio::test]
async fn app_memberships_follow_their_rules_and_remove_one_scope_at_a_time() {
    let _db = common::TestDb::new();
    let w = world().await;
    let (cy, cy_token) = w.person("cy");

    let (status, _) = w.share_app("user", &cy, "Viewer", "no-such-app").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the app must belong to the workspace"
    );
    let (status, _) = w.share_app("user", &cy, "Owner", &w.slug).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a membership limited to one app cannot be an owner"
    );

    assert_eq!(w.share("user", &cy, "Viewer").await.0, StatusCode::CREATED);
    assert_eq!(
        w.share_app("user", &cy, "Editor", &w.slug).await.0,
        StatusCode::CREATED,
        "both kinds at once"
    );
    assert_eq!(
        w.share_app("user", &cy, "Editor", &w.slug).await.0,
        StatusCode::BAD_REQUEST,
        "but not the same kind twice"
    );
    assert_eq!(
        w.level(&cy_token).await,
        "write",
        "the higher of the two applies in the app"
    );

    let uri = format!(
        "/api/v1/workspaces/{WORKSPACE}/collaborators/{cy}?app={}",
        w.slug
    );
    assert_eq!(
        send(&w.app, "DELETE", &uri, &w.owner, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        w.level(&cy_token).await,
        "read",
        "removing the app membership leaves the workspace-wide one"
    );
}

// 3. An admin can list every grant that names a person, and who made it.
#[tokio::test]
async fn an_admin_lists_direct_person_grants_with_who_made_them() {
    let _db = common::TestDb::new();
    let w = world().await;
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let (dee, _) = w.person("dee");
    let dept = w.unit(None, "dept");
    let (holder, _) = w.person("holder");
    w.role_at(&holder, dept, Some("chair"));

    assert_eq!(w.share("user", &dee, "Viewer").await.0, StatusCode::CREATED);
    assert_eq!(
        w.share_app("user", &dee, "Editor", &w.slug).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        w.share("position", &format!("chair@{dept}"), "Viewer")
            .await
            .0,
        StatusCode::CREATED
    );

    let (status, body) = send(&w.app, "GET", "/api/v1/admin/direct-grants", &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows: Vec<&Value> = body["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .filter(|g| g["who"] == dee.as_str())
        .collect();
    assert_eq!(
        rows.len(),
        2,
        "one workspace-wide and one app grant for the person: {body}"
    );
    let whole = rows
        .iter()
        .find(|g| g["scope"] == "workspace")
        .expect("a workspace-wide grant");
    let app = rows
        .iter()
        .find(|g| g["scope"] == "app")
        .expect("an app grant");
    assert_eq!(whole["role"], "Viewer");
    assert_eq!(app["role"], "Editor");
    assert_eq!(app["app_slug"], w.slug.as_str());
    for g in [whole, app] {
        assert_eq!(g["granted_by"], OWNER, "the ledger names who made it: {g}");
        assert!(
            g["granted_at"].as_str().is_some_and(|t| !t.is_empty()),
            "and when: {g}"
        );
        assert_eq!(g["kind"], "membership");
    }
    assert!(
        !body["grants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["who"].as_str().is_some_and(|x| x.contains("chair@"))),
        "a position is not a person: {body}"
    );

    // Only a Platform Admin may read it.
    let (status, _) = send(&w.app, "GET", "/api/v1/admin/direct-grants", &w.owner, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// 3b. A reason on a direct grant is optional. When given, the ledger and the admin report carry it.
#[tokio::test]
async fn a_direct_grant_may_carry_a_reason_and_does_not_need_one() {
    let _db = common::TestDb::new();
    let w = world().await;
    let admin = issue_test_token_and_user("jordan.lee@state.edu");
    let (eli, _) = w.person("eli");
    let (fay, _) = w.person("fay");

    let with = json!({ "principal_type": "user", "principal_id": eli, "role": "Viewer", "reason": "Covering for a sabbatical" });
    let (status, body) = send(
        &w.app,
        "POST",
        &format!("/api/v1/workspaces/{WORKSPACE}/collaborators"),
        &w.owner,
        Some(with),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        w.share("user", &fay, "Viewer").await.0,
        StatusCode::CREATED,
        "a grant without a reason still works"
    );

    let (_, body) = send(&w.app, "GET", "/api/v1/admin/direct-grants", &admin, None).await;
    let rows = body["grants"].as_array().expect("grants");
    let row = |who: &str| {
        rows.iter()
            .find(|g| g["who"] == who)
            .unwrap_or_else(|| panic!("no row for {who}: {body}"))
    };
    assert_eq!(row(&eli)["rationale"], "Covering for a sabbatical");
    assert!(
        row(&fay)["rationale"]
            .as_str()
            .is_some_and(|r| r.contains("added")),
        "without one, the ledger keeps its own wording: {}",
        row(&fay)
    );
}

// 4. My apps: the apps a person can reach, including one reached by an app-only membership.
#[tokio::test]
async fn my_apps_lists_the_apps_a_person_can_reach_and_how() {
    let _db = common::TestDb::new();
    let w = world().await;
    let other_app = format!("mq-{}", w.tag);
    w.state
        .engine
        .write()
        .unwrap()
        .register_manifest(manifest(&other_app))
        .unwrap();
    let (gus, gus_token) = w.person("gus");
    let (_hal, hal_token) = w.person("hal");
    assert_eq!(
        w.share_app("user", &gus, "Viewer", &w.slug).await.0,
        StatusCode::CREATED
    );

    let slugs = |body: &Value| -> Vec<(String, String)> {
        body["apps"]
            .as_array()
            .expect("apps")
            .iter()
            .map(|a| {
                (
                    a["slug"].as_str().unwrap().to_string(),
                    a["scope"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };

    let (status, body) = send(&w.app, "GET", "/api/v1/me/apps", &gus_token, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mine = slugs(&body);
    assert!(
        mine.contains(&(w.slug.clone(), "app".to_string())),
        "the app named, reached by an app membership: {mine:?}"
    );
    assert!(
        !mine.iter().any(|(s, _)| *s == other_app),
        "not the other app in the same workspace: {mine:?}"
    );

    let (_, body) = send(&w.app, "GET", "/api/v1/me/apps", &w.owner, None).await;
    let owner = slugs(&body);
    assert!(
        owner.contains(&(w.slug.clone(), "workspace".to_string()))
            && owner.contains(&(other_app.clone(), "workspace".to_string())),
        "{owner:?}"
    );

    let (_, body) = send(&w.app, "GET", "/api/v1/me/apps", &hal_token, None).await;
    assert!(
        !slugs(&body)
            .iter()
            .any(|(s, _)| *s == w.slug || *s == other_app),
        "a person with no grant sees neither"
    );

    let (status, _) = send(&w.app, "GET", "/api/v1/me/apps", "not-a-token", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
