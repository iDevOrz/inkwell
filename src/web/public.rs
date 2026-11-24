use axum::Form;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::TimeZone;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use std::net::SocketAddr;

use crate::auth::{AuthSession, OptionalUser};
use crate::error::{AppError, AppResult};
use crate::models::{Content, Pagination, TimelinePost};
use crate::repo::{comment, content, media, term};
use crate::state::AppState;
use crate::util::{
    client_ip, format_date, format_iso, format_month, now_ts, path_segment, timezone, user_agent,
    xml_escape,
};

use super::templates::{
    CommentFormView, ListTemplate, NotFoundTemplate, PageTemplate, PostTemplate, TimelineEntry,
    TimelineFragmentTemplate, TimelineMore, render,
};
use super::views::{PostLink, PublicCtx, error_for, flatten_comments, notice_for};

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub q: Option<String>,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub reply: Option<i64>,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewCommentForm {
    pub author: String,
    pub email: Option<String>,
    pub url: Option<String>,
    pub body: String,
    pub parent_id: Option<i64>,
    pub website: Option<String>,
}

const TIMELINE_PAGE_SIZE: i64 = 100;

#[derive(Debug, Deserialize, Default)]
pub struct TimelineQuery {
    pub before_ts: Option<i64>,
    pub before_id: Option<i64>,
    pub last_year: Option<String>,
}

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

async fn render_home(
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

fn build_timeline(
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

/// Drafts and private content are visible only to back-office roles.
fn can_view(content: &Content, session: Option<&AuthSession>) -> bool {
    content.is_public() || session.is_some_and(AuthSession::can_manage_content)
}

pub async fn post(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let Some(content_row) = content::get_by_slug(&state.db, &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/archives/{slug}")).await;
    };
    if content_row.kind != "post" || !can_view(&content_row, session.as_ref()) {
        return not_found_page(&state, session.as_ref(), &format!("/archives/{slug}")).await;
    }

    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/archives/{slug}"),
        notice,
        error,
    )
    .await?;
    let site = ctx.site.clone();
    let post = content::get_view(
        &state.db,
        content_row.id,
        site.timezone_offset,
        session
            .as_ref()
            .is_some_and(AuthSession::can_manage_content),
    )
    .await?;

    let comment_rows = comment::list_for_content(&state.db, post.content.id, "approved").await?;
    let comment_count = comment_rows.len() as i64;
    let comment_tree = comment::build_tree(comment_rows);
    let comments = flatten_comments(comment_tree, 0, site.timezone_offset);

    let reply_to = query.reply.and_then(|reply_id| {
        comments
            .iter()
            .find(|item| item.comment.id == reply_id)
            .cloned()
    });
    let parent_id = reply_to.as_ref().map(|item| item.comment.id).unwrap_or(0);

    let comment_form = CommentFormView {
        content_id: post.content.id,
        parent_id,
        author: String::new(),
        email: String::new(),
        url: String::new(),
        body: String::new(),
        reply_author: reply_to.as_ref().map(|item| item.comment.author.clone()),
    };

    let (older, newer) = content::adjacent_posts(&state.db, &post.content).await?;
    let prev = older.map(|(title, slug)| PostLink { title, slug });
    let next = newer.map(|(title, slug)| PostLink { title, slug });

    let template = PostTemplate {
        ctx,
        post,
        comments,
        comment_count,
        comment_form,
        reply_to,
        canonical: format!("{}/archives/{}", site.site_url, slug),
        prev,
        next,
    };
    Ok(render(template)?.into_response())
}

pub async fn page(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
) -> AppResult<Response> {
    let Some(content_row) = content::get_by_slug(&state.db, &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/pages/{slug}")).await;
    };
    if content_row.kind != "page" || !can_view(&content_row, session.as_ref()) {
        return not_found_page(&state, session.as_ref(), &format!("/pages/{slug}")).await;
    }

    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/pages/{slug}"),
        None,
        None,
    )
    .await?;
    let site = ctx.site.clone();
    let page = content::get_view(
        &state.db,
        content_row.id,
        site.timezone_offset,
        session
            .as_ref()
            .is_some_and(AuthSession::can_manage_content),
    )
    .await?;
    let template = PageTemplate {
        ctx,
        page,
        canonical: format!("{}/pages/{}", site.site_url, slug),
    };
    Ok(render(template)?.into_response())
}

