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

/// What a code binds, once it has been taken.
#[derive(Debug, Clone)]
pub struct CodeRow {
    pub client_id: String,
    pub eppn: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub ceiling_digest: String,
    /// What an agent client's person chose: `read`, `propose` or `write`. None for an app client.
    pub access: Option<String>,
}

/// An agent client's sign-in by one person.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: uuid::Uuid,
    pub access: String,
    pub task: Option<String>,
    /// Not revoked, and not past its end.
    pub active: bool,
}

/// What a refresh token turned out to be.
pub enum RefreshUse {
    /// Valid, and now used. The session it belongs to, and how many seconds the session has left.
    Fresh { session_id: uuid::Uuid, client_id: String, eppn: String, access: String, seconds_left: i64 },
    /// Used before. Its session, and every token of it, has been revoked.
    Reused,
    /// Unknown, or its session has ended.
    Missing,
}

/// What taking a code found.
pub enum CodeUse {
    /// Valid, and now used.
    Fresh(CodeRow),
    /// Used before. Every token issued from it has been revoked.
    Reused,
    /// Unknown, or past its 60 seconds.
    Missing,
}

impl PostgresRepository {
    pub fn insert_code(&self, code_hash: &str, c: &CodeRow) -> Result<(), RepositoryError> {
        let (code_hash, c) = (code_hash.to_string(), c.clone());
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO oauth_codes (code_hash, client_id, eppn, redirect_uri, code_challenge, ceiling_digest, access, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NOW() + INTERVAL '60 seconds')",
                &[&code_hash, &c.client_id, &c.eppn, &c.redirect_uri, &c.code_challenge, &c.ceiling_digest, &c.access],
            )?;
            Ok(())
        })
    }

    /// Uses a code once. A second use revokes every token the first one issued.
    pub fn take_code(&self, code_hash: &str) -> Result<CodeUse, RepositoryError> {
        let code_hash = code_hash.to_string();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let fresh = tx.query_opt(
                "UPDATE oauth_codes SET used_at = NOW() WHERE code_hash = $1 AND used_at IS NULL AND expires_at > NOW() \
                 RETURNING client_id, eppn, redirect_uri, code_challenge, ceiling_digest::text, access",
                &[&code_hash],
            )?;
            let out = if let Some(r) = fresh {
                CodeUse::Fresh(CodeRow { client_id: r.get(0), eppn: r.get(1), redirect_uri: r.get(2), code_challenge: r.get(3), ceiling_digest: r.get(4), access: r.get(5) })
            } else if tx.query_opt("SELECT 1 FROM oauth_codes WHERE code_hash = $1 AND used_at IS NOT NULL", &[&code_hash])?.is_some() {
                tx.execute(
                    "UPDATE api_tokens SET revoked_at = NOW() WHERE revoked_at IS NULL AND token_hash IN (SELECT token_hash FROM oauth_tokens WHERE code_hash = $1)",
                    &[&code_hash],
                )?;
                CodeUse::Reused
            } else {
                CodeUse::Missing
            };
            tx.commit()?;
            Ok(out)
        })
    }

    pub fn insert_oauth_token(&self, token_hash: &str, client_id: &str, eppn: &str, code_hash: &str, session: Option<uuid::Uuid>) -> Result<(), RepositoryError> {
        let (token_hash, client_id, eppn, code_hash) = (token_hash.to_string(), client_id.to_string(), eppn.to_string(), code_hash.to_string());
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO oauth_tokens (token_hash, client_id, eppn, code_hash, session_id) VALUES ($1, $2, $3, $4, $5)",
                &[&token_hash, &client_id, &eppn, &code_hash, &session],
            )?;
            Ok(())
        })
    }

    /// The client a `client` token came from, and its session when it came from an agent client. None when the token
    /// has no client.
    pub fn client_of_token(&self, token_hash: &str) -> Result<Option<(ClientRow, Option<SessionRow>)>, RepositoryError> {
        let token_hash = token_hash.to_string();
        self.with_client(move |client| {
            let columns: Vec<String> = COLUMNS.split(", ").map(|c| format!("c.{c}")).collect();
            let sql = format!(
                "SELECT {}, s.session_id, s.access, s.task, (s.revoked_at IS NULL AND s.expires_at > NOW()) \
                 FROM oauth_tokens t JOIN app_clients c ON c.client_id = t.client_id LEFT JOIN oauth_sessions s ON s.session_id = t.session_id \
                 WHERE t.token_hash = $1",
                columns.join(", ")
            );
            Ok(client.query_opt(&sql, &[&token_hash])?.map(|r| {
                let n = COLUMNS.split(", ").count();
                let session = r.get::<_, Option<uuid::Uuid>>(n).map(|session_id| SessionRow {
                    session_id,
                    access: r.get(n + 1),
                    task: r.get(n + 2),
                    active: r.get::<_, Option<bool>>(n + 3).unwrap_or(false),
                });
                (row_of(&r), session)
            }))
        })
    }

    /// Revokes one token, if it came from this client.
    pub fn revoke_client_token(&self, token_hash: &str, client_id: &str) -> Result<bool, RepositoryError> {
        let (token_hash, client_id) = (token_hash.to_string(), client_id.to_string());
        self.with_client(move |client| {
            let n = client.execute(
                "UPDATE api_tokens SET revoked_at = NOW() WHERE revoked_at IS NULL AND token_hash = $1 AND token_hash IN (SELECT token_hash FROM oauth_tokens WHERE client_id = $2)",
                &[&token_hash, &client_id],
            )?;
            Ok(n == 1)
        })
    }

    pub fn secret_hash_of_client(&self, client_id: &str) -> Result<Option<String>, RepositoryError> {
        let client_id = client_id.to_string();
        self.with_client(move |client| Ok(client.query_opt("SELECT secret_hash FROM app_clients WHERE client_id = $1", &[&client_id])?.and_then(|r| r.get::<_, Option<String>>(0))))
    }

    pub fn consent_digest(&self, eppn: &str, client_id: &str) -> Result<Option<String>, RepositoryError> {
        let (eppn, client_id) = (eppn.to_string(), client_id.to_string());
        self.with_client(move |client| Ok(client.query_opt("SELECT ceiling_digest::text FROM oauth_consents WHERE eppn = $1 AND client_id = $2", &[&eppn, &client_id])?.map(|r| r.get(0))))
    }

    pub fn upsert_consent(&self, eppn: &str, client_id: &str, digest: &str) -> Result<(), RepositoryError> {
        let (eppn, client_id, digest) = (eppn.to_string(), client_id.to_string(), digest.to_string());
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO oauth_consents (eppn, client_id, ceiling_digest) VALUES ($1, $2, $3) \
                 ON CONFLICT (eppn, client_id) DO UPDATE SET ceiling_digest = EXCLUDED.ceiling_digest, granted_at = NOW()",
                &[&eppn, &client_id, &digest],
            )?;
            Ok(())
        })
    }

    /// The clients a person has agreed to, with when.
    pub fn consents_of(&self, eppn: &str) -> Result<Vec<(String, String)>, RepositoryError> {
        let eppn = eppn.to_string();
        self.with_client(move |client| {
            Ok(client
                .query("SELECT client_id, granted_at::text FROM oauth_consents WHERE eppn = $1 ORDER BY granted_at DESC", &[&eppn])?
                .iter()
                .map(|r| (r.get(0), r.get(1)))
                .collect())
        })
    }

    /// Forgets a person's consent to a client and revokes the tokens they issued to it. False when there was nothing.
    pub fn disconnect_client(&self, eppn: &str, client_id: &str) -> Result<bool, RepositoryError> {
        let (eppn, client_id) = (eppn.to_string(), client_id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let consents = tx.execute("DELETE FROM oauth_consents WHERE eppn = $1 AND client_id = $2", &[&eppn, &client_id])?;
            let tokens = tx.execute(
                "UPDATE api_tokens SET revoked_at = NOW() WHERE revoked_at IS NULL AND token_hash IN (SELECT token_hash FROM oauth_tokens WHERE client_id = $2 AND eppn = $1)",
                &[&eppn, &client_id],
            )?;
            let sessions = tx.execute("UPDATE oauth_sessions SET revoked_at = NOW() WHERE revoked_at IS NULL AND client_id = $2 AND eppn = $1", &[&eppn, &client_id])?;
            tx.commit()?;
            Ok(consents + tokens + sessions > 0)
        })
    }

    /// The redirect addresses of every active client.
    pub fn active_client_redirects(&self) -> Result<Vec<String>, RepositoryError> {
        self.with_client(move |client| Ok(client.query("SELECT unnest(redirect_uris) FROM app_clients WHERE status = 'Active'", &[])?.iter().map(|r| r.get(0)).collect()))
    }

    /// Starts a session. It ends `hours` from now.
    pub fn create_session(&self, client_id: &str, eppn: &str, access: &str, hours: i64) -> Result<uuid::Uuid, RepositoryError> {
        let (client_id, eppn, access) = (client_id.to_string(), eppn.to_string(), access.to_string());
        let id = uuid::Uuid::new_v4();
        self.with_client(move |client| {
            client.execute(
                "INSERT INTO oauth_sessions (session_id, client_id, eppn, access, expires_at) VALUES ($1, $2, $3, $4, NOW() + make_interval(hours => $5::int))",
                &[&id, &client_id, &eppn, &access, &(hours as i32)],
            )?;
            Ok(id)
        })
    }

    /// Seconds a session has left, or None when it was revoked or has ended.
    pub fn session_seconds_left(&self, session: uuid::Uuid) -> Result<Option<i64>, RepositoryError> {
        self.with_client(move |client| {
            Ok(client
                .query_opt("SELECT EXTRACT(EPOCH FROM (expires_at - NOW()))::float8 FROM oauth_sessions WHERE session_id = $1 AND revoked_at IS NULL AND expires_at > NOW()", &[&session])?
                .map(|r| r.get::<_, f64>(0) as i64))
        })
    }

    pub fn insert_refresh(&self, refresh_hash: &str, session: uuid::Uuid) -> Result<(), RepositoryError> {
        let refresh_hash = refresh_hash.to_string();
        self.with_client(move |client| {
            client.execute("INSERT INTO oauth_refresh (refresh_hash, session_id) VALUES ($1, $2)", &[&refresh_hash, &session])?;
            Ok(())
        })
    }

    /// Uses a refresh token once. A second use ends the session and revokes every token it issued (RFC 9700 4.14).
    pub fn take_refresh(&self, refresh_hash: &str) -> Result<RefreshUse, RepositoryError> {
        let refresh_hash = refresh_hash.to_string();
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let fresh = tx.query_opt("UPDATE oauth_refresh SET used_at = NOW() WHERE refresh_hash = $1 AND used_at IS NULL RETURNING session_id", &[&refresh_hash])?;
            let out = if let Some(r) = fresh {
                let session: uuid::Uuid = r.get(0);
                match tx.query_opt(
                    "SELECT client_id, eppn, access, EXTRACT(EPOCH FROM (expires_at - NOW()))::float8 FROM oauth_sessions WHERE session_id = $1 AND revoked_at IS NULL AND expires_at > NOW()",
                    &[&session],
                )? {
                    Some(s) => RefreshUse::Fresh { session_id: session, client_id: s.get(0), eppn: s.get(1), access: s.get(2), seconds_left: s.get::<_, f64>(3) as i64 },
                    None => RefreshUse::Missing,
                }
            } else if let Some(r) = tx.query_opt("SELECT session_id FROM oauth_refresh WHERE refresh_hash = $1 AND used_at IS NOT NULL", &[&refresh_hash])? {
                let session: uuid::Uuid = r.get(0);
                revoke_session_in(&mut tx, session)?;
                RefreshUse::Reused
            } else {
                RefreshUse::Missing
            };
            tx.commit()?;
            Ok(out)
        })
    }

    /// Ends a session now.
    pub fn end_session(&self, session: uuid::Uuid) -> Result<(), RepositoryError> {
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            revoke_session_in(&mut tx, session)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Records what the tool says it is doing. False when the session is not active.
    pub fn set_session_task(&self, session: uuid::Uuid, task: &str) -> Result<bool, RepositoryError> {
        let task = task.to_string();
        self.with_client(move |client| {
            Ok(client.execute("UPDATE oauth_sessions SET task = $2 WHERE session_id = $1 AND revoked_at IS NULL AND expires_at > NOW()", &[&session, &task])? == 1)
        })
    }
}

/// Ends a session: its tokens stop and it takes no more refresh tokens.
fn revoke_session_in(tx: &mut postgres::Transaction<'_>, session: uuid::Uuid) -> Result<(), postgres::Error> {
    tx.execute("UPDATE oauth_sessions SET revoked_at = NOW() WHERE session_id = $1 AND revoked_at IS NULL", &[&session])?;
    tx.execute("UPDATE api_tokens SET revoked_at = NOW() WHERE revoked_at IS NULL AND token_hash IN (SELECT token_hash FROM oauth_tokens WHERE session_id = $1)", &[&session])?;
    Ok(())
}
