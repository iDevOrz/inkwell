//! Comment listing and moderation tools.

use super::*;

pub(super) async fn comment_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "all".to_string());
    if let Some(content_id) = arg_i64(args, "content_id") {
        let comments = comment::list_for_content(&state.db, content_id, &status).await?;
        return Ok(json!({
            "items": comments.iter().map(comment_json).collect::<Vec<_>>()
        }));
    }

    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 100);
    let offset = arg_i64(args, "offset").unwrap_or(0).max(0);
    let (comments, total) = comment::list_admin(&state.db, &status, limit, offset).await?;
    Ok(json!({
        "items": comments.iter().map(comment_with_content_json).collect::<Vec<_>>(),
        "total": total
    }))
}

pub(super) async fn comment_moderate(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    let id = required_i64(args, "id")?;
    let action = required_str(args, "action")?;
    if action == "delete" {
        require(principal, "admin")?;
    } else {
        require(principal, "review")?;
    }
    match action.as_str() {
        "approve" => comment::moderate(&state.db, id, "approved").await?,
        "pending" => comment::moderate(&state.db, id, "pending").await?,
        "spam" => comment::moderate(&state.db, id, "spam").await?,
        "trash" => comment::moderate(&state.db, id, "trash").await?,
        "delete" => comment::delete(&state.db, id).await?,
        _ => return Err(AppError::bad_request("未知的评论操作")),
    }
    Ok(json!({"id": id, "action": action}))
}
