//! Promoting a note to a post and syncing it afterwards.

use super::*;

pub async fn promote_note(
    pool: &SqlitePool,
    note_id: i64,
    options: PromoteOptions,
) -> AppResult<PromoteResult> {
    let note = get_by_id(pool, note_id)
        .await?
        .ok_or_else(|| AppError::not_found("笔记不存在"))?;
    if note.kind != "note" {
        return Err(AppError::bad_request("只有笔记可以转为文章"));
    }
    if note.status == "trash" || note.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站中的笔记不能转为文章"));
    }

    if let Some(post_id) = note.promoted_to_id {
        let post = get_by_id(pool, post_id)
            .await?
            .ok_or_else(|| AppError::not_found("关联文章不存在"))?;
        if options.sync {
            let terms = term::terms_for_contents(pool, &[note.id]).await?;
            let terms = terms.get(&note.id).cloned().unwrap_or_default();
            let tags: Vec<String> = terms
                .iter()
                .filter(|term| term.kind == "tag")
                .map(|term| term.name.clone())
                .collect();
            let category = terms
                .iter()
                .find(|term| term.kind == "category")
                .map(|term| term.name.clone());
            update_with_publish_permission(
                pool,
                post.id,
                ContentPatch {
                    title: Some(note.title.clone()),
                    body: Some(note.body.clone()),
                    excerpt: note.excerpt.clone(),
                    tags: Some(tags),
                    category: Some(category.unwrap_or_default()),
                    actor: options.actor,
                    revision_note: Some("从笔记同步".to_string()),
                    ..Default::default()
                },
                options.can_publish,
            )
            .await?;
        }
        return Ok(PromoteResult {
            id: post.id,
            slug: post.slug,
            created: false,
        });
    }

    let status = if options.status == "published" {
        "published"
    } else {
        "draft"
    };
    if status == "published" && !options.can_publish {
        return Err(AppError::forbidden("发布文章需要 publish 权限"));
    }
    let terms = term::terms_for_contents(pool, &[note.id]).await?;
    let terms = terms.get(&note.id).cloned().unwrap_or_default();
    let tags: Vec<String> = terms
        .iter()
        .filter(|term| term.kind == "tag")
        .map(|term| term.name.clone())
        .collect();
    let category = terms
        .iter()
        .find(|term| term.kind == "category")
        .map(|term| term.name.clone());

    let title = options.title.unwrap_or_else(|| note.title.clone());
    let result = super::writes::create_with_promotion(
        pool,
        ContentInput {
            kind: "post".to_string(),
            title,
            slug: options.slug,
            body: note.body.clone(),
            excerpt: note.excerpt.clone(),
            status: Some(status.to_string()),
            visibility: Some("public".to_string()),
            review_status: Some(note.review_status.clone()),
            review_notes: note.review_notes.clone(),
            allow_comments: Some(note.allow_comments != 0),
            category,
            tags,
            author_id: note.author_id,
            actor: options.actor,
            source_note_id: Some(note.id),
        },
        Some(&note),
    )
    .await;

    let (post_id, created) = match result {
        Ok(id) => (id, true),
        Err(error) => {
            // If another request won the race, return the existing promoted post.
            if let Some(existing) =
                sqlx::query_scalar::<_, i64>("SELECT id FROM contents WHERE source_note_id = ?")
                    .bind(note.id)
                    .fetch_optional(pool)
                    .await?
            {
                (existing, false)
            } else {
                return Err(error);
            }
        }
    };

    let post = get_by_id(pool, post_id)
        .await?
        .ok_or_else(|| AppError::internal("转为文章后读取失败"))?;

    if !created {
        // Repair a missing backlink left by older versions, without touching
        // the version of a note already linked by a concurrent promotion.
        sqlx::query(
            "UPDATE contents SET promoted_to_id = ?, updated_at = MAX(?, updated_at + 1) \
             WHERE id = ? AND promoted_to_id IS NULL AND deleted_at IS NULL AND status != 'trash'",
        )
        .bind(post_id)
        .bind(now_ts())
        .bind(note.id)
        .execute(pool)
        .await?;
    }

    Ok(PromoteResult {
        id: post.id,
        slug: post.slug,
        created,
    })
}
