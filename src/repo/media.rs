use sqlx::SqlitePool;

use crate::error::AppResult;
use crate::models::Media;
use crate::util::now_ts;

#[derive(Debug, Clone)]
pub struct NewMedia {
    pub storage_key: String,
    pub original_name: String,
    pub mime: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub uploader_id: Option<i64>,
}

pub async fn insert(pool: &SqlitePool, input: NewMedia) -> AppResult<i64> {
    let result = sqlx::query(
        "INSERT INTO media(storage_key, original_name, mime, size, width, height, uploader_id, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.storage_key)
    .bind(input.original_name)
    .bind(input.mime)
    .bind(input.size)
    .bind(input.width)
    .bind(input.height)
    .bind(input.uploader_id)
    .bind(now_ts())
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn list(pool: &SqlitePool, limit: i64) -> AppResult<Vec<Media>> {
    Ok(sqlx::query_as::<_, Media>(
        "SELECT id, storage_key, original_name, mime, size, width, height, uploader_id, created_at \
         FROM media ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Media>> {
    Ok(sqlx::query_as::<_, Media>(
        "SELECT id, storage_key, original_name, mime, size, width, height, uploader_id, created_at \
         FROM media WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn is_publicly_referenced(pool: &SqlitePool, storage_key: &str) -> AppResult<bool> {
    let url_path = format!("/media/{storage_key}");
    let referenced: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM contents c \
             WHERE c.kind IN ('post', 'page') AND c.status = 'published' \
               AND c.visibility = 'public' AND c.deleted_at IS NULL \
               AND instr(c.body, ?) > 0 \
         )",
    )
    .bind(url_path)
    .fetch_one(pool)
    .await?;
    Ok(referenced != 0)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM media WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
