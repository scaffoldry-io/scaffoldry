//! Access model phase 7: explaining access, and reviewing it by person and by object. What a grant reaches is decided by
//! `scaffoldry_core::reach`. This file asks that function the questions and says the answers in words an administrator
//! can read. It writes nothing.

use crate::service::access::{decide_for, principal_of, workspace_ctx};
use crate::service::coordinates::{cedar_place, object_for, with_world};
use crate::service::members::{self, Role};
use crate::service::ServiceError;
use crate::state::{AuthUser, SharedState, WorkspaceRecord};
use scaffoldry_core::reach::{role_at, trace, AnchorKind, Grant, HolderSet, Miss, Object};
use scaffoldry_engine::AppManifest;
use serde_json::{json, Value};
use uuid::Uuid;

fn bad(msg: impl Into<String>) -> ServiceError {
    ServiceError::BadRequest(msg.into())
}

/// A workspace, an app in it, or a table of that app: the three things a grant can reach.
pub struct Target {
    /// The container id, as a grant anchors on it: `workspace:<id>`, `app:<slug>`, `table:<slug>/<table id>`.
    pub container: String,
    pub workspace: WorkspaceRecord,
    pub app: Option<AppManifest>,
    pub table: Option<String>,
}

impl Target {
    pub fn object(&self, state: &SharedState) -> Option<Object> {
        with_world(state, |places, tree| {
            let app = self.app.as_ref().map(|m| (m.slug.as_str(), Some(m)));
            object_for(places, tree, &self.workspace, app, self.table.as_deref())
        })
        .ok()
    }
}

fn manifest_of(state: &SharedState, slug: &str) -> Option<AppManifest> {
    use scaffoldry_engine::HostRouter;
    state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(crate::service::preview::stored_slug(slug)).cloned()
}

fn workspace_named(state: &SharedState, id: &str) -> Option<WorkspaceRecord> {
    state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(id).cloned()
}

/// Reads `workspace:<id>`, `app:<slug>` or `table:<slug>/<table id>`.
pub fn parse_target(state: &SharedState, text: &str) -> Result<Target, ServiceError> {
    let not_found = || ServiceError::NotFound(format!("'{text}' is not a workspace, app or table here"));
    let hint = || bad("object must be workspace:<id>, app:<slug> or table:<slug>/<table id>");
    let (kind, rest) = text.split_once(':').ok_or_else(hint)?;
    match kind {
        "workspace" => {
            let workspace = workspace_named(state, rest).ok_or_else(not_found)?;
            Ok(Target { container: text.to_string(), workspace, app: None, table: None })
        }
        "app" | "table" => {
            let (slug, table) = match (kind, rest.split_once('/')) {
                ("app", None) => (rest, None),
                ("table", Some((slug, table))) if !table.is_empty() => (slug, Some(table.to_string())),
                _ => return Err(hint()),
            };
            let manifest = manifest_of(state, slug).ok_or_else(not_found)?;
            if table.as_deref().is_some_and(|t| !manifest.tables.iter().any(|x| x.id == t)) {
                return Err(not_found());
            }
            let workspace = manifest.workspace_id.as_deref().and_then(|w| workspace_named(state, w)).ok_or_else(not_found)?;
            Ok(Target { container: text.to_string(), workspace, app: Some(manifest), table })
        }
        _ => Err(hint()),
    }
}

/// Every workspace, app and table, for a review that asks about all of them.
pub fn all_targets(state: &SharedState) -> Vec<Target> {
    let workspaces: Vec<WorkspaceRecord> = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    let manifests = state.engine.read().unwrap_or_else(|p| p.into_inner()).list_manifests();
    let mut out = Vec::new();
    for ws in workspaces {
        out.push(Target { container: format!("workspace:{}", ws.id), workspace: ws.clone(), app: None, table: None });
        for m in manifests.iter().filter(|m| m.workspace_id.as_deref() == Some(ws.id.as_str())) {
            out.push(Target { container: format!("app:{}", m.slug), workspace: ws.clone(), app: Some(m.clone()), table: None });
            for t in &m.tables {
                out.push(Target { container: crate::service::coordinates::table_container(&m.slug, &t.id), workspace: ws.clone(), app: Some(m.clone()), table: Some(t.id.clone()) });
            }
        }
    }
    out
}

