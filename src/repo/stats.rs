use sqlx::SqlitePool;

use crate::error::AppResult;
use crate::models::Stats;

pub async fn load(pool: &SqlitePool) -> AppResult<Stats> {
    let posts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE kind = 'post' AND deleted_at IS NULL AND status != 'trash'",
    )
    .fetch_one(pool)
    .await?;
    let published_posts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE kind = 'post' AND status = 'published' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    let drafts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE kind IN ('post', 'page') AND status = 'draft' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    let pages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE kind = 'page' AND deleted_at IS NULL AND status != 'trash'",
    )
    .fetch_one(pool)
    .await?;
    let notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE kind = 'note' AND deleted_at IS NULL AND status != 'trash'",
    )
    .fetch_one(pool)
    .await?;
    let pending_comments: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
            .fetch_one(pool)
            .await?;
    let pending_reviews: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents WHERE review_status = 'pending' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    let total_comments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments")
        .fetch_one(pool)
        .await?;
    let media: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media")
        .fetch_one(pool)
        .await?;

    Ok(Stats {
        posts,
        published_posts,
        drafts,
        pages,
        notes,
        pending_comments,
        pending_reviews,
        total_comments,
        media,
    })
}
