//! When an owner leaves (lifecycle phase 5): who has gone, what they leave behind, and the steps of a transfer.
//!
//! A person has gone when their stored person is inactive, or on administrative hold, or when they had an appointment and
//! none is current. A person with no appointment on record at all is not counted: nothing says they ever had one.
//! A group, unit or position that owns a workspace is a directory entry, not a person, so it never counts as gone.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::{json, Value};

/// SQL for why the person in `col` has gone: `inactive`, `on hold`, `no appointment`, or NULL when they are here.
fn gone(col: &str) -> String {
    format!(
        "CASE \
           WHEN EXISTS (SELECT 1 FROM scim_users gu WHERE lower(gu.user_name) = lower({col}) AND NOT gu.active) THEN 'inactive' \
           WHEN EXISTS (SELECT 1 FROM scim_users gu WHERE lower(gu.user_name) = lower({col}) \
                          AND COALESCE((gu.payload->>'admin_hold')::boolean, false)) THEN 'on hold' \
           WHEN EXISTS (SELECT 1 FROM persons gp JOIN roles gr ON gr.person_id = gp.id WHERE lower(gp.eppn) = lower({col})) \
            AND NOT EXISTS (SELECT 1 FROM persons gp JOIN roles gr ON gr.person_id = gp.id WHERE lower(gp.eppn) = lower({col}) \
                              AND gr.effective_date <= CURRENT_DATE AND (gr.end_date IS NULL OR gr.end_date >= CURRENT_DATE)) \
             THEN 'no appointment' \
         END"
    )
}

/// Every workspace-wide owner row, with why that owner has gone, or NULL. One statement serves the list and the checks.
fn owners_cte() -> String {
    format!(
        "owners AS ( \
           SELECT c.workspace_id, c.eppn, \
                  CASE WHEN c.principal_type <> 'user' THEN NULL ELSE {} END AS why \
           FROM workspace_collaborators c WHERE c.role = 'owner' AND c.app_slug = '' \
         )",
        gone("c.eppn")
    )
}

/// One thing that has lost the person who looked after it.
#[derive(Debug, Clone)]
pub struct Orphan {
    /// `workspace`, `proposal` or `client`.
    pub kind: String,
    pub id: String,
    pub name: String,
    /// For a workspace, its owners and why each has gone. For the rest, who proposed it and why they have gone.
    pub detail: Value,
}

/// A workspace `from` owns, with what a transfer would touch in it.
#[derive(Debug, Clone)]
pub struct OwnedWorkspace {
    pub id: String,
    pub name: String,
    pub to_role: Option<String>,
    pub from_app_roles: i64,
    pub proposals: Vec<Value>,
    pub process_steps: Vec<Value>,
    pub tokens: Vec<Value>,
}

impl PostgresRepository {
    /// Workspaces with no owner who is here, and the open proposals and outside clients of people who have gone.
    /// A trial copy is not listed: it belongs to one person and ends on its own.
    pub fn orphans(&self) -> Result<Vec<Orphan>, RepositoryError> {
        let sql = format!(
            "WITH {owners} \
             SELECT 'workspace', w.id, w.name, \
                    COALESCE(jsonb_agg(jsonb_build_object('eppn', o.eppn, 'why', o.why) ORDER BY o.eppn) FILTER (WHERE o.eppn IS NOT NULL), '[]'::jsonb) \
             FROM workspaces w LEFT JOIN owners o ON o.workspace_id = w.id \
             WHERE NOT EXISTS (SELECT 1 FROM proposal_trials t WHERE t.workspace_id = w.id) \
             GROUP BY w.id, w.name \
             HAVING count(*) FILTER (WHERE o.eppn IS NOT NULL AND o.why IS NULL) = 0 \
             UNION ALL \
             SELECT 'proposal', p.id::text, p.app_slug, jsonb_build_object('proposed_by', p.proposed_by, 'why', {gone_p}, 'workspace_id', p.workspace_id) \
             FROM app_proposals p WHERE p.status = 'Pending' AND {gone_p} IS NOT NULL \
             UNION ALL \
             SELECT 'client', k.client_id, k.name, jsonb_build_object('proposed_by', k.proposed_by, 'why', {gone_k}) \
             FROM app_clients k WHERE k.status IN ('Pending', 'Active') AND {gone_k} IS NOT NULL \
             ORDER BY 1, 3, 2",
            owners = owners_cte(),
            gone_p = gone("p.proposed_by"),
            gone_k = gone("k.proposed_by"),
        );
        self.with_client(move |c| {
            let rows = c.query(&sql, &[])?;
            Ok(rows.iter().map(|r| Orphan { kind: r.get(0), id: r.get(1), name: r.get(2), detail: r.get(3) }).collect())
        })
    }

