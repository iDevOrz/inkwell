//! Home page timeline and its infinite-scroll fragments.

use super::*;

pub async fn home(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    render_home(state, session, query, false).await
}

/// Full timeline fallback for clients without JavaScript.
pub async fn home_all(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    render_home(state, session, query, true).await
}

pub(super) async fn render_home(
    state: AppState,
    session: Option<crate::auth::AuthSession>,
    query: ListQuery,
    all: bool,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let path = if all { "/timeline/all" } else { "/" };
    let ctx = PublicCtx::load(&state, session.as_ref(), path, notice, error).await?;
    let site = ctx.site.clone();
    let (posts, has_more) = if all {
        (
            content::timeline(&state.db, site.timezone_offset).await?,
            false,
        )
    } else {
        content::timeline_page(&state.db, TIMELINE_PAGE_SIZE, None, site.timezone_offset).await?
    };
    let total = content::count_timeline(&state.db).await?;
    let (timeline, timeline_more) = build_timeline(posts, None, has_more);
    let pagination = Pagination::build(path, None, 1, total, total.max(1));
    let canonical = if all {
        format!("{}/timeline/all", site.site_url)
    } else {
        format!("{}/", site.site_url)
    };

    let template = ListTemplate {
        ctx,
        heading: site.title.clone(),
        subheading: Some(site.description.clone()),
        posts: Vec::new(),
        timeline,
        timeline_more,
        pagination,
        feed_url: "/feed.xml".to_string(),
        canonical,
        search_query: None,
        is_home: true,
    };
    Ok(render(template)?.into_response())
}

pub(super) fn build_timeline(
    posts: Vec<TimelinePost>,
    last_year: Option<&str>,
    has_more: bool,
) -> (Vec<TimelineEntry>, Option<TimelineMore>) {
    let mut previous_year = last_year.map(str::to_string);
    let mut entries = Vec::with_capacity(posts.len());
    let mut cursor_ts = None;
    let mut cursor_id = None;
    let mut cursor_year = None;

    for post in posts {
        let year = post.published_label[..4].to_string();
        let year_marker = (previous_year.as_deref() != Some(year.as_str())).then(|| year.clone());
        previous_year = Some(year.clone());
        cursor_ts = Some(post.published_ts);
        cursor_id = Some(post.id);
        cursor_year = Some(year);
        entries.push(TimelineEntry {
            post,
            year: year_marker,
        });
    }

    let more = if has_more {
        match (cursor_ts, cursor_id, cursor_year) {
            (Some(before_ts), Some(before_id), Some(last_year)) => Some(TimelineMore {
                before_ts,
                before_id,
                last_year,
            }),
            _ => None,
        }
    } else {
        None
    };

    (entries, more)
}

pub async fn timeline_more(
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let before = match (query.before_ts, query.before_id) {
        (Some(published_ts), Some(id)) => Some((published_ts, id)),
        _ => None,
    };
    let (posts, has_more) =
        content::timeline_page(&state.db, TIMELINE_PAGE_SIZE, before, site.timezone_offset).await?;
    let (timeline, more) = build_timeline(posts, query.last_year.as_deref(), has_more);
    let body = render(TimelineFragmentTemplate { timeline })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-timeline-has-more",
        HeaderValue::from_static(if more.is_some() { "1" } else { "0" }),
    );
    if let Some(more) = &more {
        if let Ok(value) = HeaderValue::from_str(&more.before_ts.to_string()) {
            headers.insert("x-timeline-next-ts", value);
        }
        if let Ok(value) = HeaderValue::from_str(&more.before_id.to_string()) {
            headers.insert("x-timeline-next-id", value);
        }
        if let Ok(value) = HeaderValue::from_str(&more.last_year) {
            headers.insert("x-timeline-last-year", value);
        }
    }

    Ok((headers, body).into_response())
}
