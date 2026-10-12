//! Views, apart from the manifest (views phase 1).
//!
//! A view is presentation. It lives in `app_views`, and a save that sorts or filters on a field the
//! table does not yet index also starts that index, in the same transaction.

use crate::repository::{PostgresRepository, RepositoryError};
use scaffoldry_engine::{AppManifest, AppView, FieldSpec, PageKind, ViewType};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub const KINDS: [&str; 7] = ["grid", "kanban", "calendar", "gallery", "form", "summary", "dashboard"];
pub const OWNERSHIPS: [&str; 3] = ["collaborative", "personal", "locked"];

/// A view as stored.
#[derive(Debug, Clone, Serialize)]
pub struct StoredView {
    pub id: Uuid,
    pub app_slug: String,
    pub table_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub ownership: String,
    pub owner: Option<String>,
    pub config: Value,
    pub version: i32,
    pub created_by: String,
    pub created_at: String,
    pub updated_by: String,
    pub updated_at: String,
}

const COLUMNS: &str = "id, app_slug, table_id, name, kind, ownership, owner, config, version, created_by, \
    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), updated_by, \
    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')";

fn row_of(r: &postgres::Row) -> StoredView {
    StoredView {
        id: r.get(0),
        app_slug: r.get(1),
        table_id: r.get(2),
        name: r.get(3),
        kind: r.get(4),
        ownership: r.get(5),
        owner: r.get(6),
        config: r.get(7),
        version: r.get(8),
        created_by: r.get(9),
        created_at: r.get(10),
        updated_by: r.get(11),
        updated_at: r.get(12),
    }
}

/// The kind a manifest view type is stored as.
pub fn kind_of(t: ViewType) -> &'static str {
    match t {
        ViewType::Table | ViewType::Grid | ViewType::Detail => "grid",
        ViewType::Kanban => "kanban",
        ViewType::Calendar => "calendar",
        ViewType::Gallery => "gallery",
        ViewType::Form => "form",
        ViewType::Dashboard => "dashboard",
    }
}

fn view_type_of(kind: &str) -> ViewType {
    match kind {
        "kanban" => ViewType::Kanban,
        "calendar" => ViewType::Calendar,
        "gallery" => ViewType::Gallery,
        "form" => ViewType::Form,
        "dashboard" | "summary" => ViewType::Dashboard,
        _ => ViewType::Grid,
    }
}

/// The view as the engine knows it: the config, with the id, the name and the table put back. `None` when
/// the config is not a view.
pub fn materialize(v: &StoredView) -> Option<AppView> {
    let mut obj = v.config.as_object()?.clone();
    obj.insert("id".to_string(), json!(v.id.to_string()));
    obj.insert("title".to_string(), json!(v.name));
    obj.insert("table_id".to_string(), json!(v.table_id));
    if !obj.contains_key("view_type") {
        obj.insert("view_type".to_string(), serde_json::to_value(view_type_of(&v.kind)).ok()?);
    }
    serde_json::from_value(Value::Object(obj)).ok()
}

/// What a manifest view keeps in `config`: everything but the id, the title and the table.
fn config_of(view: &AppView) -> Value {
    let mut v = serde_json::to_value(view).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        for key in ["id", "title", "table_id"] {
            obj.remove(key);
        }
    }
    v
}

/// What a save changes in the table's index. `left` loses its rows. `joined` gets a build.
#[derive(Clone)]
pub struct IndexChange {
    pub table_id: String,
    pub left: Vec<String>,
    pub joined: Vec<FieldSpec>,
}

pub struct NewView {
    pub id: Uuid,
    pub app_slug: String,
    pub table_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub ownership: String,
    pub owner: Option<String>,
    pub config: Value,
    pub actor: String,
}

pub enum Saved {
    Done(Box<StoredView>),
    /// The version named is not the stored one. This is the stored one.
    Stale(i32),
    Missing,
}

/// A manifest whose views moved.
pub struct Migrated {
    /// The stored manifest as it is now: no views, and its pages pointing at the new ids.
    pub manifest: AppManifest,
    pub views: Vec<StoredView>,
}

fn sync(tx: &mut postgres::Transaction<'_>, app_slug: &str, change: &Option<IndexChange>, actor: &str) -> Result<(), RepositoryError> {
    if let Some(c) = change {
        crate::repository_index::sync_index(tx, app_slug, &c.table_id, &c.left, &c.joined, actor)?;
    }
    Ok(())
}

impl PostgresRepository {
    /// The views of one app that are not deleted.
    pub fn list_app_views(&self, app_slug: &str) -> Result<Vec<StoredView>, RepositoryError> {
        let slug = app_slug.to_string();
        self.with_client(move |c| {
            Ok(c.query(&format!("SELECT {COLUMNS} FROM app_views WHERE app_slug = $1 AND deleted_at IS NULL ORDER BY created_at, id"), &[&slug])?
                .iter()
                .map(row_of)
                .collect())
        })
    }

    /// Every view that is not deleted. The server reads them once, at start.
    pub fn list_all_app_views(&self) -> Result<Vec<StoredView>, RepositoryError> {
        self.with_client(|c| {
            Ok(c.query(&format!("SELECT {COLUMNS} FROM app_views WHERE deleted_at IS NULL ORDER BY created_at, id"), &[])?
                .iter()
                .map(row_of)
                .collect())
        })
    }

