//! Attachments phase 1: the rows behind a file. The bytes are on disk. See `files.rs`.

use crate::repository::{PostgresRepository, RepositoryError};
use uuid::Uuid;

/// One upload, with the blob it points at.
#[derive(Debug, Clone)]
pub struct FileRef {
    pub id: Uuid,
    pub sha256: String,
    pub size: i64,
    pub media_type: String,
    pub app_slug: String,
    pub table_id: Option<String>,
    pub record_id: Option<String>,
    pub field: Option<String>,
    pub name: String,
    pub uploaded_by: String,
    pub attached: bool,
    pub deleted: bool,
}

const REF_COLUMNS: &str = "r.id, r.sha256, f.size, f.media_type, r.app_slug, r.table_id, r.record_id, r.field, r.name, r.uploaded_by, \
    r.attached_at IS NOT NULL, r.deleted_at IS NOT NULL";

fn ref_of(r: &postgres::Row) -> FileRef {
    FileRef {
        id: r.get(0),
        sha256: r.get::<_, String>(1).trim().to_string(),
        size: r.get(2),
        media_type: r.get(3),
        app_slug: r.get(4),
        table_id: r.get(5),
        record_id: r.get(6),
        field: r.get(7),
        name: r.get(8),
        uploaded_by: r.get(9),
        attached: r.get(10),
        deleted: r.get(11),
    }
}

/// An upload to record.
pub struct NewRef<'a> {
    pub id: Uuid,
    pub sha256: &'a str,
    pub size: i64,
    pub media_type: &'a str,
    pub app_slug: &'a str,
    pub name: &'a str,
    pub uploaded_by: &'a str,
}

impl PostgresRepository {
    /// Bytes the files of an app hold, counting each blob once.
    pub fn app_file_bytes(&self, app_slug: &str) -> Result<i64, RepositoryError> {
        let app = app_slug.to_string();
        self.with_client(move |c| {
            let row = c.query_one(
                "SELECT COALESCE(SUM(f.size), 0)::BIGINT FROM files f WHERE f.sha256 IN \
                 (SELECT sha256 FROM file_refs WHERE app_slug = $1 AND deleted_at IS NULL)",
                &[&app],
            )?;
            Ok(row.get(0))
        })
    }

    /// Records an upload: the blob if it is new, and a ref to it.
    pub fn insert_file_ref(&self, new: NewRef<'_>) -> Result<(), RepositoryError> {
        let (id, size) = (new.id, new.size);
        let (sha, media, app, name, by) = (new.sha256.to_string(), new.media_type.to_string(), new.app_slug.to_string(), new.name.to_string(), new.uploaded_by.to_string());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute("INSERT INTO files (sha256, size, media_type) VALUES ($1, $2, $3) ON CONFLICT (sha256) DO NOTHING", &[&sha, &size, &media])?;
            tx.execute(
                "INSERT INTO file_refs (id, sha256, app_slug, name, uploaded_by) VALUES ($1, $2, $3, $4, $5)",
                &[&id, &sha, &app, &name, &by],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// The refs with these ids that belong to the app, live or not.
    pub fn file_refs(&self, app_slug: &str, ids: &[Uuid]) -> Result<Vec<FileRef>, RepositoryError> {
        let (app, ids) = (app_slug.to_string(), ids.to_vec());
        self.with_client(move |c| {
            let sql = format!("SELECT {REF_COLUMNS} FROM file_refs r JOIN files f ON f.sha256 = r.sha256 WHERE r.app_slug = $1 AND r.id = ANY($2)");
            Ok(c.query(&sql, &[&app, &ids])?.iter().map(ref_of).collect())
        })
    }

    /// One live ref of a record, by id.
    pub fn record_file_ref(&self, app_slug: &str, record_id: &str, id: Uuid) -> Result<Option<FileRef>, RepositoryError> {
        let (app, rec) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |c| {
            let sql = format!(
                "SELECT {REF_COLUMNS} FROM file_refs r JOIN files f ON f.sha256 = r.sha256 \
                 WHERE r.id = $1 AND r.app_slug = $2 AND r.record_id = $3 AND r.deleted_at IS NULL"
            );
            Ok(c.query_opt(&sql, &[&id, &app, &rec])?.as_ref().map(ref_of))
        })
    }

    /// Ties the refs to a record field, and lets go of the refs the field held before and no longer lists.
    pub fn attach_file_refs(&self, app_slug: &str, table_id: &str, record_id: &str, field: &str, keep: &[Uuid]) -> Result<(), RepositoryError> {
        let (app, table, rec, field, keep) = (app_slug.to_string(), table_id.to_string(), record_id.to_string(), field.to_string(), keep.to_vec());
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            tx.execute(
                "UPDATE file_refs SET table_id = $1, record_id = $2, field = $3, attached_at = COALESCE(attached_at, NOW()) \
                 WHERE app_slug = $4 AND id = ANY($5) AND deleted_at IS NULL",
                &[&table, &rec, &field, &app, &keep],
            )?;
            tx.execute(
                "UPDATE file_refs SET deleted_at = NOW() \
                 WHERE app_slug = $1 AND record_id = $2 AND field = $3 AND deleted_at IS NULL AND NOT (id = ANY($4))",
                &[&app, &rec, &field, &keep],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Deletes uploads that no record took in `hours`, then the blobs no ref points at. Returns the number of uploads
    /// removed and the hashes of the blobs whose rows went, so the caller can remove their bytes.
    pub fn purge_unattached_refs(&self, hours: i64) -> Result<(usize, Vec<String>), RepositoryError> {
        self.with_client(move |c| {
            let mut tx = c.transaction()?;
            let removed = tx.execute(
                "DELETE FROM file_refs WHERE attached_at IS NULL AND uploaded_at < NOW() - make_interval(hours => $1::INT)",
                &[&(hours as i32)],
            )? as usize;
            let orphans = tx
                .query("DELETE FROM files WHERE sha256 NOT IN (SELECT sha256 FROM file_refs) RETURNING sha256", &[])?
                .iter()
                .map(|r| r.get::<_, String>(0).trim().to_string())
                .collect();
            tx.commit()?;
            Ok((removed, orphans))
        })
    }
}
