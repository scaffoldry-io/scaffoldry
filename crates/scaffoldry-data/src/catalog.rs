//! What the catalog may say about a table's rows without saying one (data layer phase 2d): how many are live, roughly, and
//! the day the newest was added. It returns numbers and a date, never a value of a record.

use postgres::GenericClient;

/// Rows are counted up to this many. A table with more is reported as over it, so the catalog never scans a large table.
pub const COUNT_CAP: i64 = 100_000;

/// The size and freshness of the live rows of one table.
pub struct Footprint {
    /// The live rows, up to `COUNT_CAP + 1`.
    pub rows: i64,
    /// Whether the table holds more than `COUNT_CAP` rows. `rows` is then `COUNT_CAP + 1`.
    pub over_cap: bool,
    /// The UTC day the newest live row was added, as `YYYY-MM-DD`.
    pub latest_day: Option<String>,
}

/// The footprint of the live rows of a table. A record that names no table belongs to the app's first table, so the caller
/// passes the empty id as well for that one.
pub fn footprint<C: GenericClient>(c: &mut C, app_slug: &str, table_ids: &[String]) -> Result<Footprint, postgres::Error> {
    let rows: i64 = c
        .query_one(
            "SELECT count(*) FROM (SELECT 1 FROM dataset_records WHERE app_slug = $1 AND table_id = ANY($2) AND deleted_at IS NULL LIMIT $3) live",
            &[&app_slug, &table_ids, &(COUNT_CAP + 1)],
        )?
        .get(0);
    let latest_day = c
        .query_opt(
            "SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') FROM dataset_records \
             WHERE app_slug = $1 AND table_id = ANY($2) AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
            &[&app_slug, &table_ids],
        )?
        .map(|r| r.get(0));
    Ok(Footprint { rows, over_cap: rows > COUNT_CAP, latest_day })
}
