//! The format of an app manifest. `validate_manifest` returns every problem it finds, not the
//! first, so an agent can fix them in one pass.

use crate::{indexed_fields, validate_formula, AppManifest, AppTable, FieldType, PageKind, MAX_INDEXED_FIELDS};
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

/// The functions a `Rollup` can apply.
pub const ROLLUP_FUNCTIONS: [&str; 6] = ["sum", "avg", "min", "max", "count", "count_distinct"];

/// True for a field type whose value is stored as a number.
pub fn is_numeric(t: FieldType) -> bool {
    use FieldType::*;
    matches!(t, Number | Currency | Percent | Rating | Autonumber | Count | Rollup | Duration { .. })
}

/// Things that still validate but should change. A relationship joins by value, and a link joins by
/// identity (`links.md`).
pub fn manifest_warnings(m: &AppManifest) -> Vec<String> {
    m.relationships
        .iter()
        .map(|r| format!("relationship '{}' joins '{}' to '{}' by value. It is deprecated: use a Link field", r.id, r.source_table_id, r.target_table_id))
        .collect()
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
            if f.indexed && f.field_type == FieldType::LongText {
                problems.push(format!("table '{}': LongText field '{}' cannot be indexed", table.id, f.name));
            }
        }
        for f in &table.fields {
            match f.field_type {
                                FieldType::Duration { .. } => {
                    if let Some(ref df) = f.duration_format {
                        if !matches!(df.as_str(), "h:mm" | "h:mm:ss" | "d h" | "days") {
                            problems.push(format!("table '{}': field '{}' has invalid duration format '{}'", table.id, f.name, df));
                        }
                    }
                }
                FieldType::Select | FieldType::MultiSelect => {
                    let mut opt_ids = HashSet::new();
                    let mut opt_labels = HashSet::new();
                    for opt in &f.select_options {
                        if opt.id.is_empty() || !opt_ids.insert(opt.id.as_str()) {
                            problems.push(format!("table '{}': field '{}' has duplicate or empty select option id '{}'", table.id, f.name, opt.id));
                        }
                        if opt.label.is_empty() || !opt_labels.insert(opt.label.as_str()) {
                            problems.push(format!("table '{}': field '{}' has duplicate or empty select option label '{}'", table.id, f.name, opt.label));
                        }
                    }
                }
                FieldType::Formula => {
                    if let Some(expr) = &f.formula_expression {
                        match validate_formula(expr) {
                            Err(e) => problems.push(format!("table '{}': formula '{}' does not parse: {}", table.id, f.name, e.message)),
                            Ok(info) => {
                                for r in info.field_refs {
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
                    let kind = format!("{:?}", f.field_type);
                    let Some(via) = f.computed_via.as_deref() else {
                        problems.push(format!("table '{}': {kind} field '{}' names no link field in computed_via", table.id, f.name));
                        continue;
                    };
                    let Some(via_field) = table.fields.iter().find(|x| x.name == via) else {
                        problems.push(format!("table '{}': {kind} field '{}' goes through '{via}', which this table does not have", table.id, f.name));
                        continue;
                    };
                    if via_field.field_type != FieldType::Link {
                        problems.push(format!("table '{}': {kind} field '{}' goes through '{via}', which is not a Link field", table.id, f.name));
                        continue;
                    }
                    let Some(target) = via_field.link_target_table.as_deref().and_then(|t| table_of(m, t)) else { continue };
                    let target_field = match f.target_display_field.as_deref() {
                        Some(tf) => match target.fields.iter().find(|x| x.name == tf) {
                            Some(found) => Some(found),
                            None => {
                                problems.push(format!("table '{}': field '{}' targets the field '{tf}', which '{}' does not have", table.id, f.name, target.id));
                                continue;
                            }
                        },
                        None if f.field_type != FieldType::Count => {
                            problems.push(format!("table '{}': field '{}' names no target field", table.id, f.name));
                            continue;
                        }
                        None => None,
                    };
                    if f.field_type == FieldType::Rollup {
                        let function = f.rollup_function.as_deref().unwrap_or("sum");
                        if !ROLLUP_FUNCTIONS.contains(&function) {
                            problems.push(format!("table '{}': Rollup field '{}' uses the function '{function}', which is not one of {}", table.id, f.name, ROLLUP_FUNCTIONS.join(", ")));
                        } else if matches!(function, "sum" | "avg" | "min" | "max") {
                            if let Some(tf) = target_field {
                                if !is_numeric(tf.field_type) {
                                    problems.push(format!("table '{}': Rollup field '{}' applies {function} to '{}', which is not a number field", table.id, f.name, tf.name));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // A computed field must not read itself, through any number of others.
    if let Err(cycle) = crate::graph::build_graph(m) {
        problems.push(cycle.to_string());
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
                    for s in &view.sort_rules {
                        if let Some(f) = table.fields.iter().find(|x| x.name == s.field_name) {
                            if f.field_type == FieldType::LongText {
                                problems.push(format!("view '{}': LongText field '{}' cannot be used for sort", view.id, s.field_name));
                            }
                        }
                    }
                    if let Some(filters) = &view.filters {
                        if let Err(e) = crate::check_filter(filters) {
                            problems.push(format!("view '{}': {e}", view.id));
                        }
                        for c in crate::clauses_of(filters) {
                            if let Some(f) = table.fields.iter().find(|x| x.name == c.field_name) {
                                if f.field_type == FieldType::LongText {
                                    problems.push(format!("view '{}': LongText field '{}' cannot be used for filter", view.id, c.field_name));
                                }
                            }
                        }
                    }
                    let has = |name: &str| table.fields.iter().any(|x| x.name == name);
                    if view.group_by.len() > crate::MAX_GROUP_LEVELS {
                        problems.push(format!("view '{}' groups by {} fields, and {} is the most", view.id, view.group_by.len(), crate::MAX_GROUP_LEVELS));
                    }
                    for name in &view.group_by {
                        if !has(name) {
                            problems.push(format!("view '{}' groups by the field '{name}', which '{tid}' does not have", view.id));
                        }
                    }
                    for (name, function) in &view.column_summary {
                        if !has(name) {
                            problems.push(format!("view '{}' totals the field '{name}', which '{tid}' does not have", view.id));
                        }
                        if !crate::FOOTER_FUNCTIONS.contains(&function.as_str()) {
                            problems.push(format!("view '{}': '{function}' is not a footer choice. Use one of {}", view.id, crate::FOOTER_FUNCTIONS.join(", ")));
                        }
                    }
                    if view.aggregations.len() > crate::MAX_AGGREGATIONS {
                        problems.push(format!("view '{}' has {} aggregations, and {} is the most", view.id, view.aggregations.len(), crate::MAX_AGGREGATIONS));
                    }
                    for a in &view.aggregations {
                        if !crate::SUMMARY_FUNCTIONS.contains(&a.function.as_str()) {
                            problems.push(format!("view '{}': '{}' is not an aggregation. Use one of {}", view.id, a.function, crate::SUMMARY_FUNCTIONS.join(", ")));
                        }
                        match a.field.as_deref() {
                            None if a.function != "count" => problems.push(format!("view '{}': {} needs a field", view.id, a.function)),
                            Some(f) if !has(f) => problems.push(format!("view '{}' totals the field '{f}', which '{tid}' does not have", view.id)),
                            _ => {}
                        }
                    }
                    if view.color_rules.len() > crate::MAX_COLOR_RULES {
                        problems.push(format!("view '{}' has {} color rules, and {} is the most", view.id, view.color_rules.len(), crate::MAX_COLOR_RULES));
                    }
                    for rule in &view.color_rules {
                        if rule.label.trim().is_empty() {
                            problems.push(format!("view '{}': color rule '{}' needs a label, because color is never the only signal", view.id, rule.id));
                        }
                        if let Err(e) = crate::check_filter(&rule.when) {
                            problems.push(format!("view '{}': color rule '{}': {e}", view.id, rule.id));
                        }
                        for c in crate::clauses_of(&rule.when) {
                            if !has(&c.field_name) {
                                problems.push(format!("view '{}': color rule '{}' reads the field '{}', which '{tid}' does not have", view.id, rule.id, c.field_name));
                            }
                        }
                        match (&rule.target, rule.field.as_deref()) {
                            (crate::ColorTarget::Field, None) => problems.push(format!("view '{}': color rule '{}' colors a field and names none", view.id, rule.id)),
                            (crate::ColorTarget::Field, Some(f)) if !has(f) => {
                                problems.push(format!("view '{}': color rule '{}' colors the field '{f}', which '{tid}' does not have", view.id, rule.id));
                            }
                            _ => {}
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

    for rule in &m.access.rows {
        let Some(table) = table_of(m, &rule.table_id) else {
            problems.push(format!("access rule '{}': table '{}' does not exist", rule.id, rule.table_id));
            continue;
        };
        for f_name in rule.visible_when.fields() {
            let Some(f) = table.fields.iter().find(|field| field.name == f_name) else {
                problems.push(format!("table '{}': access rule '{}' names field '{f_name}', which does not exist", table.id, rule.id));
                continue;
            };
            if matches!(f.field_type, FieldType::Lookup | FieldType::Link | FieldType::Formula | FieldType::Count | FieldType::Rollup | FieldType::LongText) {
                problems.push(format!("table '{}': access rule '{}' uses non-indexable field '{f_name}'", table.id, rule.id));
            }
        }
        if rule.visible_when.node_count() > 20 {
            problems.push(format!("table '{}': access rule '{}' predicate has {} nodes, the limit is 20", table.id, rule.id, rule.visible_when.node_count()));
        }
        if rule.visible_when.depth() > 4 {
            problems.push(format!("table '{}': access rule '{}' predicate has depth {}, the limit is 4", table.id, rule.id, rule.visible_when.depth()));
        }
    }
    for rule in &m.access.columns {
        let Some(table) = table_of(m, &rule.table_id) else {
            problems.push(format!("column rule '{}': table '{}' does not exist", rule.id, rule.table_id));
            continue;
        };
        for f_name in &rule.fields {
            if !table.fields.iter().any(|f| &f.name == f_name) {
                problems.push(format!("table '{}': column rule '{}' names field '{f_name}', which does not exist", table.id, rule.id));
            }
        }
        if let Some(msg) = &rule.message {
            if msg.len() > 200 {
                problems.push(format!("table '{}': column rule '{}' message is {} characters, the limit is 200", table.id, rule.id, msg.len()));
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

pub fn detect_access_problems(m: &AppManifest) -> Vec<String> {
    let mut problems = Vec::new();
    for rule in &m.access.rows {
        if rule.visible_when.is_never_true() {
            problems.push(format!("rule '{}': never_true", rule.id));
            if rule.except_roles.is_empty() && rule.ops.contains(&scaffoldry_core::access::RowOp::Read) {
                problems.push(format!("table '{}': hides_all", rule.table_id));
            }
        }
        let Some(table) = table_of(m, &rule.table_id) else {
            problems.push(format!("rule '{}': field_missing", rule.id));
            continue;
        };
        for f_name in rule.visible_when.fields() {
            let Some(f) = table.fields.iter().find(|field| field.name == f_name) else {
                problems.push(format!("rule '{}': field_missing", rule.id));
                continue;
            };
            if matches!(f.field_type, FieldType::Lookup | FieldType::Link | FieldType::Formula | FieldType::Count | FieldType::Rollup | FieldType::LongText) {
                problems.push(format!("rule '{}': unsupported_field", rule.id));
            }
        }
    }
    for rule in &m.access.columns {
        let Some(table) = table_of(m, &rule.table_id) else {
            problems.push(format!("rule '{}': field_missing", rule.id));
            continue;
        };
        for f_name in &rule.fields {
            if !table.fields.iter().any(|f| &f.name == f_name) {
                problems.push(format!("rule '{}': field_missing", rule.id));
            }
        }
        if rule.mode == scaffoldry_core::access::ColumnMode::Hidden && rule.fields.len() >= table.fields.len() && rule.unless.is_empty() {
            problems.push(format!("table '{}': hides_all", rule.table_id));
        }
    }
    problems
}

