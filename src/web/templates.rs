use askama::Template;
use axum::response::Html;
use serde::Serialize;

use crate::error::AppResult;
use crate::models::{
    ApiToken, CommentWithContent, ContentSummary, ContentView, Media, Pagination, Revision, Stats,
    Term, TermWithCount, TimelinePost,
};
use crate::settings::SiteSettings;

use super::views::{AdminCtx, CommentItem, PostLink, PublicCtx, StatusOption};

#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntry {
    pub post: TimelinePost,
    pub year: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineMore {
    pub before_ts: i64,
    pub before_id: i64,
    pub last_year: String,
}

#[derive(Template)]
#[template(path = "timeline_fragment.html")]
pub struct TimelineFragmentTemplate {
    pub timeline: Vec<TimelineEntry>,
}

#[derive(Template)]
#[template(path = "list.html")]
pub struct ListTemplate {
    pub ctx: PublicCtx,
    pub heading: String,
    pub subheading: Option<String>,
    pub posts: Vec<ContentSummary>,
    pub timeline: Vec<TimelineEntry>,
    pub timeline_more: Option<TimelineMore>,
    pub pagination: Pagination,
    pub feed_url: String,
    pub canonical: String,
    pub search_query: Option<String>,
    pub is_home: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentFormView {
    pub content_id: i64,
    pub parent_id: i64,
    pub author: String,
    pub email: String,
    pub url: String,
    pub body: String,
    pub reply_author: Option<String>,
}

#[derive(Template)]
#[template(path = "post.html")]
pub struct PostTemplate {
    pub ctx: PublicCtx,
    pub post: ContentView,
    pub comments: Vec<CommentItem>,
    pub comment_count: i64,
    pub comment_form: CommentFormView,
    pub reply_to: Option<CommentItem>,
    pub canonical: String,
    pub prev: Option<PostLink>,
    pub next: Option<PostLink>,
}

#[derive(Template)]
#[template(path = "page.html")]
pub struct PageTemplate {
    pub ctx: PublicCtx,
    pub page: ContentView,
    pub canonical: String,
}

#[derive(Template)]
#[template(path = "404.html")]
pub struct NotFoundTemplate {
    pub ctx: PublicCtx,
}

#[derive(Template)]
#[template(path = "admin/login.html")]
pub struct LoginTemplate {
    pub site: SiteSettings,
    pub error: Option<String>,
    pub next: String,
    pub csrf: String,
    pub username: String,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub struct DashboardTemplate {
    pub ctx: AdminCtx,
    pub stats: Stats,
    pub reviews: Vec<ContentSummary>,
    pub comments: Vec<CommentAdminView>,
    pub recent_posts: Vec<ContentSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCount {
    pub value: String,
    pub label: String,
    pub count: i64,
    pub active: bool,
}

#[derive(Template)]
#[template(path = "admin/list.html")]
pub struct AdminListTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub heading: String,
    pub items: Vec<ContentSummary>,
    pub pagination: Pagination,
    pub status_filter: String,
    pub query: String,
    pub base_path: String,
    pub counts: Vec<StatusCount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentFormView {
    pub title: String,
    pub slug: String,
    pub body: String,
    pub expected_updated_at: Option<i64>,
    pub excerpt: String,
    pub status: String,
    pub visibility: String,
    pub review_status: String,
    pub review_notes: String,
    pub allow_comments: bool,
    pub category: String,
    pub tags: String,
}

impl ContentFormView {
    pub fn empty(kind: &str) -> Self {
        Self {
            title: String::new(),
            slug: String::new(),
            body: String::new(),
            expected_updated_at: None,
            excerpt: String::new(),
            status: crate::models::default_status(kind).to_string(),
            visibility: if kind == "note" {
                "private".to_string()
            } else {
                "public".to_string()
            },
            review_status: "none".to_string(),
            review_notes: String::new(),
            allow_comments: true,
            category: String::new(),
            tags: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PromotedInfo {
    pub id: i64,
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceNoteInfo {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevisionView {
    pub id: i64,
    pub actor: String,
    pub note: String,
    pub created_label: String,
    pub status_label: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationReplyView {
    pub id: i64,
    pub author: String,
    pub author_label: String,
    pub body: String,
    pub created_label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationView {
    pub id: i64,
    pub kind: String,
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_offset: Option<i64>,
    pub end_offset: Option<i64>,
    pub comment: String,
    pub status: String,
    pub status_label: String,
    pub author: String,
    pub author_label: String,
    pub created_label: String,
    pub replies: Vec<AnnotationReplyView>,
}

#[derive(Template)]
#[template(path = "admin/annotate.html")]
pub struct AnnotateTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub kind_path: String,
    pub item: ContentView,
    pub annotations: Vec<AnnotationView>,
    pub proposals: Vec<ProposalView>,
    pub general_count: usize,
    pub inline_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationQueueView {
    pub id: i64,
    pub content_id: i64,
    pub content_title: String,
    pub content_kind: String,
    pub content_status: String,
    pub annotate_url: String,
    pub kind: String,
    pub quote: Option<String>,
    pub comment: String,
    pub status: String,
    pub status_label: String,
    pub author_label: String,
    pub created_label: String,
}

#[derive(Template)]
#[template(path = "admin/annotations.html")]
pub struct AnnotationsTemplate {
    pub ctx: AdminCtx,
    pub annotations: Vec<AnnotationQueueView>,
    pub status_filter: String,
    pub open_count: i64,
    pub addressed_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposalView {
    pub id: i64,
    pub version: i64,
    pub content_id: i64,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub status_label: String,
    pub author_label: String,
    pub created_label: String,
    pub resolved_label: String,
    pub decision_note: String,
    pub is_proposed: bool,
}

#[derive(Template)]
#[template(path = "admin/proposal.html")]
pub struct ProposalDiffTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub kind_path: String,
    pub item: ContentView,
    pub proposal: ProposalView,
    pub html: String,
    pub has_mermaid: bool,
    pub old_title: String,
    pub new_title: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub kind: String,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

#[derive(Template)]
#[template(path = "admin/diff.html")]
pub struct DiffTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub kind_path: String,
    pub item: ContentView,
    pub revision: RevisionView,
    pub previous: Option<RevisionView>,
    pub old_title: String,
    pub new_title: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Template)]
#[template(path = "admin/edit.html")]
pub struct EditTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub kind_label: String,
    pub kind_path: String,
    pub content_id: Option<i64>,
    pub is_new: bool,
    pub item: Option<ContentView>,
    pub form: ContentFormView,
    pub categories: Vec<Term>,
    pub all_tags: Vec<Term>,
    pub statuses: Vec<StatusOption>,
    pub visibilities: Vec<StatusOption>,
    pub reviews: Vec<StatusOption>,
    pub revisions: Vec<RevisionView>,
    pub proposals: Vec<ProposalView>,
    pub promoted: Option<PromotedInfo>,
    pub source_note: Option<SourceNoteInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentAdminView {
    pub id: i64,
    pub author: String,
    pub email: String,
    pub url: String,
    pub body: String,
    pub status: String,
    pub status_label: String,
    pub content_id: i64,
    pub content_title: String,
    pub content_slug: String,
    pub content_admin_url: String,
    pub created_label: String,
}

impl CommentAdminView {
    pub fn from_row(row: &CommentWithContent, timezone_offset: i32) -> Self {
        Self {
            id: row.comment.id,
            author: row.comment.author.clone(),
            email: row.comment.email.clone().unwrap_or_default(),
            url: row.comment.url.clone().unwrap_or_default(),
            body: row.comment.body.clone(),
            status: row.comment.status.clone(),
            status_label: crate::models::comment_status_label(&row.comment.status).to_string(),
            content_id: row.comment.content_id,
            content_title: row.content_title.clone(),
            content_slug: row.content_slug.clone(),
            content_admin_url: format!(
                "/admin/{}/{}/edit",
                crate::models::kind_to_path(&row.content_kind),
                row.comment.content_id
            ),
            created_label: crate::util::format_datetime(row.comment.created_at, timezone_offset),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/review.html")]
pub struct ReviewTemplate {
    pub ctx: AdminCtx,
    pub reviews: Vec<ContentSummary>,
    pub comments: Vec<CommentAdminView>,
}

#[derive(Template)]
#[template(path = "admin/comments.html")]
pub struct CommentsTemplate {
    pub ctx: AdminCtx,
    pub comments: Vec<CommentAdminView>,
    pub pagination: Pagination,
    pub status_filter: String,
    pub counts: Vec<StatusCount>,
}

#[derive(Template)]
#[template(path = "admin/terms.html")]
pub struct TermsTemplate {
    pub ctx: AdminCtx,
    pub kind: String,
    pub heading: String,
    pub terms: Vec<TermWithCount>,
    pub editing: Option<Term>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
pub struct SettingsTemplate {
    pub ctx: AdminCtx,
    pub settings: SiteSettings,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaView {
    pub id: i64,
    pub original_name: String,
    pub mime: String,
    pub size_label: String,
    pub url: String,
    pub created_label: String,
}

impl MediaView {
    pub fn from_media(media: &Media, timezone_offset: i32) -> Self {
        Self {
            id: media.id,
            original_name: media.original_name.clone(),
            mime: media.mime.clone(),
            size_label: crate::util::format_bytes(media.size),
            url: format!("/media/{}", media.storage_key),
            created_label: crate::util::format_datetime(media.created_at, timezone_offset),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/media.html")]
pub struct MediaTemplate {
    pub ctx: AdminCtx,
    pub media: Vec<MediaView>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenView {
    pub id: i64,
    pub name: String,
    pub scopes: String,
    pub created_label: String,
    pub last_used_label: String,
    pub revoked: bool,
}

impl TokenView {
    pub fn from_token(token: &ApiToken, timezone_offset: i32) -> Self {
        Self {
            id: token.id,
            name: token.name.clone(),
            scopes: token.scopes.clone(),
            created_label: crate::util::format_datetime(token.created_at, timezone_offset),
            last_used_label: token
                .last_used_at
                .map(|timestamp| crate::util::format_datetime(timestamp, timezone_offset))
                .unwrap_or_else(|| "从未使用".to_string()),
            revoked: token.revoked_at.is_some(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeOption {
    pub value: String,
    pub label: String,
    pub checked: bool,
}

#[derive(Template)]
#[template(path = "admin/tokens.html")]
pub struct TokensTemplate {
    pub ctx: AdminCtx,
    pub tokens: Vec<TokenView>,
    pub new_token: Option<String>,
    pub scopes: Vec<ScopeOption>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/profile.html")]
pub struct ProfileTemplate {
    pub ctx: AdminCtx,
    pub display_name: String,
    pub email: String,
    pub homepage: String,
}

#[derive(Template)]
#[template(path = "admin/password.html")]
pub struct PasswordTemplate {
    pub ctx: AdminCtx,
    pub error: Option<String>,
}

pub fn render<T: Template>(template: T) -> AppResult<Html<String>> {
    Ok(Html(template.render()?))
}

pub fn revision_view(revision: &Revision, timezone_offset: i32) -> RevisionView {
    RevisionView {
        id: revision.id,
        actor: revision.actor.clone(),
        note: revision.note.clone().unwrap_or_default(),
        created_label: crate::util::format_datetime(revision.created_at, timezone_offset),
        status_label: crate::models::status_label(&revision.status).to_string(),
        title: revision.title.clone(),
    }
}
