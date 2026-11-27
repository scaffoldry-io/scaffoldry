//! The per-table rules a record write enforces: the declared key (phase 2b), the unit column (phase 2c) and the audience fields
//! (phase 3b). All live on the `datasets` row, so one read serves all three. Every record write calls [`write`] once instead of reading each spec on
//! its own, which kept a 1,000-record job inside its time budget.

use postgres::GenericClient;
use serde_json::Value;

use crate::audience::{self, AudienceSpec};
use crate::keys::{self, KeySpec};
use crate::units::{self, UnitSpec};

/// What a table declares: its key, its unit column and its audience fields.
pub type Specs = (Option<KeySpec>, Option<UnitSpec>, Option<AudienceSpec>);

/// The key, the unit column and the audience fields a table declares, in one read.
pub fn for_table<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<Specs, postgres::Error> {
    let row = c.query_opt("SELECT key_spec, unit_spec, audience_spec FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(match row {
        Some(r) => (
            r.get::<_, Option<Value>>(0).and_then(|v| KeySpec::from_json(&v)),
            r.get::<_, Option<Value>>(1).and_then(|v| UnitSpec::from_json(&v)),
            r.get::<_, Option<Value>>(2).and_then(|v| AudienceSpec::from_json(&v)),
        ),
        None => (None, None, None),
    })
}

/// Writes a record's key, its unit column and its audience rows, in the caller's transaction, after one read of the table's rules. A
/// table that declares no audience fields adds no statement.
pub fn write<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, record_id: &str, data: &Value) -> Result<(), postgres::Error> {
    let (key, unit, aud) = for_table(tx, app_slug, table_id)?;
    keys::write_with(tx, key.as_ref(), app_slug, table_id, record_id, data)?;
    units::write_with(tx, unit.as_ref(), record_id, data)?;
    audience::write_with(tx, aud.as_ref(), app_slug, table_id, record_id, data)
}
