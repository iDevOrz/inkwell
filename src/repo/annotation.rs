use std::collections::HashMap;

use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{
    ANNOTATION_KINDS, ANNOTATION_STATUSES, Annotation, AnnotationReply, AnnotationWithContent,
    AnnotationWithReplies,
};
use crate::util::now_ts;

#[derive(Debug, Clone)]
pub struct AnnotationInput {
    pub content_id: i64,
    pub kind: String,
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_offset: Option<i64>,
    pub end_offset: Option<i64>,
    pub comment: String,
    pub author: String,
}

pub async fn create(pool: &SqlitePool, input: AnnotationInput) -> AppResult<Annotation> {
    if !ANNOTATION_KINDS.contains(&input.kind.as_str()) {
        return Err(AppError::bad_request("批注类型必须是 inline 或 general"));
    }
    let comment = input.comment.trim();
    if comment.is_empty() {
        return Err(AppError::bad_request("批注内容不能为空"));
    }
    let quote = input
        .quote
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if input.kind == "inline" && quote.is_none() {
        return Err(AppError::bad_request("划词批注必须包含选中的原文"));
    }

    let content: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT status, deleted_at FROM contents WHERE id = ?")
            .bind(input.content_id)
            .fetch_optional(pool)
            .await?;
    let Some((status, deleted_at)) = content else {
        return Err(AppError::not_found("内容不存在"));
    };
    if deleted_at.is_some() || status == "trash" {
        return Err(AppError::bad_request("回收站内容不能添加批注"));
    }

    let now = now_ts();
    let result = sqlx::query(
        "INSERT INTO annotations(content_id, kind, quote, prefix, suffix, start_offset, \
                end_offset, comment, status, author, created_at, updated_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
    )
    .bind(input.content_id)
    .bind(&input.kind)
    .bind(&quote)
    .bind(input.prefix.as_deref())
    .bind(input.suffix.as_deref())
    .bind(input.start_offset)
    .bind(input.end_offset)
    .bind(comment)
    .bind(&input.author)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    get(pool, result.last_insert_rowid())
        .await?
        .ok_or_else(|| AppError::internal("创建批注后读取失败"))
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Annotation>> {
    Ok(sqlx::query_as::<_, Annotation>(
        "SELECT id, content_id, kind, quote, prefix, suffix, start_offset, end_offset, \
                comment, status, author, created_at, updated_at, resolved_at \
         FROM annotations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_with_replies(
    pool: &SqlitePool,
    id: i64,
) -> AppResult<Option<AnnotationWithReplies>> {
    let Some(annotation) = get(pool, id).await? else {
        return Ok(None);
    };
    let mut items = with_replies(pool, vec![annotation]).await?;
    Ok(items.pop())
}

pub async fn list_for_content(
    pool: &SqlitePool,
    content_id: i64,
    status: Option<&str>,
) -> AppResult<Vec<AnnotationWithReplies>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, content_id, kind, quote, prefix, suffix, start_offset, end_offset, \
                comment, status, author, created_at, updated_at, resolved_at \
         FROM annotations WHERE content_id = ",
    );
    query.push_bind(content_id);
    if let Some(status) = status {
        query.push(" AND status = ").push_bind(status.to_string());
    }
    query.push(" ORDER BY created_at ASC, id ASC");
    let annotations = query.build_query_as::<Annotation>().fetch_all(pool).await?;
    with_replies(pool, annotations).await
}

pub async fn queue(
    pool: &SqlitePool,
    status: &str,
    limit: i64,
) -> AppResult<Vec<AnnotationWithContent>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, content_id, kind, quote, prefix, suffix, start_offset, end_offset, \
                comment, status, author, created_at, updated_at, resolved_at \
         FROM annotations WHERE 1 = 1",
    );
    if status != "all" {
        query.push(" AND status = ").push_bind(status.to_string());
    }
    query
        .push(" ORDER BY created_at ASC, id ASC LIMIT ")
        .push_bind(limit.clamp(1, 200));
    let annotations = query.build_query_as::<Annotation>().fetch_all(pool).await?;
    if annotations.is_empty() {
        return Ok(Vec::new());
    }

    let content_ids: Vec<i64> = annotations
        .iter()
        .map(|annotation| annotation.content_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, title, slug, kind, status FROM contents WHERE id IN (",
    );
    let mut separated = query.separated(", ");
    for id in &content_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    #[derive(sqlx::FromRow)]
    struct ContentRow {
        id: i64,
        title: String,
        slug: String,
        kind: String,
        status: String,
    }

    let contents: HashMap<i64, ContentRow> = query
        .build_query_as::<ContentRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();

    let annotations = with_replies(pool, annotations).await?;
    Ok(annotations
        .into_iter()
        .filter_map(|item| {
            let content = contents.get(&item.annotation.content_id)?;
            Some(AnnotationWithContent {
                annotation: item.annotation,
                replies: item.replies,
                content_title: content.title.clone(),
                content_slug: content.slug.clone(),
                content_kind: content.kind.clone(),
                content_status: content.status.clone(),
            })
        })
        .collect())
}

