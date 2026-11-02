//! Archives (lifecycle phase 2): the apps and workspaces that are read-only and hidden from the default lists. Every
//! instance holds the list. A writer bumps `archives` in its transaction, so the others reload it.

use crate::repository::{PostgresRepository, RepositoryError};
use std::collections::HashSet;

/// `("app", slug)` or `("workspace", id)`.
pub type ArchiveKey = (String, String);

impl PostgresRepository {
    /// Every archived item.
    pub fn list_archives(&self) -> Result<HashSet<ArchiveKey>, RepositoryError> {
        self.with_client(|client| Ok(client.query("SELECT kind, id FROM archives", &[])?.iter().map(|r| (r.get(0), r.get(1))).collect()))
    }

    /// Archives an item. `false` when it was archived already, and nothing changes.
    pub fn archive(&self, kind: &str, id: &str, reason: &str, by: &str) -> Result<bool, RepositoryError> {
        let (kind, id, reason, by) = (kind.to_string(), id.to_string(), reason.to_string(), by.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let n = tx.execute(
                "INSERT INTO archives (kind, id, reason, archived_by) VALUES ($1, $2, $3, $4) ON CONFLICT (kind, id) DO NOTHING",
                &[&kind, &id, &reason, &by],
            )?;
            if n > 0 {
                crate::cache::bump(&mut tx, "archives")?;
            }
            tx.commit()?;
            Ok(n > 0)
        })
    }

    /// Ends an archive. `false` when the item was not archived.
    pub fn unarchive(&self, kind: &str, id: &str) -> Result<bool, RepositoryError> {
        let (kind, id) = (kind.to_string(), id.to_string());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let n = tx.execute("DELETE FROM archives WHERE kind = $1 AND id = $2", &[&kind, &id])?;
            if n > 0 {
                crate::cache::bump(&mut tx, "archives")?;
            }
            tx.commit()?;
            Ok(n > 0)
        })
    }
}
