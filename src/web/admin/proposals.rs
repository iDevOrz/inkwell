//! AI proposal diffs, apply/reject, and revision diffs.

use super::*;

pub(super) async fn proposal_diff(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id, proposal_id)): Path<(String, i64, i64)>,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let item = content::get_view(&state.db, id, site.timezone_offset, true).await?;
    if item.content.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }
    let proposal = proposal::get(&state.db, proposal_id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    if proposal.content_id != id {
        return Err(AppError::not_found("版本不存在"));
    }
    let lines = build_diff(&item.content.body, &proposal.body);
    let html = content::render_body(&state.db, &proposal.body, true).await?;
    let proposal_view = proposal_view(&proposal, site.timezone_offset);

    let ctx = admin_ctx(
        &state,
        &session,
        &kind_path,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;

    Ok(render(ProposalDiffTemplate {
        ctx,
        kind: kind.to_string(),
        kind_path,
        old_title: item.content.title.clone(),
        new_title: proposal.title.clone(),
        item,
        proposal: proposal_view,
        html,
        has_mermaid: markdown::contains_mermaid(&proposal.body),
        lines,
    })?
    .into_response())
}

pub(super) async fn proposal_apply(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id, proposal_id)): Path<(String, i64, i64)>,
    Form(form): Form<ProposalApplyForm>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let item = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if item.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }
    let version = form
        .version
        .ok_or_else(|| AppError::bad_request("提案版本号缺失，请重新打开差异页面"))?;
    match proposal::apply(
        &state.db,
        proposal_id,
        version,
        &format!("human:{}", session.user.id),
        form.force.is_some(),
        true,
    )
    .await
    {
        Ok(_) => {}
        Err(AppError::Conflict(_)) => {
            return Ok(Redirect::to(&format!(
                "/admin/{kind_path}/{id}/proposals/{proposal_id}?error=proposal_changed"
            ))
            .into_response());
        }
        Err(error) => return Err(error),
    }
    Ok(Redirect::to(&format!(
        "/admin/{kind_path}/{id}/edit?notice=proposal_applied"
    ))
    .into_response())
}

pub(super) async fn proposal_reject(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id, proposal_id)): Path<(String, i64, i64)>,
    Form(form): Form<ProposalRejectForm>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let item = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if item.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }
    proposal::reject(&state.db, proposal_id, form.note.as_deref()).await?;
    Ok(Redirect::to(&format!(
        "/admin/{kind_path}/{id}/edit?notice=proposal_rejected"
    ))
    .into_response())
}

pub(super) async fn revision_diff(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id, revision_id)): Path<(String, i64, i64)>,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let item = content::get_view(&state.db, id, site.timezone_offset, true).await?;
    if item.content.kind != kind {
        return Err(AppError::not_found("内容不存在"));
    }
    let revision = revision::get(&state.db, revision_id)
        .await?
        .ok_or_else(|| AppError::not_found("修订不存在"))?;
    if revision.content_id != id {
        return Err(AppError::not_found("修订不存在"));
    }
    let previous = revision::previous(&state.db, id, revision_id).await?;
    let old_body = previous
        .as_ref()
        .map(|item| item.body.as_str())
        .unwrap_or("");
    let old_title = previous
        .as_ref()
        .map(|item| item.title.clone())
        .unwrap_or_default();
    let lines = build_diff(old_body, &revision.body);
    let previous_view = previous
        .as_ref()
        .map(|item| revision_view(item, site.timezone_offset));
    let revision_view = revision_view(&revision, site.timezone_offset);

    let ctx = admin_ctx(
        &state,
        &session,
        &kind_path,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;

    Ok(render(DiffTemplate {
        ctx,
        kind: kind.to_string(),
        kind_path,
        item,
        new_title: revision.title.clone(),
        old_title,
        revision: revision_view,
        previous: previous_view,
        lines,
    })?
    .into_response())
}

pub(super) fn build_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut old_no = 0usize;
    let mut new_no = 0usize;
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let text = change.value().trim_end_matches('\n').to_string();
        match change.tag() {
            ChangeTag::Equal => {
                old_no += 1;
                new_no += 1;
                lines.push(DiffLine {
                    kind: "equal".to_string(),
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                    text,
                });
            }
            ChangeTag::Delete => {
                old_no += 1;
                lines.push(DiffLine {
                    kind: "del".to_string(),
                    old_no: Some(old_no),
                    new_no: None,
                    text,
                });
            }
            ChangeTag::Insert => {
                new_no += 1;
                lines.push(DiffLine {
                    kind: "add".to_string(),
                    old_no: None,
                    new_no: Some(new_no),
                    text,
                });
            }
        }
    }
    lines
}
