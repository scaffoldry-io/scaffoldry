//! Who a workspace membership means, and the one function that answers a person's role.
//!
//! A membership names a person, a SCIM group, an org unit, or a position at a unit. For anything
//! but a person, `eppn` holds `{type}:{principal_id}`, so the unique key on `(workspace, eppn)` holds.

use crate::state::{AuthUser, CollaboratorRecord, SharedState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Roles in rank order. A person with several memberships has the highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
    Admin,
    Owner,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_lowercase().as_str() {
            "viewer" => Some(Role::Viewer),
            "commenter" => Some(Role::Commenter),
            "editor" => Some(Role::Editor),
            "admin" => Some(Role::Admin),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Commenter => "commenter",
            Role::Editor => "editor",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }
}

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
        if let Some(id) = eppn.strip_prefix(kind.as_str()).and_then(|r| r.strip_prefix(':')) {
            return (kind, id);
        }
    }
    (Kind::User, eppn)
}

/// True when `unit` is `ancestor` or sits anywhere below it.
fn within(unit: Uuid, ancestor: Uuid, orgs: &std::collections::HashMap<Uuid, crate::state::OrganizationNode>) -> bool {
    let mut current = Some(unit);
    for _ in 0..64 {
        match current {
            Some(id) if id == ancestor => return true,
            Some(id) => current = orgs.get(&id).and_then(|o| o.parent_id),
            None => return false,
        }
    }
    false
}

/// Every unit at or below `unit`.
fn units_within(unit: Uuid, state: &SharedState) -> Vec<Uuid> {
    let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
    orgs.keys().copied().filter(|id| within(*id, unit, &orgs)).collect()
}

/// The addresses a membership means right now.
pub fn people_of(c: &CollaboratorRecord, state: &SharedState) -> Vec<String> {
    let (kind, id) = split_principal(&c.eppn);
    match kind {
        Kind::User => vec![id.to_string()],
        Kind::Group => {
            let Ok(Some(group)) = state.repository.group_by_id(id) else { return Vec::new() };
            let values: Vec<String> =
                group.members.iter().filter_map(|m| m.get("value").and_then(Value::as_str)).map(str::to_string).collect();
            let by_id: BTreeMap<String, String> = state
                .repository
                .users_by_ids(&values)
                .unwrap_or_default()
                .into_iter()
                .map(|u| (u.id, u.user_name))
                .collect();
            values.into_iter().map(|v| by_id.get(&v).cloned().unwrap_or(v)).collect()
        }
        Kind::Unit => {
            let Ok(unit) = Uuid::parse_str(id) else { return Vec::new() };
            let units = units_within(unit, state);
            state.repository.roles_in_units(&units).unwrap_or_default().into_iter().map(|r| r.eppn).collect()
        }
        Kind::Position => {
            let Some((key, unit)) = id.split_once('@') else { return Vec::new() };
            let Ok(unit) = Uuid::parse_str(unit) else { return Vec::new() };
            state.repository.roles_at(unit, key).unwrap_or_default().into_iter().map(|r| r.eppn).collect()
        }
    }
}

/// Whether the membership means this person. It asks the database about the person, not about everyone.
fn includes(c: &CollaboratorRecord, eppn: &str, state: &SharedState) -> bool {
    let (kind, id) = split_principal(&c.eppn);
    match kind {
        Kind::User => id.eq_ignore_ascii_case(eppn),
        Kind::Group => {
            let Ok(Some(group)) = state.repository.group_by_id(id) else { return false };
            let my_id = state.user_by_name(eppn).map(|u| u.id);
            group
                .members
                .iter()
                .filter_map(|m| m.get("value").and_then(Value::as_str))
                .any(|v| v.eq_ignore_ascii_case(eppn) || my_id.as_deref() == Some(v))
        }
        Kind::Unit => {
            let Ok(unit) = Uuid::parse_str(id) else { return false };
            let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
            state.roles_of(eppn).iter().any(|r| within(r.organization_id, unit, &orgs))
        }
        Kind::Position => {
            let Some((key, unit)) = id.split_once('@') else { return false };
            let Ok(unit) = Uuid::parse_str(unit) else { return false };
            state.roles_of(eppn).iter().any(|r| r.organization_id == unit && r.position_key.as_deref() == Some(key))
        }
    }
}

/// Every workspace that has a membership.
pub fn workspace_ids(state: &SharedState) -> Vec<String> {
    state.collaborators.read().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
}

