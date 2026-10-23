//! Stateless server phase 2: people, groups and appointments are read from PostgreSQL, one question at a time.
//! Nothing here loads a whole table. Every query names the person, the unit or the page it needs.

use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::{RoleRow, ScimGroup, ScimUser};
use uuid::Uuid;

const ROLE_SELECT: &str = "SELECT r.id, r.person_id, p.eppn, r.organization_id, r.role_title, r.scoped_affiliation, \
    r.is_primary, COALESCE(r.source, 'api'), r.position_key FROM roles r JOIN persons p ON r.person_id = p.id";

pub fn role_of(r: &postgres::Row) -> RoleRow {
    RoleRow {
        id: r.get(0),
        person_id: r.get(1),
        eppn: r.get(2),
        organization_id: r.get(3),
        role_title: r.get(4),
        scoped_affiliation: r.get(5),
        is_primary: r.get(6),
        source: r.get(7),
        position_key: r.get(8),
    }
}

fn user_of(r: &postgres::Row) -> Result<ScimUser, RepositoryError> {
    Ok(serde_json::from_value(r.get::<_, serde_json::Value>(0))?)
}

/// The filters of the People page. Everything is applied by the database.
#[derive(Debug, Default, Clone)]
pub struct UserFilter {
    /// A lowercase prefix of the user name, the display name or the email.
    pub prefix: Option<String>,
    /// A lowercase fragment of the user name, the display name or the email.
    pub contains: Option<String>,
    pub active: Option<bool>,
    pub hold: Option<bool>,
    pub affiliation: Option<String>,
    pub unit: Option<Uuid>,
    pub platform_admin: Option<bool>,
    pub unit_admin: Option<bool>,
}

const DISPLAY: &str = "COALESCE(NULLIF(payload->'name'->>'formatted', ''), \
    NULLIF(trim(COALESCE(payload->'name'->>'givenName', '') || ' ' || COALESCE(payload->'name'->>'familyName', '')), ''), user_name)";
const EMAIL: &str = "COALESCE(payload->'emails'->0->>'value', user_name)";
const AFFILIATION: &str = "COALESCE((SELECT lower(split_part(t.r->>'value', '@', 1)) \
    FROM jsonb_array_elements(CASE WHEN jsonb_typeof(payload->'roles') = 'array' THEN payload->'roles' ELSE '[]'::jsonb END) \
    WITH ORDINALITY t(r, i) \
    WHERE lower(t.r->>'type') = 'edupersonscopedaffiliation' \
      AND lower(split_part(t.r->>'value', '@', 1)) IN ('faculty','student','staff','employee','member','affiliate','alum') \
    ORDER BY t.i LIMIT 1), 'member')";
const HELD_ROLE: &str = "EXISTS (SELECT 1 FROM roles r JOIN persons p ON r.person_id = p.id WHERE lower(p.eppn) = lower(scim_users.user_name)";

