//! Creating, updating, publishing, trashing and reviewing content.

use super::*;

pub async fn create(pool: &SqlitePool, input: ContentInput) -> AppResult<i64> {
    if !KINDS.contains(&input.kind.as_str()) {
        return Err(AppError::bad_request("未知的内容类型"));
    }
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::bad_request("标题不能为空"));
    }

    let status = input
        .status
        .clone()
        .unwrap_or_else(|| default_status(&input.kind).to_string());
    if !is_valid_status(&input.kind, &status) {
        return Err(AppError::bad_request("非法的内容状态"));
    }
    let visibility = input
        .visibility
        .clone()
        .unwrap_or_else(|| "public".to_string());
    if !VISIBILITIES.contains(&visibility.as_str()) {
        return Err(AppError::bad_request("非法的可见性"));
    }
    let review_status = input
        .review_status
        .clone()
        .unwrap_or_else(|| "none".to_string());
    if !REVIEW_STATUSES.contains(&review_status.as_str()) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }

    let slug_base = input.slug.as_deref().unwrap_or(&title);
    let slug = unique_slug(pool, slug_base, None).await?;
    let now = now_ts();
    let published_at = (status == "published").then_some(now);
    let excerpt = input
        .excerpt
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| markdown::excerpt(&input.body, 240));
    let allow_comments = input.allow_comments.unwrap_or(true) as i64;

    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO contents(kind, title, slug, body, excerpt, status, visibility, author_id, \
                created_at, updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, '{}', NULL)",
    )
    .bind(&input.kind)
    .bind(&title)
    .bind(&slug)
    .bind(&input.body)
    .bind(&excerpt)
    .bind(&status)
    .bind(&visibility)
    .bind(input.author_id)
    .bind(now)
    .bind(now)
    .bind(published_at)
    .bind(&review_status)
    .bind(input.review_notes.as_deref())
    .bind(allow_comments)
    .bind(input.source_note_id)
    .execute(&mut *transaction)
    .await;

    let result = match result {
        Ok(result) => result,
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(AppError::conflict("缩略名或来源笔记已被使用"));
        }
        Err(error) => return Err(error.into()),
    };

    let id = result.last_insert_rowid();
    term::replace_terms_tx(&mut transaction, id, input.category.as_deref(), &input.tags).await?;
    sync_links_tx(&mut transaction, id, &input.body).await?;
    revision::insert_tx(
        &mut transaction,
        id,
        &title,
        &input.body,
        &status,
        &input.actor,
        Some("创建"),
        now,
    )
    .await?;
    transaction.commit().await?;

    if input.kind == "note" {
        resolve_pending_links(pool, id, &title, &slug).await?;
    }

    Ok(id)
}

pub async fn update(pool: &SqlitePool, id: i64, patch: ContentPatch) -> AppResult<Content> {
    update_with_publish_permission(pool, id, patch, true).await
}

