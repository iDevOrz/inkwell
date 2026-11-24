//! Proposal (AI version) tools.

use super::*;

pub(super) async fn proposal_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let content_id = required_i64(args, "content_id")?;
    let body = required_str(args, "body")?;
    let item = proposal::create(
        &state.db,
        ProposalInput {
            content_id,
            base_revision_id: arg_i64(args, "base_revision_id"),
            expected_updated_at: arg_i64(args, "expected_updated_at"),
            title: arg_str(args, "title"),
            body,
            excerpt: arg_str(args, "excerpt"),
            summary: arg_str(args, "summary"),
            author: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn proposal_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "proposed".to_string());
    if !PROPOSAL_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的版本状态"));
    }
    let status = (status != "all").then_some(status.as_str());
    let items = proposal::list_for_content(&state.db, content_id, status).await?;
    let total = items.len();
    Ok(json!({"items": items, "total": total}))
}

pub(super) async fn proposal_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let item = proposal::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn proposal_update(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let version =
        arg_i64(args, "version").ok_or_else(|| AppError::bad_request("version 必须是整数"))?;
    let item = proposal::update(
        &state.db,
        id,
        version,
        arg_str(args, "title"),
        arg_str(args, "body"),
        arg_str(args, "excerpt"),
        arg_str(args, "summary"),
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

pub(super) async fn proposal_apply(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    // Check the base scope before any lookup so a token without it cannot
    // probe which proposal ids exist.
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let version =
        arg_i64(args, "version").ok_or_else(|| AppError::bad_request("version 必须是整数"))?;
    let proposal = proposal::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    let current = content::get_by_id(&state.db, proposal.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "published" {
        require(principal, "publish")?;
    }
    let force = arg_bool(args, "force").unwrap_or(false);
    let (proposal, content) = proposal::apply(
        &state.db,
        id,
        version,
        &principal.actor(),
        force,
        principal.has("publish"),
    )
    .await?;
    Ok(json!({
        "proposal": proposal,
        "content": {
            "id": content.id,
            "title": content.title,
            "slug": content.slug,
            "status": content.status,
            "updated_at": content.updated_at
        }
    }))
}

pub(super) async fn proposal_reject(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let note = arg_str(args, "note");
    let item = proposal::reject(&state.db, id, note.as_deref()).await?;
    Ok(serde_json::to_value(item)?)
}