async fn with_replies(
    pool: &SqlitePool,
    annotations: Vec<Annotation>,
) -> AppResult<Vec<AnnotationWithReplies>> {
    if annotations.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = annotations.iter().map(|annotation| annotation.id).collect();
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, annotation_id, author, body, created_at \
         FROM annotation_replies WHERE annotation_id IN (",
    );
    let mut separated = query.separated(", ");
    for id in &ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY created_at ASC, id ASC");

    let mut replies: HashMap<i64, Vec<AnnotationReply>> = HashMap::new();
    for reply in query
        .build_query_as::<AnnotationReply>()
        .fetch_all(pool)
        .await?
    {
        replies.entry(reply.annotation_id).or_default().push(reply);
    }

    Ok(annotations
        .into_iter()
        .map(|annotation| {
            let replies = replies.remove(&annotation.id).unwrap_or_default();
            AnnotationWithReplies {
                annotation,
                replies,
            }
        })
        .collect())
}

pub async fn reply(
    pool: &SqlitePool,
    id: i64,
    author: &str,
    body: &str,
    status: Option<&str>,
) -> AppResult<Annotation> {
    let body = body.trim();
    if body.is_empty() {
        return Err(AppError::bad_request("回复内容不能为空"));
    }
    if let Some(status) = status
        && !ANNOTATION_STATUSES.contains(&status)
    {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    get(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    let now = now_ts();

    let mut transaction = crate::db::begin_write(pool).await?;
    sqlx::query(
        "INSERT INTO annotation_replies(annotation_id, author, body, created_at) \
         VALUES(?, ?, ?, ?)",
    )
    .bind(id)
    .bind(author)
    .bind(body)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    // Without an explicit status, keep whatever is stored at write time so a
    // reply never reverts a concurrent resolve by the author.
    sqlx::query(
        "UPDATE annotations SET \
             status = COALESCE(?1, status), \
             updated_at = ?2, \
             resolved_at = CASE \
                 WHEN ?1 IS NULL THEN resolved_at \
                 WHEN ?1 = 'resolved' THEN ?2 \
                 ELSE NULL \
             END \
         WHERE id = ?3",
    )
    .bind(status)
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("回复批注后读取失败"))
}

pub async fn count_status(pool: &SqlitePool, status: &str) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM annotations WHERE status = ?")
            .bind(status)
            .fetch_one(pool)
            .await?,
    )
}

pub async fn set_status(pool: &SqlitePool, id: i64, status: &str) -> AppResult<Annotation> {
    if !ANNOTATION_STATUSES.contains(&status) {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    let now = now_ts();
    let resolved_at = if status == "resolved" {
        Some(now)
    } else {
        None
    };
    let result = sqlx::query(
        "UPDATE annotations SET status = ?, updated_at = ?, resolved_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(now)
    .bind(resolved_at)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("批注不存在"));
    }
    get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("更新批注后读取失败"))
}