pub async fn update_with_publish_permission(
    pool: &SqlitePool,
    id: i64,
    patch: ContentPatch,
    can_publish: bool,
) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;

    if current.status == "published" && !can_publish {
        return Err(AppError::forbidden("修改已发布文章需要 publish 权限"));
    }

    if let Some(expected) = patch.expected_updated_at
        && current.updated_at != expected
    {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    let title = patch
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| current.title.clone());

    let body = patch.body.clone().unwrap_or_else(|| current.body.clone());
    let status = patch
        .status
        .clone()
        .unwrap_or_else(|| current.status.clone());
    if !is_valid_status(&current.kind, &status) {
        return Err(AppError::bad_request("非法的内容状态"));
    }
    let visibility = patch
        .visibility
        .clone()
        .unwrap_or_else(|| current.visibility.clone());
    if !VISIBILITIES.contains(&visibility.as_str()) {
        return Err(AppError::bad_request("非法的可见性"));
    }
    let review_status = patch
        .review_status
        .clone()
        .unwrap_or_else(|| current.review_status.clone());
    if !REVIEW_STATUSES.contains(&review_status.as_str()) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }
    let review_notes = patch
        .review_notes
        .clone()
        .or_else(|| current.review_notes.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let slug = match patch
        .slug
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => unique_slug(pool, value, Some(id)).await?,
        None => current.slug.clone(),
    };

    let excerpt = match patch.excerpt.as_deref() {
        Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Some(_) => Some(markdown::excerpt(&body, 240)),
        None if body != current.body => Some(markdown::excerpt(&body, 240)),
        None => current.excerpt.clone(),
    };

    let now = now_ts().max(current.updated_at.saturating_add(1));
    let published_at = if status == "published" {
        current.published_at.or(Some(now))
    } else {
        current.published_at
    };
    let deleted_at = if status == "trash" {
        current.deleted_at.or(Some(now))
    } else {
        None
    };
    let allow_comments = patch
        .allow_comments
        .map(|value| value as i64)
        .unwrap_or(current.allow_comments);

    let mut transaction = pool.begin().await?;
    let update = sqlx::query(
        "UPDATE contents SET title = ?, slug = ?, body = ?, excerpt = ?, status = ?, \
                visibility = ?, updated_at = ?, published_at = ?, review_status = ?, \
                review_notes = ?, allow_comments = ?, deleted_at = ? \
         WHERE id = ? AND updated_at = ? AND (? OR status != 'published')",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&body)
    .bind(&excerpt)
    .bind(&status)
    .bind(&visibility)
    .bind(now)
    .bind(published_at)
    .bind(&review_status)
    .bind(&review_notes)
    .bind(allow_comments)
    .bind(deleted_at)
    .bind(id)
    .bind(current.updated_at)
    .bind(can_publish)
    .execute(&mut *transaction)
    .await;
    // `unique_slug` runs outside the transaction, so a concurrent writer can
    // still claim the same slug; report that as a conflict, not a 500.
    let update = match update {
        Ok(result) => result,
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(AppError::conflict("缩略名已被使用，请换一个"));
        }
        Err(error) => return Err(error.into()),
    };
    if update.rows_affected() != 1 {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    if let Some(tags) = &patch.tags {
        term::set_tags_tx(&mut transaction, id, tags).await?;
    }
    if let Some(category) = &patch.category {
        term::set_category_tx(&mut transaction, id, Some(category)).await?;
    }
    if body != current.body {
        sync_links_tx(&mut transaction, id, &body).await?;
    }
    revision::insert_tx(
        &mut transaction,
        id,
        &title,
        &body,
        &status,
        &patch.actor,
        patch.revision_note.as_deref().or(Some("更新")),
        now,
    )
    .await?;
    transaction.commit().await?;

    if current.kind == "note" && (title != current.title || slug != current.slug) {
        resolve_pending_links(pool, id, &title, &slug).await?;
    }

    get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("更新后读取内容失败"))
}

pub(in crate::repo) async fn get_by_id_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> AppResult<Option<Content>> {
    Ok(sqlx::query_as::<_, Content>(
        "SELECT id, kind, title, slug, body, excerpt, status, visibility, author_id, created_at, \
                updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at \
         FROM contents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?)
}

pub(in crate::repo) struct ProposalContentUpdate<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub excerpt: Option<&'a str>,
    pub actor: &'a str,
    pub revision_note: &'a str,
    pub can_publish: bool,
}

pub(in crate::repo) async fn apply_proposal_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    current: &Content,
    update: ProposalContentUpdate<'_>,
) -> AppResult<(Content, i64)> {
    if current.status == "published" && !update.can_publish {
        return Err(AppError::forbidden("修改已发布文章需要 publish 权限"));
    }

    let title = if update.title.trim().is_empty() {
        current.title.clone()
    } else {
        update.title.trim().to_string()
    };
    let body = update.body.trim();
    if body.is_empty() {
        return Err(AppError::bad_request("版本正文不能为空"));
    }
    let excerpt = match update.excerpt {
        Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => Some(markdown::excerpt(body, 240)),
    };
    let now = now_ts().max(current.updated_at.saturating_add(1));

    let result = sqlx::query(
        "UPDATE contents SET title = ?, body = ?, excerpt = ?, updated_at = ? \
         WHERE id = ? AND updated_at = ? AND (? OR status != 'published')",
    )
    .bind(&title)
    .bind(body)
    .bind(&excerpt)
    .bind(now)
    .bind(current.id)
    .bind(current.updated_at)
    .bind(update.can_publish)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    if body != current.body {
        sync_links_tx(transaction, current.id, body).await?;
    }
    let revision_id = revision::insert_tx(
        transaction,
        current.id,
        &title,
        body,
        &current.status,
        update.actor,
        Some(update.revision_note),
        now,
    )
    .await?;
    let updated = get_by_id_in_transaction(transaction, current.id)
        .await?
        .ok_or_else(|| AppError::internal("应用版本后读取内容失败"))?;
    Ok((updated, revision_id))
}

