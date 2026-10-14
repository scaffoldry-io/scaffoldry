//! The one way a server instance learns that another wrote (stateless server phase 1).
//!
//! PostgreSQL is the source of truth. A collection on the closed list below may be held in memory, and only through
//! this mechanism: a writer calls `bump` in the transaction that writes the collection, and every request calls
//! `ServerState::refresh`, which reads all the counters in one statement and reloads a collection whose counter it
//! does not hold. `LISTEN` and `NOTIFY` are not used. A missed notice would leave a cache stale with no error.

use crate::repository::RepositoryError;

/// The closed list of cached collections. It matches the rows `0053_cache_versions.sql` inserts.
pub const NAMES: [&str; 11] = [
    "settings",
    "labels",
    "position_types",
    "organizations",
    "workspaces",
    "guards",
    "datasets",
    "relationships",
    "manifests",
    "rules",
    "policy",
];

/// The place of a name on the list.
pub fn index_of(name: &str) -> Option<usize> {
    NAMES.iter().position(|n| *n == name)
}

/// Raises the counter of a collection by one, inside the writer's transaction. A name that is not a row is an error,
/// so a typo cannot leave a cache that never reloads.
pub fn bump(tx: &mut postgres::Transaction<'_>, name: &str) -> Result<(), RepositoryError> {
    let changed = tx.execute("UPDATE cache_versions SET version = version + 1 WHERE name = $1", &[&name])?;
    if changed == 0 {
        return Err(RepositoryError::NotFound(format!("cache_versions has no row named '{name}'")));
    }
    Ok(())
}