/// A unit's name, or a workspace's, app's or table's title, for an id that a path names. An id nobody knows is returned as it is.
fn name_of(state: &SharedState, id: &str) -> String {
    if let Some(unit) = Uuid::parse_str(id).ok().and_then(|u| state.organizations.read().unwrap_or_else(|p| p.into_inner()).get(&u).map(|o| o.name.clone())) {
        return unit;
    }
    match parse_target(state, id) {
        Ok(t) => match (&t.app, &t.table) {
            (None, _) => t.workspace.name.clone(),
            (Some(m), None) => m.title.clone(),
            (Some(m), Some(table)) => format!("{} / {}", m.title, m.tables.iter().find(|x| &x.id == table).map(|x| x.name.clone()).unwrap_or_else(|| table.clone())),
        },
        Err(_) => id.to_string(),
    }
}

/// Where a grant is anchored, in words: the unit's name, the container's title, or "the holder's own unit".
fn anchor_name(state: &SharedState, g: &Grant) -> String {
    match g.anchor_kind {
        AnchorKind::HolderUnit => "the holder's own unit".to_string(),
        _ => g.anchor_id.as_deref().map(|a| name_of(state, a)).unwrap_or_default(),
    }
}

/// One grant in words: who holds it and where it is anchored.
fn grant_json(g: &Grant) -> Value {
    let anchor = match g.anchor_kind {
        AnchorKind::Container => g.anchor_id.clone().unwrap_or_default(),
        AnchorKind::Unit => format!("unit:{}", g.anchor_id.clone().unwrap_or_default()),
        AnchorKind::HolderUnit => "holder_unit".to_string(),
    };
    // A membership or an appointment is a grant that is not stored. It has no id to give.
    let (id, source) = match g.id {
        i if i > 0 => (json!(i), "stored"),
        i if i <= -1_000_000 => (Value::Null, "appointment"),
        _ => (Value::Null, "membership"),
    };
    json!({
        "grant_id": id, "source": source, "holder": format!("{}:{}", g.holder_kind.as_str(), g.holder_id),
        "role": g.role.as_str(), "anchor": anchor, "depth": g.depth, "domain": g.domain,
    })
}

/// Everything one person holds that could reach an object in this workspace: their holder set, and every grant they hold,
/// including memberships of the workspace.
pub fn grants_of(state: &SharedState, eppn: &str, workspace: &str) -> Option<(HolderSet, Vec<Grant>)> {
    let (holders, mut grants) = members::standing(eppn, state)?;
    grants.extend(members::membership_grants(workspace, state).into_iter().filter(|g| holders.is_holder(g)));
    Some((holders, grants))
}

/// Who may ask about a person's access to an object: the person, a Platform Admin or compliance, or an Org Unit Admin whose
/// subtree holds a unit the object sits at.
fn may_explain(state: &SharedState, caller: &AuthUser, person: &str, object: &Object) -> bool {
    if caller.eppn.eq_ignore_ascii_case(person) || crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok() {
        return true;
    }
    let admin_units: Vec<Uuid> = state.roles_of(&caller.eppn).iter().filter(|r| r.scoped_affiliation == "unit_admin").map(|r| r.organization_id).collect();
    let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
    admin_units.iter().any(|a| object.units.values().any(|u| tree.contains(*a, *u)))
}

/// The person as the policy engine knows them, with a plain member standing in for an address the directory lacks.
fn user_named(state: &SharedState, eppn: &str, department: &str) -> AuthUser {
    crate::service::identity::resolve_user(eppn, state).unwrap_or_else(|| AuthUser {
        eppn: eppn.to_string(),
        name: eppn.to_string(),
        role_title: "Member".to_string(),
        affiliation: "member".to_string(),
        department: department.to_string(),
        view_as_by: None,
        via_client: None,
    })
}

