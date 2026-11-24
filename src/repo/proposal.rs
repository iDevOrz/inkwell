use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{Content, ContentProposal};
use crate::repo::content;
use crate::util::now_ts;
use sqlx::{FromRow, Transaction};

const STALE_BASE_MESSAGE: &str = "原文在读取后已被修改，请重新读取原文后再生成版本";

#[derive(Debug, Clone)]
pub struct ProposalInput {
    pub content_id: i64,
    pub base_revision_id: Option<i64>,
    /// `updated_at` the caller saw when it read the content. When set, the
    /// proposal is rejected if the content has changed since.
    pub expected_updated_at: Option<i64>,
    pub title: Option<String>,
    pub body: String,
    pub excerpt: Option<String>,
    pub summary: Option<String>,
    pub author: String,
}

#[derive(Debug, Clone, FromRow)]
struct ProposalVersion {
    title: String,
    body: String,
    excerpt: Option<String>,
    summary: Option<String>,
}

pub async fn create(pool: &SqlitePool, input: ProposalInput) -> AppResult<ContentProposal> {
    let current = content::get_by_id(pool, input.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "trash" || current.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站内容不能生成版本"));
    }
    if let Some(expected) = input.expected_updated_at
        && current.updated_at != expected
    {
        return Err(AppError::conflict(STALE_BASE_MESSAGE));
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

    // Every content write records a revision, so the latest revision is the
    // live text. A caller-supplied base must be that revision; anything older
    // means the content changed after the AI read it.
    let latest_revision_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE content_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(input.content_id)
    .fetch_optional(pool)
    .await?;
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
            if Some(id) != latest_revision_id {
                return Err(AppError::conflict(STALE_BASE_MESSAGE));
            }
            Some(id)
        }
        None => latest_revision_id,
    };

    let now = now_ts();
    let mut transaction = crate::db::begin_write(pool).await?;
    let result = sqlx::query(
        "INSERT INTO content_proposals(content_id, base_revision_id, base_updated_at, title, \
                body, excerpt, summary, status, author, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, 'proposed', ?, ?)",
    )
    .bind(input.content_id)
    .bind(base_revision_id)
    .bind(current.updated_at)
    .bind(&title)
    .bind(body)
    .bind(input.excerpt.as_deref())
    .bind(input.summary.as_deref())
    .bind(&input.author)
    .bind(now)
    .execute(&mut *transaction)
    .await?;

    let proposal_id = result.last_insert_rowid();
    sqlx::query(
        "INSERT INTO content_proposal_versions( \
             proposal_id, version, title, body, excerpt, summary, author, created_at \
         ) VALUES(?, 1, ?, ?, ?, ?, ?, ?)",
    )
    .bind(proposal_id)
    .bind(&title)
    .bind(body)
    .bind(input.excerpt.as_deref())
    .bind(input.summary.as_deref())
    .bind(&input.author)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    get(pool, proposal_id)
        .await?
        .ok_or_else(|| AppError::internal("创建版本后读取失败"))
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<ContentProposal>> {
    Ok(sqlx::query_as::<_, ContentProposal>(
        "SELECT id, version, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
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
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, version, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
                summary, status, author, created_at, resolved_at, decision_note, \
                applied_revision_id \
         FROM content_proposals WHERE content_id = ",
    );
    query.push_bind(content_id);
    if let Some(status) = status {
        query.push(" AND status = ").push_bind(status);
    }
    query.push(" ORDER BY created_at DESC, id DESC");
    Ok(query
        .build_query_as::<ContentProposal>()
        .fetch_all(pool)
        .await?)
}

pub async fn latest_proposed(
    pool: &SqlitePool,
    content_id: i64,
) -> AppResult<Option<ContentProposal>> {
    Ok(sqlx::query_as::<_, ContentProposal>(
        "SELECT id, version, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
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
    expected_version: i64,
    title: Option<String>,
    body: Option<String>,
    excerpt: Option<String>,
    summary: Option<String>,
) -> AppResult<ContentProposal> {
    let mut transaction = crate::db::begin_write(pool).await?;
    let current = get_in_transaction(&mut transaction, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    if current.status != "proposed" {
        return Err(AppError::bad_request("这个版本已经处理过了"));
    }
    if current.version != expected_version {
        return Err(AppError::conflict("提案在读取后已更新，请重新读取后再编辑"));
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

    let next_version = current.version + 1;
    let now = now_ts();
    let update = sqlx::query(
        "UPDATE content_proposals SET title = ?, body = ?, excerpt = ?, summary = ?, \
                version = ? \
         WHERE id = ? AND status = 'proposed' AND version = ?",
    )
    .bind(&title)
    .bind(&body)
    .bind(&excerpt)
    .bind(&summary)
    .bind(next_version)
    .bind(id)
    .bind(current.version)
    .execute(&mut *transaction)
    .await?;
    if update.rows_affected() != 1 {
        return Err(AppError::conflict("提案在读取后已更新，请重新读取后再编辑"));
    }

    sqlx::query(
        "INSERT INTO content_proposal_versions( \
             proposal_id, version, title, body, excerpt, summary, author, created_at \
         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(next_version)
    .bind(&title)
    .bind(&body)
    .bind(&excerpt)
    .bind(&summary)
    .bind(&current.author)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    get(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("更新版本后读取失败"))
}

pub async fn apply(
    pool: &SqlitePool,
    id: i64,
    expected_version: i64,
    actor: &str,
    force: bool,
    can_publish: bool,
) -> AppResult<(ContentProposal, Content)> {
    let mut transaction = crate::db::begin_write(pool).await?;
    let proposal = get_in_transaction(&mut transaction, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    if proposal.status != "proposed" {
        return Err(AppError::bad_request("这个版本已经处理过了"));
    }
    if proposal.version != expected_version {
        return Err(AppError::conflict(
            "提案在预览后已更新，请检查最新差异后再应用",
        ));
    }
    let snapshot = get_version_in_transaction(&mut transaction, id, expected_version)
        .await?
        .ok_or_else(|| AppError::internal("提案版本记录不存在"))?;
    let current = content::get_by_id_in_transaction(&mut transaction, proposal.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if !force && current.updated_at != proposal.base_updated_at {
        return Err(AppError::conflict(
            "文章在生成这个版本后有改动，请重新生成版本，或选择强制应用",
        ));
    }

    let summary = snapshot
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("：{value}"))
        .unwrap_or_default();
    let revision_note = format!("应用 AI 版本 #{id}{summary}");
    let (updated, applied_revision_id) = content::apply_proposal_in_transaction(
        &mut transaction,
        &current,
        content::ProposalContentUpdate {
            title: &snapshot.title,
            body: &snapshot.body,
            excerpt: snapshot.excerpt.as_deref(),
            actor,
            revision_note: &revision_note,
            can_publish,
        },
    )
    .await?;

    let now = now_ts();
    let result = sqlx::query(
        "UPDATE content_proposals SET status = 'applied', resolved_at = ?, \
                decision_note = ?, applied_revision_id = ? \
         WHERE id = ? AND status = 'proposed' AND version = ?",
    )
    .bind(now)
    .bind("已应用")
    .bind(applied_revision_id)
    .bind(id)
    .bind(expected_version)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::conflict("提案状态已变化，请重新读取后再试"));
    }

    let proposal = get_in_transaction(&mut transaction, id)
        .await?
        .ok_or_else(|| AppError::internal("应用版本后读取失败"))?;
    transaction.commit().await?;

    if current.kind == "note" && updated.title != current.title {
        content::resolve_pending_links(pool, updated.id, &updated.title, &updated.slug).await?;
    }
    Ok((proposal, updated))
}

async fn get_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> AppResult<Option<ContentProposal>> {
    Ok(sqlx::query_as::<_, ContentProposal>(
        "SELECT id, version, content_id, base_revision_id, base_updated_at, title, body, excerpt, \
                summary, status, author, created_at, resolved_at, decision_note, \
                applied_revision_id \
         FROM content_proposals WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?)
}

async fn get_version_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    proposal_id: i64,
    version: i64,
) -> AppResult<Option<ProposalVersion>> {
    Ok(sqlx::query_as::<_, ProposalVersion>(
        "SELECT title, body, excerpt, summary \
         FROM content_proposal_versions WHERE proposal_id = ? AND version = ?",
    )
    .bind(proposal_id)
    .bind(version)
    .fetch_optional(&mut **transaction)
    .await?)
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
