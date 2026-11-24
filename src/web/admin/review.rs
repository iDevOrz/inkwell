//! The review queue and review verdicts.

use super::*;

pub(super) async fn review_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let ctx = admin_ctx(
        &state,
        &session,
        "review",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let site = ctx.site.clone();
    let (reviews, _) = content::list(
        &state.db,
        &content::ContentFilter {
            review_status: Some("pending".to_string()),
            include_deleted: false,
            include_private: true,
            limit: 100,
            order: content::ContentOrder::UpdatedDesc,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    let (comments, _) = comment::list_admin(&state.db, "pending", 50, 0).await?;
    let comments = comments
        .iter()
        .map(|row| CommentAdminView::from_row(row, site.timezone_offset))
        .collect();

    Ok(render(ReviewTemplate {
        ctx,
        reviews,
        comments,
    })?
    .into_response())
}

pub(super) async fn review_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ReviewForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    if form.verdict != "approved" && form.verdict != "changes_requested" {
        return Err(AppError::bad_request("非法的 review 结论"));
    }
    let actor = format!("human:{}", session.user.id);
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if form.publish.is_some() && form.verdict == "approved" && current.kind != "note" {
        content::approve_and_publish(&state.db, id, form.notes.as_deref(), &actor).await?;
    } else {
        content::set_review(
            &state.db,
            id,
            &form.verdict,
            form.notes.as_deref(),
            &actor,
            true,
        )
        .await?;
    }

    Ok(Redirect::to("/admin/review?notice=review_saved").into_response())
}
