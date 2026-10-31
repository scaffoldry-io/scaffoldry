//! Lifecycle phase 5: when an owner leaves.
//!
//! A workspace whose every owner has gone is listed. A transfer is previewed before it is made, and the preview changes
//! nothing. A transfer takes the old owner's access away first, so a failure part way leaves the old owner without
//! access and never both with it. No route removes, demotes or holds the last owner. Nobody is reattributed.
//! NIST SP 800-53 AC-2 and PS-4.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use scaffoldry_server::build_app_with_state;
use scaffoldry_server::service::identity::issue_test_token_and_user;
use scaffoldry_server::state::{CollaboratorRecord, ServerState, WorkspaceRecord};
use serde_json::{json, Value};
use tower::ServiceExt;

const PLATFORM_ADMIN: &str = "jordan.lee@state.edu";

struct World {
    db: common::TestDb,
    app: Router,
    admin: String,
    tag: String,
}

impl World {
    fn eppn(&self, who: &str) -> String {
        format!("{who}-{}@state.edu", self.tag)
    }
    fn token(&self, who: &str) -> String {
        issue_test_token_and_user(&self.eppn(who))
    }
}

async fn world() -> World {
    let db = common::TestDb::new();
    let state = db.state();
    state.seed_demo().expect("seed");
    let tag = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let app = build_app_with_state(state).expect("router");
    World { db, app, admin: issue_test_token_and_user(PLATFORM_ADMIN), tag }
}

async fn rest(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
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

fn member(state: &ServerState, workspace: &str, eppn: &str, role: &str) {
    let record = CollaboratorRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: workspace.to_string(),
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role: role.to_string(),
        scoped_affiliation: "faculty".to_string(),
        department: "physics".to_string(),
        added_at: String::new(),
        app_slug: String::new(),
    };
    state.repository.upsert_collaborator(&record).expect("a member");
    state.collaborators.write().unwrap().entry(workspace.to_string()).or_default().push(record);
}

/// A workspace of its own with these people in it. Stored, so the caches reload it.
fn workspace(state: &ServerState, id: &str, people: &[(&str, &str)]) {
    let mut ws: WorkspaceRecord = state.workspaces.read().unwrap().get("ws-physics-optics").cloned().expect("the demo workspace");
    ws.id = id.to_string();
    ws.name = format!("Workspace {id}");
    ws.code = id.to_uppercase();
    state.repository.upsert_workspace(&ws).expect("a workspace");
    state.workspaces.write().unwrap().insert(id.to_string(), ws);
    for (eppn, role) in people {
        member(state, id, eppn, role);
    }
}

/// What the stored memberships of a workspace say, as (address, role).
fn stored_members(db: &common::TestDb, workspace: &str) -> Vec<(String, String)> {
    let workspace = workspace.to_string();
    db.run(move |c| {
        c.query("SELECT eppn, role FROM workspace_collaborators WHERE workspace_id = $1 AND app_slug = '' ORDER BY eppn", &[&workspace])
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect()
    })
}

fn hold_in_database(db: &common::TestDb, eppn: &str) {
    let eppn = eppn.to_string();
    db.run(move |c| {
        let n = c
            .execute("UPDATE scim_users SET payload = jsonb_set(payload, '{admin_hold}', 'true') WHERE lower(user_name) = lower($1)", &[&eppn])
            .unwrap();
        assert_eq!(n, 1, "one stored person to hold");
    });
}

async fn user_id(w: &World, eppn: &str) -> String {
    let (_, body) = rest(&w.app, "GET", &format!("/api/v1/admin/users?search={eppn}"), &w.admin, None).await;
    body["users"][0]["id"].as_str().unwrap_or_else(|| panic!("no stored person for {eppn}: {body}")).to_string()
}

fn orphan_ids(body: &Value) -> Vec<String> {
    body["workspaces"].as_array().unwrap_or(&vec![]).iter().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()
}

fn ledger_count(db: &common::TestDb, decision: &str) -> i64 {
    let decision = decision.to_string();
    db.run(move |c| c.query_one("SELECT count(*) FROM governance_ledger WHERE decision_type = $1", &[&decision]).unwrap().get(0))
}

