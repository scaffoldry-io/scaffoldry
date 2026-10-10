//! People: the users the platform knows, the appointments they hold, and the administrative
//! actions on them. Both SCIM and the admin console write the one user store through here.

use crate::service::admin::{admin_write, decode_cursor, encode_cursor};
use crate::service::identity::scim_affiliation;
use crate::service::organizations::{is_platform_admin, OrgCaller};
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, OrganizationNode, RoleRow, ScimUser, SharedState};
use chrono::Utc;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The eduPerson affiliations an administrator may give a new user.
pub const AFFILIATIONS: [&str; 7] = ["faculty", "student", "staff", "employee", "member", "affiliate", "alum"];

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const LEDGER_ROWS: usize = 20;

// ---------------------------------------------------------------------------------------------
// The user store
// ---------------------------------------------------------------------------------------------

/// What a writer says about a user. `None` means "leave it as it is" on an existing user, and
/// "use the default" on a new one.
#[derive(Debug, Clone, Default)]
pub struct UserFields {
    pub user_name: String,
    pub name: Option<Value>,
    pub active: Option<bool>,
    pub emails: Option<Vec<Value>>,
    pub roles: Option<Vec<Value>>,
    pub enterprise: Option<Value>,
    pub title: Option<String>,
}

pub fn find_by_user_name(state: &SharedState, user_name: &str) -> Option<ScimUser> {
    let users = state.users.read().unwrap_or_else(|p| p.into_inner());
    users.values().find(|u| u.user_name.eq_ignore_ascii_case(user_name)).cloned()
}

pub fn find_by_id(state: &SharedState, id: &str) -> Option<ScimUser> {
    state.users.read().unwrap_or_else(|p| p.into_inner()).get(id).cloned()
}

fn persist(state: &SharedState, user: &ScimUser) -> Result<(), ServiceError> {
    if let Some(ref repo) = state.repository {
        repo.upsert_scim_user(user)
            .map_err(|e| ServiceError::Internal(format!("Failed to persist user: {e}")))?;
    }
    Ok(())
}

/// Creates the user, or updates the one that already has this `userName`. An update keeps the
/// id and the administrative hold. Returns the user and whether it was created.
pub fn upsert_user(state: &SharedState, fields: UserFields) -> Result<(ScimUser, bool), ServiceError> {
    let (user, created) = {
        let mut users = state.users.write().unwrap_or_else(|p| p.into_inner());
        let existing = users
            .values()
            .find(|u| u.user_name.eq_ignore_ascii_case(&fields.user_name))
            .map(|u| u.id.clone());
        match existing {
            Some(id) => {
                let u = users.get_mut(&id).expect("the id was just read");
                if let Some(v) = fields.name {
                    u.name = v;
                }
                if let Some(v) = fields.active {
                    u.active = v;
                }
                if let Some(v) = fields.emails {
                    u.emails = v;
                }
                if let Some(v) = fields.roles {
                    u.roles = v;
                }
                if let Some(v) = fields.enterprise {
                    u.enterprise_extension = Some(v);
                }
                if let Some(v) = fields.title {
                    u.title = Some(v);
                }
                (u.clone(), false)
            }
            None => {
                let u = ScimUser {
                    id: Uuid::new_v4().to_string(),
                    user_name: fields.user_name,
                    name: fields.name.unwrap_or_else(|| json!({})),
                    active: fields.active.unwrap_or(true),
                    admin_hold: false,
                    emails: fields.emails.unwrap_or_default(),
                    roles: fields.roles.unwrap_or_default(),
                    enterprise_extension: fields.enterprise,
                    title: fields.title,
                };
                users.insert(u.id.clone(), u.clone());
                (u, true)
            }
        }
    };
    persist(state, &user)?;
    Ok((user, created))
}

/// Replaces the organization roles that come from this user's SCIM record.
pub fn sync_roles(state: &SharedState, user: &ScimUser) {
    let orgs: Vec<OrganizationNode> = state.organizations.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    let roles = crate::routes::scim::sync_org_roles(user, &orgs);
    let _ = state.delete_scim_roles(&user.user_name);
    for r in roles {
        let _ = state.persist_role(&r);
    }
}

fn set_hold(state: &SharedState, id: &str, hold: bool) -> Result<ScimUser, ServiceError> {
    let user = {
        let mut users = state.users.write().unwrap_or_else(|p| p.into_inner());
        let u = users
            .get_mut(id)
            .ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
        u.admin_hold = hold;
        u.clone()
    };
    persist(state, &user)?;
    Ok(user)
}

// ---------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------

