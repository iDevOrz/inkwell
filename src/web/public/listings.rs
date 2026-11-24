//! Category, tag, search and date archive listings.

use super::*;

pub async fn category(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/categories/{slug}"),
        notice,
        error,
    )
    .await?;
    let site = ctx.site.clone();
    let Some(term) = term::get_public_by_slug(&state.db, "category", &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/categories/{slug}")).await;
    };

    let page = query.page.unwrap_or(1).max(1);
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        term_id: Some(term.id),
        limit: site.page_size,
        offset: (page - 1) * site.page_size,
        ..Default::default()
    };
    let (posts, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let pagination = Pagination::build(
        &format!("/categories/{slug}"),
        None,
        page,
        total,
        site.page_size,
    );

    let template = ListTemplate {
        ctx,
        heading: term.name.clone(),
        subheading: term.description.clone(),
        posts,
        timeline: Vec::new(),
        timeline_more: None,
        pagination,
        feed_url: "/feed.xml".to_string(),
        canonical: format!("{}/categories/{}", site.site_url, slug),
        search_query: None,
        is_home: false,
    };
    Ok(render(template)?.into_response())
}

pub async fn tag(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/tags/{slug}"),
        notice,
        error,
    )
    .await?;
    let site = ctx.site.clone();
    let Some(term) = term::get_public_by_slug(&state.db, "tag", &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/tags/{slug}")).await;
    };

    let page = query.page.unwrap_or(1).max(1);
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        term_id: Some(term.id),
        limit: site.page_size,
        offset: (page - 1) * site.page_size,
        ..Default::default()
    };
    let (posts, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let pagination = Pagination::build(&format!("/tags/{slug}"), None, page, total, site.page_size);

    let template = ListTemplate {
        ctx,
        heading: format!("#{}", term.name),
        subheading: term.description.clone(),
        posts,
        timeline: Vec::new(),
        timeline_more: None,
        pagination,
        feed_url: "/feed.xml".to_string(),
        canonical: format!("{}/tags/{}", site.site_url, slug),
        search_query: None,
        is_home: false,
    };
    Ok(render(template)?.into_response())
}

pub async fn search(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let search_query = query.q.clone().unwrap_or_default().trim().to_string();
    let ctx = PublicCtx::load(&state, session.as_ref(), "/search", notice, error).await?;
    let site = ctx.site.clone();
    let page = query.page.unwrap_or(1).max(1);
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        query: (!search_query.is_empty()).then(|| search_query.clone()),
        limit: site.page_size,
        offset: (page - 1) * site.page_size,
        ..Default::default()
    };
    let (posts, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let encoded = utf8_percent_encode(&search_query, NON_ALPHANUMERIC).to_string();
    let query_suffix = (!search_query.is_empty()).then(|| format!("q={encoded}"));
    let pagination = Pagination::build(
        "/search",
        query_suffix.as_deref(),
        page,
        total,
        site.page_size,
    );

    let heading = if search_query.is_empty() {
        "搜索".to_string()
    } else {
        format!("搜索：{search_query}")
    };
    let template = ListTemplate {
        ctx,
        heading,
        subheading: None,
        posts,
        timeline: Vec::new(),
        timeline_more: None,
        pagination,
        feed_url: "/feed.xml".to_string(),
        canonical: if search_query.is_empty() {
            format!("{}/search", site.site_url)
        } else {
            format!("{}/search?q={encoded}", site.site_url)
        },
        search_query: Some(search_query),
        is_home: false,
    };
    Ok(render(template)?.into_response())
}

pub async fn archive(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path((year, month)): Path<(i32, u32)>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/archive/{year}/{month}"),
        notice,
        error,
    )
    .await?;
    let site = ctx.site.clone();

    if !(1..=12).contains(&month) {
        return not_found_page(&state, session.as_ref(), "/").await;
    }

    let start = timezone(site.timezone_offset)
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| AppError::bad_request("非法的日期"))?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = timezone(site.timezone_offset)
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| AppError::bad_request("非法的日期"))?;

    let page = query.page.unwrap_or(1).max(1);
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        published_after: Some(start.timestamp()),
        published_before: Some(end.timestamp()),
        limit: site.page_size,
        offset: (page - 1) * site.page_size,
        ..Default::default()
    };
    let (posts, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let pagination = Pagination::build(
        &format!("/archive/{year}/{month}"),
        None,
        page,
        total,
        site.page_size,
    );

    let template = ListTemplate {
        ctx,
        heading: format_month(start.timestamp(), site.timezone_offset),
        subheading: None,
        posts,
        timeline: Vec::new(),
        timeline_more: None,
        pagination,
        feed_url: "/feed.xml".to_string(),
        canonical: format!("{}/archive/{}/{}", site.site_url, year, month),
        search_query: None,
        is_home: false,
    };
    Ok(render(template)?.into_response())
}