// 1. A workspace whose only owner is put on hold appears in the orphan list.
#[tokio::test]
async fn a_workspace_whose_only_owner_is_on_hold_is_listed() {
    let w = world().await;
    let state = w.db.state();
    let (owner, other, keeper) = (w.eppn("owner"), w.eppn("other"), w.eppn("keeper"));
    for who in ["owner", "other", "keeper"] {
        w.token(who);
    }
    workspace(&state, "ws-lone", &[(&owner, "owner")]);
    workspace(&state, "ws-pair", &[(&other, "owner"), (&keeper, "owner")]);

    let (s, before) = rest(&w.app, "GET", "/api/v1/admin/ownership/orphans", &w.admin, None).await;
    assert_eq!(s, StatusCode::OK, "{before}");
    assert!(!orphan_ids(&before).contains(&"ws-lone".to_string()), "an owner who is here is not an orphan: {before}");

    // The hold is already in the data, the way a sync or an older state leaves it. The hold route refuses it, below.
    hold_in_database(&w.db, &owner);
    hold_in_database(&w.db, &other);
    let (_, after) = rest(&w.app, "GET", "/api/v1/admin/ownership/orphans", &w.admin, None).await;
    let ids = orphan_ids(&after);
    assert!(ids.contains(&"ws-lone".to_string()), "the lone owner is on hold, so the workspace is listed: {after}");
    assert!(!ids.contains(&"ws-pair".to_string()), "a second owner who is here keeps it out of the list: {after}");
    let row = after["workspaces"].as_array().unwrap().iter().find(|r| r["id"] == "ws-lone").unwrap();
    assert!(row["owners"].to_string().contains(&owner), "the row names the owner who is gone: {row}");
    assert!(row["owners"].to_string().contains("hold"), "and says why: {row}");

    // Reading the list needs a platform admin or compliance.
    let (s, _) = rest(&w.app, "GET", "/api/v1/admin/ownership/orphans", &w.token("keeper"), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// 2. The preview names what waits on the old owner and changes nothing.
#[tokio::test]
async fn the_preview_lists_what_waits_and_changes_nothing() {
    let w = world().await;
    let state = w.db.state();
    let (from, to) = (w.eppn("from"), w.eppn("to"));
    w.token("from");
    w.token("to");
    workspace(&state, "ws-leaving", &[(&from, "owner")]);
    let proposal = uuid::Uuid::new_v4();
    w.db.run(move |c| {
        c.execute(
            "INSERT INTO app_proposals (id, workspace_id, app_slug, manifest, checks, summary, status, proposed_by) \
             VALUES ($1, 'ws-leaving', 'waiting-app', '{}', '[]', 'A change waiting for a decision', 'Pending', 'someone@state.edu')",
            &[&proposal],
        )
        .unwrap();
    });
    let count = |db: &common::TestDb| -> (i64, i64, i64, i64) {
        db.run(|c| {
            let mut one = |q: &str| -> i64 { c.query_one(q, &[]).unwrap().get(0) };
            (
                one("SELECT count(*) FROM workspace_collaborators"),
                one("SELECT count(*) FROM app_proposals"),
                one("SELECT count(*) FROM api_tokens"),
                one("SELECT count(*) FROM governance_ledger"),
            )
        })
    };
    let before = count(&w.db);

    let (s, body) = rest(&w.app, "POST", "/api/v1/admin/ownership/transfer/preview", &w.admin, Some(json!({ "from": from, "to": to }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let row = body["workspaces"].as_array().unwrap().iter().find(|r| r["id"] == "ws-leaving").unwrap_or_else(|| panic!("the workspace is in the preview: {body}"));
    assert_eq!(row["to_role"], Value::Null, "the new owner has no role there yet: {row}");
    assert!(row["gains"].as_array().is_some_and(|g| !g.is_empty()), "what the new owner gains is written out: {row}");
    assert!(row["loses"].as_array().is_some_and(|g| !g.is_empty()), "what the old owner loses is written out: {row}");
    assert!(row["open_proposals"].to_string().contains(&proposal.to_string()), "the proposal waiting on the owner is named: {row}");
    assert_eq!(body["can_transfer"], true, "{body}");

    assert_eq!(count(&w.db), before, "a preview changes nothing: no member, proposal, token or ledger row");
    assert_eq!(stored_members(&w.db, "ws-leaving"), vec![(from.clone(), "owner".to_string())]);

    // The blockers are written out.
    let blocked = |from: &str, to: &str| {
        let app = w.app.clone();
        let admin = w.admin.clone();
        let body = json!({ "from": from, "to": to });
        async move { rest(&app, "POST", "/api/v1/admin/ownership/transfer/preview", &admin, Some(body)).await.1 }
    };
    let same = blocked(&from, &from).await;
    assert_eq!(same["can_transfer"], false, "{same}");
    assert!(same.to_string().contains("same person"), "{same}");
    let gone = w.eppn("gone");
    w.token("gone");
    w.db.run({
        let gone = gone.clone();
        move |c| {
            c.execute("UPDATE scim_users SET active = false WHERE lower(user_name) = lower($1)", &[&gone]).unwrap();
        }
    });
    let inactive = blocked(&from, &gone).await;
    assert_eq!(inactive["can_transfer"], false, "{inactive}");
    assert!(inactive.to_string().contains("not active"), "{inactive}");
}

// 3. A transfer revokes the old owner before it adds the new one. A failed add leaves the revocation and the ledger entry.
#[tokio::test]
async fn a_transfer_revokes_from_before_it_adds_to() {
    let w = world().await;
    let state = w.db.state();
    let (from, to) = (w.eppn("from"), w.eppn("to"));
    w.token("from");
    w.token("to");
    workspace(&state, "ws-handover", &[(&from, "owner")]);

    // Make the add fail, whatever the reason: a trigger that refuses any row for the new owner.
    w.db.run({
        let to = to.clone();
        move |c| {
            c.batch_execute(&format!(
                "CREATE FUNCTION refuse_new_owner() RETURNS trigger AS $$ BEGIN \
                   IF lower(NEW.eppn) = lower('{to}') THEN RAISE EXCEPTION 'the add fails'; END IF; RETURN NEW; END $$ LANGUAGE plpgsql; \
                 CREATE TRIGGER refuse_new_owner BEFORE INSERT OR UPDATE ON workspace_collaborators FOR EACH ROW EXECUTE FUNCTION refuse_new_owner();"
            ))
            .unwrap();
        }
    });
    let entries = ledger_count(&w.db, "OwnershipTransferred");
    let (s, body) = rest(&w.app, "POST", "/api/v1/admin/ownership/transfer", &w.admin, Some(json!({ "from": from, "to": to, "reason": "Moved to another school" }))).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "the add failed, so the call failed: {body}");

    let members = stored_members(&w.db, "ws-handover");
    assert!(!members.iter().any(|(e, _)| *e == from), "the old owner is already without access: {members:?}");
    assert!(!members.iter().any(|(e, _)| *e == to), "and the new owner was not added: {members:?}");
    assert_eq!(ledger_count(&w.db, "OwnershipTransferred"), entries + 1, "the ledger entry was written before the change");
    let (_, listed) = rest(&w.app, "GET", "/api/v1/admin/ownership/orphans", &w.admin, None).await;
    assert!(orphan_ids(&listed).contains(&"ws-handover".to_string()), "the workspace now shows in the orphan list to be fixed: {listed}");

    // Without the fault the transfer completes. It is the same call, with `from` already gone from the workspace.
    w.db.run(|c| c.batch_execute("DROP TRIGGER refuse_new_owner ON workspace_collaborators").unwrap());
    workspace(&state, "ws-clean", &[(&from, "owner")]);
    let (s, body) = rest(&w.app, "POST", "/api/v1/admin/ownership/transfer", &w.admin, Some(json!({ "from": from, "to": to, "reason": "Moved to another school", "workspaces": ["ws-clean"] }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let members = stored_members(&w.db, "ws-clean");
    assert_eq!(members, vec![(to.clone(), "owner".to_string())], "the new owner holds it and the old one does not");

    // A blocked transfer is refused before anything is written.
    let (s, body) = rest(&w.app, "POST", "/api/v1/admin/ownership/transfer", &w.admin, Some(json!({ "from": to, "to": to, "reason": "Nothing to do" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
}

// 4. The last owner cannot be removed, demoted or held, through any route.
#[tokio::test]
async fn the_last_owner_cannot_be_removed_demoted_or_held() {
    let w = world().await;
    let state = w.db.state();
    let (owner, manager) = (w.eppn("owner"), w.eppn("manager"));
    w.token("owner");
    let manager_token = w.token("manager");
    workspace(&state, "ws-keep", &[(&owner, "owner"), (&manager, "admin")]);

    // The member routes.
    let (s, body) = rest(&w.app, "DELETE", &format!("/api/v1/workspaces/ws-keep/collaborators/{owner}"), &manager_token, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_owner", "{body}");
    let (s, body) = rest(&w.app, "PUT", &format!("/api/v1/workspaces/ws-keep/collaborators/{owner}"), &manager_token, Some(json!({ "role": "viewer" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_owner", "{body}");

    // The platform admin goes through the same function.
    let (s, body) = rest(&w.app, "DELETE", &format!("/api/v1/workspaces/ws-keep/collaborators/{owner}"), &w.admin, None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_owner", "{body}");

    // The one function that changes memberships refuses it for every caller, so a route added later cannot skip the rule.
    assert!(state.repository.remove_collaborator("ws-keep", &owner, "").is_err(), "the repository refuses to remove the last owner");
    let mut demoted = state.collaborators.read().unwrap()["ws-keep"].iter().find(|c| c.eppn == owner).cloned().unwrap();
    demoted.role = "viewer".to_string();
    assert!(state.repository.upsert_collaborator(&demoted).is_err(), "and to demote the last owner");

    // The hold route.
    let id = user_id(&w, &owner).await;
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), &w.admin, Some(json!({ "hold": true, "reason": "Under review" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_owner", "{body}");
    assert!(body.to_string().contains("ws-keep") || body.to_string().contains("Workspace ws-keep"), "the refusal names the workspace: {body}");

    // Nothing changed.
    assert_eq!(stored_members(&w.db, "ws-keep"), vec![(manager.clone(), "admin".to_string()), (owner.clone(), "owner".to_string())].into_iter().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>());

    // With a second owner, each of these is allowed.
    let second = w.eppn("second");
    w.token("second");
    member(&state, "ws-keep", &second, "owner");
    let (s, body) = rest(&w.app, "PUT", &format!("/api/v1/workspaces/ws-keep/collaborators/{owner}"), &manager_token, Some(json!({ "role": "viewer" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let id = user_id(&w, &second).await;
    let (s, body) = rest(&w.app, "POST", &format!("/api/v1/admin/users/{id}/hold"), &w.admin, Some(json!({ "hold": true, "reason": "Under review" }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "now `second` is the last live owner: {body}");

    // A directory sync reports a person who has left. The registry has to accept that, so it is listed, not refused.
    let sync_id = user_id(&w, &second).await;
    let (s, _) = rest(&w.app, "PATCH", &format!("/scim/v2/Users/{sync_id}"), "test-scim-token", Some(json!({ "Operations": [{ "op": "replace", "path": "active", "value": false }] }))).await;
    assert!(s.is_success(), "an IdP that deprovisions someone is not refused");
    let (_, listed) = rest(&w.app, "GET", "/api/v1/admin/ownership/orphans", &w.admin, None).await;
    assert!(orphan_ids(&listed).contains(&"ws-keep".to_string()), "the workspace whose last owner left is listed: {listed}");
}

// 5. History written by the old owner before a transfer still names them.
#[tokio::test]
async fn history_written_by_the_old_owner_still_names_them() {
    let w = world().await;
    let state = w.db.state();
    let (from, to) = (w.eppn("from"), w.eppn("to"));
    w.token("from");
    w.token("to");
    workspace(&state, "ws-history", &[(&from, "owner")]);
    w.db.run({
        let from = from.clone();
        move |c| {
            c.execute(
                "INSERT INTO record_history (app_slug, table_id, record_id, version, action, changes, actor, actor_kind, batch_id) \
                 VALUES ('history-app', 't1', 'r1', 1, 'create', '{}', $1, 'user', gen_random_uuid())",
                &[&from],
            )
            .unwrap();
        }
    });
    let actors = |db: &common::TestDb| -> Vec<(String, String)> {
        db.run(|c| c.query("SELECT actor, actor_kind FROM record_history WHERE app_slug = 'history-app' ORDER BY id", &[]).unwrap().iter().map(|r| (r.get(0), r.get(1))).collect())
    };
    let before = actors(&w.db);
    let (s, body) = rest(&w.app, "POST", "/api/v1/admin/ownership/transfer", &w.admin, Some(json!({ "from": from, "to": to, "reason": "Moved to another school" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(actors(&w.db), before, "no history row was reattributed");
    assert_eq!(before, vec![(from.clone(), "user".to_string())]);
}