/// The memberships that cover the whole workspace. One limited to an app is not among them, so it never grants a
/// workspace-level action.
fn memberships(workspace: &str, state: &SharedState) -> Vec<CollaboratorRecord> {
    let all = state.collaborators.read().unwrap_or_else(|p| p.into_inner()).get(workspace).cloned().unwrap_or_default();
    all.into_iter().filter(|c| c.app_slug.is_empty()).collect()
}

/// Every membership in every workspace that names one person, whether it covers the workspace or one app.
pub fn direct_memberships(state: &SharedState) -> Vec<CollaboratorRecord> {
    let all = state.collaborators.read().unwrap_or_else(|p| p.into_inner());
    all.values().flatten().filter(|c| split_principal(&c.eppn).0 == Kind::User).cloned().collect()
}

/// The memberships limited to one app of the workspace.
fn app_memberships(workspace: &str, app: &str, state: &SharedState) -> Vec<CollaboratorRecord> {
    let all = state.collaborators.read().unwrap_or_else(|p| p.into_inner()).get(workspace).cloned().unwrap_or_default();
    all.into_iter().filter(|c| c.app_slug == app).collect()
}

/// A person's role in one app: the highest of their workspace-wide memberships and those limited to this app.
pub fn member_role_in_app(user: &AuthUser, workspace: &str, app: &str, state: &SharedState) -> Option<Role> {
    member_role_of_in_app(&user.eppn, workspace, app, state)
}

/// The same for a person named by address.
pub fn member_role_of_in_app(eppn: &str, workspace: &str, app: &str, state: &SharedState) -> Option<Role> {
    let limited = app_memberships(workspace, app, state)
        .into_iter()
        .filter_map(|c| Role::parse(&c.role).filter(|_| includes(&c, eppn, state)))
        .max();
    limited.max(member_role_of(eppn, workspace, state))
}

/// The highest role among every membership that matches the person. This is the only place that
/// turns the workspace's memberships into a person's role.
pub fn member_role(user: &AuthUser, workspace: &str, state: &SharedState) -> Option<Role> {
    memberships(workspace, state)
        .iter()
        .filter_map(|c| Role::parse(&c.role).filter(|_| includes(c, &user.eppn, state)))
        .max()
}

/// The role of a person named by address, for a caller who has no `AuthUser`.
pub fn member_role_of(eppn: &str, workspace: &str, state: &SharedState) -> Option<Role> {
    memberships(workspace, state)
        .iter()
        .filter_map(|c| Role::parse(&c.role).filter(|_| includes(c, eppn, state)))
        .max()
}

/// Everyone with a role in the workspace and their highest role, by address.
pub fn people_with_roles(workspace: &str, state: &SharedState) -> BTreeMap<String, Role> {
    let mut out: BTreeMap<String, Role> = BTreeMap::new();
    for c in memberships(workspace, state) {
        let Some(role) = Role::parse(&c.role) else { continue };
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
        Kind::Group => state.repository.group_by_id(id).ok().flatten().map(|g| g.display_name).unwrap_or_else(|| id.to_string()),
        Kind::Unit => Uuid::parse_str(id)
            .ok()
            .and_then(|u| state.organizations.read().ok().and_then(|o| o.get(&u).map(|o| o.name.clone())))
            .unwrap_or_else(|| id.to_string()),
        Kind::Position => id.split_once('@').map(|(key, unit)| format!("{key} of {}", Uuid::parse_str(unit).ok().and_then(|u| state.organizations.read().ok().and_then(|o| o.get(&u).map(|o| o.name.clone()))).unwrap_or_else(|| unit.to_string()))).unwrap_or_else(|| id.to_string()),
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
    let own_of = |eppn: &str| direct.iter().find(|c| split_principal(&c.eppn).0 == Kind::User && c.eppn.eq_ignore_ascii_case(eppn));
    // The names of those who come in through a group, unit or position, read in one statement.
    let unnamed: Vec<String> = people.keys().filter(|e| own_of(e).is_none()).cloned().collect();
    let known: BTreeMap<String, String> = state
        .repository
        .users_by_names(&unnamed)
        .unwrap_or_default()
        .into_iter()
        .map(|u| (u.user_name.to_lowercase(), crate::service::people::display_name(&u)))
        .collect();
    people
        .into_iter()
        .map(|(eppn, role)| {
            let mut rec = own_of(&eppn).cloned().unwrap_or_else(|| CollaboratorRecord {
                id: Uuid::new_v4().to_string(),
                workspace_id: workspace.to_string(),
                eppn: eppn.clone(),
                name: known.get(&eppn.to_lowercase()).cloned().unwrap_or_else(|| eppn.clone()),
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