impl PostgresRepository {
    pub fn user_by_id(&self, id: &str) -> Result<Option<ScimUser>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| c.query_opt("SELECT payload FROM scim_users WHERE id = $1", &[&id])?.map(|r| user_of(&r)).transpose())
    }

    /// The user whose name is `name`, or failing that whose email is `name`. Case does not matter.
    pub fn user_by_name_or_email(&self, name: &str) -> Result<Option<ScimUser>, RepositoryError> {
        let name = name.to_string();
        self.with_client(move |c| {
            let by_name = "SELECT payload FROM scim_users WHERE lower(user_name) = lower($1) ORDER BY updated_at DESC, id LIMIT 1";
            if let Some(r) = c.query_opt(by_name, &[&name])? {
                return Ok(Some(user_of(&r)?));
            }
            let by_email = "SELECT payload FROM scim_users WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(payload->'emails') e \
                            WHERE lower(e->>'value') = lower($1)) ORDER BY updated_at DESC, id LIMIT 1";
            c.query_opt(by_email, &[&name])?.map(|r| user_of(&r)).transpose()
        })
    }

    /// The user whose name is exactly `name`, ignoring case. The id is the tie-break when a name is stored twice.
    pub fn user_by_name(&self, name: &str) -> Result<Option<ScimUser>, RepositoryError> {
        let name = name.to_string();
        self.with_client(move |c| {
            let q = "SELECT payload FROM scim_users WHERE lower(user_name) = lower($1) ORDER BY updated_at DESC, id LIMIT 1";
            c.query_opt(q, &[&name])?.map(|r| user_of(&r)).transpose()
        })
    }

    pub fn users_by_ids(&self, ids: &[String]) -> Result<Vec<ScimUser>, RepositoryError> {
        let ids = ids.to_vec();
        self.with_client(move |c| c.query("SELECT payload FROM scim_users WHERE id = ANY($1)", &[&ids])?.iter().map(user_of).collect())
    }

    pub fn users_by_names(&self, names: &[String]) -> Result<Vec<ScimUser>, RepositoryError> {
        let names: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
        self.with_client(move |c| {
            c.query("SELECT payload FROM scim_users WHERE lower(user_name) = ANY($1)", &[&names])?.iter().map(user_of).collect()
        })
    }

    /// One page of users in `(lower(user_name), id)` order. Returns one row more than `limit` when there is a next page.
    pub fn search_users(&self, f: &UserFilter, offset: i64, limit: i64) -> Result<Vec<ScimUser>, RepositoryError> {
        let f = f.clone();
        self.with_client(move |c| {
            let mut sql = String::from("SELECT payload FROM scim_users WHERE true");
            let mut args: Vec<Box<dyn postgres::types::ToSql + Sync>> = Vec::new();
            let arg = |v: Box<dyn postgres::types::ToSql + Sync>, args: &mut Vec<Box<dyn postgres::types::ToSql + Sync>>| {
                args.push(v);
                args.len()
            };
            if let Some(p) = &f.prefix {
                let like = format!("{}%", p.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                let n = arg(Box::new(like), &mut args);
                sql += &format!(
                    " AND (lower(user_name) LIKE ${n} OR lower({DISPLAY}) LIKE ${n} OR lower({EMAIL}) LIKE ${n})"
                );
            }
            if let Some(p) = &f.contains {
                let like = format!("%{}%", p.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                let n = arg(Box::new(like), &mut args);
                sql += &format!(
                    " AND (lower(user_name) LIKE ${n} OR lower({DISPLAY}) LIKE ${n} OR lower({EMAIL}) LIKE ${n})"
                );
            }
            if let Some(a) = f.active {
                let n = arg(Box::new(a), &mut args);
                sql += &format!(" AND active = ${n}");
            }
            if let Some(h) = f.hold {
                let n = arg(Box::new(h), &mut args);
                sql += &format!(" AND COALESCE((payload->>'admin_hold')::boolean, false) = ${n}");
            }
            if let Some(a) = &f.affiliation {
                let n = arg(Box::new(a.to_lowercase()), &mut args);
                sql += &format!(" AND {AFFILIATION} = ${n}");
            }
            if let Some(u) = f.unit {
                let n = arg(Box::new(u), &mut args);
                sql += &format!(" AND {HELD_ROLE} AND r.organization_id = ${n})");
            }
            if let Some(want) = f.platform_admin {
                let cond = format!(
                    "{HELD_ROLE} AND r.scoped_affiliation = 'platform_admin' \
                     AND EXISTS (SELECT 1 FROM organizations o WHERE o.id = r.organization_id AND o.parent_id IS NULL))"
                );
                sql += &format!(" AND {}{cond}", if want { "" } else { "NOT " });
            }
            if let Some(want) = f.unit_admin {
                let cond = format!("{HELD_ROLE} AND r.scoped_affiliation = 'unit_admin')");
                sql += &format!(" AND {}{cond}", if want { "" } else { "NOT " });
            }
            let n = arg(Box::new(limit), &mut args);
            let m = arg(Box::new(offset), &mut args);
            sql += &format!(" ORDER BY lower(user_name) COLLATE \"C\", id LIMIT ${n} OFFSET ${m}");
            let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = args.iter().map(|b| b.as_ref()).collect();
            c.query(&sql, &refs)?.iter().map(user_of).collect()
        })
    }

    pub fn group_by_id(&self, id: &str) -> Result<Option<ScimGroup>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| {
            c.query_opt("SELECT payload FROM scim_groups WHERE id = $1", &[&id])?
                .map(|r| Ok(serde_json::from_value(r.get::<_, serde_json::Value>(0))?))
                .transpose()
        })
    }

    /// The ids of the groups that list any of `values` as a member. A member is an id or a user name.
    pub fn group_ids_with_member(&self, values: &[String]) -> Result<Vec<String>, RepositoryError> {
        let values: Vec<String> = values.iter().map(|v| v.to_lowercase()).collect();
        self.with_client(move |c| {
            let q = "SELECT id FROM scim_groups WHERE EXISTS (SELECT 1 FROM jsonb_array_elements(payload->'members') m \
                     WHERE lower(m->>'value') = ANY($1))";
            Ok(c.query(q, &[&values])?.iter().map(|r| r.get::<_, String>(0)).collect())
        })
    }

    /// `(id, member_count)` of every group, by name. The People page lists these.
    pub fn group_summaries(&self) -> Result<Vec<(String, String, i64)>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT id, display_name, COALESCE(jsonb_array_length(CASE WHEN jsonb_typeof(payload->'members') = 'array' \
                     THEN payload->'members' ELSE '[]'::jsonb END), 0)::bigint FROM scim_groups ORDER BY lower(display_name), id";
            Ok(c.query(q, &[])?.iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
        })
    }

    pub fn roles_of_person(&self, eppn: &str) -> Result<Vec<RoleRow>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| {
            Ok(c.query(&format!("{ROLE_SELECT} WHERE lower(p.eppn) = lower($1) ORDER BY r.created_at, r.id"), &[&eppn])?.iter().map(role_of).collect())
        })
    }

    pub fn roles_of_people(&self, eppns: &[String]) -> Result<Vec<RoleRow>, RepositoryError> {
        let eppns: Vec<String> = eppns.iter().map(|e| e.to_lowercase()).collect();
        self.with_client(move |c| {
            Ok(c.query(&format!("{ROLE_SELECT} WHERE lower(p.eppn) = ANY($1)"), &[&eppns])?.iter().map(role_of).collect())
        })
    }

    pub fn roles_in_units(&self, units: &[Uuid]) -> Result<Vec<RoleRow>, RepositoryError> {
        let units = units.to_vec();
        self.with_client(move |c| {
            Ok(c.query(&format!("{ROLE_SELECT} WHERE r.organization_id = ANY($1)"), &[&units])?.iter().map(role_of).collect())
        })
    }

    pub fn roles_at(&self, unit: Uuid, key: &str) -> Result<Vec<RoleRow>, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| {
            Ok(c.query(&format!("{ROLE_SELECT} WHERE r.organization_id = $1 AND r.position_key = $2"), &[&unit, &key])?.iter().map(role_of).collect())
        })
    }

    pub fn roles_with_key(&self, key: &str) -> Result<Vec<RoleRow>, RepositoryError> {
        let key = key.to_string();
        self.with_client(move |c| Ok(c.query(&format!("{ROLE_SELECT} WHERE r.position_key = $1"), &[&key])?.iter().map(role_of).collect()))
    }

    pub fn role_by_id(&self, id: Uuid) -> Result<Option<RoleRow>, RepositoryError> {
        self.with_client(move |c| Ok(c.query_opt(&format!("{ROLE_SELECT} WHERE r.id = $1"), &[&id])?.as_ref().map(role_of)))
    }

    pub fn count_users(&self) -> Result<i64, RepositoryError> {
        self.with_client(|c| Ok(c.query_one("SELECT COUNT(*) FROM scim_users", &[])?.get(0)))
    }

    /// `(active, on hold, inactive)`. A user on hold counts once, as on hold.
    pub fn people_counts(&self) -> Result<(usize, usize, usize), RepositoryError> {
        self.with_client(|c| {
            let r = c.query_one(
                "SELECT COUNT(*) FILTER (WHERE COALESCE((payload->>'admin_hold')::boolean, false)), \
                        COUNT(*) FILTER (WHERE NOT COALESCE((payload->>'admin_hold')::boolean, false) AND active), \
                        COUNT(*) FILTER (WHERE NOT COALESCE((payload->>'admin_hold')::boolean, false) AND NOT active) \
                 FROM scim_users",
                &[],
            )?;
            let n = |i: usize| r.get::<_, i64>(i) as usize;
            Ok((n(1), n(0), n(2)))
        })
    }

    /// The names of those who hold a `platform_admin` appointment on the root unit. A title on a record grants
    /// nothing and is not counted.
    pub fn platform_admin_names(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT p.eppn FROM roles r JOIN persons p ON r.person_id = p.id JOIN organizations o ON r.organization_id = o.id \
                     WHERE r.scoped_affiliation = 'platform_admin' AND o.parent_id IS NULL AND p.eppn IS NOT NULL";
            Ok(c.query(q, &[])?.iter().map(|r| r.get::<_, String>(0)).collect())
        })
    }

    /// The live tokens of each kind.
    pub fn active_token_counts(&self) -> Result<Vec<(String, i64)>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT kind, COUNT(*) FROM api_tokens WHERE revoked_at IS NULL AND expires_at > NOW() GROUP BY kind";
            Ok(c.query(q, &[])?.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
    }

    /// Every token of these people, newest first, keyed by lowercase name. One statement for a page.
    pub fn tokens_of_people(&self, names: &[String]) -> Result<Vec<crate::state::ApiToken>, RepositoryError> {
        let names: Vec<String> = names.to_vec();
        self.with_client(move |c| {
            let q = "SELECT token_hash, id, kind, eppn, label, original_admin, created_at::text, expires_at::text, last_used_at::text, \
                            revoked_at::text, app_slug, scope FROM api_tokens WHERE eppn = ANY($1) ORDER BY created_at DESC";
            c.query(q, &[&names])?.iter().map(crate::repository::api_token_of_text_row).collect()
        })
    }

    /// How many people hold each position, over every unit.
    pub fn holder_counts(&self) -> Result<std::collections::HashMap<String, i64>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT position_key, COUNT(*) FROM roles WHERE position_key IS NOT NULL GROUP BY position_key";
            Ok(c.query(q, &[])?.iter().map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1))).collect())
        })
    }

    /// The `(position, unit)` pairs that have at least one holder.
    pub fn filled_positions(&self) -> Result<std::collections::HashSet<(String, Uuid)>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT DISTINCT position_key, organization_id FROM roles WHERE position_key IS NOT NULL";
            Ok(c.query(q, &[])?.iter().map(|r| (r.get::<_, String>(0), r.get::<_, Uuid>(1))).collect())
        })
    }

    /// Adds a position type. False when the key is taken, decided by the database so two servers cannot both win.
    pub fn create_position_type(&self, pt: &crate::state::PositionType) -> Result<bool, RepositoryError> {
        let pt = pt.clone();
        self.with_client(move |c| {
            let org_types = serde_json::to_value(&pt.org_types)?;
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) \
                 VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (key) DO NOTHING",
                &[&pt.key, &pt.name, &pt.description, &org_types, &pt.max_holders, &pt.created_by],
            )?;
            if n == 1 {
                crate::cache::bump(&mut tx, "position_types")?;
            }
            tx.commit()?;
            Ok(n == 1)
        })
    }

    /// Changes a position type. The row is locked first, so a holder cannot arrive between the count and the write.
    /// `Err(Conflict)` when the unit types change while the position has holders.
    pub fn change_position_type(&self, pt: &crate::state::PositionType) -> Result<(), RepositoryError> {
        let pt = pt.clone();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let row = tx.query_opt("SELECT org_types FROM position_types WHERE key = $1 FOR UPDATE", &[&pt.key])?;
            let Some(row) = row else { return Err(RepositoryError::NotFound(format!("position '{}'", pt.key))) };
            let stored: Vec<String> = serde_json::from_value(row.get::<_, serde_json::Value>(0)).unwrap_or_default();
            if stored != pt.org_types {
                let held: i64 = tx.query_one("SELECT COUNT(*) FROM roles WHERE position_key = $1", &[&pt.key])?.get(0);
                if held > 0 {
                    return Err(RepositoryError::Conflict("The unit types cannot change while the position has holders".into()));
                }
            }
            let org_types = serde_json::to_value(&pt.org_types)?;
            let retired = pt.retired_at.is_some();
            tx.execute(
                "UPDATE position_types SET name = $2, description = $3, org_types = $4, max_holders = $5, \
                    retired_at = CASE WHEN $6::bool THEN COALESCE(retired_at, NOW()) END WHERE key = $1",
                &[&pt.key, &pt.name, &pt.description, &org_types, &pt.max_holders, &retired],
            )?;
            crate::cache::bump(&mut tx, "position_types")?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Gives a position to a person, or says why not. The position type's row is locked for the whole decision, so
    /// two servers filling the last seat at once cannot both succeed.
    pub fn assign_holding(&self, holding: &RoleRow, max_holders: i32, replace: bool) -> Result<Holding, RepositoryError> {
        let holding = holding.clone();
        self.with_client(move |c| {
            let key = holding.position_key.clone().unwrap_or_default();
            let mut tx = c.transaction()?;
            tx.query_opt("SELECT 1 FROM position_types WHERE key = $1 FOR UPDATE", &[&key])?;
            let current: Vec<RoleRow> = tx
                .query(&format!("{ROLE_SELECT} WHERE r.organization_id = $1 AND r.position_key = $2"), &[&holding.organization_id, &key])?
                .iter()
                .map(role_of)
                .collect();
            if current.iter().any(|r| r.eppn.eq_ignore_ascii_case(&holding.eppn)) {
                return Ok(Holding::AlreadyHeld);
            }
            let full = current.len() as i32 >= max_holders;
            if full && !replace {
                return Ok(Holding::Full(current.len()));
            }
            let vacate: Vec<RoleRow> = if full { current } else { Vec::new() };
            if vacate.iter().any(|r| r.source == "scim") {
                return Ok(Holding::RegistryOwned);
            }
            for r in &vacate {
                tx.execute("DELETE FROM roles WHERE id = $1", &[&r.id])?;
            }
            crate::repository::upsert_role_in(&mut tx, &holding)?;
            tx.commit()?;
            Ok(Holding::Given)
        })
    }
}

