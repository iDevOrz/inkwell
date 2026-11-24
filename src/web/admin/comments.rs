//! Comment moderation.

use super::*;

pub(super) async fn comments_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let status_filter = query
        .status
        .clone()
        .filter(|value| value == "all" || COMMENT_STATUSES.contains(&value.as_str()))
        .unwrap_or_else(|| "pending".to_string());
    let page = query.page.unwrap_or(1).max(1);
    let ctx = admin_ctx(
        &state,
        &session,
        "comments",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let site = ctx.site.clone();
    let page_size = 20;
    let (rows, total) =
        comment::list_admin(&state.db, &status_filter, page_size, (page - 1) * page_size).await?;
    let comments = rows
        .iter()
        .map(|row| CommentAdminView::from_row(row, site.timezone_offset))
        .collect();
    let counts = build_comment_counts(&state.db, &status_filter).await?;
    let suffix = if status_filter == "all" {
        String::new()
    } else {
        format!("status={status_filter}")
    };
    let pagination = Pagination::build(
        "/admin/comments",
        (!suffix.is_empty()).then_some(suffix.as_str()),
        page,
        total,
        page_size,
    );

    Ok(render(CommentsTemplate {
        ctx,
        comments,
        pagination,
        status_filter,
        counts,
    })?
    .into_response())
}

pub(super) async fn build_comment_counts(
    pool: &sqlx::SqlitePool,
    active: &str,
) -> AppResult<Vec<StatusCount>> {
    let counts = comment::count_by_status(pool).await?;
    let total: i64 = counts.values().sum();
    let mut result = vec![StatusCount {
        value: "all".to_string(),
        label: "全部".to_string(),
        count: total,
        active: active == "all",
    }];
    for status in COMMENT_STATUSES {
        result.push(StatusCount {
            value: status.to_string(),
            label: comment_status_label(status).to_string(),
            count: counts.get(*status).copied().unwrap_or(0),
            active: active == *status,
        });
    }
    Ok(result)
}

pub(super) async fn comment_status(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<CommentActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    match form.action.as_str() {
        "approve" => comment::moderate(&state.db, id, "approved").await?,
        "pending" => comment::moderate(&state.db, id, "pending").await?,
        "spam" => comment::moderate(&state.db, id, "spam").await?,
        "trash" => comment::moderate(&state.db, id, "trash").await?,
        "delete" => comment::delete(&state.db, id).await?,
        _ => return Err(AppError::bad_request("未知操作")),
    }
    Ok(Redirect::to("/admin/comments?notice=moderated").into_response())
}