pub async fn publish(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    ensure_publishable(&current)?;

    update(
        pool,
        id,
        ContentPatch {
            status: Some("published".to_string()),
            review_status: (current.review_status == "pending").then(|| "approved".to_string()),
            actor: actor.to_string(),
            revision_note: Some("发布".to_string()),
            ..Default::default()
        },
    )
    .await
}

/// Approves and publishes in a single guarded write, so a failed publish never
/// leaves the review status changed on its own.
pub async fn approve_and_publish(
    pool: &SqlitePool,
    id: i64,
    review_notes: Option<&str>,
    actor: &str,
) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    ensure_publishable(&current)?;
    update(
        pool,
        id,
        ContentPatch {
            status: Some("published".to_string()),
            review_status: Some("approved".to_string()),
            // An empty string clears stale notes, matching `set_review`.
            review_notes: Some(review_notes.unwrap_or_default().to_string()),
            expected_updated_at: Some(current.updated_at),
            actor: actor.to_string(),
            revision_note: Some("review: approved，发布".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub(super) fn ensure_publishable(content: &Content) -> AppResult<()> {
    if content.kind == "note" {
        return Err(AppError::bad_request("笔记请使用 promote 转为文章"));
    }
    if content.status == "trash" || content.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站中的内容不能直接发布"));
    }
    Ok(())
}

pub async fn trash(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    trash_with_publish_permission(pool, id, actor, true).await
}

/// Moves content to the trash. Without `can_publish`, published content is
/// refused inside the guarded UPDATE, so a concurrent publish cannot slip by.
pub async fn trash_with_publish_permission(
    pool: &SqlitePool,
    id: i64,
    actor: &str,
    can_publish: bool,
) -> AppResult<Content> {
    update_with_publish_permission(
        pool,
        id,
        ContentPatch {
            status: Some("trash".to_string()),
            actor: actor.to_string(),
            revision_note: Some("移入回收站".to_string()),
            ..Default::default()
        },
        can_publish,
    )
    .await
}

pub async fn restore(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let status = if current.kind == "note" {
        "inbox"
    } else {
        "draft"
    };
    update(
        pool,
        id,
        ContentPatch {
            status: Some(status.to_string()),
            actor: actor.to_string(),
            revision_note: Some("恢复".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn delete_permanent(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM contents WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_review(
    pool: &SqlitePool,
    id: i64,
    review_status: &str,
    review_notes: Option<&str>,
    actor: &str,
    can_publish: bool,
) -> AppResult<Content> {
    if !REVIEW_STATUSES.contains(&review_status) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "published" && !can_publish {
        return Err(AppError::forbidden("修改已发布文章需要 publish 权限"));
    }
    let now = now_ts().max(current.updated_at.saturating_add(1));
    let notes = review_notes
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut transaction = pool.begin().await?;
    // The revision below snapshots `current`, so only write if it is still
    // the stored version.
    let result = sqlx::query(
        "UPDATE contents SET review_status = ?, review_notes = ?, updated_at = ? \
         WHERE id = ? AND updated_at = ? AND (? OR status != 'published')",
    )
    .bind(review_status)
    .bind(notes)
    .bind(now)
    .bind(id)
    .bind(current.updated_at)
    .bind(can_publish)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }
    revision::insert_tx(
        &mut transaction,
        id,
        &current.title,
        &current.body,
        &current.status,
        actor,
        Some(&format!("review: {review_status}")),
        now,
    )
    .await?;
    transaction.commit().await?;

    get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("review 后读取内容失败"))
}

pub async fn unique_slug(
    pool: &SqlitePool,
    base: &str,
    exclude_id: Option<i64>,
) -> AppResult<String> {
    let base = slugify(base);
    let mut candidate = base.clone();
    let mut counter = 2_i64;

    loop {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM contents WHERE slug = ? AND (? IS NULL OR id != ?)")
                .bind(&candidate)
                .bind(exclude_id)
                .bind(exclude_id)
                .fetch_optional(pool)
                .await?;

        if exists.is_none() {
            return Ok(candidate);
        }

        candidate = format!("{}-{}", truncate_chars(&base, 110), counter);
        counter += 1;
        if counter > 10_000 {
            return Err(AppError::conflict("无法生成唯一的缩略名"));
        }
    }
}
