//! Review queue and verdict tools.

use super::*;

pub(super) async fn review_queue(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 100);
    review_queue_value(state, limit).await
}

pub(super) async fn review_submit(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let notes = arg_str(args, "notes");
    let content = content::set_review(
        &state.db,
        id,
        "pending",
        notes.as_deref(),
        &principal.actor(),
        principal.has("publish"),
    )
    .await?;
    Ok(json!({
        "id": content.id,
        "review_status": content.review_status,
        "review_notes": content.review_notes
    }))
}

pub(super) async fn review_decide(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "review")?;
    let id = required_i64(args, "id")?;
    let verdict = required_str(args, "verdict")?;
    if verdict != "approved" && verdict != "changes_requested" {
        return Err(AppError::bad_request(
            "verdict 必须是 approved 或 changes_requested",
        ));
    }
    let publish = arg_bool(args, "publish").unwrap_or(false);
    let notes = arg_str(args, "notes");
    if publish {
        require(principal, "publish")?;
        if verdict != "approved" {
            return Err(AppError::bad_request("只有 approved 才能同时发布"));
        }
        content::approve_and_publish(&state.db, id, notes.as_deref(), &principal.actor()).await?;
    } else {
        content::set_review(
            &state.db,
            id,
            &verdict,
            notes.as_deref(),
            &principal.actor(),
            principal.has("publish"),
        )
        .await?;
    }

    let updated = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::internal("review 后读取失败"))?;
    Ok(json!({
        "id": updated.id,
        "review_status": updated.review_status,
        "review_notes": updated.review_notes,
        "status": updated.status,
        "published_at": updated.published_at
    }))
}
