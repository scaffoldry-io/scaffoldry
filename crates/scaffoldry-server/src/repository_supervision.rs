//! Access model phase 10: the supervision tree. SCIM's `manager` attribute is kept as units of a `people` hierarchy whose
//! source is `scim`: one unit per manager, its parent the unit of that manager's manager. One transaction writes the
//! units, the person's appointment and the supervisors' positions, and raises the `organizations` counter.

use crate::repository::{upsert_role_in, PostgresRepository, RepositoryError};
use crate::state::RoleRow;
use uuid::Uuid;

/// The lock that `upsert_organization` and `move_organization` take. "SCORGTRE".
const TREE_LOCK: i64 = 0x5343_4f52_4754_5245;

/// The source of the rows this module writes. SCIM's own role sync deletes rows whose source is `scim`, so these differ. The column holds 8 characters.
pub const SOURCE: &str = "manager";
/// The position a manager holds at their own unit.
pub const SUPERVISOR: &str = "supervisor";

impl PostgresRepository {
    /// The keys of the active `people` hierarchies that SCIM keeps.
    pub fn scim_people_hierarchies(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(|c| {
            let rows = c.query("SELECT key FROM hierarchies WHERE kind = 'people' AND source = 'scim' AND status = 'Active' ORDER BY key", &[])?;
            Ok(rows.iter().map(|r| r.get(0)).collect())
        })
    }

    /// Places `person` under the first manager of `chain` (the person's manager, then that manager's manager, and so on),
    /// building the units the chain needs. An empty chain hangs the person from the hierarchy's root. `active` false
    /// removes the person's appointments in the hierarchy. A hierarchy with no root is left alone.
    pub fn apply_supervision(&self, hierarchy: &str, person: &str, chain: &[(String, String)], active: bool) -> Result<bool, RepositoryError> {
        let (hierarchy, person, chain) = (hierarchy.to_string(), person.to_string(), chain.to_vec());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            // The same lock every writer of the tree takes, so the closure never sees two moves at once.
            tx.query_one("SELECT pg_advisory_xact_lock($1)", &[&TREE_LOCK])?;
            let Some(root) = tx.query_opt("SELECT id FROM organizations WHERE hierarchy_key = $1 AND parent_id IS NULL", &[&hierarchy])? else {
                return Ok(false);
            };
            let root: Uuid = root.get(0);
            tx.execute(
                "INSERT INTO position_types (key, name, description, org_types, max_holders, created_by) \
                 VALUES ($1, 'Supervisor', 'Supervises the people who report to them', '[\"Institution\",\"College\",\"Department\",\"Center\",\"Program\",\"Unit\"]'::jsonb, 1000, 'system') \
                 ON CONFLICT (key) DO NOTHING",
                &[&SUPERVISOR],
            )?;
            // The units, from the top manager down. Each takes the unit above as its parent, so a manager who moved takes their unit along.
            let mut parent = root;
            let mut units: Vec<Uuid> = Vec::new();
            for (eppn, name) in chain.iter().rev() {
                let unit = ensure_unit(&mut tx, &hierarchy, eppn, name, parent)?;
                units.push(unit);
                parent = unit;
            }
            units.reverse(); // units[i] is the unit of chain[i]
            // The person's own unit, when they already have one, follows their manager.
            if let Some(own) = unit_of(&mut tx, &hierarchy, &person)? {
                move_unit(&mut tx, &hierarchy, own, parent_of(&units, root))?;
            }
            // The person's appointment. Their old ones in this hierarchy go.
            tx.execute(
                "DELETE FROM roles r USING persons p, organizations o \
                 WHERE r.person_id = p.id AND p.eppn = $1 AND r.organization_id = o.id AND o.hierarchy_key = $2 \
                   AND r.source = $3 AND r.position_key IS NULL AND ($4::bool = false OR r.organization_id <> $5)",
                &[&person, &hierarchy, &SOURCE, &active, &parent_of(&units, root)],
            )?;
            if active {
                let target = parent_of(&units, root);
                let held = tx.query_opt(
                    "SELECT 1 FROM roles r JOIN persons p ON p.id = r.person_id WHERE p.eppn = $1 AND r.organization_id = $2 AND r.source = $3 AND r.position_key IS NULL",
                    &[&person, &target, &SOURCE],
                )?;
                if held.is_none() {
                    upsert_role_in(&mut tx, &role(&person, target, None))?;
                }
            }
            // Each manager in the chain supervises their own unit.
            for ((eppn, _), unit) in chain.iter().zip(&units) {
                let held = tx.query_opt(
                    "SELECT 1 FROM roles r JOIN persons p ON p.id = r.person_id WHERE p.eppn = $1 AND r.organization_id = $2 AND r.position_key = $3",
                    &[eppn, unit, &SUPERVISOR],
                )?;
                if held.is_none() {
                    upsert_role_in(&mut tx, &role(eppn, *unit, Some(SUPERVISOR)))?;
                }
            }
            crate::cache::bump(&mut tx, "organizations")?;
            tx.commit()?;
            Ok(true)
        })
    }
}

