//! Registered outside apps (external apps phase 1). Every function reads or writes PostgreSQL. Nothing is cached.

use crate::repository::{PostgresRepository, RepositoryError};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ClientRow {
    pub client_id: String,
    pub name: String,
    pub operator: String,
    pub kind: String,
    pub redirect_uris: Vec<String>,
    pub ceiling: Value,
    pub status: String,
    pub proposed_by: String,
    pub proposed_at: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub disabled_by: Option<String>,
    pub disabled_at: Option<String>,
}

const COLUMNS: &str = "client_id, name, operator, kind, redirect_uris, ceiling, status, proposed_by, proposed_at::text, decided_by, decided_at::text, disabled_by, disabled_at::text";

fn row_of(r: &postgres::Row) -> ClientRow {
    ClientRow {
        client_id: r.get(0),
        name: r.get(1),
        operator: r.get(2),
        kind: r.get(3),
        redirect_uris: r.get(4),
        ceiling: r.get(5),
        status: r.get(6),
        proposed_by: r.get(7),
        proposed_at: r.get(8),
        decided_by: r.get(9),
        decided_at: r.get(10),
        disabled_by: r.get(11),
        disabled_at: r.get(12),
    }
}

impl PostgresRepository {
    pub fn insert_client(&self, c: &ClientRow) -> Result<(), RepositoryError> {
        let c = c.clone();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO app_clients (client_id, name, operator, kind, redirect_uris, ceiling, status, proposed_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, 'Pending', $7)",
                &[&c.client_id, &c.name, &c.operator, &c.kind, &c.redirect_uris, &c.ceiling, &c.proposed_by],
            )?;
            Ok(())
        })
    }

    pub fn get_client(&self, id: &str) -> Result<Option<ClientRow>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |client| Ok(client.query_opt(&format!("SELECT {COLUMNS} FROM app_clients WHERE client_id = $1"), &[&id])?.as_ref().map(row_of)))
    }

    pub fn list_clients(&self) -> Result<Vec<ClientRow>, RepositoryError> {
        self.with_client(move |client| Ok(client.query(&format!("SELECT {COLUMNS} FROM app_clients ORDER BY proposed_at DESC, client_id"), &[])?.iter().map(row_of).collect()))
    }

    /// Moves a `Pending` client to `Active` or `Rejected`. False when it was no longer pending. `secret_hash` is kept
    /// for a confidential client that is approved.
    pub fn decide_client(&self, id: &str, approve: bool, decider: &str, secret_hash: Option<&str>) -> Result<bool, RepositoryError> {
        let (id, decider, secret_hash) = (id.to_string(), decider.to_string(), secret_hash.map(str::to_string));
        let status = if approve { "Active" } else { "Rejected" };
        self.with_client(move |client| {
            let n = client.execute(
                "UPDATE app_clients SET status = $2, decided_by = $3, decided_at = NOW(), secret_hash = $4 WHERE client_id = $1 AND status = 'Pending'",
                &[&id, &status, &decider, &secret_hash],
            )?;
            Ok(n == 1)
        })
    }

    /// Moves an `Active` client to `Disabled`. False when it was not active.
    pub fn disable_client(&self, id: &str, by: &str) -> Result<bool, RepositoryError> {
        let (id, by) = (id.to_string(), by.to_string());
        self.with_client(move |client| {
            let n = client.execute("UPDATE app_clients SET status = 'Disabled', disabled_by = $2, disabled_at = NOW() WHERE client_id = $1 AND status = 'Active'", &[&id, &by])?;
            Ok(n == 1)
        })
    }
}