    /// The workspaces where this person is an owner who is here and no other owner is. Holding or removing them would
    /// leave each with nobody.
    pub fn sole_live_owner_of(&self, eppn: &str) -> Result<Vec<(String, String)>, RepositoryError> {
        let eppn = eppn.to_string();
        let sql = format!(
            "WITH {owners} \
             SELECT w.id, w.name FROM workspaces w \
             WHERE EXISTS (SELECT 1 FROM owners o WHERE o.workspace_id = w.id AND lower(o.eppn) = lower($1) AND o.why IS NULL) \
               AND NOT EXISTS (SELECT 1 FROM owners o WHERE o.workspace_id = w.id AND lower(o.eppn) <> lower($1) AND o.why IS NULL) \
               AND NOT EXISTS (SELECT 1 FROM proposal_trials t WHERE t.workspace_id = w.id) \
             ORDER BY w.name, w.id",
            owners = owners_cte()
        );
        self.with_client(move |c| {
            let rows = c.query(&sql, &[&eppn])?;
            Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
        })
    }

    /// Why this person has gone, or None when they are here or are not known.
    pub fn person_gone(&self, eppn: &str) -> Result<Option<String>, RepositoryError> {
        let eppn = eppn.to_string();
        let sql = format!("SELECT {} FROM (SELECT $1::text AS e) x", gone("x.e"));
        self.with_client(move |c| Ok(c.query_one(&sql, &[&eppn])?.get(0)))
    }

