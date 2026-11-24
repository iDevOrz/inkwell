//! Argument parsing and scope checks shared by all tools.

use super::*;

pub(super) async fn load_view(state: &AppState, args: &Value) -> AppResult<ContentView> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let content = if let Some(id) = arg_i64(args, "id") {
        content::get_by_id(&state.db, id).await?
    } else if let Some(slug) = arg_str(args, "slug") {
        content::get_by_slug(&state.db, &slug).await?
    } else {
        return Err(AppError::bad_request("请提供 id 或 slug"));
    }
    .ok_or_else(|| AppError::not_found("内容不存在"))?;

    content::get_view(&state.db, content.id, site.timezone_offset, true).await
}

pub(super) fn build_patch(args: &Value, principal: &McpPrincipal) -> ContentPatch {
    ContentPatch {
        title: arg_str(args, "title"),
        slug: arg_str(args, "slug"),
        body: arg_str(args, "body"),
        excerpt: arg_str(args, "excerpt"),
        status: arg_str(args, "status"),
        visibility: arg_str(args, "visibility"),
        review_status: arg_str(args, "review_status"),
        review_notes: args
            .get("review_notes")
            .map(|_| arg_str(args, "review_notes").unwrap_or_default()),
        allow_comments: arg_bool(args, "allow_comments"),
        category: args
            .get("category")
            .map(|_| arg_str(args, "category").unwrap_or_default()),
        tags: args.get("tags").map(|_| arg_string_array(args, "tags")),
        actor: principal.actor(),
        expected_updated_at: arg_i64(args, "expected_updated_at"),
        revision_note: Some("MCP 更新".to_string()),
    }
}

pub(super) async fn admin_user_id(pool: &SqlitePool) -> AppResult<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::internal("没有可用的管理员账号"))
}

pub(super) fn require(principal: &McpPrincipal, scope: &str) -> AppResult<()> {
    if principal.has(scope) {
        Ok(())
    } else {
        Err(AppError::forbidden(format!("当前 token 缺少 {scope} 权限")))
    }
}

pub(super) fn check_write_fields(principal: &McpPrincipal, args: &Value) -> AppResult<()> {
    if arg_str(args, "status").as_deref() == Some("published") {
        require(principal, "publish")?;
    }
    if matches!(
        arg_str(args, "review_status").as_deref(),
        Some("approved") | Some("changes_requested")
    ) {
        require(principal, "review")?;
    }
    Ok(())
}

pub(super) fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

pub(super) fn required_str(args: &Value, key: &str) -> AppResult<String> {
    arg_str(args, key).ok_or_else(|| AppError::bad_request(format!("缺少参数 {key}")))
}

pub(super) fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

pub(super) fn required_i64(args: &Value, key: &str) -> AppResult<i64> {
    arg_i64(args, key).ok_or_else(|| AppError::bad_request(format!("缺少参数 {key}")))
}

pub(super) fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|value| {
        value
            .as_bool()
            .or_else(|| value.as_str().map(crate::util::parse_bool))
    })
}

pub(super) fn arg_string_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
