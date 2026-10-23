//! People: the users the platform knows, the appointments they hold, and the administrative
//! actions on them. Both SCIM and the admin console write the one user store through here.

use crate::repository_people::UserFilter;
use crate::service::admin::{admin_write, decode_cursor, encode_cursor};
use crate::service::identity::scim_affiliation;
use crate::service::organizations::OrgCaller;
use crate::service::ServiceError;
use crate::state::{ApiToken, AuthUser, OrganizationNode, RoleRow, ScimUser, SharedState};
use chrono::Utc;
use scaffoldry_core::ledger::DecisionType;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The eduPerson affiliations an administrator may give a new user.
pub const AFFILIATIONS: [&str; 7] = [
    "faculty",
    "student",
    "staff",
    "employee",
    "member",
    "affiliate",
    "alum",
];

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
    state
        .repository
        .user_by_name_or_email(user_name)
        .ok()
        .flatten()
}

pub fn render_collaborator(addr: &str, state: &SharedState) -> String {
    if let Some(user) = find_by_user_name(state, addr) {
        let name = display_name(&user);
        if !user.active {
            format!("{name} (no longer active)")
        } else {
            format!("{name} <{}>", user.user_name)
        }
    } else {
        addr.to_string()
    }
}

pub fn find_by_id(state: &SharedState, id: &str) -> Option<ScimUser> {
    state.user_by_id(id)
}

fn persist(state: &SharedState, user: &ScimUser) -> Result<(), ServiceError> {
    {
        let repo = &state.repository;
        repo.upsert_scim_user(user)
            .map_err(|e| ServiceError::Internal(format!("Failed to persist user: {e}")))?;
    }
    Ok(())
}

/// Creates the user, or updates the one that already has this `userName`. An update keeps the
/// id and the administrative hold. Returns the user and whether it was created.
pub fn upsert_user(
    state: &SharedState,
    fields: UserFields,
) -> Result<(ScimUser, bool), ServiceError> {
    let existing = state
        .repository
        .user_by_name(&fields.user_name)
        .map_err(|e| ServiceError::Internal(format!("Failed to read user: {e}")))?;
    let (user, created) = match existing {
        Some(mut u) => {
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
            (u, false)
        }
        None => {
            let is_synthetic = fields.user_name.to_lowercase().ends_with("@synthetic.invalid");
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
                synthetic: is_synthetic,
            };
            (u, true)
        }
    };
    persist(state, &user)?;
    Ok((user, created))
}

/// Replaces the organization roles that come from this user's SCIM record.
pub fn sync_roles(state: &SharedState, user: &ScimUser) {
    let orgs: Vec<OrganizationNode> = state
        .organizations
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .cloned()
        .collect();
    let positions: Vec<crate::state::PositionType> = state
        .position_types
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .cloned()
        .collect();
    let roles = crate::routes::scim::sync_org_roles(user, &orgs, &positions);
    // An admin row SCIM wrote earlier stays while the person is active. Nothing new is written for it.
    let _ = state.delete_scim_roles(&user.user_name, user.active);
    for r in roles {
        let _ = state.persist_role(&r);
    }
}

fn set_hold(state: &SharedState, id: &str, hold: bool) -> Result<ScimUser, ServiceError> {
    let mut user = find_by_id(state, id)
        .ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    user.admin_hold = hold;
    persist(state, &user)?;
    Ok(user)
}

// ---------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------

pub fn display_name(u: &ScimUser) -> String {
    if let Some(f) = u
        .name
        .get("formatted")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return f.to_string();
    }
    let given = u
        .name
        .get("givenName")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let family = u
        .name
        .get("familyName")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let full = format!("{given} {family}").trim().to_string();
    if full.is_empty() {
        u.user_name.clone()
    } else {
        full
    }
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
    /// The units, and the appointments of the people named. The units are the cached tree. The appointments are one
    /// statement.
    fn read(state: &SharedState, people: &[String]) -> Self {
        let orgs = state
            .organizations
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|o| (o.id, o.clone()))
            .collect();
        let roles = state.repository.roles_of_people(people).unwrap_or_default();
        Self { orgs, roles }
    }

    fn roles_of<'a>(&'a self, user_name: &'a str) -> impl Iterator<Item = &'a RoleRow> {
        self.roles
            .iter()
            .filter(move |r| r.eppn.eq_ignore_ascii_case(user_name))
    }

    fn unit(&self, id: Uuid) -> Value {
        let name = self
            .orgs
            .get(&id)
            .map(|o| o.name.clone())
            .unwrap_or_default();
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
    Basics {
        units,
        unit_admin_of,
        platform_admin,
    }
}

