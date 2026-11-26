//! The per-table rules a record write enforces: the declared key (phase 2b) and the unit column (phase 2c). Both live on
//! the `datasets` row, so one read serves both. Every record write calls [`write`] once instead of reading each spec on
//! its own, which kept a 1,000-record job inside its time budget.

use postgres::GenericClient;
use serde_json::Value;

use crate::keys::{self, KeySpec};
use crate::units::{self, UnitSpec};

/// The key and the unit column a table declares, in one read.
pub fn for_table<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<(Option<KeySpec>, Option<UnitSpec>), postgres::Error> {
    let row = c.query_opt("SELECT key_spec, unit_spec FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(match row {
        Some(r) => (
            r.get::<_, Option<Value>>(0).and_then(|v| KeySpec::from_json(&v)),
            r.get::<_, Option<Value>>(1).and_then(|v| UnitSpec::from_json(&v)),
        ),
        None => (None, None),
    })
}

/// Writes a record's key and its unit column, in the caller's transaction, after one read of the table's rules.
pub fn write<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let (key, unit) = for_table(tx, app_slug, table_id)?;
    keys::write_with(tx, key.as_ref(), app_slug, table_id, record_id, data)?;
    units::write_with(tx, unit.as_ref(), record_id, data)
}
