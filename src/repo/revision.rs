use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::AppResult;
use crate::models::Revision;

#[allow(clippy::too_many_arguments)]
pub async fn insert_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    content_id: i64,
    title: &str,
    body: &str,
    status: &str,
    actor: &str,
    note: Option<&str>,
    created_at: i64,
) -> AppResult<i64> {
    let result = sqlx::query(
        "INSERT INTO revisions(content_id, title, body, status, actor, note, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(content_id)
    .bind(title)
    .bind(body)
    .bind(status)
    .bind(actor)
    .bind(note)
    .bind(created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn list(pool: &SqlitePool, content_id: i64, limit: i64) -> AppResult<Vec<Revision>> {
    Ok(sqlx::query_as::<_, Revision>(
        "SELECT id, content_id, title, body, status, actor, note, created_at \
         FROM revisions WHERE content_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(content_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

pub async fn previous(
    pool: &SqlitePool,
    content_id: i64,
    revision_id: i64,
) -> AppResult<Option<Revision>> {
    Ok(sqlx::query_as::<_, Revision>(
        "SELECT id, content_id, title, body, status, actor, note, created_at \
         FROM revisions WHERE content_id = ? AND id < ? ORDER BY id DESC LIMIT 1",
    )
    .bind(content_id)
    .bind(revision_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Revision>> {
    Ok(sqlx::query_as::<_, Revision>(
        "SELECT id, content_id, title, body, status, actor, note, created_at \
         FROM revisions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}
