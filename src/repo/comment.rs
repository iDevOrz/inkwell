use std::collections::HashMap;

use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{COMMENT_STATUSES, Comment, CommentNode, CommentWithContent};
use crate::util::now_ts;

#[derive(Debug, Clone)]
pub struct NewComment {
    pub content_id: i64,
    pub parent_id: i64,
    pub author: String,
    pub email: Option<String>,
    pub url: Option<String>,
    pub body: String,
    pub status: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

pub async fn create(pool: &SqlitePool, input: NewComment) -> AppResult<i64> {
    let author = input.author.trim();
    let body = input.body.trim();
    if author.is_empty() {
        return Err(AppError::bad_request("请填写昵称"));
    }
    if author.chars().count() > 80 {
        return Err(AppError::bad_request("昵称过长"));
    }
    if body.is_empty() {
        return Err(AppError::bad_request("评论内容不能为空"));
    }
    if body.chars().count() > 10_000 {
        return Err(AppError::bad_request("评论内容过长"));
    }
    if !COMMENT_STATUSES.contains(&input.status.as_str()) {
        return Err(AppError::bad_request("非法的评论状态"));
    }

    let email = input
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(email) = email
        && (email.chars().count() > 150 || !email.contains('@'))
    {
        return Err(AppError::bad_request("邮箱格式不正确"));
    }

    let url = input.url.as_deref().map(str::trim).filter(|value| {
        !value.is_empty()
            && value.chars().count() <= 255
            && (value.starts_with("http://") || value.starts_with("https://"))
    });

    let result = sqlx::query(
        "INSERT INTO comments(content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.content_id)
    .bind(input.parent_id.max(0))
    .bind(author)
    .bind(email)
    .bind(url)
    .bind(body)
    .bind(&input.status)
    .bind(input.ip.as_deref())
    .bind(input.user_agent.as_deref())
    .bind(now_ts())
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Comment>> {
    Ok(sqlx::query_as::<_, Comment>(
        "SELECT id, content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at \
         FROM comments WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_for_content(
    pool: &SqlitePool,
    content_id: i64,
    status: &str,
) -> AppResult<Vec<Comment>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at \
         FROM comments WHERE content_id = ",
    );
    query.push_bind(content_id);
    if status != "all" {
        query.push(" AND status = ").push_bind(status.to_string());
    }
    query.push(" ORDER BY created_at ASC, id ASC");
    Ok(query.build_query_as::<Comment>().fetch_all(pool).await?)
}

pub fn build_tree(comments: Vec<Comment>) -> Vec<CommentNode> {
    let mut by_parent: HashMap<i64, Vec<Comment>> = HashMap::new();
    for comment in comments {
        by_parent
            .entry(comment.parent_id)
            .or_default()
            .push(comment);
    }

    fn build(parent_id: i64, by_parent: &mut HashMap<i64, Vec<Comment>>) -> Vec<CommentNode> {
        by_parent
            .remove(&parent_id)
            .unwrap_or_default()
            .into_iter()
            .map(|comment| {
                let children = build(comment.id, by_parent);
                CommentNode { comment, children }
            })
            .collect()
    }

    build(0, &mut by_parent)
}

pub async fn list_admin(
    pool: &SqlitePool,
    status: &str,
    limit: i64,
    offset: i64,
) -> AppResult<(Vec<CommentWithContent>, i64)> {
    let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM comments cm WHERE 1=1");
    if status != "all" {
        count
            .push(" AND cm.status = ")
            .push_bind(status.to_string());
    }
    let total: i64 = count.build_query_scalar().fetch_one(pool).await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT cm.id, cm.content_id, cm.parent_id, cm.author, cm.email, cm.url, cm.body, \
                cm.status, cm.ip, cm.user_agent, cm.created_at, \
                c.title AS content_title, c.slug AS content_slug, c.kind AS content_kind \
         FROM comments cm JOIN contents c ON c.id = cm.content_id WHERE 1=1",
    );
    if status != "all" {
        query
            .push(" AND cm.status = ")
            .push_bind(status.to_string());
    }
    query
        .push(" ORDER BY cm.created_at DESC, cm.id DESC LIMIT ")
        .push_bind(limit.clamp(1, 200))
        .push(" OFFSET ")
        .push_bind(offset.max(0));

    let rows = query
        .build_query_as::<CommentWithContent>()
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

pub async fn moderate(pool: &SqlitePool, id: i64, status: &str) -> AppResult<()> {
    if !COMMENT_STATUSES.contains(&status) {
        return Err(AppError::bad_request("非法的评论状态"));
    }
    sqlx::query("UPDATE comments SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM comments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_by_status(pool: &SqlitePool) -> AppResult<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM comments GROUP BY status")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub async fn count_pending(pool: &SqlitePool) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
            .fetch_one(pool)
            .await?,
    )
}

pub async fn count_all(pool: &SqlitePool) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM comments")
        .fetch_one(pool)
        .await?)
}

pub async fn recent_approved(pool: &SqlitePool, limit: i64) -> AppResult<Vec<CommentWithContent>> {
    Ok(sqlx::query_as::<_, CommentWithContent>(
        "SELECT cm.id, cm.content_id, cm.parent_id, cm.author, cm.email, cm.url, cm.body, \
                cm.status, cm.ip, cm.user_agent, cm.created_at, \
                c.title AS content_title, c.slug AS content_slug, c.kind AS content_kind \
         FROM comments cm JOIN contents c ON c.id = cm.content_id \
         WHERE cm.status = 'approved' ORDER BY cm.created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?)
}
