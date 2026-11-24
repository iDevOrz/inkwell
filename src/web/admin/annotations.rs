//! Annotation pages and actions.

use super::*;

pub(super) async fn annotate_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let view = content::get_view(&state.db, id, site.timezone_offset, true).await?;
    if view.content.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }

    let rows = annotation::list_for_content(&state.db, id, None).await?;
    let general_count = rows
        .iter()
        .filter(|item| item.annotation.kind == "general")
        .count();
    let inline_count = rows.len() - general_count;
    let annotations = rows
        .into_iter()
        .map(|item| annotation_view(item, site.timezone_offset))
        .collect();
    let proposals = proposal::list_for_content(&state.db, id, None)
        .await?
        .iter()
        .map(|proposal| proposal_view(proposal, site.timezone_offset))
        .collect();

    let ctx = admin_ctx(
        &state,
        &session,
        &kind_path,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;

    Ok(render(AnnotateTemplate {
        ctx,
        kind: kind.to_string(),
        kind_path,
        item: view,
        annotations,
        proposals,
        general_count,
        inline_count,
    })?
    .into_response())
}

pub(super) fn annotation_view(
    item: crate::models::AnnotationWithReplies,
    timezone_offset: i32,
) -> AnnotationView {
    let annotation = item.annotation;
    let replies = item
        .replies
        .into_iter()
        .map(|reply| AnnotationReplyView {
            id: reply.id,
            author: reply.author.clone(),
            author_label: actor_label(&reply.author),
            body: reply.body,
            created_label: format_datetime(reply.created_at, timezone_offset),
        })
        .collect();
    AnnotationView {
        id: annotation.id,
        kind: annotation.kind,
        quote: annotation.quote,
        prefix: annotation.prefix,
        suffix: annotation.suffix,
        start_offset: annotation.start_offset,
        end_offset: annotation.end_offset,
        comment: annotation.comment,
        status: annotation.status.clone(),
        status_label: annotation_status_label(&annotation.status).to_string(),
        author: annotation.author.clone(),
        author_label: actor_label(&annotation.author),
        created_label: format_datetime(annotation.created_at, timezone_offset),
        replies,
    }
}

pub(super) fn proposal_view(
    proposal: &crate::models::ContentProposal,
    timezone_offset: i32,
) -> ProposalView {
    ProposalView {
        id: proposal.id,
        version: proposal.version,
        content_id: proposal.content_id,
        title: proposal.title.clone(),
        summary: proposal.summary.clone().unwrap_or_default(),
        status: proposal.status.clone(),
        status_label: crate::models::proposal_status_label(&proposal.status).to_string(),
        author_label: actor_label(&proposal.author),
        created_label: format_datetime(proposal.created_at, timezone_offset),
        resolved_label: proposal
            .resolved_at
            .map(|value| format_datetime(value, timezone_offset))
            .unwrap_or_default(),
        decision_note: proposal.decision_note.clone().unwrap_or_default(),
        is_proposed: proposal.status == "proposed",
    }
}

pub(super) async fn annotation_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Form(form): Form<AnnotationForm>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }

    let back = format!("/admin/{kind_path}/{id}/annotate");
    if form.comment.trim().is_empty() {
        return Ok(Redirect::to(&format!("{back}?error=invalid")).into_response());
    }
    if form.kind == "inline"
        && form
            .quote
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Ok(Redirect::to(&format!("{back}?error=invalid")).into_response());
    }

    let start_offset = form
        .start_offset
        .as_deref()
        .and_then(|value| value.parse().ok());
    let end_offset = form
        .end_offset
        .as_deref()
        .and_then(|value| value.parse().ok());
    annotation::create(
        &state.db,
        AnnotationInput {
            content_id: id,
            kind: form.kind,
            quote: form.quote,
            prefix: form.prefix,
            suffix: form.suffix,
            start_offset,
            end_offset,
            comment: form.comment,
            author: format!("human:{}", session.user.id),
        },
    )
    .await?;

    Ok(Redirect::to(&format!("{back}?notice=annotation_created")).into_response())
}

pub(super) async fn annotation_reply(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<AnnotationReplyForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let current = annotation::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    let content = content::get_by_id(&state.db, current.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let back = format!(
        "/admin/{}/{}/annotate",
        kind_to_path(&content.kind),
        content.id
    );
    if form.body.trim().is_empty() {
        return Ok(Redirect::to(&format!("{back}?error=invalid")).into_response());
    }
    annotation::reply(
        &state.db,
        id,
        &format!("human:{}", session.user.id),
        &form.body,
        None,
    )
    .await?;
    Ok(Redirect::to(&format!("{back}?notice=annotation_replied")).into_response())
}

pub(super) async fn annotation_resolve(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<AnnotationActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    annotation_status_action(&state, id, "resolved", "annotation_resolved").await
}

pub(super) async fn annotation_reopen(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<AnnotationActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    annotation_status_action(&state, id, "open", "annotation_reopened").await
}

pub(super) async fn annotation_status_action(
    state: &AppState,
    id: i64,
    status: &str,
    notice: &str,
) -> AppResult<Response> {
    let current = annotation::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    let content = content::get_by_id(&state.db, current.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    annotation::set_status(&state.db, id, status).await?;
    Ok(Redirect::to(&format!(
        "/admin/{}/{}/annotate?notice={notice}",
        kind_to_path(&content.kind),
        content.id
    ))
    .into_response())
}

pub(super) async fn annotations_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let status_filter = query
        .status
        .clone()
        .filter(|value| matches!(value.as_str(), "open" | "addressed" | "all"))
        .unwrap_or_else(|| "open".to_string());
    let rows = annotation::queue(&state.db, &status_filter, 200).await?;
    let open_count = annotation::count_status(&state.db, "open").await?;
    let addressed_count = annotation::count_status(&state.db, "addressed").await?;
    let timezone_offset = settings::load(&state.db, &state.config.site_url)
        .await?
        .timezone_offset;
    let annotations = rows
        .into_iter()
        .map(|item| {
            let kind_path = kind_to_path(&item.content_kind);
            AnnotationQueueView {
                id: item.annotation.id,
                content_id: item.annotation.content_id,
                content_title: item.content_title,
                content_kind: kind_label(&item.content_kind).to_string(),
                content_status: item.content_status,
                annotate_url: format!("/admin/{kind_path}/{}/annotate", item.annotation.content_id),
                kind: item.annotation.kind,
                quote: item.annotation.quote,
                comment: item.annotation.comment,
                status: item.annotation.status.clone(),
                status_label: annotation_status_label(&item.annotation.status).to_string(),
                author_label: actor_label(&item.annotation.author),
                created_label: format_datetime(item.annotation.created_at, timezone_offset),
            }
        })
        .collect();

    let ctx = admin_ctx(
        &state,
        &session,
        "annotations",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;

    Ok(render(AnnotationsTemplate {
        ctx,
        annotations,
        status_filter,
        open_count,
        addressed_count,
    })?
    .into_response())
}
