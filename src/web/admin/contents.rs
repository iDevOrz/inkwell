//! Post, page and note editing, listing, publishing and live preview.

use super::*;

pub(super) async fn content_list(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(kind_path): Path<String>,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    let status_filter = query
        .status
        .clone()
        .filter(|value| value == "all" || statuses_for_kind(kind).contains(&value.as_str()))
        .unwrap_or_else(|| "all".to_string());
    let search_query = query.q.clone().unwrap_or_default().trim().to_string();
    let page = query.page.unwrap_or(1).max(1);
    let ctx = admin_ctx(
        &state,
        &session,
        &kind_path,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let site = ctx.site.clone();
    let page_size = 20;

    let filter = content::ContentFilter {
        kind: Some(kind.to_string()),
        status: (status_filter != "all").then(|| status_filter.clone()),
        query: (!search_query.is_empty()).then(|| search_query.clone()),
        include_deleted: true,
        include_private: true,
        limit: page_size,
        offset: (page - 1) * page_size,
        order: content::ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let counts = build_content_counts(&state.db, kind, &status_filter).await?;
    let suffix = list_query_suffix(&status_filter, &search_query);
    let pagination = Pagination::build(
        &format!("/admin/{kind_path}"),
        (!suffix.is_empty()).then_some(suffix.as_str()),
        page,
        total,
        page_size,
    );

    Ok(render(AdminListTemplate {
        ctx,
        kind: kind.to_string(),
        heading: kind_label(kind).to_string(),
        items,
        pagination,
        status_filter,
        query: search_query,
        base_path: format!("/admin/{kind_path}"),
        counts,
    })?
    .into_response())
}

pub(super) async fn build_content_counts(
    pool: &sqlx::SqlitePool,
    kind: &str,
    active: &str,
) -> AppResult<Vec<StatusCount>> {
    let counts = content::count_by_status(pool, kind).await?;
    let total: i64 = counts.values().sum();
    let mut result = vec![StatusCount {
        value: "all".to_string(),
        label: "全部".to_string(),
        count: total,
        active: active == "all",
    }];
    for status in statuses_for_kind(kind) {
        result.push(StatusCount {
            value: status.to_string(),
            label: status_label(status).to_string(),
            count: counts.get(*status).copied().unwrap_or(0),
            active: active == *status,
        });
    }
    Ok(result)
}

pub(super) fn list_query_suffix(status: &str, query: &str) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if status != "all" {
        serializer.append_pair("status", status);
    }
    if !query.is_empty() {
        serializer.append_pair("q", query);
    }
    serializer.finish()
}

pub(super) async fn content_new(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(kind_path): Path<String>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    edit_response(
        &state,
        &session,
        kind,
        &kind_path,
        None,
        ContentFormView::empty(kind),
        None,
        None,
    )
    .await
}

pub(super) async fn content_edit(
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
    let form = form_from_view(&view);
    edit_response(
        &state,
        &session,
        kind,
        &kind_path,
        Some(view),
        form,
        notice_for(query.notice.as_deref()),
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn edit_response(
    state: &AppState,
    session: &AuthSession,
    kind: &str,
    kind_path: &str,
    item: Option<crate::models::ContentView>,
    form: ContentFormView,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, kind_path, notice, error.clone()).await?;
    let site = ctx.site.clone();
    let categories = term::list(&state.db, "category")
        .await?
        .into_iter()
        .map(|term| term.term)
        .collect();
    let all_tags = term::list(&state.db, "tag")
        .await?
        .into_iter()
        .map(|term| term.term)
        .collect();

    let revisions = match &item {
        Some(item) => revision::list(&state.db, item.content.id, 10)
            .await?
            .iter()
            .map(|revision| revision_view(revision, site.timezone_offset))
            .collect(),
        None => Vec::new(),
    };

    let proposals = match &item {
        Some(item) => proposal::list_for_content(&state.db, item.content.id, None)
            .await?
            .iter()
            .map(|proposal| proposal_view(proposal, site.timezone_offset))
            .collect(),
        None => Vec::new(),
    };

    let promoted = match &item {
        Some(item) => match item.content.promoted_to_id {
            Some(post_id) => {
                content::get_by_id(&state.db, post_id)
                    .await?
                    .map(|post| PromotedInfo {
                        id: post.id,
                        title: post.title,
                        slug: post.slug,
                    })
            }
            None => None,
        },
        None => None,
    };

    let source_note = match &item {
        Some(item) => match item.content.source_note_id {
            Some(note_id) => {
                content::get_by_id(&state.db, note_id)
                    .await?
                    .map(|note| SourceNoteInfo {
                        id: note.id,
                        title: note.title,
                    })
            }
            None => None,
        },
        None => None,
    };

    let statuses = status_options(kind, &form.status);
    let visibilities = visibility_options(&form.visibility);
    let reviews = crate::web::views::review_options(&form.review_status);

    Ok(render(EditTemplate {
        ctx,
        kind: kind.to_string(),
        kind_label: kind_label(kind).to_string(),
        kind_path: kind_path.to_string(),
        content_id: item.as_ref().map(|item| item.content.id),
        is_new: item.is_none(),
        item,
        form,
        categories,
        all_tags,
        statuses,
        visibilities,
        reviews,
        revisions,
        proposals,
        promoted,
        source_note,
        error,
    })?
    .into_response())
}

pub(super) fn form_from_view(view: &crate::models::ContentView) -> ContentFormView {
    ContentFormView {
        title: view.content.title.clone(),
        slug: view.content.slug.clone(),
        body: view.content.body.clone(),
        expected_updated_at: Some(view.content.updated_at),
        excerpt: view.content.excerpt.clone().unwrap_or_default(),
        status: view.content.status.clone(),
        visibility: view.content.visibility.clone(),
        review_status: view.content.review_status.clone(),
        review_notes: view.content.review_notes.clone().unwrap_or_default(),
        allow_comments: view.content.allow_comments != 0,
        category: view
            .categories
            .first()
            .map(|term| term.slug.clone())
            .unwrap_or_default(),
        tags: view
            .tags
            .iter()
            .map(|term| term.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

pub(super) fn form_from_submission(form: &ContentForm, kind: &str) -> ContentFormView {
    ContentFormView {
        title: form.title.clone(),
        slug: form.slug.clone().unwrap_or_default(),
        body: form.body.clone(),
        expected_updated_at: form.expected_updated_at,
        excerpt: form.excerpt.clone().unwrap_or_default(),
        status: form
            .status
            .clone()
            .filter(|value| is_valid_status(kind, value))
            .unwrap_or_else(|| default_status(kind).to_string()),
        visibility: form
            .visibility
            .clone()
            .unwrap_or_else(|| "public".to_string()),
        review_status: form
            .review_status
            .clone()
            .filter(|value| REVIEW_STATUSES.contains(&value.as_str()))
            .unwrap_or_else(|| "none".to_string()),
        review_notes: form.review_notes.clone().unwrap_or_default(),
        allow_comments: form.allow_comments.is_some(),
        category: form.category.clone().unwrap_or_default(),
        tags: form.tags.clone().unwrap_or_default(),
    }
}

pub(super) fn resolved_status(kind: &str, form: &ContentForm) -> (String, String) {
    let action = form.action.as_deref().unwrap_or("save");
    let mut status = form
        .status
        .clone()
        .filter(|value| is_valid_status(kind, value))
        .unwrap_or_else(|| default_status(kind).to_string());
    let mut review_status = form
        .review_status
        .clone()
        .filter(|value| REVIEW_STATUSES.contains(&value.as_str()))
        .unwrap_or_else(|| "none".to_string());

    if action == "publish" && kind != "note" {
        status = "published".to_string();
        review_status = "approved".to_string();
    } else if action == "submit_review" {
        review_status = "pending".to_string();
    }

    (status, review_status)
}

pub(super) fn parse_tags(value: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .unwrap_or("")
        .split([',', '，'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| {
            let value = value.to_string();
            seen.insert(value.clone()).then_some(value)
        })
        .collect()
}

pub(super) async fn content_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(kind_path): Path<String>,
    Form(form): Form<ContentForm>,
) -> AppResult<Response> {
    let Some(kind) = kind_from_path(&kind_path) else {
        return Err(AppError::not_found("页面不存在"));
    };
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let (status, review_status) = resolved_status(kind, &form);
    let form_view = form_from_submission(&form, kind);
    let input = content::ContentInput {
        kind: kind.to_string(),
        title: form.title.clone(),
        slug: form.slug.clone().filter(|value| !value.trim().is_empty()),
        body: form.body.clone(),
        excerpt: form
            .excerpt
            .clone()
            .filter(|value| !value.trim().is_empty()),
        status: Some(status.clone()),
        visibility: Some(
            form.visibility
                .clone()
                .unwrap_or_else(|| "public".to_string()),
        ),
        review_status: Some(review_status),
        review_notes: form
            .review_notes
            .clone()
            .filter(|value| !value.trim().is_empty()),
        allow_comments: Some(form.allow_comments.is_some()),
        category: Some(form.category.clone().unwrap_or_default()),
        tags: parse_tags(form.tags.as_deref()),
        author_id: session.user.id,
        actor: format!("human:{}", session.user.id),
        source_note_id: None,
    };

    match content::create(&state.db, input).await {
        Ok(id) => Ok(
            Redirect::to(&format!("/admin/{kind_path}/{id}/edit?notice=created")).into_response(),
        ),
        Err(error) => {
            edit_response(
                &state,
                &session,
                kind,
                &kind_path,
                None,
                form_view,
                None,
                Some(error.message()),
            )
            .await
        }
    }
}

pub(super) async fn content_update(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Form(form): Form<ContentForm>,
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

    let action = form.action.as_deref().unwrap_or("save");
    let (status, review_status) = resolved_status(kind, &form);
    let patch = content::ContentPatch {
        title: Some(form.title.clone()),
        slug: form.slug.clone().filter(|value| !value.trim().is_empty()),
        body: Some(form.body.clone()),
        excerpt: Some(form.excerpt.clone().unwrap_or_default()),
        status: Some(status),
        visibility: Some(
            form.visibility
                .clone()
                .unwrap_or_else(|| "public".to_string()),
        ),
        review_status: Some(review_status),
        review_notes: Some(form.review_notes.clone().unwrap_or_default()),
        allow_comments: Some(form.allow_comments.is_some()),
        category: Some(form.category.clone().unwrap_or_default()),
        tags: Some(parse_tags(form.tags.as_deref())),
        actor: format!("human:{}", session.user.id),
        expected_updated_at: form.expected_updated_at,
        revision_note: Some(
            if action == "publish" {
                "发布"
            } else if action == "submit_review" {
                "提交 review"
            } else {
                "更新"
            }
            .to_string(),
        ),
    };

    match content::update(&state.db, id, patch).await {
        Ok(updated) => {
            let notice = if action == "publish" && updated.status == "published" {
                "published"
            } else {
                "saved"
            };
            Ok(
                Redirect::to(&format!("/admin/{kind_path}/{id}/edit?notice={notice}"))
                    .into_response(),
            )
        }
        Err(error) => {
            let site = settings::load(&state.db, &state.config.site_url).await?;
            let view = content::get_view(&state.db, id, site.timezone_offset, true).await?;
            edit_response(
                &state,
                &session,
                kind,
                &kind_path,
                Some(view),
                form_from_submission(&form, kind),
                None,
                Some(error.message()),
            )
            .await
        }
    }
}

pub(super) async fn content_delete(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Form(form): Form<ActionForm>,
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

    if form.permanent.as_deref() == Some("1") && current.status == "trash" {
        content::delete_permanent(&state.db, id).await?;
        Ok(Redirect::to(&format!("/admin/{kind_path}?notice=permanently_deleted")).into_response())
    } else {
        content::trash(&state.db, id, &format!("human:{}", session.user.id)).await?;
        Ok(Redirect::to(&format!("/admin/{kind_path}?notice=deleted")).into_response())
    }
}

pub(super) async fn content_restore(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Form(form): Form<ActionForm>,
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
    content::restore(&state.db, id, &format!("human:{}", session.user.id)).await?;
    Ok(Redirect::to(&format!("/admin/{kind_path}?notice=restored")).into_response())
}

pub(super) async fn content_publish(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path((kind_path, id)): Path<(String, i64)>,
    Form(form): Form<ActionForm>,
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
    content::publish(&state.db, id, &format!("human:{}", session.user.id)).await?;
    Ok(Redirect::to(&format!("/admin/{kind_path}/{id}/edit?notice=published")).into_response())
}

pub(super) async fn note_promote(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<PromoteForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let note = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("笔记不存在"))?;
    if note.kind != "note" {
        return Err(AppError::bad_request("只有笔记可以转为文章"));
    }

    let status = if form.status.as_deref() == Some("published") {
        "published"
    } else {
        "draft"
    };
    let result = content::promote_note(
        &state.db,
        id,
        content::PromoteOptions {
            title: form.title.filter(|value| !value.trim().is_empty()),
            slug: form.slug.filter(|value| !value.trim().is_empty()),
            status: status.to_string(),
            sync: form.sync.is_some(),
            can_publish: true,
            actor: format!("human:{}", session.user.id),
        },
    )
    .await?;

    let notice = if result.created { "promoted" } else { "synced" };
    Ok(Redirect::to(&format!("/admin/posts/{}/edit?notice={notice}", result.id)).into_response())
}

pub(super) async fn preview(
    AdminSession(session): AdminSession,
    headers: HeaderMap,
    Json(request): Json<PreviewRequest>,
) -> AppResult<Html<String>> {
    let token = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    verify_csrf(&session, token)?;
    let html = markdown::render_markdown(&request.body, &HashMap::new());
    Ok(Html(html))
}