pub async fn comment(
    State(state): State<AppState>,
    Path(content_id): Path<i64>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<NewCommentForm>,
) -> AppResult<Response> {
    const MAX_COMMENT_DEPTH: i64 = 8;

    let Some(content_row) = content::get_by_id(&state.db, content_id).await? else {
        return Err(AppError::not_found("内容不存在"));
    };
    if content_row.kind != "post" || !content_row.is_public() {
        return Err(AppError::not_found("内容不存在"));
    }
    if content_row.allow_comments == 0 {
        return Err(AppError::bad_request("该内容已关闭评论"));
    }
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    if !site.comments_enabled {
        return Err(AppError::bad_request("评论功能已关闭"));
    }

    let target = format!("{}#comments", content_row.public_url());
    if !form.website.as_deref().unwrap_or("").trim().is_empty() {
        // Honeypot filled: pretend success.
        return Ok(axum::response::Redirect::to(&target).into_response());
    }

    let ip = client_ip(&headers, peer, state.config.trust_proxy_headers);
    let rate_limit_key = format!("comment:{ip}");
    if !state.comment_limiter.try_acquire(&rate_limit_key) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            "评论提交过于频繁，请稍后再试。",
        )
            .into_response());
    }

    if site.comments_require_email
        && form
            .email
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Ok(axum::response::Redirect::to(&format!(
            "{}?comment=error#comments",
            content_row.public_url()
        ))
        .into_response());
    }

    let mut parent_id = form.parent_id.unwrap_or(0).max(0);
    if parent_id > 0 {
        match comment::get(&state.db, parent_id).await? {
            Some(parent) if parent.content_id == content_row.id && parent.status == "approved" => {
                if comment::reply_depth(&state.db, parent_id).await? >= MAX_COMMENT_DEPTH {
                    return Err(AppError::bad_request("评论回复层级不能超过 8 层"));
                }
            }
            _ => parent_id = 0,
        }
    }

    let status = if site.comments_moderation {
        "pending"
    } else {
        "approved"
    };

    let result = comment::create(
        &state.db,
        comment::NewComment {
            content_id: content_row.id,
            parent_id,
            author: form.author,
            email: form.email,
            url: form.url,
            body: form.body,
            status: status.to_string(),
            ip: Some(ip),
            user_agent: user_agent(&headers),
        },
    )
    .await;

    match result {
        Ok(_) => {
            let code = if site.comments_moderation {
                "pending"
            } else {
                "ok"
            };
            Ok(axum::response::Redirect::to(&format!(
                "{}?comment={}#comments",
                content_row.public_url(),
                code
            ))
            .into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "comment submission failed");
            Ok(axum::response::Redirect::to(&format!(
                "{}?comment=error#comments",
                content_row.public_url()
            ))
            .into_response())
        }
    }
}

pub async fn feed_rss(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        limit: 20,
        ..Default::default()
    };
    let (posts, _) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let mut items = String::new();

    for post in &posts {
        let view =
            content::get_view(&state.db, post.content.id, site.timezone_offset, false).await?;
        let description = if site.feed_full_text {
            view.html
        } else {
            view.excerpt_text
        };
        let link = format!(
            "{}/archives/{}",
            site.site_url,
            path_segment(&post.content.slug)
        );
        let published = post.content.published_at.unwrap_or(post.content.created_at);
        items.push_str(&format!(
            "<item><title>{}</title><link>{}</link><guid isPermaLink=\"true\">{}</guid>\
             <pubDate>{}</pubDate><description>{}</description></item>",
            xml_escape(&post.content.title),
            xml_escape(&link),
            xml_escape(&link),
            format_date(published, site.timezone_offset, "%a, %d %b %Y %H:%M:%S %z"),
            xml_escape(&description),
        ));
    }

    let last_build = posts
        .first()
        .map(|post| post.content.updated_at)
        .unwrap_or_else(now_ts);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <rss version=\"2.0\"><channel><title>{}</title><link>{}/</link>\
         <description>{}</description><language>zh-cn</language>\
         <lastBuildDate>{}</lastBuildDate>{}</channel></rss>",
        xml_escape(&site.title),
        xml_escape(&site.site_url),
        xml_escape(&site.description),
        format_date(last_build, site.timezone_offset, "%a, %d %b %Y %H:%M:%S %z"),
        items,
    );

    Ok(([(CONTENT_TYPE, "application/rss+xml; charset=utf-8")], xml).into_response())
}