    pub fn get_app_view(&self, app_slug: &str, id: Uuid) -> Result<Option<StoredView>, RepositoryError> {
        let slug = app_slug.to_string();
        self.with_client(move |c| {
            Ok(c.query_opt(&format!("SELECT {COLUMNS} FROM app_views WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL"), &[&slug, &id])?.map(|r| row_of(&r)))
        })
    }

    /// Writes a view, and the index it needs, in one transaction.
    pub fn create_app_view(&self, new: NewView, change: Option<IndexChange>) -> Result<StoredView, RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let row = tx.query_one(
                &format!(
                    "INSERT INTO app_views (id, app_slug, table_id, name, kind, ownership, owner, config, created_by, updated_by) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) RETURNING {COLUMNS}"
                ),
                &[&new.id, &new.app_slug, &new.table_id, &new.name, &new.kind, &new.ownership, &new.owner, &new.config, &new.actor],
            )?;
            sync(&mut tx, &new.app_slug, &change, &new.actor)?;
            tx.commit()?;
            Ok(row_of(&row))
        })
    }

    /// Changes a view that is at `expected` version.
    #[allow(clippy::too_many_arguments)]
    pub fn update_app_view(
        &self,
        app_slug: &str,
        id: Uuid,
        name: String,
        config: Value,
        expected: i32,
        actor: &str,
        change: Option<IndexChange>,
    ) -> Result<Saved, RepositoryError> {
        let (slug, actor) = (app_slug.to_string(), actor.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let updated = tx.query_opt(
                &format!(
                    "UPDATE app_views SET name = $3, config = $4, version = version + 1, updated_by = $5, updated_at = NOW() \
                     WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL AND version = $6 RETURNING {COLUMNS}"
                ),
                &[&slug, &id, &name, &config, &actor, &expected],
            )?;
            let Some(row) = updated else {
                let current = tx.query_opt("SELECT version FROM app_views WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL", &[&slug, &id])?;
                return Ok(match current {
                    Some(r) => Saved::Stale(r.get(0)),
                    None => Saved::Missing,
                });
            };
            sync(&mut tx, &slug, &change, &actor)?;
            tx.commit()?;
            Ok(Saved::Done(Box::new(row_of(&row))))
        })
    }

    /// Soft deletes a view. `false` when there was none.
    pub fn delete_app_view(&self, app_slug: &str, id: Uuid, actor: &str, change: Option<IndexChange>) -> Result<bool, RepositoryError> {
        let (slug, actor) = (app_slug.to_string(), actor.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let n = tx.execute(
                "UPDATE app_views SET deleted_at = NOW(), updated_by = $3, updated_at = NOW() WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL",
                &[&slug, &id, &actor],
            )?;
            if n == 0 {
                return Ok(false);
            }
            sync(&mut tx, &slug, &change, &actor)?;
            tx.commit()?;
            Ok(true)
        })
    }

    /// Moves a stored manifest's views into `app_views`, once. Each becomes a collaborative view that keeps
    /// its id when that is a UUID and gets a new one otherwise. The manifest keeps none, and its pages point at
    /// the new ids. `None` when the manifest holds no views, which is also what a second caller gets.
    pub fn migrate_manifest_views(&self, slug: &str) -> Result<Option<Migrated>, RepositoryError> {
        let slug = slug.to_string();
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let Some(row) = tx.query_opt("SELECT manifest FROM app_manifests WHERE slug = $1 FOR UPDATE", &[&slug])? else { return Ok(None) };
            let mut manifest: AppManifest = serde_json::from_value(row.get::<_, Value>(0))?;
            if manifest.views.is_empty() {
                return Ok(None);
            }
            let mut moved: HashMap<String, Uuid> = HashMap::new();
            let mut ids: Vec<Uuid> = Vec::new();
            for view in &manifest.views {
                let id = Uuid::parse_str(&view.id).unwrap_or_else(|_| Uuid::new_v4());
                moved.insert(view.id.clone(), id);
                ids.push(id);
                tx.execute(
                    "INSERT INTO app_views (id, app_slug, table_id, name, kind, ownership, config, created_by, updated_by) \
                     VALUES ($1, $2, $3, $4, $5, 'collaborative', $6, 'system@scaffoldry.local', 'system@scaffoldry.local') ON CONFLICT (id) DO NOTHING",
                    &[&id, &slug, &view.table_id, &view.title, &kind_of(view.view_type), &config_of(view)],
                )?;
            }
            for page in &mut manifest.pages {
                if let PageKind::View { view_id } = &mut page.kind {
                    if let Some(id) = moved.get(view_id) {
                        *view_id = id.to_string();
                    }
                }
            }
            manifest.views.clear();
            tx.execute("UPDATE app_manifests SET manifest = $2 WHERE slug = $1", &[&slug, &serde_json::to_value(&manifest)?])?;
            let views = tx
                .query(&format!("SELECT {COLUMNS} FROM app_views WHERE app_slug = $1 AND id = ANY($2) AND deleted_at IS NULL ORDER BY created_at, id"), &[&slug, &ids])?
                .iter()
                .map(row_of)
                .collect();
            tx.commit()?;
            Ok(Some(Migrated { manifest, views }))
        })
    }
}

/// A manifest with the stored views of its app added, as the engine runs it. A stored view replaces a
/// manifest view that has its id. The stored manifest itself holds none.
pub fn merged(mut m: AppManifest, views: &[StoredView]) -> AppManifest {
    let ids: std::collections::HashSet<String> = views.iter().map(|v| v.id.to_string()).collect();
    m.views.retain(|v| !ids.contains(&v.id));
    m.views.extend(views.iter().filter_map(materialize));
    m
}