fn display_name(u: &ScimUser) -> String {
    if let Some(f) = u.name.get("formatted").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        return f.to_string();
    }
    let given = u.name.get("givenName").and_then(|v| v.as_str()).unwrap_or("");
    let family = u.name.get("familyName").and_then(|v| v.as_str()).unwrap_or("");
    let full = format!("{given} {family}").trim().to_string();
    if full.is_empty() { u.user_name.clone() } else { full }
}

fn email(u: &ScimUser) -> String {
    u.emails
        .iter()
        .find_map(|e| e.get("value").and_then(|v| v.as_str()))
        .unwrap_or(&u.user_name)
        .to_string()
}

/// The organization data a page of rows needs, read once.
struct Directory {
    orgs: HashMap<Uuid, OrganizationNode>,
    roles: Vec<RoleRow>,
}

impl Directory {
    fn read(state: &SharedState) -> Self {
        let orgs = state
            .organizations
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|o| (o.id, o.clone()))
            .collect();
        let roles = state.roles.read().unwrap_or_else(|p| p.into_inner()).clone();
        Self { orgs, roles }
    }

    fn roles_of<'a>(&'a self, user_name: &'a str) -> impl Iterator<Item = &'a RoleRow> {
        self.roles.iter().filter(move |r| r.eppn.eq_ignore_ascii_case(user_name))
    }

    fn unit(&self, id: Uuid) -> Value {
        let name = self.orgs.get(&id).map(|o| o.name.clone()).unwrap_or_default();
        json!({ "id": id, "name": name })
    }

    fn is_root(&self, id: Uuid) -> bool {
        self.orgs.get(&id).is_some_and(|o| o.parent_id.is_none())
    }
}

struct Basics {
    units: Vec<Uuid>,
    unit_admin_of: Vec<Uuid>,
    platform_admin: bool,
}

fn basics(dir: &Directory, u: &ScimUser) -> Basics {
    let mut units = Vec::new();
    let mut unit_admin_of = Vec::new();
    let mut platform_admin = false;
    for r in dir.roles_of(&u.user_name) {
        if !units.contains(&r.organization_id) {
            units.push(r.organization_id);
        }
        if r.scoped_affiliation == "unit_admin" && !unit_admin_of.contains(&r.organization_id) {
            unit_admin_of.push(r.organization_id);
        }
        if r.scoped_affiliation == "platform_admin" && dir.is_root(r.organization_id) {
            platform_admin = true;
        }
    }
    Basics { units, unit_admin_of, platform_admin }
}

fn token_summary(tokens: &[ApiToken]) -> (usize, Option<String>) {
    let now = Utc::now();
    let active_agents = tokens
        .iter()
        .filter(|t| t.kind == "agent" && t.revoked_at.is_none() && t.expires_at > now)
        .count();
    let latest = tokens.iter().filter_map(|t| t.last_used_at).max().map(|t| t.to_rfc3339());
    (active_agents, latest)
}

fn row(dir: &Directory, u: &ScimUser, tokens: &[ApiToken]) -> Value {
    let b = basics(dir, u);
    let (active_agents, latest) = token_summary(tokens);
    json!({
        "id": u.id,
        "user_name": u.user_name,
        "display_name": display_name(u),
        "email": email(u),
        "affiliation": scim_affiliation(u),
        "units": b.units.iter().map(|id| dir.unit(*id)).collect::<Vec<_>>(),
        "active": u.active,
        "hold": u.admin_hold,
        "platform_admin": b.platform_admin,
        "unit_admin_of": b.unit_admin_of.iter().map(|id| dir.unit(*id)).collect::<Vec<_>>(),
        "active_agent_tokens": active_agents,
        "latest_token_use": latest,
    })
}

// ---------------------------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct UserQuery {
    pub search: Option<String>,
    pub active: Option<bool>,
    pub hold: Option<bool>,
    pub affiliation: Option<String>,
    pub unit: Option<Uuid>,
    pub platform_admin: Option<bool>,
    pub unit_admin: Option<bool>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

