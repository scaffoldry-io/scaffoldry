//! The format of an app manifest. `validate_manifest` returns every problem it finds, not the
//! first, so an agent can fix them in one pass.

use crate::{formula_field_refs, indexed_fields, AppManifest, AppTable, FieldType, PageKind, MAX_INDEXED_FIELDS};
use std::collections::HashSet;

pub const MAX_TABLES: usize = 50;
pub const MAX_FIELDS_PER_TABLE: usize = 200;
pub const MAX_VIEWS: usize = 200;
pub const MAX_PAGES: usize = 50;
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

fn is_slug(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=63).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// `^[a-z][a-z0-9<sep>]{0,62}$`
fn is_name(s: &str, sep: u8) -> bool {
    let b = s.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == sep)
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c))
}

fn table_of<'a>(m: &'a AppManifest, id: &str) -> Option<&'a AppTable> {
    m.tables.iter().find(|t| t.id == id)
}

pub fn validate_manifest(m: &AppManifest) -> Result<(), Vec<String>> {
    let mut problems: Vec<String> = Vec::new();

    if !is_slug(&m.slug) {
        problems.push(format!("slug '{}' must be 2 to 63 characters of a-z, 0-9 and '-', starting with a letter or digit", m.slug));
    }
    if m.tables.len() > MAX_TABLES {
        problems.push(format!("an app may have {MAX_TABLES} tables, this one has {}", m.tables.len()));
    }
    if m.views.len() > MAX_VIEWS {
        problems.push(format!("an app may have {MAX_VIEWS} views, this one has {}", m.views.len()));
    }
    if m.pages.len() > MAX_PAGES {
        problems.push(format!("an app may have {MAX_PAGES} pages, this one has {}", m.pages.len()));
    }
    match serde_json::to_vec(m) {
        Ok(bytes) if bytes.len() > MAX_MANIFEST_BYTES => {
            problems.push(format!("the manifest is {} bytes, the limit is {MAX_MANIFEST_BYTES}", bytes.len()));
        }
        Ok(_) => {}
        Err(e) => problems.push(format!("the manifest cannot be serialized: {e}")),
    }

    let mut table_ids = HashSet::new();
    for table in &m.tables {
        if !table_ids.insert(table.id.as_str()) {
            problems.push(format!("table id '{}' is used twice", table.id));
        }
        if table.fields.len() > MAX_FIELDS_PER_TABLE {
            problems.push(format!("table '{}' may have {MAX_FIELDS_PER_TABLE} fields, it has {}", table.id, table.fields.len()));
        }
        let mut names = HashSet::new();
        for f in &table.fields {
            if !is_name(&f.name, b'_') {
                problems.push(format!("table '{}': field name '{}' must match ^[a-z][a-z0-9_]{{0,62}}$", table.id, f.name));
            }
            if !names.insert(f.name.as_str()) {
                problems.push(format!("table '{}': field name '{}' is used twice", table.id, f.name));
            }
        }
        let indexed = indexed_fields(m, &table.id);
        if indexed.len() > MAX_INDEXED_FIELDS {
            problems.push(format!("table '{}' indexes {} fields, the limit is {MAX_INDEXED_FIELDS}", table.id, indexed.len()));
        }
        for f in &table.fields {
            if f.indexed && f.field_type == FieldType::Lookup {
                problems.push(format!("table '{}': Lookup field '{}' cannot be indexed", table.id, f.name));
            }
        }
        for f in &table.fields {
            match f.field_type {
                FieldType::Formula => {
                    if let Some(expr) = &f.formula_expression {
                        match formula_field_refs(expr) {
                            None => problems.push(format!("table '{}': formula '{}' does not parse", table.id, f.name)),
                            Some(refs) => {
                                for r in refs {
                                    if !names.contains(r.as_str()) {
                                        problems.push(format!("table '{}': formula '{}' names the field '{r}', which does not exist", table.id, f.name));
                                    }
                                }
                            }
                        }
                    }
                }
                FieldType::Link => {
                    if f.indexed {
                        problems.push(format!("table '{}': Link field '{}' cannot be indexed, links have their own index", table.id, f.name));
                    }
                    if let Some(app) = f.link_target_app.as_deref() {
                        problems.push(format!("table '{}': Link field '{}' names the app '{app}', which is not a dataset app", table.id, f.name));
                        continue;
                    }
                    let Some(target_id) = f.link_target_table.as_deref() else {
                        problems.push(format!("table '{}': Link field '{}' names no target table", table.id, f.name));
                        continue;
                    };
                    let Some(target) = table_of(m, target_id) else {
                        problems.push(format!("table '{}': Link field '{}' points at the table '{target_id}', which does not exist", table.id, f.name));
                        continue;
                    };
                    if let Some(related) = f.link_related_field.as_deref() {
                        match target.fields.iter().find(|x| x.name == related) {
                            None => problems.push(format!("table '{}': Link field '{}' names the related field '{related}', which '{target_id}' does not have", table.id, f.name)),
                            Some(back) if back.field_type != FieldType::Link => {
                                problems.push(format!("table '{}': Link field '{}' names the related field '{related}', which is not a Link field", table.id, f.name))
                            }
                            Some(back) => {
                                let points_back = back.link_target_table.as_deref() == Some(table.id.as_str())
                                    && back.link_target_app.is_none()
                                    && back.link_related_field.as_deref() == Some(f.name.as_str());
                                if !points_back {
                                    problems.push(format!("table '{}': the related field '{related}' on '{target_id}' does not point back to '{}'", table.id, f.name));
                                }
                            }
                        }
                    }
                }
                FieldType::Lookup | FieldType::Count | FieldType::Rollup => {
                    let Some(target_id) = f.target_table_id.as_deref() else {
                        problems.push(format!("table '{}': {:?} field '{}' names no target table", table.id, f.field_type, f.name));
                        continue;
                    };
                    let Some(target) = table_of(m, target_id) else {
                        problems.push(format!("table '{}': field '{}' targets the table '{target_id}', which does not exist", table.id, f.name));
                        continue;
                    };
                    let related = m.relationships.iter().any(|r| {
                        (r.source_table_id == table.id && r.target_table_id == target.id)
                            || (r.target_table_id == table.id && r.source_table_id == target.id)
                    });
                    if !related {
                        problems.push(format!("table '{}': field '{}' reaches '{target_id}' through no relationship", table.id, f.name));
                    }
                    match f.target_display_field.as_deref() {
                        Some(tf) if !target.fields.iter().any(|x| x.name == tf) => {
                            problems.push(format!("table '{}': field '{}' targets the field '{tf}', which '{target_id}' does not have", table.id, f.name));
                        }
                        None if f.field_type != FieldType::Count => {
                            problems.push(format!("table '{}': field '{}' names no target field", table.id, f.name));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let mut view_ids = HashSet::new();
    for view in &m.views {
        view_ids.insert(view.id.as_str());
        if let Some(tid) = view.table_id.as_deref() {
            match table_of(m, tid) {
                None => problems.push(format!("view '{}' belongs to the table '{tid}', which does not exist", view.id)),
                Some(table) => {
                    for f in &view.fields {
                        if !table.fields.iter().any(|x| x.name == f.name) {
                            problems.push(format!("view '{}' lists the field '{}', which '{tid}' does not have", view.id, f.name));
                        }
                    }
                }
            }
        }
    }

    let mut page_ids = HashSet::new();
    for page in &m.pages {
        if !is_name(&page.id, b'-') {
            problems.push(format!("page id '{}' must match ^[a-z][a-z0-9-]{{0,62}}$", page.id));
        }
        if !page_ids.insert(page.id.as_str()) {
            problems.push(format!("page id '{}' is used twice", page.id));
        }
        match &page.kind {
            PageKind::View { view_id } => {
                if !view_ids.contains(view_id.as_str()) {
                    problems.push(format!("page '{}' shows the view '{view_id}', which does not exist", page.id));
                }
            }
            PageKind::Custom { source_sha256, grants } => {
                if !is_sha256(source_sha256) {
                    problems.push(format!("page '{}': source_sha256 must be 64 hex characters", page.id));
                }
                for g in grants {
                    let Some(table) = table_of(m, &g.table_id) else {
                        problems.push(format!("page '{}': a grant names the table '{}', which does not exist", page.id, g.table_id));
                        continue;
                    };
                    for name in g.read.iter().chain(g.write.iter()) {
                        if !table.fields.iter().any(|f| &f.name == name) {
                            problems.push(format!("page '{}': a grant names the field '{name}', which '{}' does not have", page.id, g.table_id));
                        }
                    }
                    for name in &g.write {
                        if table.fields.iter().any(|f| &f.name == name && f.field_type.is_computed()) {
                            problems.push(format!("page '{}': a grant writes the computed field '{name}'", page.id));
                        }
                        if !g.read.contains(name) {
                            problems.push(format!("page '{}': a grant writes '{name}' without reading it", page.id));
                        }
                    }
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}
