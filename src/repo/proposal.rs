use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::models::{Content, ContentProposal};
use crate::repo::content::{self, ContentPatch};
use crate::util::now_ts;

#[derive(Debug, Clone)]
pub struct ProposalInput {
    pub content_id: i64,
    pub base_revision_id: Option<i64>,
    pub title: Option<String>,
    pub body: String,
    pub excerpt: Option<String>,
    pub summary: Option<String>,
    pub author: String,
}

pub async fn create(pool: &SqlitePool, input: ProposalInput) -> AppResult<ContentProposal> {
    let current = content::get_by_id(pool, input.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "trash" || current.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站内容不能生成版本"));
    }

    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| current.title.clone());
    let body = input.body.trim();
    if body.is_empty() {
        return Err(AppError::bad_request("版本正文不能为空"));
    }

    let base_revision_id = match input.base_revision_id {
        Some(id) => {
            let revision_content: Option<i64> =
                sqlx::query_scalar("SELECT content_id FROM revisions WHERE id = ?")
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;
            if revision_content != Some(input.content_id) {
                return Err(AppError::bad_request("基础修订不属于这篇文章"));
            }
            Some(id)
        }
        None => {
            sqlx::query_scalar(
                "SELECT id FROM revisions WHERE content_id = ? ORDER BY id DESC LIMIT 1",
            )
            .bind(input.content_id)
            .fetch_optional(pool)
            .await?
        }
    };

    let now = now_ts();
    let result = sqlx::query(
        "INSERT INTO content_proposals(content_id, base_revision_id, base_updated_at, title, \
                body, excerpt, summary, status, author, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, 'proposed', ?, ?)",
    )
    .bind(input.content_id)
    .bind(base_revision_id)
    .bind(current.updated_at)
    .bind(title)
    .bind(body)
    .bind(input.excerpt.as_deref())
    .bind(input.summary.as_deref())
    .bind(&input.author)
    .bind(now)
    .execute(pool)
    .await?;

    get(pool, result.last_insert_rowid())
        .await?
        .ok_or_else(|| AppError::internal("创建版本后读取失败"))
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<ContentProposal>> {
    Ok(sqlx::query_as::<_, ContentProposal>(
        "SELECT id, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
                summary, status, author, created_at, resolved_at, decision_note, \
                applied_revision_id \
         FROM content_proposals WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_for_content(
    pool: &SqlitePool,
    content_id: i64,
    status: Option<&str>,
) -> AppResult<Vec<ContentProposal>> {
    let mut sql = String::from(
        "SELECT id, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
                summary, status, author, created_at, resolved_at, decision_note, \
                applied_revision_id \
         FROM content_proposals WHERE content_id = ?",
    );
    if status.is_some() {
        sql.push_str(" AND status = ?");
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC");

    let mut query = sqlx::query_as::<_, ContentProposal>(&sql).bind(content_id);
    if let Some(status) = status {
        query = query.bind(status);
    }
    Ok(query.fetch_all(pool).await?)
}

pub async fn latest_proposed(
    pool: &SqlitePool,
    content_id: i64,
) -> AppResult<Option<ContentProposal>> {
    Ok(sqlx::query_as::<_, ContentProposal>(
        "SELECT id, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
                summary, status, author, created_at, resolved_at, decision_note, \
                applied_revision_id \
         FROM content_proposals WHERE content_id = ? AND status = 'proposed' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(content_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn update(
    pool: &SqlitePool,
    id: i64,
    title: Option<String>,
    body: Option<String>,
    excerpt: Option<String>,
    summary: Option<String>,
) -> AppResult<ContentProposal> {
    let current = get(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    if current.status != "proposed" {
        return Err(AppError::bad_request("这个版本已经处理过了"));
    }

    let title = title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| current.title.clone());
    let body = body
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| current.body.clone());
    let excerpt = match excerpt {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value.trim().to_string()),
        None => current.excerpt.clone(),
    };
    let summary = match summary {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value.trim().to_string()),
        None => current.summary.clone(),
    };

    sqlx::query(
        "UPDATE content_proposals SET title = ?, body = ?, excerpt = ?, summary = ? \
         WHERE id = ? AND status = 'proposed'",
    )
    .bind(title)
    .bind(body)
    .bind(excerpt)
    .bind(summary)
    .bind(id)
    .execute(pool)
    .await?;

    get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("更新版本后读取失败"))
}

pub async fn apply(
    pool: &SqlitePool,
    id: i64,
    actor: &str,
    force: bool,
) -> AppResult<(ContentProposal, Content)> {
    let proposal = get(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    if proposal.status != "proposed" {
        return Err(AppError::bad_request("这个版本已经处理过了"));
    }
    let current = content::get_by_id(pool, proposal.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if !force && current.updated_at != proposal.base_updated_at {
        return Err(AppError::conflict(
            "文章在生成这个版本后有改动，请重新生成版本，或选择强制应用",
        ));
    }

    let summary = proposal
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("：{value}"))
        .unwrap_or_default();
    let patch = ContentPatch {
        title: Some(proposal.title.clone()),
        slug: None,
        body: Some(proposal.body.clone()),
        excerpt: proposal.excerpt.clone(),
        status: None,
        visibility: None,
        review_status: None,
        review_notes: None,
        allow_comments: None,
        category: None,
        tags: None,
        actor: actor.to_string(),
        expected_updated_at: (!force).then_some(proposal.base_updated_at),
        revision_note: Some(format!("应用 AI 版本 #{id}{summary}")),
    };
    let updated = content::update(pool, proposal.content_id, patch).await?;
    let applied_revision_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE content_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(proposal.content_id)
    .fetch_optional(pool)
    .await?;

    let now = now_ts();
    sqlx::query(
        "UPDATE content_proposals SET status = 'applied', resolved_at = ?, \
                decision_note = ?, applied_revision_id = ? WHERE id = ?",
    )
    .bind(now)
    .bind("已应用")
    .bind(applied_revision_id)
    .bind(id)
    .execute(pool)
    .await?;

    let proposal = get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("应用版本后读取失败"))?;
    Ok((proposal, updated))
}

pub async fn reject(pool: &SqlitePool, id: i64, note: Option<&str>) -> AppResult<ContentProposal> {
    let now = now_ts();
    let note = note.map(str::trim).filter(|value| !value.is_empty());
    let result = sqlx::query(
        "UPDATE content_proposals SET status = 'rejected', resolved_at = ?, decision_note = ? \
         WHERE id = ? AND status = 'proposed'",
    )
    .bind(now)
    .bind(note)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::bad_request("这个版本已经处理过了"));
    }
    get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("拒绝版本后读取失败"))
}

pub async fn count_proposed(pool: &SqlitePool) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM content_proposals WHERE status = 'proposed'")
            .fetch_one(pool)
            .await?,
    )
}
