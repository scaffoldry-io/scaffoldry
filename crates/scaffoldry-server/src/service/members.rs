//! Who a workspace membership means, and the one function that answers a person's role.
//!
//! A membership names a person, a SCIM group, an org unit, or a position at a unit. For anything
//! but a person, `eppn` holds `{type}:{principal_id}`, so the unique key on `(workspace, eppn)` holds.

use crate::state::{AuthUser, CollaboratorRecord, SharedState};
use scaffoldry_core::reach::{fold_reach, holder_set, role_at, row_contributions, AnchorKind, Contribution, Grant, HolderKind, HolderSet, Holding, Object, RowReach};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

pub use scaffoldry_core::reach::Role;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Group,
    Unit,
    Position,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "user" => Some(Kind::User),
            "group" => Some(Kind::Group),
            "unit" => Some(Kind::Unit),
            "position" => Some(Kind::Position),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Group => "group",
            Kind::Unit => "unit",
            Kind::Position => "position",
        }
    }
}

/// The `eppn` a membership is stored under.
pub fn stored_key(kind: Kind, principal_id: &str) -> String {
    if kind == Kind::User {
        principal_id.to_string()
    } else {
        format!("{}:{principal_id}", kind.as_str())
    }
}

/// The principal a stored `eppn` names. An address is a person.
pub fn split_principal(eppn: &str) -> (Kind, &str) {
    for kind in [Kind::Group, Kind::Unit, Kind::Position] {
        if let Some(id) = eppn
            .strip_prefix(kind.as_str())
            .and_then(|r| r.strip_prefix(':'))
        {
            return (kind, id);
        }
    }
    (Kind::User, eppn)
}

/// The addresses a membership means right now.
pub fn people_of(c: &CollaboratorRecord, state: &SharedState) -> Vec<String> {
    let (kind, id) = split_principal(&c.eppn);
    match kind {
        Kind::User => vec![id.to_string()],
        Kind::Group => {
            let Ok(Some(group)) = state.repository.group_by_id(id) else {
                return Vec::new();
            };
            let values: Vec<String> = group
                .members
                .iter()
                .filter_map(|m| m.get("value").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            let by_id: BTreeMap<String, String> = state
                .repository
                .users_by_ids(&values)
                .unwrap_or_default()
                .into_iter()
                .map(|u| (u.id, u.user_name))
                .collect();
            values
                .into_iter()
                .map(|v| by_id.get(&v).cloned().unwrap_or(v))
                .collect()
        }
        Kind::Unit => {
            let Ok(unit) = Uuid::parse_str(id) else {
                return Vec::new();
            };
            let units = state
                .tree
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .descendants(unit, None);
            state
                .repository
                .roles_in_units(&units)
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.eppn)
                .collect()
        }
        Kind::Position => {
            let Some((key, unit)) = id.split_once('@') else {
                return Vec::new();
            };
            let Ok(unit) = Uuid::parse_str(unit) else {
                return Vec::new();
            };
            state
                .repository
                .roles_at(unit, key)
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.eppn)
                .collect()
        }
    }
}

/// Every workspace that has a membership.
pub fn workspace_ids(state: &SharedState) -> Vec<String> {
    state
        .collaborators
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .keys()
        .cloned()
        .collect()
}

/// The memberships that cover the whole workspace. One limited to an app is not among them, so it never grants a
/// workspace-level action.
fn memberships(workspace: &str, state: &SharedState) -> Vec<CollaboratorRecord> {
    let all = state
        .collaborators
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(workspace)
        .cloned()
        .unwrap_or_default();
    all.into_iter().filter(|c| c.app_slug.is_empty()).collect()
}

/// Every membership in every workspace that names one person, whether it covers the workspace or one app.
pub fn direct_memberships(state: &SharedState) -> Vec<CollaboratorRecord> {
    let all = state
        .collaborators
        .read()
        .unwrap_or_else(|p| p.into_inner());
    all.values()
        .flatten()
        .filter(|c| split_principal(&c.eppn).0 == Kind::User)
        .cloned()
        .collect()
}

/// A person's role in one app: the highest of their workspace-wide memberships and those limited to this app.
pub fn member_role_in_app(
    user: &AuthUser,
    workspace: &str,
    app: &str,
    state: &SharedState,
) -> Option<Role> {
    role_for(&user.eppn, workspace, Some(app), state)
}

/// The same for a person named by address.
pub fn member_role_of_in_app(
    eppn: &str,
    workspace: &str,
    app: &str,
    state: &SharedState,
) -> Option<Role> {
    role_for(eppn, workspace, Some(app), state)
}