pub async fn feed_atom(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        limit: 20,
        ..Default::default()
    };
    let (posts, _) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let mut entries = String::new();

    for post in &posts {
        let view =
            content::get_view(&state.db, post.content.id, site.timezone_offset, false).await?;
        let content = if site.feed_full_text {
            view.html
        } else {
            view.excerpt_text
        };
        let link = format!(
            "{}/archives/{}",
            site.site_url,
            path_segment(&post.content.slug)
        );
        let published = post.content.published_at.unwrap_or(post.content.created_at);
        entries.push_str(&format!(
            "<entry><title>{}</title><id>{}</id><link href=\"{}\"/>\
             <updated>{}</updated><published>{}</published>\
             <content type=\"html\">{}</content></entry>",
            xml_escape(&post.content.title),
            xml_escape(&link),
            xml_escape(&link),
            format_iso(post.content.updated_at, site.timezone_offset),
            format_iso(published, site.timezone_offset),
            xml_escape(&content),
        ));
    }

    let updated = posts
        .first()
        .map(|post| post.content.updated_at)
        .unwrap_or_else(now_ts);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{}</title>\
         <id>{}/</id><link href=\"{}/\"/><updated>{}</updated>\
         <subtitle>{}</subtitle>{}</feed>",
        xml_escape(&site.title),
        xml_escape(&site.site_url),
        xml_escape(&site.site_url),
        format_iso(updated, site.timezone_offset),
        xml_escape(&site.description),
        entries,
    );

    Ok(([(CONTENT_TYPE, "application/atom+xml; charset=utf-8")], xml).into_response())
}

pub async fn sitemap(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let mut urls = vec![format!(
        "<url><loc>{}/</loc><changefreq>daily</changefreq></url>",
        xml_escape(&site.site_url)
    )];

    let (posts, _) = content::list(
        &state.db,
        &content::ContentFilter {
            kind: Some("post".to_string()),
            status: Some("published".to_string()),
            limit: 500,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    for post in posts {
        urls.push(format!(
            "<url><loc>{}/archives/{}</loc><lastmod>{}</lastmod></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&post.content.slug)),
            format_iso(post.content.updated_at, site.timezone_offset)
        ));
    }

    let (pages, _) = content::list(
        &state.db,
        &content::ContentFilter {
            kind: Some("page".to_string()),
            status: Some("published".to_string()),
            limit: 500,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    for page in pages {
        urls.push(format!(
            "<url><loc>{}/pages/{}</loc><lastmod>{}</lastmod></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&page.content.slug)),
            format_iso(page.content.updated_at, site.timezone_offset)
        ));
    }

    for term in term::list_public(&state.db, "category").await? {
        urls.push(format!(
            "<url><loc>{}/categories/{}</loc></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&term.term.slug))
        ));
    }
    for term in term::list_public(&state.db, "tag").await? {
        urls.push(format!(
            "<url><loc>{}/tags/{}</loc></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&term.term.slug))
        ));
    }

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{}</urlset>",
        urls.join("")
    );
    Ok(([(CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response())
}

pub async fn robots(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let body = format!(
        "User-agent: *\nAllow: /\nDisallow: /admin\n\nSitemap: {}/sitemap.xml\n",
        site.site_url
    );
    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response())
}

pub async fn media(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(path): Path<String>,
) -> AppResult<Response> {
    let key = path.trim_start_matches('/');
    let publicly_referenced = media::is_publicly_referenced(&state.db, key).await?;
    let can_view_private = session
        .as_ref()
        .is_some_and(AuthSession::can_manage_content);
    if !publicly_referenced && !can_view_private {
        return Ok((
            StatusCode::NOT_FOUND,
            [(CACHE_CONTROL, "private, no-store")],
        )
            .into_response());
    }

    let bytes = state
        .storage
        .get(key)
        .await
        .map_err(|_| AppError::not_found("文件不存在"))?;
    let mime = mime_guess::from_path(key)
        .first_or_octet_stream()
        .to_string();
    let cache_control = if publicly_referenced {
        "public, max-age=3600"
    } else {
        "private, no-store"
    };
    Response::builder()
        .header(CONTENT_TYPE, mime)
        .header(CACHE_CONTROL, cache_control)
        .body(Body::from(bytes))
        .map_err(|error| AppError::internal(error.to_string()))
}

pub async fn not_found(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> AppResult<Response> {
    not_found_page(&state, session.as_ref(), uri.path()).await
}

async fn not_found_page(
    state: &AppState,
    session: Option<&crate::auth::AuthSession>,
    path: &str,
) -> AppResult<Response> {
    let ctx = PublicCtx::load(state, session, path, None, None).await?;
    Ok((StatusCode::NOT_FOUND, render(NotFoundTemplate { ctx })?).into_response())
}

fn query_messages(query: &ListQuery) -> (Option<String>, Option<String>) {
    let mut notice = notice_for(query.notice.as_deref());
    let mut error = error_for(query.error.as_deref());
    match query.comment.as_deref() {
        Some("pending") => notice = notice_for(Some("comment_pending")),
        Some("ok") => notice = notice_for(Some("comment_approved")),
        Some("error") => error = error_for(Some("invalid")),
        _ => {}
    }
    (notice, error)
}