fn token_summary(tokens: &[ApiToken]) -> (usize, Option<String>) {
    let now = Utc::now();
    let active_agents = tokens
        .iter()
        .filter(|t| t.kind == "agent" && t.revoked_at.is_none() && t.expires_at > now)
        .count();
    let latest = tokens
        .iter()
        .filter_map(|t| t.last_used_at)
        .max()
        .map(|t| t.to_rfc3339());
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

/// One page of users sorted by `user_name`. Returns the rows and the cursor for the next page. The database filters,
/// sorts and cuts the page. Three statements: the users, their appointments, their tokens.
pub fn list_users(
    state: &SharedState,
    q: &UserQuery,
) -> Result<(Vec<Value>, Option<String>), ServiceError> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = match &q.cursor {
        Some(c) => decode_cursor(c)?,
        None => 0,
    };
    let filter = UserFilter {
        prefix: q
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase),
        active: q.active,
        hold: q.hold,
        affiliation: q.affiliation.clone(),
        unit: q.unit,
        platform_admin: q.platform_admin,
        unit_admin: q.unit_admin,
        ..Default::default()
    };
    let mut page = state
        .repository
        .search_users(&filter, offset as i64, limit as i64 + 1)
        .map_err(|e| ServiceError::Internal(format!("Failed to read users: {e}")))?;
    let next = (page.len() > limit).then(|| encode_cursor(offset + limit));
    page.truncate(limit);

    let names: Vec<String> = page.iter().map(|u| u.user_name.clone()).collect();
    let dir = Directory::read(state, &names);
    let tokens = state
        .repository
        .tokens_of_people(&names)
        .unwrap_or_default();
    let rows = page
        .iter()
        .map(|u| {
            let mine: Vec<ApiToken> = tokens
                .iter()
                .filter(|t| t.eppn == u.user_name)
                .cloned()
                .collect();
            row(&dir, u, &mine)
        })
        .collect();
    Ok((rows, next))
}

fn ledger_rows(state: &SharedState, principal: &str) -> Vec<Value> {
    // Newest first. The database picks the rows, so the chain is not read to find one person's.
    state
        .repository
        .ledger_by_principal(principal, LEDGER_ROWS as i64)
        .unwrap_or_default()
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect()
}

/// The user row, plus appointments, workspace memberships, tokens (never hashes), and the last
/// 20 ledger entries where the user is the principal.
pub fn user_detail(state: &SharedState, id: &str) -> Result<Value, ServiceError> {
    let u = find_by_id(state, id)
        .ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let dir = Directory::read(state, std::slice::from_ref(&u.user_name));
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

    // One read of the person's appointments, groups and grants serves every workspace.
    let workspace_ids: Vec<String> = crate::service::members::workspace_ids(state);
    let memberships: Vec<Value> = workspace_ids
        .iter()
        .zip(crate::service::members::member_roles(&u.user_name, &workspace_ids, state))
        .filter_map(|(ws, role)| role.map(|r| json!({ "workspace_id": ws, "role": r.as_str() })))
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
    state
        .repository
        .group_summaries()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, name, members)| json!({ "id": id, "name": name, "member_count": members }))
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

pub fn create_user(
    state: &SharedState,
    admin: &AuthUser,
    new: NewUser,
    reason: &str,
) -> Result<Value, ServiceError> {
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
    let user = admin_write(
        state,
        admin,
        DecisionType::UserAccessChanged,
        "AC-02",
        reason,
        &payload,
        || {
            let (user, _) = upsert_user(state, fields_for(&new))?;
            sync_roles(state, &user);
            Ok(user)
        },
    )?;
    let dir = Directory::read(state, std::slice::from_ref(&user.user_name));
    Ok(row(&dir, &user, &[]))
}

pub fn set_user_hold(
    state: &SharedState,
    admin: &AuthUser,
    id: &str,
    hold: bool,
    reason: &str,
) -> Result<Value, ServiceError> {
    let target = find_by_id(state, id)
        .ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let payload = json!({ "action": if hold { "hold" } else { "release" }, "user_id": target.id, "user_name": target.user_name });
    let user = admin_write(
        state,
        admin,
        DecisionType::UserAccessChanged,
        "AC-02",
        reason,
        &payload,
        || set_hold(state, id, hold),
    )?;
    let dir = Directory::read(state, std::slice::from_ref(&user.user_name));
    Ok(row(
        &dir,
        &user,
        &state.list_all_api_tokens(&user.user_name),
    ))
}