/// The highest role among every grant that reaches the workspace. This is the only place that turns grants and
/// memberships into a person's role.
pub fn member_role(user: &AuthUser, workspace: &str, state: &SharedState) -> Option<Role> {
    role_for(&user.eppn, workspace, None, state)
}

/// The role of a person named by address, for a caller who has no `AuthUser`.
pub fn member_role_of(eppn: &str, workspace: &str, state: &SharedState) -> Option<Role> {
    role_for(eppn, workspace, None, state)
}

/// A membership is a container grant: on the workspace when it covers the workspace, on the app when it is limited to one.
fn membership_grant(c: &CollaboratorRecord, index: usize) -> Option<Grant> {
    let (kind, id) = split_principal(&c.eppn);
    let (holder_kind, holder_id) = match kind {
        Kind::User => (HolderKind::Person, id.to_string()),
        Kind::Group => (HolderKind::Group, id.to_string()),
        Kind::Unit => (HolderKind::Unit, id.to_string()),
        Kind::Position => (HolderKind::Position, id.to_string()),
    };
    let anchor = if c.app_slug.is_empty() { format!("workspace:{}", c.workspace_id) } else { format!("app:{}", c.app_slug) };
    Some(Grant {
        id: -(index as i64) - 1,
        holder_kind,
        holder_id,
        role: Role::parse(&c.role)?,
        hierarchy: None,
        anchor_kind: AnchorKind::Container,
        anchor_id: Some(anchor),
        depth: None,
        domain: None,
        valid_from: 0,
        valid_until: None,
        revoked: false,
        clearance: None,
    })
}

/// The `(kind, id)` pairs a stored grant may be held under for this person.
fn holder_pairs(h: &HolderSet) -> Vec<(String, String)> {
    let mut pairs = vec![("person".to_string(), h.person.clone())];
    for (key, unit) in &h.positions {
        pairs.push(("position".to_string(), format!("{key}@{unit}")));
        pairs.push(("position".to_string(), format!("{key}@*")));
    }
    pairs.extend(h.units.iter().map(|u| ("unit".to_string(), u.to_string())));
    pairs.extend(h.groups.iter().map(|g| ("group".to_string(), g.clone())));
    pairs
}

/// `role_at` for a workspace, or for one app in it.
fn role_for(eppn: &str, workspace: &str, app: Option<&str>, state: &SharedState) -> Option<Role> {
    roles_for(eppn, &[(workspace, app)], state).into_iter().next().flatten()
}

/// The role of one person in each of these workspaces, in order. Their appointments, groups and grants are read once
/// for the whole list, so the database work does not grow with the number of workspaces.
pub fn member_roles(eppn: &str, workspaces: &[String], state: &SharedState) -> Vec<Option<Role>> {
    let targets: Vec<(&str, Option<&str>)> = workspaces.iter().map(|w| (w.as_str(), None)).collect();
    roles_for(eppn, &targets, state)
}

/// The role of one person in each `(workspace, app)` pair, in order, with one read of their roles, groups and grants.
pub fn member_roles_in_apps(eppn: &str, pairs: &[(String, String)], state: &SharedState) -> Vec<Option<Role>> {
    let targets: Vec<(&str, Option<&str>)> = pairs.iter().map(|(w, a)| (w.as_str(), Some(a.as_str()))).collect();
    roles_for(eppn, &targets, state)
}

/// Who a person is to the reach rule, and every grant that could name them: the unit-administrator grants derived from their
/// appointments, and the stored grants held by them or by anything they belong to. Memberships are per workspace and are
/// added by the caller. `None` when the unit tree cannot be read.
pub fn standing(eppn: &str, state: &SharedState) -> Option<(HolderSet, Vec<Grant>)> {
    let now = chrono::Utc::now().timestamp();
    let rows = state.repository.holdings_of(eppn).unwrap_or_default();
    let holdings: Vec<Holding> = rows.iter().map(|(h, _)| h.clone()).collect();
    let groups = state.group_ids_of(eppn);
    // A unit_admin appointment is a grant: the person is admin of the unit's whole subtree. It is derived here, never stored.
    let mut shared: Vec<Grant> = rows
        .iter()
        .enumerate()
        .filter(|(_, (_, affiliation))| affiliation == "unit_admin")
        .map(|(i, (h, _))| Grant {
            id: -1_000_000 - i as i64,
            holder_kind: HolderKind::Person,
            holder_id: eppn.to_string(),
            role: Role::Admin,
            hierarchy: None,
            anchor_kind: AnchorKind::Unit,
            anchor_id: Some(h.unit.to_string()),
            depth: None,
            domain: None,
            valid_from: h.effective.unwrap_or(0),
            valid_until: h.end,
            revoked: false,
            clearance: None,
        })
        .collect();
    // Who the person is, from the unit tree. Then their stored grants, read with no lock held.
    let holders = crate::service::coordinates::with_world(state, |_, tree| {
        for g in shared.iter_mut() {
            g.hierarchy = g.anchor_id.as_deref().and_then(|a| Uuid::parse_str(a).ok()).and_then(|u| tree.hierarchy_of(u)).map(str::to_string);
        }
        holder_set(eppn, &holdings, tree, groups.clone(), now)
    })
    .ok()?;
    shared.extend(state.repository.grants_held_by(&holder_pairs(&holders)).unwrap_or_default());
    Some((holders, shared))
}

