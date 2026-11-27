//! Reads and writes of `data_labels`.

use postgres::GenericClient;

/// One stored label, keyed by app, table and field.
pub struct LabelRow {
    pub app_slug: String,
    pub table_id: String,
    pub field: String,
    pub ferpa_sensitive: bool,
    pub note: String,
    pub set_by: String,
    pub categories: Vec<String>,
}

/// Every label.
pub fn list<C: GenericClient>(c: &mut C) -> Result<Vec<LabelRow>, postgres::Error> {
    Ok(c.query("SELECT app_slug, table_id, field, ferpa_sensitive, note, set_by, categories FROM data_labels", &[])?
        .iter()
        .map(|r| LabelRow {
            app_slug: r.get(0),
            table_id: r.get(1),
            field: r.get(2),
            ferpa_sensitive: r.get(3),
            note: r.get(4),
            set_by: r.get(5),
            categories: r.get(6),
        })
        .collect())
}

/// Sets a field's sensitivity flag, with who set it and why.
pub fn set_sensitive<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, field: &str, sensitive: bool, note: &str, by: &str) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO data_labels (app_slug, table_id, field, ferpa_sensitive, note, set_by) VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (app_slug, table_id, field) DO UPDATE SET (ferpa_sensitive, note, set_by, set_at) = ($4, $5, $6, NOW())",
        &[&app_slug, &table_id, &field, &sensitive, &note, &by],
    )?;
    Ok(())
}

/// Removes a field's label.
pub fn remove<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, field: &str) -> Result<(), postgres::Error> {
    tx.execute("DELETE FROM data_labels WHERE app_slug = $1 AND table_id = $2 AND field = $3", &[&app_slug, &table_id, &field])?;
    Ok(())
}

/// Puts a category on a field's label. An existing label keeps its flag and gains the category.
pub fn add_category<C: GenericClient>(tx: &mut C, app_slug: &str, table_id: &str, field: &str, category: &str, note: &str, by: &str) -> Result<(), postgres::Error> {
    tx.execute(
        "INSERT INTO data_labels (app_slug, table_id, field, ferpa_sensitive, note, set_by, categories) VALUES ($1, $2, $3, false, $4, $5, ARRAY[$6]::text[]) \
         ON CONFLICT (app_slug, table_id, field) DO UPDATE SET \
            categories = (SELECT COALESCE(array_agg(DISTINCT x), '{}') FROM unnest(data_labels.categories || ARRAY[$6]::text[]) AS x), \
            note = $4, set_by = $5, set_at = NOW()",
        &[&app_slug, &table_id, &field, &note, &by, &category],
    )?;
    Ok(())
}

/// The floor of a dataset: the categories every row of it carries (data layer phase 3). Empty for a dataset with none, and for a
/// table that has no dataset row.
pub fn floor_of<C: GenericClient>(c: &mut C, app_slug: &str, table_id: &str) -> Result<Vec<String>, postgres::Error> {
    Ok(c.query_opt("SELECT label_floor FROM datasets WHERE app_slug = $1 AND table_id = $2", &[&app_slug, &table_id])?.map(|r| r.get(0)).unwrap_or_default())
}

/// Sets a dataset's floor, kept sorted and without repeats. The change is a governed one, and the caller writes its ledger entry
/// in the same transaction. Returns the floor it replaced, or `None` when the dataset does not exist.
pub fn set_floor<C: GenericClient>(tx: &mut C, dataset_id: &str, floor: &[String]) -> Result<Option<Vec<String>>, postgres::Error> {
    let Some(was) = tx.query_opt("SELECT label_floor FROM datasets WHERE id = $1 FOR UPDATE", &[&dataset_id])?.map(|r| r.get::<_, Vec<String>>(0)) else { return Ok(None) };
    let mut next = floor.to_vec();
    next.sort();
    next.dedup();
    tx.execute("UPDATE datasets SET label_floor = $2 WHERE id = $1", &[&dataset_id, &next])?;
    Ok(Some(was))
}