/// Revokes every agent and impersonation token the user holds. SCIM tokens are not theirs.
pub fn revoke_user_tokens(
    state: &SharedState,
    admin: &AuthUser,
    id: &str,
    reason: &str,
) -> Result<usize, ServiceError> {
    let target = find_by_id(state, id)
        .ok_or_else(|| ServiceError::NotFound(format!("User '{id}' not found")))?;
    let now = Utc::now();
    let live: Vec<ApiToken> = state
        .list_all_api_tokens(&target.user_name)
        .into_iter()
        .filter(|t| {
            (t.kind == "agent" || t.kind == "impersonation")
                && t.revoked_at.is_none()
                && t.expires_at > now
        })
        .collect();
    let payload = json!({ "action": "revoke_tokens", "user_id": target.id, "user_name": target.user_name, "count": live.len() });
    admin_write(
        state,
        admin,
        DecisionType::TokenRevoked,
        "IA-05",
        reason,
        &payload,
        || {
            let mut revoked = 0;
            for t in &live {
                if state
                    .revoke_api_token(t.id, &admin.eppn, true)
                    .unwrap_or(false)
                {
                    revoked += 1;
                }
            }
            Ok(revoked)
        },
    )
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
            Self::OwnedByRegistry => {
                "The registry owns this appointment. Change it in the registry"
            }
            Self::LastPlatformAdmin => {
                "This is the last Platform Admin appointment. Appoint another Platform Admin first"
            }
        }
    }
}

/// Whether an appointment may be revoked, whoever asks. Authorization is a separate question.
pub fn revoke_check(
    role: &RoleRow,
    all_roles: &[RoleRow],
    root_unit: Uuid,
) -> Result<(), RevokeRefusal> {
    if role.source == "scim" {
        return Err(RevokeRefusal::OwnedByRegistry);
    }
    if role.scoped_affiliation == "platform_admin" {
        let others: HashSet<&str> = all_roles
            .iter()
            .filter(|r| {
                r.id != role.id
                    && r.scoped_affiliation == "platform_admin"
                    && r.organization_id == root_unit
            })
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
    tree: &scaffoldry_core::tree::OrgTree,
    roles: &[RoleRow],
) -> bool {
    let org_caller = OrgCaller {
        eppn: caller.eppn.clone(),
        affiliation: caller.affiliation.clone(),
    };
    if caller.affiliation == "central_admin"
        || roles.iter().any(|r| {
            r.eppn == caller.eppn
                && r.scoped_affiliation == "platform_admin"
                && tree.parent_of(r.organization_id).is_none()
        })
    {
        return true;
    }
    scoped_affiliation == "unit_admin"
        && crate::service::organizations::unit_in_reach(&org_caller, unit, tree, roles)
}

/// The most people one search for a collaborator considers. A narrower search finds the rest.
const COLLABORATOR_CANDIDATES: i64 = 200;

pub fn list_app_collaborators(
    app_slug: &str,
    query: &str,
    state: &SharedState,
) -> Result<Vec<Value>, ServiceError> {
    let q = query.trim().to_lowercase();
    let filter = UserFilter {
        contains: Some(q).filter(|q| !q.is_empty()),
        active: Some(true),
        hold: Some(false),
        ..Default::default()
    };
    let candidates = state
        .repository
        .search_users(&filter, 0, COLLABORATOR_CANDIDATES)
        .unwrap_or_default();

    let mut out = Vec::new();
    for u in candidates {
        let d_name = display_name(&u);
        let user_email = email(&u);
        let user_name = u.user_name.clone();

        if let Some(candidate) = crate::service::identity::resolve_user(&user_name, state) {
            if crate::service::access::authorize_app(
                &candidate,
                app_slug,
                crate::service::access::AppAction::Read,
                state,
            )
            .is_ok()
            {
                out.push(serde_json::json!({
                    "id": user_name,
                    "label": d_name,
                    "email": user_email,
                }));
            }
        }
    }

    out.sort_by(|a, b| {
        let la = a.get("label").and_then(Value::as_str).unwrap_or("");
        let lb = b.get("label").and_then(Value::as_str).unwrap_or("");
        la.cmp(lb)
    });
    Ok(out)
}