    /// The workspaces this person owns (all of them, or only those named), each with what a transfer to `to` would touch.
    pub fn owned_workspaces(&self, from: &str, to: &str, only: &[String]) -> Result<Vec<OwnedWorkspace>, RepositoryError> {
        let (from, to, only) = (from.to_string(), to.to_string(), only.to_vec());
        self.with_client(move |c| {
            let rows = c.query(
                "SELECT w.id, w.name FROM workspace_collaborators m JOIN workspaces w ON w.id = m.workspace_id \
                 WHERE lower(m.eppn) = lower($1) AND m.role = 'owner' AND m.app_slug = '' AND m.principal_type = 'user' \
                   AND (cardinality($2::text[]) = 0 OR w.id = ANY($2)) \
                   AND NOT EXISTS (SELECT 1 FROM proposal_trials t WHERE t.workspace_id = w.id) \
                 ORDER BY w.name, w.id",
                &[&from, &only],
            )?;
            let mut out = Vec::new();
            for r in rows {
                let (id, name): (String, String) = (r.get(0), r.get(1));
                let to_role: Option<String> = c
                    .query_opt("SELECT role FROM workspace_collaborators WHERE workspace_id = $1 AND lower(eppn) = lower($2) AND app_slug = ''", &[&id, &to])?
                    .map(|r| r.get(0));
                let from_app_roles: i64 = c
                    .query_one("SELECT count(*) FROM workspace_collaborators WHERE workspace_id = $1 AND lower(eppn) = lower($2) AND app_slug <> ''", &[&id, &from])?
                    .get(0);
                let proposals = c
                    .query("SELECT id::text, app_slug, summary FROM app_proposals WHERE workspace_id = $1 AND status = 'Pending' ORDER BY created_at, id", &[&id])?
                    .iter()
                    .map(|r| json!({ "id": r.get::<_, String>(0), "app_slug": r.get::<_, String>(1), "summary": r.get::<_, String>(2) }))
                    .collect();
                let process_steps = c
                    .query(
                        "SELECT p.id, p.app_slug, COALESCE(p.instance_json->>'prompt', '') FROM process_instances p \
                         JOIN app_manifests m ON m.slug = p.app_slug AND m.workspace_id = $1 \
                         WHERE p.status = 'Waiting' AND (lower(p.instance_json->>'assigned_to') = lower($2) \
                               OR (p.instance_json->>'assigned_to' IS NULL AND p.instance_json->>'role' = 'owner')) \
                         ORDER BY p.updated_at, p.id",
                        &[&id, &from],
                    )?
                    .iter()
                    .map(|r| json!({ "id": r.get::<_, String>(0), "app_slug": r.get::<_, String>(1), "prompt": r.get::<_, String>(2) }))
                    .collect();
                let tokens = c
                    .query(
                        "SELECT t.id::text, t.label FROM api_tokens t \
                         WHERE lower(t.eppn) = lower($1) AND t.kind = 'agent' AND t.revoked_at IS NULL AND t.expires_at > NOW() \
                           AND t.scope IS NOT NULL AND t.scope->'apps' ?| ARRAY(SELECT slug FROM app_manifests WHERE workspace_id = $2) \
                         ORDER BY t.created_at, t.id",
                        &[&from, &id],
                    )?
                    .iter()
                    .map(|r| json!({ "id": r.get::<_, String>(0), "label": r.get::<_, String>(1) }))
                    .collect();
                out.push(OwnedWorkspace { id, name, to_role, from_app_roles, proposals, process_steps, tokens });
            }
            Ok(out)
        })
    }

    /// The outside clients this person proposed that are still pending or active.
    pub fn clients_proposed_by(&self, eppn: &str) -> Result<Vec<Value>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |c| {
            let rows = c.query("SELECT client_id, name, status FROM app_clients WHERE lower(proposed_by) = lower($1) AND status IN ('Pending', 'Active') ORDER BY name, client_id", &[&eppn])?;
            Ok(rows.iter().map(|r| json!({ "client_id": r.get::<_, String>(0), "name": r.get::<_, String>(1), "status": r.get::<_, String>(2) })).collect())
        })
    }

    /// Takes the old owner's memberships in these workspaces away, all of them, app-scoped ones too. This is the one writer
    /// that may remove a last owner: a transfer names who takes over next, and the order is chosen so a failure part way
    /// leaves the old owner without access, never both with it.
    pub fn revoke_memberships_for_transfer(&self, workspaces: &[String], from: &str) -> Result<usize, RepositoryError> {
        let (workspaces, from) = (workspaces.to_vec(), from.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let n = tx.execute("DELETE FROM workspace_collaborators WHERE workspace_id = ANY($1) AND lower(eppn) = lower($2)", &[&workspaces, &from])?;
            crate::cache::bump(&mut tx, "workspaces")?;
            tx.commit()?;
            Ok(n as usize)
        })
    }

    /// Revokes the old owner's agent tokens that are limited to apps in these workspaces. Returns how many.
    pub fn revoke_agent_tokens_in(&self, workspaces: &[String], from: &str) -> Result<usize, RepositoryError> {
        let (workspaces, from) = (workspaces.to_vec(), from.to_string());
        self.with_client(move |c| {
            let n = c.execute(
                "UPDATE api_tokens SET revoked_at = NOW() \
                 WHERE lower(eppn) = lower($1) AND kind = 'agent' AND revoked_at IS NULL AND scope IS NOT NULL \
                   AND scope->'apps' ?| ARRAY(SELECT slug FROM app_manifests WHERE workspace_id = ANY($2))",
                &[&from, &workspaces],
            )?;
            Ok(n as usize)
        })
    }
}
