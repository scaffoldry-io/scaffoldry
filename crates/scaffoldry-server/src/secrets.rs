//! Integrations phase 1: the secrets store.
//!
//! A secret is a credential the server uses to reach something outside: a webhook's signing key, a feed's password. It
//! is stored as AES-256-GCM ciphertext under a master key that lives in a file the operator mounts, named by
//! `SCAFFOLDRY_MASTER_KEY_FILE`, and never in the database. Each write uses a fresh random 96-bit nonce, and the row's id
//! is the associated data, so a ciphertext copied to another row does not open.
//!
//! The API can write a secret and the server can use one. Nothing reads one back: `Secret` has no `Display`, its
//! `Debug` prints no contents, it is not `Serialize`, and its buffer is zeroed when it is dropped. The master key is read
//! from its file each time it is needed and zeroed after, so no request leaves it in memory.
//!
//! Something refers to a secret by naming it in a column called `secret` on its own table, as the webhook and
//! connection briefs do. `references` finds those, so a secret in use cannot be deleted.
//!
//! NIST SP 800-53 SC-12, SC-28 and IA-5(7); NIST SP 800-38D (GCM).

use crate::repository::{PostgresRepository, RepositoryError};
use crate::state::ServerState;
use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine;
use std::path::Path;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

/// What a secret's name may be.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() >= 2 && name.len() <= 64 && chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Why a secret could not be used. No variant carries a secret's value.
#[derive(Debug)]
pub enum SecretError {
    /// No master key file is configured, or it cannot be read.
    MasterKeyMissing(String),
    NotFound(String),
    /// The ciphertext did not open: the wrong key, or a row that was tampered with.
    Decrypt(String),
    Storage(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretError::MasterKeyMissing(m) | SecretError::NotFound(m) | SecretError::Decrypt(m) | SecretError::Storage(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SecretError {}

impl From<RepositoryError> for SecretError {
    fn from(e: RepositoryError) -> Self {
        SecretError::Storage(e.to_string())
    }
}

/// The master key: 32 bytes, zeroed when dropped.
pub struct MasterKey([u8; 32]);

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

impl MasterKey {
    /// Reads a key file: 32 random bytes in base64, as `openssl rand -base64 32` writes.
    pub fn from_file(path: &Path) -> Result<MasterKey, SecretError> {
        let text = Zeroizing::new(
            std::fs::read_to_string(path).map_err(|_| SecretError::MasterKeyMissing(format!("The master key file {} cannot be read", path.display())))?,
        );
        let bytes = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|_| SecretError::MasterKeyMissing(format!("The master key file {} does not hold base64", path.display())))?,
        );
        let key: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| SecretError::MasterKeyMissing(format!("The master key file {} must hold exactly 32 bytes", path.display())))?;
        Ok(MasterKey(key))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.0))
    }
}

/// A secret's value, for the server's own use. It prints as `Secret(..)` and is zeroed when dropped.
pub struct Secret(Zeroizing<Vec<u8>>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Secret {
    /// The bytes, to hand to the one thing that needs them. Never log or return them.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

/// Encrypts `value` for the row `id`: the ciphertext and a fresh 96-bit nonce.
pub fn seal(key: &MasterKey, id: Uuid, value: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = key
        .cipher()
        .encrypt(&nonce, Payload { msg: value, aad: id.as_bytes() })
        // AES-GCM encryption fails only for a message longer than about 64 GB.
        .expect("a secret encrypts");
    (ciphertext, nonce.to_vec())
}

/// Decrypts the row `id`. A different key, a different id, or a changed byte fails.
pub fn open(key: &MasterKey, id: Uuid, ciphertext: &[u8], nonce: &[u8]) -> Result<Secret, SecretError> {
    if nonce.len() != 12 {
        return Err(SecretError::Decrypt("A stored secret has a damaged nonce".to_string()));
    }
    key.cipher()
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: id.as_bytes() })
        .map(|v| Secret(Zeroizing::new(v)))
        .map_err(|_| SecretError::Decrypt("A stored secret does not open with this master key".to_string()))
}

/// The master key this server is configured with.
pub fn master_key(state: &ServerState) -> Result<MasterKey, SecretError> {
    let path = state
        .master_key_file
        .as_deref()
        .ok_or_else(|| SecretError::MasterKeyMissing("No master key is configured. Set SCAFFOLDRY_MASTER_KEY_FILE to a key file".to_string()))?;
    MasterKey::from_file(path)
}

/// A secret, by name, for the server's use.
pub fn get(state: &ServerState, name: &str) -> Result<Secret, SecretError> {
    let key = master_key(state)?;
    let owned = name.to_string();
    let row = state
        .repository
        .with_client(move |c| Ok(c.query_opt("SELECT id, ciphertext, nonce FROM secrets WHERE name = $1", &[&owned])?))?
        .ok_or_else(|| SecretError::NotFound(format!("No secret is named {name}")))?;
    open(&key, row.get(0), &row.get::<_, Vec<u8>>(1), &row.get::<_, Vec<u8>>(2))
}

/// A secret's listing: never its value.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Listed {
    pub name: String,
    pub created_by: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub rotated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub references: usize,
}

