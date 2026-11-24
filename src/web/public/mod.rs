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

mod articles;
mod comments;
mod feeds;
mod listings;
mod timeline;
mod uploads;

pub use articles::*;
pub use comments::*;
pub use feeds::*;
pub use listings::*;
pub use timeline::*;
pub use uploads::*;

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
