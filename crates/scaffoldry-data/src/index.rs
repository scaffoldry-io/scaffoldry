//! Reads and writes of `record_index`, the pivot index. The server decides which fields are indexed and what
//! their values are. This module only stores and removes rows.

use postgres::GenericClient;

/// One index row of one record: a field's value in the columns the index keeps.
pub struct IndexRow {
    pub field: String,
    pub ord: i16,
    pub num: Option<f64>,
    pub txt: Option<String>,
    pub ts: Option<String>,
}

/// Inserts rows for records of one table. A row already present is left as it is.
pub fn insert_rows<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, rows: &[(String, IndexRow)]) -> Result<(), postgres::Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let fields: Vec<&str> = rows.iter().map(|(_, r)| r.field.as_str()).collect();
    let ids: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
    let nums: Vec<Option<f64>> = rows.iter().map(|(_, r)| r.num).collect();
    let txts: Vec<Option<&str>> = rows.iter().map(|(_, r)| r.txt.as_deref()).collect();
    let tss: Vec<Option<&str>> = rows.iter().map(|(_, r)| r.ts.as_deref()).collect();
    let ords: Vec<i16> = rows.iter().map(|(_, r)| r.ord).collect();
    tx.execute(
        "INSERT INTO record_index (app_slug, table_id, field, record_id, num, txt, ts, ord) \
         SELECT $1, $2, u.f, u.r, u.n, u.t, u.s::text::timestamptz, u.o \
         FROM unnest($3::text[], $4::text[], $5::float8[], $6::text[], $7::text[], $8::int2[]) AS u(f, r, n, t, s, o) \
         ON CONFLICT (record_id, field, ord) DO NOTHING",
        &[&app_slug, &table_id, &fields, &ids, &nums, &txts, &tss, &ords],
    )?;
    Ok(())
}

/// Removes every index row of one record.
pub fn delete_for_record<C: GenericClient>(tx: &mut C, record_id: &str) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_index WHERE record_id = $1", &[&record_id])?;
    Ok(())
}

/// Removes every index row of the given fields of a table.
pub fn delete_for_fields<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, fields: &[String]) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2 AND field = ANY($3)", &[&app_slug, &table_id, &fields])?;
    Ok(())
}

/// Removes every index row of a table.
pub fn delete_for_table<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM record_index WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?;
    Ok(())
}