/// Every membership of a workspace as a container grant. Indexes start below the derived grants' ids.
pub fn membership_grants(workspace: &str, state: &SharedState) -> Vec<Grant> {
    memberships(workspace, state).iter().enumerate().filter_map(|(i, c)| membership_grant(c, i)).collect()
}

/// `role_at` for each `(workspace, app)` target. No lock is held while the database is read.
fn roles_for(eppn: &str, targets: &[(&str, Option<&str>)], state: &SharedState) -> Vec<Option<Role>> {
    // A preview of a proposal is the person's rights in the live app.
    let stored: Vec<(&str, Option<&str>)> = targets.iter().map(|(w, a)| (*w, a.map(crate::service::preview::stored_slug))).collect();
    let targets = stored.as_slice();
    let none = || vec![None; targets.len()];
    let now = chrono::Utc::now().timestamp();

    // A copy of what the targets need, taken under short locks.
    let (workspaces, memberships) = {
        let ws = state.workspaces.read().unwrap_or_else(|p| p.into_inner());
        let collabs = state.collaborators.read().unwrap_or_else(|p| p.into_inner());
        let mut w = std::collections::HashMap::new();
        let mut m = std::collections::HashMap::new();
        for (workspace, _) in targets {
            if let Some(rec) = ws.get(*workspace) {
                w.insert(workspace.to_string(), rec.clone());
            }
            m.insert(workspace.to_string(), collabs.get(*workspace).cloned().unwrap_or_default());
        }
        (w, m)
    };
    let manifests: std::collections::HashMap<String, scaffoldry_engine::AppManifest> = {
        use scaffoldry_engine::HostRouter;
        let engine = state.engine.read().unwrap_or_else(|p| p.into_inner());
        targets.iter().filter_map(|(_, app)| *app).filter_map(|slug| engine.resolve_by_slug(slug).map(|m| (slug.to_string(), m.clone()))).collect()
    };

    let Some((holders, shared)) = standing(eppn, state) else { return none() };

    crate::service::coordinates::with_world(state, |places, tree| {
        targets
            .iter()
            .map(|(workspace, app)| {
                let mut grants = shared.clone();
                let members = memberships.get(*workspace).map(Vec::as_slice).unwrap_or_default();
                grants.extend(members.iter().filter(|c| c.app_slug.is_empty() || Some(c.app_slug.as_str()) == *app).enumerate().filter_map(|(i, c)| membership_grant(c, i)));
                let object = match workspaces.get(*workspace) {
                    Some(ws) => crate::service::coordinates::object_for(places, tree, ws, app.map(|a| (a, manifests.get(a))), None),
                    None => Object { containers: vec![format!("workspace:{workspace}")], units: Default::default(), domain: scaffoldry_core::coordinates::ROOT_DOMAIN.to_string(), sealed: Default::default() },
                };
                role_at(&holders, &grants, &object, tree, places, now).map(|(role, _)| role)
            })
            .collect()
    })
    .unwrap_or_else(|_| none())
}

/// Everyone with a role in the workspace and their highest role, by address.
pub fn people_with_roles(workspace: &str, state: &SharedState) -> BTreeMap<String, Role> {
    let mut out: BTreeMap<String, Role> = BTreeMap::new();
    for c in memberships(workspace, state) {
        let Some(role) = Role::parse(&c.role) else {
            continue;
        };
        for p in people_of(&c, state) {
            let slot = out.entry(p.to_lowercase()).or_insert(role);
            *slot = (*slot).max(role);
        }
    }
    out
}

