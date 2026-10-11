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

/// The addresses a membership means right now.
pub fn people_of(c: &CollaboratorRecord, state: &SharedState) -> Vec<String> {
    let (kind, id) = split_principal(&c.eppn);
    match kind {
        Kind::User => vec![id.to_string()],
        Kind::Group => {
            let groups = state.groups.read().unwrap_or_else(|p| p.into_inner());
            let users = state.users.read().unwrap_or_else(|p| p.into_inner());
            let by_id: BTreeMap<&str, &str> = users.values().map(|u| (u.id.as_str(), u.user_name.as_str())).collect();
            groups
                .get(id)
                .map(|g| {
                    g.members
                        .iter()
                        .filter_map(|m| m.get("value").and_then(Value::as_str))
                        .map(|v| by_id.get(v).copied().unwrap_or(v).to_string())
                        .collect()
                })
                .unwrap_or_default()
        }
        Kind::Unit => {
            let Ok(unit) = Uuid::parse_str(id) else { return Vec::new() };
            let orgs = state.organizations.read().unwrap_or_else(|p| p.into_inner());
            let roles = state.roles.read().unwrap_or_else(|p| p.into_inner());
            roles.iter().filter(|r| within(r.organization_id, unit, &orgs)).map(|r| r.eppn.clone()).collect()
        }
        Kind::Position => {
            let Some((key, unit)) = id.split_once('@') else { return Vec::new() };
            let Ok(unit) = Uuid::parse_str(unit) else { return Vec::new() };
            let roles = state.roles.read().unwrap_or_else(|p| p.into_inner());
            roles.iter().filter(|r| r.organization_id == unit && r.position_key.as_deref() == Some(key)).map(|r| r.eppn.clone()).collect()
        }
    }
}

/// Every workspace that has a membership.
pub fn workspace_ids(state: &SharedState) -> Vec<String> {
    state.collaborators.read().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
}

fn memberships(workspace: &str, state: &SharedState) -> Vec<CollaboratorRecord> {
    state.collaborators.read().unwrap_or_else(|p| p.into_inner()).get(workspace).cloned().unwrap_or_default()
}

/// The highest role among every membership that matches the person. This is the only place that
/// turns the workspace's memberships into a person's role.
pub fn member_role(user: &AuthUser, workspace: &str, state: &SharedState) -> Option<Role> {
    memberships(workspace, state)
        .iter()
        .filter_map(|c| Role::parse(&c.role).filter(|_| people_of(c, state).iter().any(|p| p.eq_ignore_ascii_case(&user.eppn))))
        .max()
}

/// The role of a person named by address, for a caller who has no `AuthUser`.
pub fn member_role_of(eppn: &str, workspace: &str, state: &SharedState) -> Option<Role> {
    memberships(workspace, state)
        .iter()
        .filter_map(|c| Role::parse(&c.role).filter(|_| people_of(c, state).iter().any(|p| p.eq_ignore_ascii_case(eppn))))
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
        Kind::Group => state
            .groups
            .read()
            .ok()
            .and_then(|g| g.get(id).map(|g| g.display_name.clone()))
            .unwrap_or_else(|| id.to_string()),
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
    people_with_roles(workspace, state)
        .into_iter()
        .map(|(eppn, role)| {
            let own = direct.iter().find(|c| split_principal(&c.eppn).0 == Kind::User && c.eppn.eq_ignore_ascii_case(&eppn));
            let mut rec = own.cloned().unwrap_or_else(|| CollaboratorRecord {
                id: Uuid::new_v4().to_string(),
                workspace_id: workspace.to_string(),
                eppn: eppn.clone(),
                name: crate::service::people::find_by_user_name(state, &eppn).map(|u| crate::service::people::display_name(&u)).unwrap_or_else(|| eppn.clone()),
                role: String::new(),
                scoped_affiliation: "member".to_string(),
                department: String::new(),
                added_at: String::new(),
            });
            rec.role = role.as_str().to_string();
            rec
        })
        .collect()
}