impl PostgresRepository {
    /// Deletes an appointment. A Platform Admin appointment at `root` is deleted only if another person keeps
    /// one. The admin rows are locked in id order first, so two revokes at once cannot each see the other as the
    /// spare.
    pub fn revoke_appointment(&self, role: &RoleRow, root: uuid::Uuid) -> Result<Revoked, RepositoryError> {
        let role = role.clone();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            if role.scoped_affiliation == "platform_admin" && role.organization_id == root {
                let admins: Vec<RoleRow> = tx
                    .query(
                        &format!("{ROLE_SELECT} WHERE r.organization_id = $1 AND r.scoped_affiliation = 'platform_admin' ORDER BY r.id FOR UPDATE OF r"),
                        &[&root],
                    )?
                    .iter()
                    .map(role_of)
                    .collect();
                if !admins.iter().any(|r| r.id == role.id) {
                    return Ok(Revoked::Gone);
                }
                if !admins.iter().any(|r| r.id != role.id && !r.eppn.eq_ignore_ascii_case(&role.eppn)) {
                    return Ok(Revoked::LastPlatformAdmin);
                }
            }
            let n = tx.execute("DELETE FROM roles WHERE id = $1", &[&role.id])?;
            tx.commit()?;
            Ok(if n == 0 { Revoked::Gone } else { Revoked::Done })
        })
    }
}