/// What the workspace's guards and the app's rules say about this person on this object.
fn narrowing(state: &SharedState, target: &Target, person: &AuthUser, role: Option<Role>) -> (Value, Vec<Value>, Vec<String>) {
    let ws = &target.workspace;
    let mut ctx = workspace_ctx(state, ws, role.map(|r| r.as_str()), target.app.as_ref().map(|_| Default::default()));
    if let Some(m) = &target.app {
        (ctx.domain_path, ctx.sensitivity_rank) = cedar_place(state, ws, Some(m));
    }
    let action = if target.app.is_some() { "read_app" } else { "access_workspace" };
    let decision = decide_for(state, &principal_of(state, person), action, &scaffoldry_policy::entities::Resource::Workspace(ctx));
    let policy = json!({
        "action": action, "allowed": decision.allowed, "deciding": decision.policy, "reasons": decision.reasons, "error": decision.error,
    });
    let guards: Vec<Value> = state
        .guards
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(&ws.id)
        .and_then(|versions| versions.iter().max_by_key(|g| g.version))
        .and_then(|g| g.rules.as_array().cloned())
        .unwrap_or_default();
    let rules: Vec<String> = match (&target.app, &target.table) {
        (Some(m), Some(table)) => {
            let mine = role.map(|r| r.as_str());
            m.access
                .rows
                .iter()
                .filter(|r| &r.table_id == table && r.ops.contains(&scaffoldry_core::access::RowOp::Read))
                .filter(|r| !mine.is_some_and(|me| r.except_roles.iter().any(|x| x.eq_ignore_ascii_case(me))))
                .map(|r| r.id.clone())
                .collect()
        }
        _ => Vec::new(),
    };
    (policy, guards, rules)
}

/// Why a person has the role they have on an object, or none: each grant that reached it with its path, each grant that did
/// not with the first axis that failed, any sealed container that stopped one, and what the policy engine, the guards and
/// the rules add.
pub fn explain(state: &SharedState, caller: &AuthUser, person: &str, object_text: &str) -> Result<Value, ServiceError> {
    let person = person.trim().to_lowercase();
    if person.is_empty() {
        return Err(bad("person is required"));
    }
    let target = parse_target(state, object_text)?;
    let object = target.object(state).ok_or_else(|| ServiceError::Internal("The unit tree could not be read".to_string()))?;
    if !may_explain(state, caller, &person, &object) {
        return Err(ServiceError::forbidden("You may explain your own access. Another person's needs a Platform Admin, compliance, or an Org Unit Admin of the unit the object sits in"));
    }
    let (holders, grants) = grants_of(state, &person, &target.workspace.id).ok_or_else(|| ServiceError::Internal("The person's grants could not be read".to_string()))?;
    let now = chrono::Utc::now().timestamp();
    let (traces, role) = with_world(state, |places, tree| {
        let traces: Vec<(Grant, scaffoldry_core::reach::Trace)> = grants.iter().map(|g| (g.clone(), trace(g, &holders, &object, tree, places, now))).collect();
        (traces, role_at(&holders, &grants, &object, tree, places, now).map(|(r, _)| r))
    })?;
    let mut reached = Vec::new();
    let mut missed = Vec::new();
    let mut sealed_by: Vec<String> = Vec::new();
    for (g, t) in &traces {
        let mut j = grant_json(g);
        match &t.missed {
            None => {
                j["path"] = json!(t.path);
                j["path_names"] = json!(t.path.iter().map(|id| name_of(state, id)).collect::<Vec<_>>());
                j["anchor_name"] = json!(anchor_name(state, g));
                reached.push(j);
            }
            Some(m) => {
                j["axis"] = json!(m.axis());
                j["anchor_name"] = json!(anchor_name(state, g));
                if let Miss::Sealed(container) = m {
                    j["sealed_by"] = json!(container);
                    if !sealed_by.contains(container) {
                        sealed_by.push(container.clone());
                    }
                }
                missed.push(j);
            }
        }
    }
    let user = user_named(state, &person, &target.workspace.department);
    let (policy, guards, rules) = narrowing(state, &target, &user, role);
    Ok(json!({
        "person": person, "object": object_text, "role": role.map(|r| r.as_str()),
        "platform_admin": principal_of(state, &user).is_platform_admin,
        "reached": reached, "missed": missed, "sealed_by": sealed_by,
        "policy": policy, "guards": guards, "rules": rules,
    }))
}

/// A Platform Admin or compliance, or an Org Unit Admin of a unit above one the object sits at. The review of an object
/// shows who holds access to it, so it is for the people who administer it.
fn may_review_object(state: &SharedState, caller: &AuthUser, object: &Object) -> bool {
    may_explain(state, caller, "", object)
}

fn count_kind(target: &Target) -> &'static str {
    match (&target.app, &target.table) {
        (_, Some(_)) => "tables",
        (Some(_), None) => "apps",
        (None, None) => "workspaces",
    }
}