/// One page of users sorted by `user_name`. Returns the rows and the cursor for the next page.
pub fn list_users(state: &SharedState, q: &UserQuery) -> Result<(Vec<Value>, Option<String>), ServiceError> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = match &q.cursor {
        Some(c) => decode_cursor(c)?,
        None => 0,
    };
    let search = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let dir = Directory::read(state);

    let mut matched: Vec<ScimUser> = state
        .users
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|u| {
            if let Some(s) = &search {
                let starts = |v: &str| v.to_lowercase().starts_with(s.as_str());
                if !(starts(&u.user_name) || starts(&display_name(u)) || starts(&email(u))) {
                    return false;
                }
            }
            if q.active.is_some_and(|a| a != u.active) || q.hold.is_some_and(|h| h != u.admin_hold) {
                return false;
            }
            if q.affiliation.as_deref().is_some_and(|a| !scim_affiliation(u).eq_ignore_ascii_case(a)) {
                return false;
            }
            let b = basics(&dir, u);
            if q.unit.is_some_and(|unit| !b.units.contains(&unit)) {
                return false;
            }
            if q.platform_admin.is_some_and(|p| p != b.platform_admin) {
                return false;
            }
            if q.unit_admin.is_some_and(|a| a != !b.unit_admin_of.is_empty()) {
                return false;
            }
            true
        })
        .cloned()
        .collect();
    matched.sort_by_key(|u| (u.user_name.to_lowercase(), u.id.clone()));

    let total = matched.len();
    let page: Vec<ScimUser> = matched.into_iter().skip(offset).take(limit).collect();
    let next = (offset + page.len() < total).then(|| encode_cursor(offset + page.len()));
    let rows = page.iter().map(|u| row(&dir, u, &state.list_all_api_tokens(&u.user_name))).collect();
    Ok((rows, next))
}

fn ledger_rows(state: &SharedState, principal: &str) -> Vec<Value> {
    let ledger = match state.repository {
        Some(ref repo) => repo
            .get_ledger()
            .unwrap_or_else(|_| state.ledger.read().unwrap_or_else(|p| p.into_inner()).clone()),
        None => state.ledger.read().unwrap_or_else(|p| p.into_inner()).clone(),
    };
    let mut mine: Vec<Value> = ledger
        .iter()
        .filter(|e| e.principal.eq_ignore_ascii_case(principal))
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    let keep_from = mine.len().saturating_sub(LEDGER_ROWS);
    mine.drain(..keep_from);
    mine.reverse(); // newest first
    mine
}

/// The user row, plus appointments, workspace memberships, tokens (never hashes), and the last
/// 20 ledger entries where the user is the principal.
pub fn user_detail(state: &SharedState, id: &str) -> Result<Value, ServiceError> {
    let u = find_by_id(state, id).ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let dir = Directory::read(state);
    let tokens = state.list_all_api_tokens(&u.user_name);

    let appointments: Vec<Value> = dir
        .roles_of(&u.user_name)
        .map(|r| {
            json!({
                "organization_id": r.organization_id,
                "unit": dir.orgs.get(&r.organization_id).map(|o| o.name.clone()).unwrap_or_default(),
                "scoped_affiliation": r.scoped_affiliation,
                "role_title": r.role_title,
                "source": r.source,
            })
        })
        .collect();

    let memberships: Vec<Value> = state
        .collaborators
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .flat_map(|(ws, members)| {
            members
                .iter()
                .filter(|m| m.eppn.eq_ignore_ascii_case(&u.user_name))
                .map(move |m| json!({ "workspace_id": ws, "role": m.role }))
        })
        .collect();

    let token_rows: Vec<Value> = tokens
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "kind": t.kind,
                "label": t.label,
                "created_at": t.created_at.to_rfc3339(),
                "expires_at": t.expires_at.to_rfc3339(),
                "last_used_at": t.last_used_at.map(|x| x.to_rfc3339()),
                "revoked": t.revoked_at.is_some(),
            })
        })
        .collect();

    Ok(json!({
        "user": row(&dir, &u, &tokens),
        "appointments": appointments,
        "workspace_memberships": memberships,
        "tokens": token_rows,
        "ledger": ledger_rows(state, &u.user_name),
    }))
}

