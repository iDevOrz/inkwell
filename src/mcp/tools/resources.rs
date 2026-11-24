//! MCP resources (resources/read) and their URI scheme.

use super::*;

/// Scheme for MCP resource URIs.
pub const RESOURCE_SCHEME: &str = "inkwell://";
/// Pre-rename scheme, still accepted for reads (see the naming ADR).
pub(super) const LEGACY_RESOURCE_SCHEME: &str = "typecho://";

pub async fn resource_value(state: &AppState, uri: &str) -> AppResult<Value> {
    let principal = McpPrincipal::local();
    let path = uri
        .strip_prefix(RESOURCE_SCHEME)
        .or_else(|| uri.strip_prefix(LEGACY_RESOURCE_SCHEME))
        .ok_or_else(|| AppError::not_found("未知资源"))?;
    match path {
        "review/queue" => review_queue_value(state, 50).await,
        "annotations/queue" => {
            annotation_queue(state, &principal, &json!({"status": "open", "limit": 50})).await
        }
        "notes/recent" => note_list(state, &principal, &json!({"limit": 20})).await,
        _ => {
            let id = path
                .strip_prefix("content/")
                .ok_or_else(|| AppError::not_found("未知资源"))?
                .parse::<i64>()
                .map_err(|_| AppError::bad_request("无效的内容 URI"))?;
            content_get(state, &principal, &json!({"id": id})).await
        }
    }
}

pub async fn review_queue_value(state: &AppState, limit: i64) -> AppResult<Value> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let (reviews, _) = content::list(
        &state.db,
        &ContentFilter {
            review_status: Some("pending".to_string()),
            include_private: true,
            include_deleted: false,
            limit,
            offset: 0,
            order: ContentOrder::UpdatedDesc,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    let (comments, _) = comment::list_admin(&state.db, "pending", limit, 0).await?;
    Ok(json!({
        "content": reviews.iter().map(summary_json).collect::<Vec<_>>(),
        "comments": comments.iter().map(comment_with_content_json).collect::<Vec<_>>()
    }))
}
