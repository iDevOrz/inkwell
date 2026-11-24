//! Post and page tools.

use super::*;

pub(super) async fn content_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 100);
    let offset = arg_i64(args, "offset").unwrap_or(0).max(0);
    let status = arg_str(args, "status");
    let filter = ContentFilter {
        kind: arg_str(args, "kind"),
        status: status.clone(),
        query: arg_str(args, "query"),
        include_private: arg_bool(args, "include_private").unwrap_or(true),
        include_deleted: arg_bool(args, "include_deleted")
            .unwrap_or_else(|| status.as_deref() == Some("trash")),
        limit,
        offset,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total,
        "limit": limit,
        "offset": offset
    }))
}

pub(super) async fn content_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let view = load_view(state, args).await?;
    Ok(content_view_json(&view))
}

pub(super) async fn content_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let kind = required_str(args, "kind")?;
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::bad_request("kind 必须是 post、page 或 note"));
    }
    check_write_fields(principal, args)?;
    let title = required_str(args, "title")?;
    let body = required_str(args, "body")?;
    let author_id = admin_user_id(&state.db).await?;
    let is_note = kind == "note";
    let id = content::create(
        &state.db,
        ContentInput {
            kind,
            title,
            slug: arg_str(args, "slug"),
            body,
            excerpt: arg_str(args, "excerpt"),
            status: arg_str(args, "status"),
            visibility: arg_str(args, "visibility")
                .or_else(|| is_note.then(|| "private".to_string())),
            review_status: arg_str(args, "review_status"),
            review_notes: None,
            allow_comments: arg_bool(args, "allow_comments").or_else(|| is_note.then_some(false)),
            category: arg_str(args, "category"),
            tags: arg_string_array(args, "tags"),
            author_id,
            actor: principal.actor(),
            source_note_id: None,
        },
    )
    .await?;
    let content = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::internal("创建后读取内容失败"))?;
    Ok(json!({
        "id": content.id,
        "kind": content.kind,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "review_status": content.review_status,
        "updated_at": content.updated_at
    }))
}

pub(super) async fn content_update(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    check_write_fields(principal, args)?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "published" {
        require(principal, "publish")?;
    }
    let patch = build_patch(args, principal);
    let content =
        content::update_with_publish_permission(&state.db, id, patch, principal.has("publish"))
            .await?;
    Ok(json!({
        "id": content.id,
        "kind": content.kind,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "review_status": content.review_status,
        "review_notes": content.review_notes,
        "updated_at": content.updated_at,
        "previous_updated_at": current.updated_at
    }))
}

pub(super) async fn content_delete(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let permanent = arg_bool(args, "permanent").unwrap_or(false);
    if permanent {
        require(principal, "admin")?;
        if current.status != "trash" {
            return Err(AppError::bad_request("请先移入回收站，再彻底删除"));
        }
        content::delete_permanent(&state.db, id).await?;
        Ok(json!({"id": id, "permanent": true}))
    } else {
        if current.status == "published" {
            require(principal, "publish")?;
        }
        let content = content::trash_with_publish_permission(
            &state.db,
            id,
            &principal.actor(),
            principal.has("publish"),
        )
        .await?;
        Ok(json!({"id": content.id, "status": content.status, "permanent": false}))
    }
}

pub(super) async fn content_publish(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "publish")?;
    let id = required_i64(args, "id")?;
    let content = content::publish(&state.db, id, &principal.actor()).await?;
    Ok(json!({
        "id": content.id,
        "slug": content.slug,
        "status": content.status,
        "published_at": content.published_at
    }))
}

pub(super) async fn content_search(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let query = required_str(args, "query")?;
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 100);
    let filter = ContentFilter {
        kind: arg_str(args, "kind"),
        query: Some(query),
        include_private: true,
        include_deleted: false,
        limit,
        offset: 0,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total
    }))
}

pub(super) async fn content_stats(state: &AppState, principal: &McpPrincipal) -> AppResult<Value> {
    require(principal, "read")?;
    let stats = stats::load(&state.db).await?;
    Ok(serde_json::to_value(stats)?)
}