/// The tables that can name a secret: those with a text column called `secret`, other than the store itself.
fn referring_tables(c: &mut postgres::Client) -> Result<Vec<(String, bool)>, RepositoryError> {
    let rows = c.query(
        "SELECT t.table_name::text, EXISTS (SELECT 1 FROM information_schema.columns i WHERE i.table_schema = t.table_schema AND i.table_name = t.table_name AND i.column_name = 'id') \
         FROM information_schema.columns t \
         WHERE t.table_schema = current_schema() AND t.column_name = 'secret' AND t.table_name <> 'secrets' \
           AND t.data_type IN ('character varying', 'text', 'character') \
         ORDER BY 1",
        &[],
    )?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

fn quoted(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// What names the secret `name`, as `table id` (or `table` for a table with no `id` column).
pub fn references(repo: &PostgresRepository, name: &str) -> Result<Vec<String>, RepositoryError> {
    let name = name.to_string();
    repo.with_client(move |c| {
        let mut found = Vec::new();
        for (table, has_id) in referring_tables(c)? {
            let id = if has_id { "id::text" } else { "NULL::text" };
            let sql = format!("SELECT {id} FROM {} WHERE secret = $1 ORDER BY 1 LIMIT 50", quoted(&table));
            for r in c.query(&sql, &[&name])? {
                let id: Option<String> = r.get(0);
                found.push(match id {
                    Some(id) => format!("{table} {id}"),
                    None => table.clone(),
                });
            }
        }
        Ok(found)
    })
}

/// Every secret's listing, with how many things name it.
pub fn list(repo: &PostgresRepository) -> Result<Vec<Listed>, RepositoryError> {
    repo.with_client(|c| {
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for (table, _) in referring_tables(c)? {
            let sql = format!("SELECT secret, count(*) FROM {} WHERE secret IS NOT NULL GROUP BY secret", quoted(&table));
            for r in c.query(&sql, &[])? {
                *counts.entry(r.get(0)).or_default() += r.get::<_, i64>(1) as usize;
            }
        }
        Ok(c.query("SELECT name, created_by, created_at, rotated_at FROM secrets ORDER BY name", &[])?
            .iter()
            .map(|r| {
                let name: String = r.get(0);
                Listed { references: counts.get(&name).copied().unwrap_or(0), name, created_by: r.get(1), created_at: r.get(2), rotated_at: r.get(3) }
            })
            .collect())
    })
}

/// Creates or replaces a secret. True when it was new.
pub fn put(repo: &PostgresRepository, key: &MasterKey, name: &str, value: &[u8], by: &str) -> Result<bool, RepositoryError> {
    let existing: Option<Uuid> = {
        let name = name.to_string();
        repo.with_client(move |c| Ok(c.query_opt("SELECT id FROM secrets WHERE name = $1", &[&name])?.map(|r| r.get(0))))?
    };
    let id = existing.unwrap_or_else(Uuid::new_v4);
    let (ciphertext, nonce) = seal(key, id, value);
    let (name, by) = (name.to_string(), by.to_string());
    repo.with_client(move |c| {
        let n = c.execute(
            "INSERT INTO secrets (id, name, ciphertext, nonce, created_by) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (name) DO UPDATE SET ciphertext = EXCLUDED.ciphertext, nonce = EXCLUDED.nonce, rotated_at = NOW() \
             WHERE secrets.id = EXCLUDED.id",
            &[&id, &name, &ciphertext, &nonce, &by],
        )?;
        if n == 0 {
            return Err(RepositoryError::Conflict("The secret was replaced by someone else at the same time. Try again".to_string()));
        }
        Ok(existing.is_none())
    })
}

/// Deletes a secret. False when there was none.
pub fn delete(repo: &PostgresRepository, name: &str) -> Result<bool, RepositoryError> {
    let name = name.to_string();
    repo.with_client(move |c| Ok(c.execute("DELETE FROM secrets WHERE name = $1", &[&name])? > 0))
}

/// The check at boot. With no secrets stored, a missing key is fine: the secrets routes say it is missing. With
/// secrets stored, the key must be there and must open every one of them, or the server does not start.
pub fn boot_check(repo: &PostgresRepository, key_file: Option<&Path>) -> Result<(), String> {
    let rows = repo.with_client(|c| Ok(c.query("SELECT id, ciphertext, nonce FROM secrets", &[])?)).map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(());
    }
    let path = key_file.ok_or_else(|| {
        format!("{} secrets are stored, and SCAFFOLDRY_MASTER_KEY_FILE names no key file. Mount the master key file and set the variable", rows.len())
    })?;
    let key = MasterKey::from_file(path).map_err(|e| e.to_string())?;
    for r in &rows {
        open(&key, r.get(0), &r.get::<_, Vec<u8>>(1), &r.get::<_, Vec<u8>>(2))
            .map_err(|_| format!("The master key in {} does not open the stored secrets. It is the wrong key, or the rows were changed", path.display()))?;
    }
    Ok(())
}

/// Re-encrypts every secret from the old key to the new one, in one transaction. Nothing changes unless the old key
/// opens all of them. Returns how many were re-encrypted.
pub fn rotate_master_key(repo: &PostgresRepository, old_file: &Path, new_file: &Path) -> Result<usize, String> {
    let old = MasterKey::from_file(old_file).map_err(|e| e.to_string())?;
    let new = MasterKey::from_file(new_file).map_err(|e| e.to_string())?;
    repo.with_client(move |c| {
        let mut tx = c.transaction()?;
        let rows = tx.query("SELECT id, ciphertext, nonce FROM secrets ORDER BY id FOR UPDATE", &[])?;
        for r in &rows {
            let id: Uuid = r.get(0);
            let secret = open(&old, id, &r.get::<_, Vec<u8>>(1), &r.get::<_, Vec<u8>>(2))
                .map_err(|_| RepositoryError::Conflict("The old master key does not open every stored secret. Nothing was changed".to_string()))?;
            let (ciphertext, nonce) = seal(&new, id, secret.expose());
            tx.execute("UPDATE secrets SET ciphertext = $2, nonce = $3 WHERE id = $1", &[&id, &ciphertext, &nonce])?;
        }
        tx.commit()?;
        Ok(rows.len())
    })
    .map_err(|e| match e {
        RepositoryError::Conflict(m) => m,
        e => e.to_string(),
    })
}