/// The unit the person goes under: their direct manager's unit, or the root.
fn parent_of(units: &[Uuid], root: Uuid) -> Uuid {
    units.first().copied().unwrap_or(root)
}

fn code_of(hierarchy: &str, eppn: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{hierarchy}:{}", eppn.to_lowercase()).as_bytes());
    format!("SUP-{}", digest.iter().take(6).map(|b| format!("{b:02x}")).collect::<String>())
}

fn unit_of(tx: &mut postgres::Transaction<'_>, hierarchy: &str, eppn: &str) -> Result<Option<Uuid>, RepositoryError> {
    Ok(tx.query_opt("SELECT id FROM organizations WHERE hierarchy_key = $1 AND code = $2", &[&hierarchy, &code_of(hierarchy, eppn)])?.map(|r| r.get(0)))
}

/// The manager's unit, made when it is missing and moved under `parent` when it is elsewhere.
fn ensure_unit(tx: &mut postgres::Transaction<'_>, hierarchy: &str, eppn: &str, name: &str, parent: Uuid) -> Result<Uuid, RepositoryError> {
    if let Some(id) = unit_of(tx, hierarchy, eppn)? {
        move_unit(tx, hierarchy, id, parent)?;
        return Ok(id);
    }
    let id = Uuid::new_v4();
    tx.execute(
        "INSERT INTO organizations (id, parent_id, name, code, org_type, hierarchy_key) VALUES ($1, $2, $3, $4, 'Unit', $5)",
        &[&id, &parent, &format!("Reports of {name}"), &code_of(hierarchy, eppn), &hierarchy],
    )?;
    // The closure holds the unit under itself and under each ancestor of its parent.
    tx.execute("INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) VALUES ($1, $2, $2, 0) ON CONFLICT (ancestor, descendant) DO NOTHING", &[&hierarchy, &id])?;
    tx.execute(
        "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) SELECT $1, ancestor, $2, (depth + 1)::smallint FROM org_closure WHERE descendant = $3 ON CONFLICT (ancestor, descendant) DO NOTHING",
        &[&hierarchy, &id, &parent],
    )?;
    Ok(id)
}

/// Moves a unit, and everything under it, to a new parent, when it is somewhere else. The closure follows.
fn move_unit(tx: &mut postgres::Transaction<'_>, hierarchy: &str, id: Uuid, parent: Uuid) -> Result<(), RepositoryError> {
    let moved = tx.execute("UPDATE organizations SET parent_id = $2 WHERE id = $1 AND parent_id IS DISTINCT FROM $2", &[&id, &parent])?;
    if moved == 0 {
        return Ok(());
    }
    tx.execute(
        "DELETE FROM org_closure WHERE descendant IN (SELECT descendant FROM org_closure WHERE ancestor = $1) \
           AND ancestor NOT IN (SELECT descendant FROM org_closure WHERE ancestor = $1)",
        &[&id],
    )?;
    tx.execute(
        "INSERT INTO org_closure (hierarchy_key, ancestor, descendant, depth) \
         SELECT $3, p_anc.ancestor, sub_desc.descendant, (p_anc.depth + 1 + sub_desc.depth)::smallint \
         FROM org_closure p_anc CROSS JOIN org_closure sub_desc WHERE p_anc.descendant = $1 AND sub_desc.ancestor = $2 \
         ON CONFLICT (ancestor, descendant) DO NOTHING",
        &[&parent, &id, &hierarchy],
    )?;
    Ok(())
}

fn role(eppn: &str, unit: Uuid, position: Option<&str>) -> RoleRow {
    RoleRow {
        id: Uuid::new_v4(),
        person_id: Uuid::new_v4(),
        eppn: eppn.to_string(),
        organization_id: unit,
        role_title: position.unwrap_or("Report").to_string(),
        scoped_affiliation: "member".to_string(),
        is_primary: false,
        source: SOURCE.to_string(),
        position_key: position.map(str::to_string),
    }
}