/// A membership as the member list shows it: the record, its principal, and a label for people.
pub fn describe(c: &CollaboratorRecord, state: &SharedState) -> Value {
    let (kind, id) = split_principal(&c.eppn);
    let label = match kind {
        Kind::User => c.name.clone(),
        Kind::Group => state
            .repository
            .group_by_id(id)
            .ok()
            .flatten()
            .map(|g| g.display_name)
            .unwrap_or_else(|| id.to_string()),
        Kind::Unit => Uuid::parse_str(id)
            .ok()
            .and_then(|u| {
                state
                    .organizations
                    .read()
                    .ok()
                    .and_then(|o| o.get(&u).map(|o| o.name.clone()))
            })
            .unwrap_or_else(|| id.to_string()),
        Kind::Position => id
            .split_once('@')
            .map(|(key, unit)| {
                format!(
                    "{key} of {}",
                    Uuid::parse_str(unit)
                        .ok()
                        .and_then(|u| state
                            .organizations
                            .read()
                            .ok()
                            .and_then(|o| o.get(&u).map(|o| o.name.clone())))
                        .unwrap_or_else(|| unit.to_string())
                )
            })
            .unwrap_or_else(|| id.to_string()),
    };
    let mut v = serde_json::to_value(c).unwrap_or_else(|_| json!({}));
    v["principal_type"] = json!(kind.as_str());
    v["principal_id"] = json!(id);
    v["label"] = json!(label);
    v
}

/// One record per person with a role in the workspace, at their highest role. A person named
/// directly keeps their own row. A person who comes in through a group, unit, or position gets a
/// plain row. For the readers that act on people, such as approver rules and guard previews.
pub fn people_records(workspace: &str, state: &SharedState) -> Vec<CollaboratorRecord> {
    let direct = memberships(workspace, state);
    let people = people_with_roles(workspace, state);
    let own_of = |eppn: &str| {
        direct
            .iter()
            .find(|c| split_principal(&c.eppn).0 == Kind::User && c.eppn.eq_ignore_ascii_case(eppn))
    };
    // The names of those who come in through a group, unit or position, read in one statement.
    let unnamed: Vec<String> = people
        .keys()
        .filter(|e| own_of(e).is_none())
        .cloned()
        .collect();
    let known: BTreeMap<String, String> = state
        .repository
        .users_by_names(&unnamed)
        .unwrap_or_default()
        .into_iter()
        .map(|u| {
            (
                u.user_name.to_lowercase(),
                crate::service::people::display_name(&u),
            )
        })
        .collect();
    people
        .into_iter()
        .map(|(eppn, role)| {
            let mut rec = own_of(&eppn)
                .cloned()
                .unwrap_or_else(|| CollaboratorRecord {
                    id: Uuid::new_v4().to_string(),
                    workspace_id: workspace.to_string(),
                    eppn: eppn.clone(),
                    name: known
                        .get(&eppn.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| eppn.clone()),
                    role: String::new(),
                    scoped_affiliation: "member".to_string(),
                    department: String::new(),
                    added_at: String::new(),
                    app_slug: String::new(),
                });
            rec.role = role.as_str().to_string();
            rec
        })
        .collect()
}

/// What one person reaches among the rows of `table_id` in an app that places its rows. `None` when the app, its
/// workspace, or the person's directory record cannot be read, so a caller treats it as reaching nothing.
pub fn rows_reach(eppn: &str, workspace: &str, app: &str, table_id: &str, state: &SharedState) -> Option<RowReach> {
    contributions_to(eppn, workspace, app, table_id, state).map(|c| fold_reach(&c))
}

/// What each of one person's grants contributes to the rows of a table, with what each clears.
pub fn contributions_to(eppn: &str, workspace: &str, app: &str, table_id: &str, state: &SharedState) -> Option<Vec<Contribution>> {
    let app = crate::service::preview::stored_slug(app);
    let now = chrono::Utc::now().timestamp();
    let ws = state.workspaces.read().unwrap_or_else(|p| p.into_inner()).get(workspace).cloned()?;
    let memberships: Vec<CollaboratorRecord> = state.collaborators.read().unwrap_or_else(|p| p.into_inner()).get(workspace).cloned().unwrap_or_default();
    let manifest = {
        use scaffoldry_engine::HostRouter;
        state.engine.read().unwrap_or_else(|p| p.into_inner()).resolve_by_slug(app).cloned()?
    };
    let row_hierarchies: std::collections::BTreeSet<String> = manifest.tables.iter().find(|t| t.id == table_id).map(|t| t.row_place.keys().cloned().collect()).unwrap_or_default();

    let (holders, mut shared) = standing(eppn, state)?;
    shared.extend(memberships.iter().filter(|c| c.app_slug.is_empty() || c.app_slug == app).enumerate().filter_map(|(i, c)| membership_grant(c, i)));

    crate::service::coordinates::with_world(state, |places, tree| {
        let table = crate::service::coordinates::object_for(places, tree, &ws, Some((app, Some(&manifest))), Some(table_id));
        row_contributions(&holders, &shared, &table, &row_hierarchies, tree, places, now)
    })
    .ok()
}
