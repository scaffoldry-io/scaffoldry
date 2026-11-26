//! Data layer phase 1: the stored id of each app table.
//!
//! A manifest write gives each of its tables a dataset id in the same transaction. Migration 0114 did the same for the
//! apps that already existed. The row holds identity only. A dataset that no table backs any longer is not listed.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_engine::AppManifest;

/// A table that has no dataset gets one. A table that has one keeps it, so two instances saving at once agree.
pub fn ensure_datasets_in_tx(tx: &mut postgres::Transaction<'_>, m: &AppManifest) -> Result<(), RepositoryError> {
    let tables: Vec<String> = m.tables.iter().map(|t| t.id.clone()).collect();
    tx.execute(
        "INSERT INTO datasets (id, app_slug, table_id) \
         SELECT 'ds_' || replace(gen_random_uuid()::text, '-', ''), $1, t FROM unnest($2::text[]) t \
         ON CONFLICT (app_slug, table_id) DO NOTHING",
        &[&m.slug, &tables],
    )?;
    sync_keys_in_tx(tx, m)?;
    sync_units_in_tx(tx, m)
}

/// The unit column each table names is the one its dataset holds (data layer phase 2c). A table whose column is new or
/// changed has the unit of every live row copied to the column. If a live row holds a value that is not a unit, the
/// transaction is left to roll back and the error lists the rows. A table that names what its dataset already holds is not
/// touched.
fn sync_units_in_tx(tx: &mut postgres::Transaction<'_>, m: &AppManifest) -> Result<(), RepositoryError> {
    use scaffoldry_data::units::{declare, spec_for, UnitSpec};
    for table in &m.tables {
        let want = table.unit_field.as_ref().and_then(|name| {
            let hierarchy = table.fields.iter().find(|f| &f.name == name)?.hierarchy.clone()?;
            Some(UnitSpec { field: name.clone(), hierarchy })
        });
        if spec_for(tx, &m.slug, &table.id)? == want {
            continue;
        }
        if let Err(ids) = declare(tx, &m.slug, &table.id, want.as_ref())? {
            let more = if ids.len() >= scaffoldry_data::units::MAX_LISTED { " (the list is cut here)" } else { "" };
            return Err(RepositoryError::UnitValues(format!(
                "The unit column cannot be set on the table '{}' while rows hold a value that is not a unit of the '{}' hierarchy. Fix or delete these rows, then approve again. Rows, by id: {}{more}",
                table.name,
                want.map(|w| w.hierarchy).unwrap_or_default(),
                ids.join(", ")
            )));
        }
    }
    Ok(())
}

/// The key each table declares is the key its dataset holds (data layer phase 2b). A table whose key is new or changed is
/// keyed again: the earlier keys go, and every live row gets one. If live rows share a key, the transaction is left to
/// roll back and the error lists the groups. A table that declares what its dataset already holds is not touched.
fn sync_keys_in_tx(tx: &mut postgres::Transaction<'_>, m: &AppManifest) -> Result<(), RepositoryError> {
    use scaffoldry_data::keys::{declare, spec_for, KeySpec};
    for table in &m.tables {
        let want = table.key.as_ref().map(|k| KeySpec { fields: k.fields.clone(), case_fold: k.case_fold });
        if spec_for(tx, &m.slug, &table.id)? == want {
            continue;
        }
        if let Err(dups) = declare(tx, &m.slug, &table.id, want.as_ref())? {
            return Err(RepositoryError::KeyDuplicates(duplicates_sentence(&table.name, &dups)));
        }
    }
    Ok(())
}

/// What the steward is told: how many groups, and the record ids of each. It names no value of the key.
fn duplicates_sentence(table: &str, d: &scaffoldry_data::keys::Duplicates) -> String {
    let groups: Vec<String> = d.groups.iter().enumerate().map(|(i, g)| format!("group {}: {}", i + 1, g.join(", "))).collect();
    let more = if d.more > 0 { format!(" and {} more records not listed", d.more) } else { String::new() };
    format!(
        "The key cannot be set on the table '{table}' while records share a key. {} group(s) of records share one. Change or delete the extra records, then approve again. Records that share a key, by id: {}{more}",
        d.groups.len(),
        groups.join("; ")
    )
}

impl PostgresRepository {
    /// Every stored dataset as (id, app slug, table id).
    pub fn list_datasets(&self) -> Result<Vec<(String, String, String)>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query("SELECT id, app_slug, table_id FROM datasets ORDER BY app_slug, table_id", &[])?
                .iter()
                .map(|r| (r.get(0), r.get(1), r.get(2)))
                .collect())
        })
    }

    /// The app slug and table id a dataset id names.
    pub fn dataset_by_id(&self, id: &str) -> Result<Option<(String, String)>, RepositoryError> {
        let id = id.to_string();
        self.with_client(move |c| Ok(c.query_opt("SELECT app_slug, table_id FROM datasets WHERE id = $1", &[&id])?.map(|r| (r.get(0), r.get(1)))))
    }
}