/// Every active stored grant, a page at a time.
fn active_grants(state: &SharedState) -> Result<Vec<crate::repository_grants::GrantRow>, ServiceError> {
    let mut out = Vec::new();
    let mut after = 0;
    loop {
        let filter = crate::repository_grants::GrantFilter { state: "active".to_string(), after, limit: 500, ..Default::default() };
        let page = state.repository.list_grant_rows(&filter).map_err(|e| ServiceError::Internal(e.to_string()))?;
        let done = (page.len() as i64) < 500;
        if let Some(last) = page.last() {
            after = last.grant.id;
        }
        out.extend(page);
        if done {
            return Ok(out);
        }
    }
}

/// Every grant whose reach contains the object, from the grants everyone holds: the stored ones, and the memberships of its
/// workspace. A grant is asked what it reaches whoever holds it. A position held at any unit has no one holder to ask with,
/// so it is listed apart.
pub fn review_object(state: &SharedState, caller: &AuthUser, object_text: &str) -> Result<Value, ServiceError> {
    let target = parse_target(state, object_text)?;
    let object = target.object(state).ok_or_else(|| ServiceError::Internal("The unit tree could not be read".to_string()))?;
    if !may_review_object(state, caller, &object) {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or an Org Unit Admin of the unit the object sits in required"));
    }
    let now = chrono::Utc::now().timestamp();
    let stored = active_grants(state)?;
    let memberships = members::membership_grants(&target.workspace.id, state);
    let (mut grants, mut unplaced) = (Vec::new(), Vec::new());
    with_world(state, |places, tree| {
        let rows = stored.iter().map(|r| (&r.grant, Some(r))).chain(memberships.iter().map(|g| (g, None)));
        for (g, row) in rows {
            let holder = HolderSet::holding(g);
            let t = trace(g, &holder, &object, tree, places, now);
            let mut j = grant_json(g);
            if let Some(r) = row {
                j["reason"] = json!(r.reason);
                j["created_by"] = json!(r.created_by);
            }
            match &t.missed {
                None => {
                    j["path"] = json!(t.path);
                    grants.push(j);
                }
                Some(Miss::Holder) if g.anchor_kind == AnchorKind::HolderUnit => unplaced.push(j),
                Some(_) => {}
            }
        }
    })?;
    Ok(json!({ "object": object_text, "grants": grants, "depends_on_holder": unplaced, "sealed": object.sealed.iter().collect::<Vec<_>>() }))
}

/// A person's holder set, every grant that names them or a holder they belong to, and how many workspaces, apps and tables
/// each reaches. The counts are also added up by the hierarchy the grant is anchored in, with `container` for the grants
/// that are anchored at a workspace or an app.
pub fn review_person(state: &SharedState, caller: &AuthUser, person: &str) -> Result<Value, ServiceError> {
    let person = person.trim().to_lowercase();
    let (holders, standing) = members::standing(&person, state).ok_or_else(|| ServiceError::Internal("The person's grants could not be read".to_string()))?;
    let allowed = crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok() || {
        let tree = state.tree.read().unwrap_or_else(|p| p.into_inner());
        state.roles_of(&caller.eppn).iter().any(|r| r.scoped_affiliation == "unit_admin" && holders.units.contains(&r.organization_id) && tree.contains(r.organization_id, r.organization_id))
    };
    if !allowed {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or an Org Unit Admin of the person's unit required"));
    }
    let now = chrono::Utc::now().timestamp();
    let targets = all_targets(state);
    let objects: Vec<Option<Object>> = targets.iter().map(|t| t.object(state)).collect();
    let mut grants: Vec<Grant> = standing;
    for ws in state.workspaces.read().unwrap_or_else(|p| p.into_inner()).keys() {
        grants.extend(members::membership_grants(ws, state).into_iter().filter(|g| holders.is_holder(g)));
    }
    let mut by_hierarchy: std::collections::BTreeMap<String, std::collections::BTreeSet<usize>> = Default::default();
    let mut described = Vec::new();
    with_world(state, |places, tree| {
        for g in &grants {
            let mut reaches = json!({ "workspaces": 0, "apps": 0, "tables": 0 });
            for (i, (t, o)) in targets.iter().zip(&objects).enumerate() {
                let Some(o) = o else { continue };
                if trace(g, &holders, o, tree, places, now).missed.is_none() {
                    let kind = count_kind(t);
                    reaches[kind] = json!(reaches[kind].as_i64().unwrap_or(0) + 1);
                    by_hierarchy.entry(g.hierarchy.clone().unwrap_or_else(|| "container".to_string())).or_default().insert(i);
                }
            }
            let mut j = grant_json(g);
            j["reaches"] = reaches;
            described.push(j);
        }
    })?;
    let counts: serde_json::Map<String, Value> = by_hierarchy
        .into_iter()
        .map(|(h, seen)| {
            let mut c = json!({ "workspaces": 0, "apps": 0, "tables": 0 });
            for i in seen {
                let kind = count_kind(&targets[i]);
                c[kind] = json!(c[kind].as_i64().unwrap_or(0) + 1);
            }
            (h, c)
        })
        .collect();
    Ok(json!({
        "person": person,
        "holder_set": {
            "positions": holders.positions.iter().map(|(k, u)| json!({ "key": k, "unit": u })).collect::<Vec<_>>(),
            "units": holders.units, "groups": holders.groups,
        },
        "grants": described, "by_hierarchy": counts,
    }))
}