/// What `revoke_appointment` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Revoked {
    Done,
    /// Someone else removed it first.
    Gone,
    LastPlatformAdmin,
}

/// The latest ledger entry that granted a membership, for the report of direct grants.
#[derive(Debug, Clone)]
pub struct GrantEntry {
    pub workspace_id: String,
    pub target: String,
    pub app_slug: String,
    pub principal: String,
    pub timestamp: String,
    pub rationale: String,
}

impl PostgresRepository {
    /// For each workspace, member, and app, the latest entry that added them. One statement.
    pub fn membership_grant_entries(&self) -> Result<Vec<GrantEntry>, RepositoryError> {
        self.with_client(|c| {
            let q = "SELECT DISTINCT ON (payload->>'workspace_id', payload->>'target_eppn', COALESCE(payload->>'app_slug', '')) \
                            payload->>'workspace_id', payload->>'target_eppn', COALESCE(payload->>'app_slug', ''), \
                            principal, timestamp_iso, rationale \
                     FROM governance_ledger \
                     WHERE decision_type IN ('WorkspaceMemberAdded', 'AccessRoleGranted') AND payload->>'target_eppn' IS NOT NULL \
                     ORDER BY payload->>'workspace_id', payload->>'target_eppn', COALESCE(payload->>'app_slug', ''), sequence DESC";
            Ok(c.query(q, &[])?
                .iter()
                .map(|r| GrantEntry {
                    workspace_id: r.get(0),
                    target: r.get(1),
                    app_slug: r.get(2),
                    principal: r.get(3),
                    timestamp: r.get(4),
                    rationale: r.get(5),
                })
                .collect())
        })
    }
}

/// What `assign_holding` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Holding {
    Given,
    AlreadyHeld,
    /// The seats are taken. The number is how many holders there are.
    Full(usize),
    RegistryOwned,
}
