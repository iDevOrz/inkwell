//! Private note tools.

use super::*;

pub(super) async fn note_list(
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
        kind: Some("note".to_string()),
        status: status.clone(),
        query: arg_str(args, "query"),
        include_private: true,
        include_deleted: status.as_deref() == Some("trash"),
        limit,
        offset,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total
    }))
}

pub(super) async fn note_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let view = load_view(state, args).await?;
    if view.content.kind != "note" {
        return Err(AppError::bad_request("该内容不是笔记"));
    }
    Ok(content_view_json(&view))
}

pub(super) async fn note_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let title = required_str(args, "title")?;
    let body = required_str(args, "body")?;
    let author_id = admin_user_id(&state.db).await?;
    let id = content::create(
        &state.db,
        ContentInput {
            kind: "note".to_string(),
            title,
            slug: arg_str(args, "slug"),
            body,
            excerpt: None,
            status: Some(arg_str(args, "status").unwrap_or_else(|| "inbox".to_string())),
            visibility: Some("private".to_string()),
            review_status: Some("none".to_string()),
            review_notes: None,
            allow_comments: Some(false),
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
        .ok_or_else(|| AppError::internal("创建笔记后读取失败"))?;
    Ok(json!({
        "id": content.id,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "updated_at": content.updated_at
    }))
}

pub(super) async fn note_update(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    check_write_fields(principal, args)?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("笔记不存在"))?;
    if current.kind != "note" {
        return Err(AppError::bad_request("该内容不是笔记"));
    }
    let content = content::update(&state.db, id, build_patch(args, principal)).await?;
    Ok(json!({
        "id": content.id,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "updated_at": content.updated_at
    }))
}

pub(super) async fn note_promote(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "draft".to_string());
    if status != "draft" && status != "published" {
        return Err(AppError::bad_request("status 必须是 draft 或 published"));
    }
    if status == "published" {
        require(principal, "publish")?;
    }
    let result = content::promote_note(
        &state.db,
        id,
        PromoteOptions {
            title: arg_str(args, "title"),
            slug: arg_str(args, "slug"),
            status,
            sync: arg_bool(args, "sync").unwrap_or(false),
            can_publish: principal.has("publish"),
            actor: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(result)?)
}
