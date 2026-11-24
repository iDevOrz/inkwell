//! Revision history tools.

use super::*;

pub(super) async fn revision_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 200);
    let revisions = revision::list(&state.db, content_id, limit).await?;
    Ok(json!({
        "items": revisions.iter().map(|revision| json!({
            "id": revision.id,
            "content_id": revision.content_id,
            "title": revision.title,
            "status": revision.status,
            "actor": revision.actor,
            "note": revision.note,
            "created_at": revision.created_at,
            "body": revision.body
        })).collect::<Vec<_>>()
    }))
}

pub(super) async fn revision_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let revision = revision::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("修订不存在"))?;
    Ok(serde_json::to_value(revision)?)
}
