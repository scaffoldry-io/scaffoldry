//! Comments on records. A comment write also writes a `record_history` entry in the same
//! transaction. The entry says a comment was added, edited, or removed, and by whom. It holds no text.

use crate::repository::{write_history, PostgresRepository, RepositoryError, Touched, WriteCtx};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

/// One comment as stored. A removed comment has no body and no mentions.
#[derive(Debug, Clone, Serialize)]
pub struct CommentRow {
    pub id: Uuid,
    pub table_id: String,
    pub record_id: String,
    pub parent_id: Option<Uuid>,
    pub author: String,
    pub body: Option<String>,
    pub mentions: Vec<String>,
    pub deleted: bool,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted_at: Option<String>,
}

/// What a new comment holds. The author is the actor of the write.
pub struct NewComment {
    pub table_id: String,
    pub record_id: String,
    pub parent_id: Option<Uuid>,
    pub body: String,
    pub mentions: Vec<String>,
}

const COLUMNS: &str = "id, table_id, record_id, parent_id, author, body, mentions, deleted_at IS NOT NULL, \
    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), \
    to_char(edited_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'), \
    to_char(deleted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')";

fn row_of(r: &postgres::Row) -> CommentRow {
    let deleted: bool = r.get(7);
    CommentRow {
        id: r.get(0),
        table_id: r.get(1),
        record_id: r.get(2),
        parent_id: r.get(3),
        author: r.get(4),
        body: if deleted { None } else { Some(r.get(5)) },
        mentions: r.get(6),
        deleted,
        created_at: r.get(8),
        edited_at: r.get(9),
        deleted_at: r.get(10),
    }
}

/// What a comment write did, for the history entry.
fn note(tx: &mut postgres::Transaction<'_>, app_slug: &str, table_id: &str, record_id: &str, comment: Uuid, what: &str, ctx: &WriteCtx) -> Result<(), postgres::Error> {
    let version: i32 = tx
        .query_opt("SELECT version FROM dataset_records WHERE app_slug = $1 AND id = $2", &[&app_slug, &record_id])?
        .map(|r| r.get(0))
        .unwrap_or(0);
    let changes = json!({ "comment": { "id": comment.to_string(), "what": what } });
    let changes = changes.as_object().cloned().unwrap_or_default();
    write_history(tx, Touched { app_slug, table_id, record_id, version, action: "comment" }, &changes, ctx)
}

impl PostgresRepository {
    /// A new comment, and its history entry.
    pub fn insert_comment(&self, app_slug: &str, new: NewComment, ctx: &WriteCtx) -> Result<CommentRow, RepositoryError> {
        let (slug, ctx) = (app_slug.to_string(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO record_comments (id, app_slug, table_id, record_id, parent_id, author, body, mentions) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&id, &slug, &new.table_id, &new.record_id, &new.parent_id, &ctx.actor, &new.body, &new.mentions],
            )?;
            note(&mut tx, &slug, &new.table_id, &new.record_id, id, "added", &ctx)?;
            let row = tx.query_one(&format!("SELECT {COLUMNS} FROM record_comments WHERE id = $1"), &[&id])?;
            let out = row_of(&row);
            tx.commit()?;
            Ok(out)
        })
    }

    pub fn get_comment(&self, app_slug: &str, id: Uuid) -> Result<Option<CommentRow>, RepositoryError> {
        let slug = app_slug.to_string();
        self.with_client(move |client| {
            Ok(client
                .query_opt(&format!("SELECT {COLUMNS} FROM record_comments WHERE app_slug = $1 AND id = $2"), &[&slug, &id])?
                .map(|r| row_of(&r)))
        })
    }

    /// The comments of a record, oldest first.
    pub fn list_comments(&self, app_slug: &str, record_id: &str, limit: i64) -> Result<Vec<CommentRow>, RepositoryError> {
        let (slug, record) = (app_slug.to_string(), record_id.to_string());
        self.with_client(move |client| {
            Ok(client
                .query(
                    &format!("SELECT {COLUMNS} FROM record_comments WHERE app_slug = $1 AND record_id = $2 ORDER BY created_at, id LIMIT $3"),
                    &[&slug, &record, &limit],
                )?
                .iter()
                .map(row_of)
                .collect())
        })
    }

    /// Changes the text of a comment that is not removed. Returns `None` when there is no such comment.
    pub fn edit_comment(&self, app_slug: &str, id: Uuid, body: &str, mentions: &[String], ctx: &WriteCtx) -> Result<Option<CommentRow>, RepositoryError> {
        let (slug, body, mentions, ctx) = (app_slug.to_string(), body.to_string(), mentions.to_vec(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let Some(row) = tx.query_opt(
                "UPDATE record_comments SET body = $3, mentions = $4, edited_at = NOW() \
                 WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL RETURNING table_id, record_id",
                &[&slug, &id, &body, &mentions],
            )? else {
                return Ok(None);
            };
            let (table, record): (String, String) = (row.get(0), row.get(1));
            note(&mut tx, &slug, &table, &record, id, "edited", &ctx)?;
            let out = row_of(&tx.query_one(&format!("SELECT {COLUMNS} FROM record_comments WHERE id = $1"), &[&id])?);
            tx.commit()?;
            Ok(Some(out))
        })
    }

    /// Removes a comment: its text and its mentions are cleared from the row, and the author and time stay.
    pub fn remove_comment(&self, app_slug: &str, id: Uuid, ctx: &WriteCtx) -> Result<Option<CommentRow>, RepositoryError> {
        let (slug, ctx) = (app_slug.to_string(), ctx.clone());
        self.with_client(move |client| {
            let mut tx = client.transaction()?;
            let Some(row) = tx.query_opt(
                "UPDATE record_comments SET body = '', mentions = '{}', deleted_at = NOW() \
                 WHERE app_slug = $1 AND id = $2 AND deleted_at IS NULL RETURNING table_id, record_id",
                &[&slug, &id],
            )? else {
                return Ok(None);
            };
            let (table, record): (String, String) = (row.get(0), row.get(1));
            note(&mut tx, &slug, &table, &record, id, "removed", &ctx)?;
            let out = row_of(&tx.query_one(&format!("SELECT {COLUMNS} FROM record_comments WHERE id = $1"), &[&id])?);
            tx.commit()?;
            Ok(Some(out))
        })
    }
}