/// The addresses of the people who hold a grant's holder: the person, or everyone appointed in a unit's subtree, with a
/// position, or in a group.
fn people_of_holder(state: &SharedState, kind: scaffoldry_core::reach::HolderKind, id: &str) -> Vec<String> {
    use scaffoldry_core::reach::HolderKind;
    match kind {
        HolderKind::Person => vec![id.to_lowercase()],
        HolderKind::Unit => {
            let Ok(unit) = Uuid::parse_str(id) else { return Vec::new() };
            let units = state.tree.read().unwrap_or_else(|p| p.into_inner()).descendants(unit, None);
            state.repository.appointed_addresses(&units).unwrap_or_default()
        }
        HolderKind::Position => {
            let (key, unit) = id.split_once('@').unwrap_or((id, "*"));
            state.repository.appointed_to_position(key, Uuid::parse_str(unit).ok()).unwrap_or_default()
        }
        HolderKind::Group => state
            .repository
            .group_by_id(id)
            .ok()
            .flatten()
            .map(|g| {
                g.members
                    .iter()
                    .filter_map(|m| m.get("value").and_then(Value::as_str))
                    .map(|v| state.user_by_id(v).or_else(|| state.user_by_name(v)).map(|u| u.user_name).unwrap_or_else(|| v.to_string()).to_lowercase())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The most people one impact answer looks at.
const MOST_PEOPLE: usize = 2000;

/// What each person's role would be on each of these objects before and after: the same grants, with one added, or the
/// same workspace with new coordinates. Only a role that changes is reported.
struct Compare<'a> {
    state: &'a SharedState,
    now: i64,
    /// Each person's standing, read once.
    standing: std::collections::HashMap<String, Option<(HolderSet, Vec<Grant>)>>,
    memberships: std::collections::HashMap<String, Vec<Grant>>,
    gain: Vec<Value>,
    lose: Vec<Value>,
    per_role: std::collections::BTreeMap<String, (u32, u32)>,
}

impl<'a> Compare<'a> {
    fn new(state: &'a SharedState) -> Self {
        Compare { state, now: chrono::Utc::now().timestamp(), standing: Default::default(), memberships: Default::default(), gain: Vec::new(), lose: Vec::new(), per_role: Default::default() }
    }

    fn person(&mut self, eppn: &str, object_text: &str, before: &Object, after: &Object, workspace: &str, extra: Option<&Grant>) {
        let state = self.state;
        let standing = self.standing.entry(eppn.to_string()).or_insert_with(|| members::standing(eppn, state)).clone();
        let Some((holders, mut grants)) = standing else { return };
        let ms = self.memberships.entry(workspace.to_string()).or_insert_with(|| members::membership_grants(workspace, state));
        grants.extend(ms.iter().filter(|g| holders.is_holder(g)).cloned());
        let mut with_extra = grants.clone();
        with_extra.extend(extra.cloned());
        let now = self.now;
        let Ok((was, will)) = with_world(state, |places, tree| (role_at(&holders, &grants, before, tree, places, now).map(|(r, _)| r), role_at(&holders, &with_extra, after, tree, places, now).map(|(r, _)| r))) else { return };
        if was == will {
            return;
        }
        let entry = json!({ "person": eppn, "object": object_text, "before": was.map(|r| r.as_str()), "after": will.map(|r| r.as_str()) });
        if will > was {
            self.per_role.entry(will.map_or("none", |r| r.as_str()).to_string()).or_default().0 += 1;
            self.gain.push(entry);
        } else {
            self.per_role.entry(was.map_or("none", |r| r.as_str()).to_string()).or_default().1 += 1;
            self.lose.push(entry);
        }
    }

    fn finish(self, id: Uuid, kind: &str, people: usize, truncated: bool) -> Value {
        let per_role: serde_json::Map<String, Value> = self.per_role.into_iter().map(|(r, (g, l))| (r, json!({ "gain": g, "lose": l }))).collect();
        json!({ "change_id": id, "kind": kind, "gain": self.gain, "lose": self.lose, "per_role": per_role, "people_considered": people, "truncated": truncated })
    }
}

/// Who would gain and who would lose, per role, if a pending change were confirmed. It changes nothing. For a grant it asks
/// `role_at` with and without the grant. For a change of a workspace's coordinates, a seal among them, it asks with the
/// workspace as it is and as it would be.
pub fn impact(state: &SharedState, caller: &AuthUser, id: Uuid) -> Result<Value, ServiceError> {
    if let Some(grant) = crate::service::access_grants::pending_grant(state, caller, id)? {
        return impact_of_grant(state, id, &grant);
    }
    let Some((workspace_id, coordinates, status)) = state.repository.coordinate_change(id).map_err(|e| ServiceError::Internal(e.to_string()))? else {
        return Err(crate::service::access_grants::missing(state, caller, "Change"));
    };
    let ws = workspace_named(state, &workspace_id).ok_or_else(|| ServiceError::NotFound("Workspace not found".to_string()))?;
    let may = crate::service::admin::require_platform_admin_or_compliance(caller, state).is_ok() || members::member_role(caller, &ws.id, state).is_some_and(|r| r >= Role::Admin);
    if !may {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or an owner or admin of the workspace required"));
    }
    if status != "Pending" {
        return Err(ServiceError::conflict("That change has already been decided"));
    }
    impact_of_coordinates(state, id, &ws, coordinates)
}

/// The impact of a grant that is not stored yet, for the form that proposes it.
pub fn impact_of_new_grant(state: &SharedState, grant: &Grant) -> Result<Value, ServiceError> {
    impact_of_grant(state, Uuid::nil(), grant)
}

fn impact_of_grant(state: &SharedState, id: Uuid, grant: &Grant) -> Result<Value, ServiceError> {
    let now = chrono::Utc::now().timestamp();
    let mut people = people_of_holder(state, grant.holder_kind, &grant.holder_id);
    let truncated = people.len() > MOST_PEOPLE;
    people.truncate(MOST_PEOPLE);
    let holder = HolderSet::holding(grant);
    let targets = all_targets(state);
    let mut cmp = Compare::new(state);
    for t in &targets {
        let Some(object) = t.object(state) else { continue };
        // A grant held by a position at any unit reaches an object for the people who hold it where it counts. Ask each.
        let may_reach = !holder.is_holder(grant) || with_world(state, |places, tree| trace(grant, &holder, &object, tree, places, now).missed.is_none()).unwrap_or(false);
        if !may_reach {
            continue;
        }
        for p in &people {
            cmp.person(p, &t.container, &object, &object, &t.workspace.id, Some(grant));
        }
    }
    Ok(cmp.finish(id, "grant", people.len(), truncated))
}

fn impact_of_coordinates(state: &SharedState, id: Uuid, ws: &WorkspaceRecord, coordinates: scaffoldry_core::coordinates::Coordinates) -> Result<Value, ServiceError> {
    let now = chrono::Utc::now().timestamp();
    let mut after_ws = ws.clone();
    after_ws.coordinates = coordinates;
    let targets: Vec<Target> = all_targets(state).into_iter().filter(|t| t.workspace.id == ws.id).collect();
    let mut changed: Vec<(String, Object, Object, String)> = Vec::new();
    for t in &targets {
        let moved = Target { container: t.container.clone(), workspace: after_ws.clone(), app: t.app.clone(), table: t.table.clone() };
        if let (Some(b), Some(a)) = (t.object(state), moved.object(state)) {
            changed.push((t.container.clone(), b, a, t.workspace.id.clone()));
        }
    }
    // The people to ask: those holding a grant that reaches one of the objects before or after, and everyone appointed above
    // the workspace, whose unit-administrator grants are derived and not stored.
    let mut people: std::collections::BTreeSet<String> = Default::default();
    let stored = active_grants(state)?;
    let memberships = members::membership_grants(&ws.id, state);
    with_world(state, |places, tree| {
        for g in stored.iter().map(|r| &r.grant).chain(memberships.iter()) {
            let holder = HolderSet::holding(g);
            let differs = changed.iter().any(|(_, b, a, _)| {
                let reaches = |o: &Object| trace(g, &holder, o, tree, places, now).missed.is_none();
                reaches(b) != reaches(a) || !holder.is_holder(g)
            });
            if differs {
                people.extend(people_of_holder(state, g.holder_kind, &g.holder_id));
            }
        }
    })?;
    let above: Vec<Uuid> = ws.organization_id.map(|u| state.tree.read().unwrap_or_else(|p| p.into_inner()).ancestors(u)).unwrap_or_default();
    people.extend(state.repository.appointed_addresses(&above).unwrap_or_default());
    let truncated = people.len() > MOST_PEOPLE;
    let mut cmp = Compare::new(state);
    for p in people.iter().take(MOST_PEOPLE) {
        for (container, before, after, workspace) in &changed {
            cmp.person(p, container, before, after, workspace, None);
        }
    }
    Ok(cmp.finish(id, "coordinates", people.len().min(MOST_PEOPLE), truncated))
}

/// Workspaces, apps and tables whose name holds `q`, in order, a page at a time. The cursor is the container text of the last
/// one returned. The objects live in memory, so this pages there.
pub fn search_objects(state: &SharedState, caller: &AuthUser, q: &str, cursor: Option<&str>, limit: Option<i64>) -> Result<Value, ServiceError> {
    if crate::service::admin::require_platform_admin_or_compliance(caller, state).is_err() && state.roles_of(&caller.eppn).iter().all(|r| r.scoped_affiliation != "unit_admin") {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
    }
    let limit = limit.unwrap_or(50).clamp(1, 200) as usize;
    let needle = q.trim().to_lowercase();
    let mut found: Vec<(String, String, String)> = all_targets(state)
        .into_iter()
        .filter(|t| t.object(state).is_some_and(|o| may_review_object(state, caller, &o)))
        .map(|t| {
            let (kind, title) = match (&t.app, &t.table) {
                (None, _) => ("workspace", t.workspace.name.clone()),
                (Some(m), None) => ("app", m.title.clone()),
                (Some(m), Some(table)) => ("table", format!("{} / {}", m.title, m.tables.iter().find(|x| &x.id == table).map(|x| x.name.clone()).unwrap_or_else(|| table.clone()))),
            };
            (t.container, kind.to_string(), title)
        })
        .filter(|(container, _, title)| needle.is_empty() || container.to_lowercase().contains(&needle) || title.to_lowercase().contains(&needle))
        .collect();
    found.sort();
    let start = cursor.map(|c| found.partition_point(|(container, _, _)| container.as_str() <= c)).unwrap_or(0);
    let page: Vec<&(String, String, String)> = found.iter().skip(start).take(limit + 1).collect();
    let more = page.len() > limit;
    let objects: Vec<Value> = page.iter().take(limit).map(|(container, kind, title)| json!({ "container": container, "kind": kind, "title": title })).collect();
    let next = if more { objects.last().and_then(|o| o["container"].as_str().map(str::to_string)) } else { None };
    Ok(json!({ "objects": objects, "next_cursor": next }))
}

/// The domains a grant can be limited to, for the form that makes one.
pub fn domains(state: &SharedState, caller: &AuthUser) -> Result<Value, ServiceError> {
    if crate::service::admin::require_platform_admin_or_compliance(caller, state).is_err() && state.roles_of(&caller.eppn).iter().all(|r| r.scoped_affiliation != "unit_admin") {
        return Err(ServiceError::forbidden("Platform Admin, compliance, or Org Unit Admin required"));
    }
    let list = state.repository.domain_names().map_err(|e| ServiceError::Internal(e.to_string()))?;
    Ok(json!({ "domains": list.into_iter().map(|(key, name)| json!({ "key": key, "name": name })).collect::<Vec<_>>() }))
}
