//! Annotation tools for the AI editing workflow.

use super::*;

pub(super) async fn annotation_queue(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "open".to_string());
    if !ANNOTATION_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 200);
    let items = annotation::queue(&state.db, &status, limit).await?;
    let total = items.len();
    Ok(json!({
        "items": items,
        "total": total,
        "status": status
    }))
}

pub(super) async fn annotation_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "open".to_string());
    if !ANNOTATION_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    let status = (status != "all").then_some(status.as_str());
    let items = annotation::list_for_content(&state.db, content_id, status).await?;
    let total = items.len();
    Ok(json!({
        "items": items,
        "total": total
    }))
}

pub(super) async fn annotation_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn annotation_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let content_id = required_i64(args, "content_id")?;
    let kind = required_str(args, "kind")?;
    if !ANNOTATION_KINDS.contains(&kind.as_str()) {
        return Err(AppError::bad_request("kind 必须是 inline 或 general"));
    }
    let comment = required_str(args, "comment")?;
    let item = annotation::create(
        &state.db,
        AnnotationInput {
            content_id,
            kind,
            quote: arg_str(args, "quote"),
            prefix: arg_str(args, "prefix"),
            suffix: arg_str(args, "suffix"),
            start_offset: arg_i64(args, "start_offset"),
            end_offset: arg_i64(args, "end_offset"),
            comment,
            author: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn annotation_reply(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let body = required_str(args, "body")?;
    let status = arg_str(args, "status");
    if let Some(status) = status.as_deref()
        && !ANNOTATION_STATUSES.contains(&status)
    {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    if status.as_deref() == Some("resolved") {
        // Closing feedback is the author's decision; AI clients mark it addressed.
        require(principal, "review")?;
    }
    annotation::reply(&state.db, id, &principal.actor(), &body, status.as_deref()).await?;
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn annotation_resolve(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "review")?;
    let id = required_i64(args, "id")?;
    if let Some(note) = arg_str(args, "note").filter(|value| !value.trim().is_empty()) {
        annotation::reply(&state.db, id, &principal.actor(), &note, Some("resolved")).await?;
    } else {
        annotation::set_status(&state.db, id, "resolved").await?;
    }
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}