pub fn list_groups(state: &SharedState) -> Vec<Value> {
    let mut groups: Vec<_> = state.groups.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
    groups.sort_by_key(|g| g.display_name.to_lowercase());
    groups
        .iter()
        .map(|g| json!({ "id": g.id, "name": g.display_name, "member_count": g.members.len() }))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Writes. Each one is a ledger entry first, then the change (see `admin_write`).
// ---------------------------------------------------------------------------------------------

pub struct NewUser {
    pub user_name: String,
    pub name: String,
    pub email: String,
    pub affiliation: String,
    pub department: String,
    pub title: String,
}

/// The SCIM-shaped fields for a user an administrator describes in plain words.
fn fields_for(new: &NewUser) -> UserFields {
    let domain = new.user_name.split('@').nth(1).unwrap_or("university.edu");
    UserFields {
        user_name: new.user_name.clone(),
        name: Some(json!({ "formatted": new.name })),
        active: Some(true),
        emails: Some(vec![json!({ "value": new.email, "primary": true })]),
        roles: Some(vec![json!({
            "type": "eduPersonScopedAffiliation",
            "value": format!("{}@{}", new.affiliation, domain),
        })]),
        enterprise: Some(json!({ "department": new.department })),
        title: Some(new.title.clone()),
    }
}

pub fn create_user(state: &SharedState, admin: &AuthUser, new: NewUser, reason: &str) -> Result<Value, ServiceError> {
    if new.user_name.trim().is_empty() {
        return Err(ServiceError::bad_request("userName is required"));
    }
    if !AFFILIATIONS.contains(&new.affiliation.as_str()) {
        return Err(ServiceError::bad_request(format!(
            "affiliation must be one of: {}",
            AFFILIATIONS.join(", ")
        )));
    }
    if find_by_user_name(state, &new.user_name).is_some() {
        return Err(ServiceError::conflict(
            "A user with this userName already exists. The registry updates it through SCIM",
        ));
    }
    let payload = json!({ "action": "create_user", "user_name": new.user_name, "affiliation": new.affiliation });
    let user = admin_write(state, admin, DecisionType::UserAccessChanged, "AC-02", reason, &payload, || {
        let (user, _) = upsert_user(state, fields_for(&new))?;
        sync_roles(state, &user);
        Ok(user)
    })?;
    let dir = Directory::read(state);
    Ok(row(&dir, &user, &[]))
}

pub fn set_user_hold(
    state: &SharedState,
    admin: &AuthUser,
    id: &str,
    hold: bool,
    reason: &str,
) -> Result<Value, ServiceError> {
    let target = find_by_id(state, id).ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let payload = json!({ "action": if hold { "hold" } else { "release" }, "user_id": target.id, "user_name": target.user_name });
    let user = admin_write(state, admin, DecisionType::UserAccessChanged, "AC-02", reason, &payload, || {
        set_hold(state, id, hold)
    })?;
    let dir = Directory::read(state);
    Ok(row(&dir, &user, &state.list_all_api_tokens(&user.user_name)))
}

/// Revokes every agent and impersonation token the user holds. SCIM tokens are not theirs.
pub fn revoke_user_tokens(state: &SharedState, admin: &AuthUser, id: &str, reason: &str) -> Result<usize, ServiceError> {
    let target = find_by_id(state, id).ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let now = Utc::now();
    let live: Vec<ApiToken> = state
        .list_all_api_tokens(&target.user_name)
        .into_iter()
        .filter(|t| (t.kind == "agent" || t.kind == "impersonation") && t.revoked_at.is_none() && t.expires_at > now)
        .collect();
    let payload = json!({ "action": "revoke_tokens", "user_id": target.id, "user_name": target.user_name, "count": live.len() });
    admin_write(state, admin, DecisionType::TokenRevoked, "IA-05", reason, &payload, || {
        let mut revoked = 0;
        for t in &live {
            if state.revoke_api_token(t.id, &admin.eppn, true).unwrap_or(false) {
                revoked += 1;
            }
        }
        Ok(revoked)
    })
}

// ---------------------------------------------------------------------------------------------
// Appointments
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum RevokeRefusal {
    /// The registry (SCIM) owns this appointment. Change it there.
    OwnedByRegistry,
    /// It is the last Platform Admin appointment. Appoint another first.
    LastPlatformAdmin,
}

impl RevokeRefusal {
    pub fn message(&self) -> &'static str {
        match self {
            Self::OwnedByRegistry => "The registry owns this appointment. Change it in the registry",
            Self::LastPlatformAdmin => "This is the last Platform Admin appointment. Appoint another Platform Admin first",
        }
    }
}

/// Whether an appointment may be revoked, whoever asks. Authorization is a separate question.
pub fn revoke_check(role: &RoleRow, all_roles: &[RoleRow], root_unit: Uuid) -> Result<(), RevokeRefusal> {
    if role.source == "scim" {
        return Err(RevokeRefusal::OwnedByRegistry);
    }
    if role.scoped_affiliation == "platform_admin" {
        let others: HashSet<&str> = all_roles
            .iter()
            .filter(|r| r.id != role.id && r.scoped_affiliation == "platform_admin" && r.organization_id == root_unit)
            .map(|r| r.eppn.as_str())
            .collect();
        if others.is_empty() {
            return Err(RevokeRefusal::LastPlatformAdmin);
        }
    }
    Ok(())
}

/// Whether the caller may grant or revoke an appointment: a Platform Admin always, an Org Unit
/// Admin in scope only for a `unit_admin` appointment.
pub fn may_manage_appointment(
    caller: &AuthUser,
    unit: Uuid,
    scoped_affiliation: &str,
    orgs: &[OrganizationNode],
    roles: &[RoleRow],
) -> bool {
    let org_caller = OrgCaller { eppn: caller.eppn.clone(), affiliation: caller.affiliation.clone() };
    if is_platform_admin(&org_caller, orgs, roles) {
        return true;
    }
    scoped_affiliation == "unit_admin" && crate::service::organizations::unit_in_scope(&org_caller, unit, orgs, roles)
}
