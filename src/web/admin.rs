use std::collections::{HashMap, HashSet};

use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::SET_COOKIE;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{
    AdminSession, AuthSession, LOGIN_CSRF_COOKIE, OptionalUser, clear_login_csrf_cookie,
    clear_session_cookie, cookie_value, create_session, delete_session, hash_password,
    login_csrf_cookie, session_cookie, verify_csrf, verify_password,
};
use crate::error::{AppError, AppResult};
use crate::markdown;
use crate::models::{
    COMMENT_STATUSES, Pagination, REVIEW_STATUSES, annotation_status_label, comment_status_label,
    default_status, is_valid_status, kind_from_path, kind_label, kind_to_path, status_label,
    statuses_for_kind,
};
use crate::repo::annotation::{self, AnnotationInput};
use crate::repo::proposal;
use crate::repo::{comment, content, media, revision, stats, term, token, user};
use crate::settings::{self, SiteSettings};
use crate::state::AppState;
use crate::util::{
    client_ip, constant_time_eq, format_datetime, random_token, safe_next, user_agent,
};

use super::templates::{
    AdminListTemplate, AnnotateTemplate, AnnotationQueueView, AnnotationReplyView, AnnotationView,
    AnnotationsTemplate, CommentAdminView, CommentsTemplate, ContentFormView, DashboardTemplate,
    DiffLine, DiffTemplate, EditTemplate, LoginTemplate, MediaTemplate, MediaView,
    PasswordTemplate, PromotedInfo, ProposalDiffTemplate, ProposalView, ReviewTemplate,
    ScopeOption, SettingsTemplate, SourceNoteInfo, StatusCount, TermsTemplate, TokenView,
    TokensTemplate, render, revision_view,
};
use super::views::{
    AdminCtx, actor_label, error_for, notice_for, status_options, visibility_options,
};
use similar::{ChangeTag, TextDiff};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/login", get(login_page).post(login_submit))
        .route("/admin/logout", post(logout))
        .route("/admin", get(dashboard))
        .route("/admin/password", get(password_page).post(password_submit))
        .route("/admin/review", get(review_page))
        .route("/admin/review/{id}", post(review_submit))
        .route("/admin/annotations", get(annotations_page))
        .route("/admin/annotations/{id}/reply", post(annotation_reply))
        .route("/admin/annotations/{id}/resolve", post(annotation_resolve))
        .route("/admin/annotations/{id}/reopen", post(annotation_reopen))
        .route("/admin/comments", get(comments_page))
        .route("/admin/comments/{id}/status", post(comment_status))
        .route("/admin/terms", get(terms_page).post(term_create))
        .route("/admin/terms/{id}/delete", post(term_delete))
        .route("/admin/settings", get(settings_page).post(settings_submit))
        .route(
            "/admin/media",
            get(media_page)
                .post(media_upload)
                .layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/admin/media/{id}/delete", post(media_delete))
        .route("/admin/tokens", get(tokens_page).post(token_create))
        .route("/admin/tokens/{id}/revoke", post(token_revoke))
        .route("/admin/preview", post(preview))
        .route("/admin/vditor/preview", post(vditor_preview))
        .route(
            "/admin/vditor/upload",
            post(vditor_upload).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/admin/notes/{id}/promote", post(note_promote))
        .route("/admin/{kind}", get(content_list).post(content_create))
        .route("/admin/{kind}/new", get(content_new))
        .route("/admin/{kind}/{id}/edit", get(content_edit))
        .route("/admin/{kind}/{id}/annotate", get(annotate_page))
        .route("/admin/{kind}/{id}/annotations", post(annotation_create))
        .route(
            "/admin/{kind}/{id}/revisions/{revision_id}",
            get(revision_diff),
        )
        .route(
            "/admin/{kind}/{id}/proposals/{proposal_id}",
            get(proposal_diff),
        )
        .route(
            "/admin/{kind}/{id}/proposals/{proposal_id}/apply",
            post(proposal_apply),
        )
        .route(
            "/admin/{kind}/{id}/proposals/{proposal_id}/reject",
            post(proposal_reject),
        )
        .route("/admin/{kind}/{id}", post(content_update))
        .route("/admin/{kind}/{id}/delete", post(content_delete))
        .route("/admin/{kind}/{id}/restore", post(content_restore))
        .route("/admin/{kind}/{id}/publish", post(content_publish))
}

#[derive(Debug, Deserialize, Default)]
pub struct AdminListQuery {
    pub status: Option<String>,
    pub q: Option<String>,
    pub page: Option<i64>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LoginQuery {
    pub next: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
    pub next: Option<String>,
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub permanent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContentForm {
    pub title: String,
    pub slug: Option<String>,
    pub body: String,
    pub excerpt: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub review_status: Option<String>,
    pub review_notes: Option<String>,
    pub allow_comments: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub action: Option<String>,
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromoteForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub status: Option<String>,
    pub sync: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub verdict: String,
    pub notes: Option<String>,
    pub publish: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub kind: String,
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_offset: Option<String>,
    pub end_offset: Option<String>,
    pub comment: String,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationReplyForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProposalApplyForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub force: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProposalRejectForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommentActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct TermForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub kind: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub site_title: String,
    pub site_description: String,
    pub site_keywords: String,
    pub site_url: String,
    pub footer_text: String,
    pub page_size: String,
    pub timezone_offset: String,
    pub comments_enabled: Option<String>,
    pub comments_moderation: Option<String>,
    pub comments_require_email: Option<String>,
    pub feed_full_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub name: String,
    pub scope_read: Option<String>,
    pub scope_write: Option<String>,
    pub scope_publish: Option<String>,
    pub scope_review: Option<String>,
    pub scope_admin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PasswordForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub old_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub body: String,
}

async fn admin_ctx(
    state: &AppState,
    session: &AuthSession,
    active: &str,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<AdminCtx> {
    AdminCtx::load(state, session, active, notice, error).await
}

async fn login_page(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Query(query): Query<LoginQuery>,
) -> AppResult<Response> {
    if session.is_some() {
        return Ok(Redirect::to("/admin").into_response());
    }
    render_login(
        &state,
        error_for(query.error.as_deref()),
        safe_next(query.next.as_deref()),
        String::new(),
    )
    .await
}

async fn login_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> AppResult<Response> {
    let next = safe_next(form.next.as_deref());
    let csrf_cookie = cookie_value(&headers, LOGIN_CSRF_COOKIE);
    let csrf_ok = match (csrf_cookie.as_deref(), form.csrf.as_deref()) {
        (Some(expected), Some(actual)) => constant_time_eq(expected, actual),
        _ => false,
    };
    if !csrf_ok {
        return render_login(
            &state,
            Some("页面已过期，请重新登录。".to_string()),
            next,
            form.username,
        )
        .await;
    }

    let ip = client_ip(&headers).unwrap_or_else(|| "unknown".to_string());
    let key = format!("{}:{}", form.username.trim(), ip);
    if !state.login_limiter.allowed(&key) {
        return render_login(
            &state,
            Some("尝试次数过多，请 10 分钟后再试。".to_string()),
            next,
            form.username,
        )
        .await;
    }

    let Some(user) = user::find_by_username(&state.db, form.username.trim()).await? else {
        state.login_limiter.record_failure(&key);
        return render_login(
            &state,
            Some("用户名或密码错误。".to_string()),
            next,
            form.username,
        )
        .await;
    };

    if !verify_password(&form.password, &user.password_hash) {
        state.login_limiter.record_failure(&key);
        return render_login(
            &state,
            Some("用户名或密码错误。".to_string()),
            next,
            form.username,
        )
        .await;
    }

    state.login_limiter.record_success(&key);
    let (token, _) = create_session(
        &state.db,
        user.id,
        state.config.session_days,
        Some(&ip),
        user_agent(&headers).as_deref(),
    )
    .await?;

    let mut response = Redirect::to(&next).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        session_cookie(
            &token,
            state.config.session_days * 24 * 3600,
            state.config.secure_cookies,
        ),
    );
    response.headers_mut().append(
        SET_COOKIE,
        clear_login_csrf_cookie(state.config.secure_cookies),
    );
    Ok(response)
}

async fn render_login(
    state: &AppState,
    error: Option<String>,
    next: String,
    username: String,
) -> AppResult<Response> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let csrf = random_token(24);
    let template = LoginTemplate {
        site,
        error,
        next,
        csrf: csrf.clone(),
        username,
    };
    let mut response = render(template)?.into_response();
    response.headers_mut().append(
        SET_COOKIE,
        login_csrf_cookie(&csrf, state.config.secure_cookies),
    );
    Ok(response)
}

async fn logout(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    delete_session(&state.db, &session.token_hash).await?;
    let mut response = Redirect::to("/admin/login").into_response();
    response.headers_mut().append(
        SET_COOKIE,
        clear_session_cookie(state.config.secure_cookies),
    );
    Ok(response)
}

async fn dashboard(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
) -> AppResult<Response> {
    let ctx = admin_ctx(&state, &session, "dashboard", None, None).await?;
    let stats = stats::load(&state.db).await?;
    let (reviews, _) = content::list(
        &state.db,
        &content::ContentFilter {
            review_status: Some("pending".to_string()),
            include_deleted: false,
            include_private: true,
            limit: 5,
            order: content::ContentOrder::UpdatedDesc,
            ..Default::default()
        },
        ctx.site.timezone_offset,
    )
    .await?;
    let (comments, _) = comment::list_admin(&state.db, "pending", 5, 0).await?;
    let comments = comments
        .iter()
        .map(|row| CommentAdminView::from_row(row, ctx.site.timezone_offset))
        .collect();
    let (recent_posts, _) = content::list(
        &state.db,
        &content::ContentFilter {
            kind: Some("post".to_string()),
            include_deleted: false,
            include_private: true,
            limit: 5,
            order: content::ContentOrder::UpdatedDesc,
            ..Default::default()
        },
        ctx.site.timezone_offset,
    )
    .await?;

    Ok(render(DashboardTemplate {
        ctx,
        stats,
        reviews,
        comments,
        recent_posts,
    })?
    .into_response())
}

async fn content_list(
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

async fn build_content_counts(
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

fn list_query_suffix(status: &str, query: &str) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if status != "all" {
        serializer.append_pair("status", status);
    }
    if !query.is_empty() {
        serializer.append_pair("q", query);
    }
    serializer.finish()
}

async fn content_new(
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

async fn content_edit(
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
async fn edit_response(
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
    let reviews = super::views::review_options(&form.review_status);

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

fn form_from_view(view: &crate::models::ContentView) -> ContentFormView {
    ContentFormView {
        title: view.content.title.clone(),
        slug: view.content.slug.clone(),
        body: view.content.body.clone(),
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

fn form_from_submission(form: &ContentForm, kind: &str) -> ContentFormView {
    ContentFormView {
        title: form.title.clone(),
        slug: form.slug.clone().unwrap_or_default(),
        body: form.body.clone(),
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

fn resolved_status(kind: &str, form: &ContentForm) -> (String, String) {
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

fn parse_tags(value: Option<&str>) -> Vec<String> {
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

async fn content_create(
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
                Some(error.to_string()),
            )
            .await
        }
    }
}

async fn content_update(
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
        expected_updated_at: None,
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
                Some(error.to_string()),
            )
            .await
        }
    }
}

async fn content_delete(
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

async fn content_restore(
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

async fn content_publish(
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

async fn note_promote(
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
            actor: format!("human:{}", session.user.id),
        },
    )
    .await?;

    let notice = if result.created { "promoted" } else { "synced" };
    Ok(Redirect::to(&format!("/admin/posts/{}/edit?notice={notice}", result.id)).into_response())
}

async fn review_page(
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

async fn review_submit(
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
    let content =
        content::set_review(&state.db, id, &form.verdict, form.notes.as_deref(), &actor).await?;

    if form.publish.is_some() && form.verdict == "approved" && content.kind != "note" {
        content::publish(&state.db, id, &actor).await?;
    }

    Ok(Redirect::to("/admin/review?notice=review_saved").into_response())
}

async fn annotate_page(
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

fn annotation_view(
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

fn proposal_view(proposal: &crate::models::ContentProposal, timezone_offset: i32) -> ProposalView {
    ProposalView {
        id: proposal.id,
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

async fn annotation_create(
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

async fn annotation_reply(
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

async fn annotation_resolve(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<AnnotationActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    annotation_status_action(&state, id, "resolved", "annotation_resolved").await
}

async fn annotation_reopen(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<AnnotationActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    annotation_status_action(&state, id, "open", "annotation_reopened").await
}

async fn annotation_status_action(
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

async fn annotations_page(
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

async fn proposal_diff(
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
    let html = content::render_body(&state.db, id, &proposal.body, true).await?;
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

async fn proposal_apply(
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
    proposal::apply(
        &state.db,
        proposal_id,
        &format!("human:{}", session.user.id),
        form.force.is_some(),
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/admin/{kind_path}/{id}/edit?notice=proposal_applied"
    ))
    .into_response())
}

async fn proposal_reject(
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

async fn revision_diff(
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

fn build_diff(old: &str, new: &str) -> Vec<DiffLine> {
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

async fn comments_page(
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

async fn build_comment_counts(
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

async fn comment_status(
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

async fn terms_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let kind = query
        .status
        .clone()
        .filter(|value| value == "category" || value == "tag")
        .unwrap_or_else(|| "category".to_string());
    let ctx = admin_ctx(
        &state,
        &session,
        "terms",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let terms = term::list(&state.db, &kind).await?;
    Ok(render(TermsTemplate {
        ctx,
        kind: kind.clone(),
        heading: if kind == "category" {
            "分类".to_string()
        } else {
            "标签".to_string()
        },
        terms,
        error: None,
    })?
    .into_response())
}

async fn term_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<TermForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let result = term::create(
        &state.db,
        &form.kind,
        &form.name,
        form.slug.as_deref(),
        form.description.as_deref(),
    )
    .await;

    match result {
        Ok(_) => Ok(
            Redirect::to(&format!("/admin/terms?status={}&notice=created", form.kind))
                .into_response(),
        ),
        Err(error) => {
            let ctx = admin_ctx(&state, &session, "terms", None, Some(error.to_string())).await?;
            let terms = term::list(&state.db, &form.kind).await?;
            Ok(render(TermsTemplate {
                ctx,
                kind: form.kind.clone(),
                heading: if form.kind == "category" {
                    "分类".to_string()
                } else {
                    "标签".to_string()
                },
                terms,
                error: Some(error.to_string()),
            })?
            .into_response())
        }
    }
}

async fn term_delete(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let kind = term::get_by_id(&state.db, id)
        .await?
        .map(|term| term.kind)
        .unwrap_or_else(|| "category".to_string());
    term::delete(&state.db, id).await?;
    Ok(Redirect::to(&format!("/admin/terms?status={kind}&notice=deleted")).into_response())
}

async fn settings_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let ctx = admin_ctx(
        &state,
        &session,
        "settings",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let settings = settings::load(&state.db, &state.config.site_url).await?;
    Ok(render(SettingsTemplate {
        ctx,
        settings,
        error: None,
    })?
    .into_response())
}

async fn settings_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<SettingsForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let page_size = form
        .page_size
        .parse::<i64>()
        .ok()
        .filter(|value| (1..=100).contains(value));
    let timezone_offset = form
        .timezone_offset
        .parse::<i32>()
        .ok()
        .filter(|value| (-12 * 60..=14 * 60).contains(value));

    let Some(page_size) = page_size else {
        return settings_error(&state, &session, &form, "每页文章数必须在 1 到 100 之间").await;
    };
    let Some(timezone_offset) = timezone_offset else {
        return settings_error(&state, &session, &form, "时区偏移不合法").await;
    };
    if !form.site_url.starts_with("http://") && !form.site_url.starts_with("https://") {
        return settings_error(
            &state,
            &session,
            &form,
            "站点地址必须以 http:// 或 https:// 开头",
        )
        .await;
    }

    let settings = SiteSettings {
        title: form.site_title.trim().to_string(),
        description: form.site_description.trim().to_string(),
        keywords: form.site_keywords.trim().to_string(),
        site_url: form.site_url.trim().trim_end_matches('/').to_string(),
        footer_text: form.footer_text.trim().to_string(),
        page_size,
        timezone_offset,
        comments_enabled: form.comments_enabled.is_some(),
        comments_moderation: form.comments_moderation.is_some(),
        comments_require_email: form.comments_require_email.is_some(),
        feed_full_text: form.feed_full_text.is_some(),
    };
    settings::update(&state.db, &settings).await?;
    Ok(Redirect::to("/admin/settings?notice=settings_saved").into_response())
}

async fn settings_error(
    state: &AppState,
    session: &AuthSession,
    form: &SettingsForm,
    message: &str,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "settings", None, Some(message.to_string())).await?;
    let settings = SiteSettings {
        title: form.site_title.clone(),
        description: form.site_description.clone(),
        keywords: form.site_keywords.clone(),
        site_url: form.site_url.clone(),
        footer_text: form.footer_text.clone(),
        page_size: form.page_size.parse().unwrap_or(8),
        timezone_offset: form.timezone_offset.parse().unwrap_or(480),
        comments_enabled: form.comments_enabled.is_some(),
        comments_moderation: form.comments_moderation.is_some(),
        comments_require_email: form.comments_require_email.is_some(),
        feed_full_text: form.feed_full_text.is_some(),
    };
    Ok(render(SettingsTemplate {
        ctx,
        settings,
        error: Some(message.to_string()),
    })?
    .into_response())
}

async fn media_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    media_response(
        &state,
        &session,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await
}

async fn media_response(
    state: &AppState,
    session: &AuthSession,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "media", notice, error.clone()).await?;
    let site = ctx.site.clone();
    let media = media::list(&state.db, 200)
        .await?
        .iter()
        .map(|item| MediaView::from_media(item, site.timezone_offset))
        .collect();
    Ok(render(MediaTemplate { ctx, media, error })?.into_response())
}

struct StoredUpload {
    url: String,
}

async fn store_upload(
    state: &AppState,
    session: &AuthSession,
    original_name: String,
    declared_mime: String,
    bytes: Vec<u8>,
) -> AppResult<StoredUpload> {
    if bytes.is_empty() {
        return Err(AppError::bad_request("文件为空"));
    }
    if bytes.len() > state.config.max_upload_bytes {
        return Err(AppError::bad_request("文件超过大小限制"));
    }

    let original_name = if original_name.trim().is_empty() {
        "upload".to_string()
    } else {
        original_name
    };
    let extension = std::path::Path::new(&original_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "pdf" | "txt" | "md" | "zip"
            )
        })
        .ok_or_else(|| AppError::bad_request("不支持的文件类型"))?;

    let mime = mime_guess::from_path(&original_name)
        .first_or_octet_stream()
        .to_string();
    if declared_mime.contains("html")
        || declared_mime.contains("javascript")
        || mime.contains("html")
    {
        return Err(AppError::bad_request("不支持的文件类型"));
    }

    let now = chrono::Utc::now();
    let storage_key = format!(
        "{}/{:02}/{}.{}",
        now.format("%Y"),
        now.format("%m"),
        random_token(16),
        extension
    );
    let url = format!("/media/{storage_key}");

    state
        .storage
        .put(&storage_key, &bytes)
        .await
        .map_err(|error| AppError::internal(format!("文件保存失败: {error}")))?;

    media::insert(
        &state.db,
        media::NewMedia {
            storage_key,
            original_name,
            mime: mime.clone(),
            size: bytes.len() as i64,
            width: None,
            height: None,
            uploader_id: Some(session.user.id),
        },
    )
    .await?;

    Ok(StoredUpload { url })
}

async fn media_upload(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    mut multipart: Multipart,
) -> AppResult<Response> {
    let mut csrf = None;
    let mut file: Option<(String, String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or("").to_string();
        if name == "_csrf" {
            csrf = Some(field.text().await?);
        } else if name == "file" || name == "file[]" {
            let original_name = field
                .file_name()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "upload".to_string());
            let mime = field
                .content_type()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let bytes = field.bytes().await?;
            file = Some((original_name, mime, bytes.to_vec()));
        }
    }

    verify_csrf(&session, csrf.as_deref().unwrap_or(""))?;
    let Some((original_name, declared_mime, bytes)) = file else {
        return media_response(
            &state,
            &session,
            None,
            Some("请选择要上传的文件".to_string()),
        )
        .await;
    };

    if let Err(error) = store_upload(&state, &session, original_name, declared_mime, bytes).await {
        return media_response(&state, &session, None, Some(error.to_string())).await;
    }

    Ok(Redirect::to("/admin/media?notice=uploaded").into_response())
}

#[derive(Debug, Deserialize)]
pub struct VditorPreviewRequest {
    #[serde(rename = "markdownText")]
    pub markdown_text: String,
}

async fn vditor_preview(
    OptionalUser(session): OptionalUser,
    Json(request): Json<VditorPreviewRequest>,
) -> Response {
    if session.is_none() {
        return Json(json!({
            "code": 1,
            "msg": "登录已过期，请刷新页面",
            "data": ""
        }))
        .into_response();
    }

    let html = markdown::render_markdown(&request.markdown_text, &HashMap::new());
    Json(json!({
        "code": 0,
        "msg": "",
        "data": html
    }))
    .into_response()
}

async fn vditor_upload(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let Some(session) = session else {
        return Json(json!({
            "code": 1,
            "msg": "登录已过期，请刷新页面",
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    };

    let mut csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_string();
                if name == "_csrf" {
                    if let Ok(value) = field.text().await {
                        csrf = Some(value);
                    }
                } else if name == "file" || name == "file[]" {
                    let original_name = field
                        .file_name()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "upload".to_string());
                    let mime = field
                        .content_type()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    match field.bytes().await {
                        Ok(bytes) => files.push((original_name, mime, bytes.to_vec())),
                        Err(error) => {
                            return Json(json!({
                                "code": 1,
                                "msg": format!("读取上传文件失败: {error}"),
                                "data": { "errFiles": [], "succMap": {} }
                            }))
                            .into_response();
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                return Json(json!({
                    "code": 1,
                    "msg": format!("上传失败: {error}"),
                    "data": { "errFiles": [], "succMap": {} }
                }))
                .into_response();
            }
        }
    }

    if let Err(error) = verify_csrf(&session, csrf.as_deref().unwrap_or("")) {
        return Json(json!({
            "code": 1,
            "msg": error.to_string(),
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    }

    if files.is_empty() {
        return Json(json!({
            "code": 1,
            "msg": "请选择要上传的文件",
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    }

    let mut succ_map = HashMap::new();
    let mut err_files = Vec::new();
    let mut first_error = String::new();

    for (original_name, declared_mime, bytes) in files {
        match store_upload(
            &state,
            &session,
            original_name.clone(),
            declared_mime,
            bytes,
        )
        .await
        {
            Ok(upload) => {
                succ_map.insert(original_name, upload.url);
            }
            Err(error) => {
                if first_error.is_empty() {
                    first_error = error.to_string();
                }
                err_files.push(original_name);
            }
        }
    }

    let code = if succ_map.is_empty() { 1 } else { 0 };
    Json(json!({
        "code": code,
        "msg": first_error,
        "data": {
            "errFiles": err_files,
            "succMap": succ_map
        }
    }))
    .into_response()
}

async fn media_delete(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    if let Some(item) = media::get(&state.db, id).await? {
        if let Err(error) = state.storage.delete(&item.storage_key).await {
            tracing::warn!(error = ?error, "delete upload file failed");
        }
        media::delete(&state.db, id).await?;
    }
    Ok(Redirect::to("/admin/media?notice=deleted").into_response())
}

async fn tokens_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    tokens_response(
        &state,
        &session,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
        None,
        &["read", "write", "review"],
    )
    .await
}

async fn tokens_response(
    state: &AppState,
    session: &AuthSession,
    notice: Option<String>,
    error: Option<String>,
    new_token: Option<String>,
    selected: &[&str],
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "tokens", notice, error.clone()).await?;
    let site = ctx.site.clone();
    let tokens = token::list(&state.db)
        .await?
        .iter()
        .map(|token| TokenView::from_token(token, site.timezone_offset))
        .collect();
    let scopes = token::SCOPES
        .iter()
        .map(|scope| ScopeOption {
            value: scope.to_string(),
            label: match *scope {
                "read" => "读取",
                "write" => "写入草稿/笔记",
                "publish" => "发布",
                "review" => "Review",
                "admin" => "全部权限",
                _ => scope,
            }
            .to_string(),
            checked: selected.contains(scope),
        })
        .collect();

    Ok(render(TokensTemplate {
        ctx,
        tokens,
        new_token,
        scopes,
        error,
    })?
    .into_response())
}

async fn token_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<TokenForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let mut scopes = Vec::new();
    if form.scope_read.is_some() {
        scopes.push("read".to_string());
    }
    if form.scope_write.is_some() {
        scopes.push("write".to_string());
    }
    if form.scope_publish.is_some() {
        scopes.push("publish".to_string());
    }
    if form.scope_review.is_some() {
        scopes.push("review".to_string());
    }
    if form.scope_admin.is_some() {
        scopes.push("admin".to_string());
    }

    match token::create(&state.db, &form.name, &scopes).await {
        Ok((_, plaintext)) => {
            let selected: Vec<&str> = scopes.iter().map(String::as_str).collect();
            tokens_response(
                &state,
                &session,
                notice_for(Some("token_created")),
                None,
                Some(plaintext),
                &selected,
            )
            .await
        }
        Err(error) => {
            tokens_response(
                &state,
                &session,
                None,
                Some(error.to_string()),
                None,
                &["read", "write", "review"],
            )
            .await
        }
    }
}

async fn token_revoke(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    token::revoke(&state.db, id).await?;
    Ok(Redirect::to("/admin/tokens?notice=token_revoked").into_response())
}

async fn preview(
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

async fn password_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let ctx = admin_ctx(
        &state,
        &session,
        "password",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    Ok(render(PasswordTemplate { ctx, error: None })?.into_response())
}

async fn password_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<PasswordForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    if !verify_password(&form.old_password, &session.user.password_hash) {
        return password_error(&state, &session, "原密码不正确").await;
    }
    if form.new_password.chars().count() < 8 {
        return password_error(&state, &session, "新密码至少需要 8 个字符").await;
    }
    if form.new_password != form.confirm_password {
        return password_error(&state, &session, "两次输入的新密码不一致").await;
    }

    let hash = hash_password(&form.new_password)?;
    user::update_password(&state.db, session.user.id, &hash).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(session.user.id)
        .bind(&session.token_hash)
        .execute(&state.db)
        .await?;
    Ok(Redirect::to("/admin/password?notice=password_changed").into_response())
}

async fn password_error(
    state: &AppState,
    session: &AuthSession,
    message: &str,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "password", None, Some(message.to_string())).await?;
    Ok(render(PasswordTemplate {
        ctx,
        error: Some(message.to_string()),
    })?
    .into_response())
}
