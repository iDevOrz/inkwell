use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::SET_COOKIE;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{
    AdminSession, AuthSession, LOGIN_CSRF_COOKIE, OptionalUser, check_login_password,
    clear_legacy_session_cookie, clear_login_csrf_cookie, clear_session_cookie, cookie_value,
    create_session, delete_session, hash_password_async, login_csrf_cookie, session_cookie,
    verify_csrf,
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

mod annotations;
mod comments;
mod contents;
mod forms;
mod media_library;
mod proposals;
mod review;
mod session;
mod site_settings;
mod terms;
mod tokens;

use annotations::*;
use comments::*;
use contents::*;
use forms::*;
use media_library::*;
use proposals::*;
use review::*;
use session::*;
use site_settings::*;
use terms::*;
use tokens::*;

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

async fn admin_ctx(
    state: &AppState,
    session: &AuthSession,
    active: &str,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<AdminCtx> {
    AdminCtx::load(state, session, active, notice, error).await
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
