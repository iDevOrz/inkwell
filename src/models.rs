use serde::Serialize;
use sqlx::FromRow;

pub const KINDS: &[&str] = &["post", "page", "note"];
pub const POST_STATUSES: &[&str] = &["draft", "published", "trash"];
pub const NOTE_STATUSES: &[&str] = &["inbox", "developing", "evergreen", "archived", "trash"];
pub const REVIEW_STATUSES: &[&str] = &["none", "pending", "approved", "changes_requested"];
pub const COMMENT_STATUSES: &[&str] = &["pending", "approved", "spam", "trash"];
pub const VISIBILITIES: &[&str] = &["public", "private"];

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserView {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub role: String,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            role: user.role.clone(),
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Content {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub slug: String,
    pub body: String,
    pub excerpt: Option<String>,
    pub status: String,
    pub visibility: String,
    pub author_id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub published_at: Option<i64>,
    pub review_status: String,
    pub review_notes: Option<String>,
    pub allow_comments: i64,
    pub source_note_id: Option<i64>,
    pub promoted_to_id: Option<i64>,
    pub metadata: String,
    pub deleted_at: Option<i64>,
}

impl Content {
    pub fn is_note(&self) -> bool {
        self.kind == "note"
    }

    pub fn is_public(&self) -> bool {
        self.visibility == "public" && self.status == "published" && self.deleted_at.is_none()
    }

    pub fn public_url(&self) -> String {
        match self.kind.as_str() {
            "page" => format!("/pages/{}", self.slug),
            "note" => format!("/admin/notes/{}/edit", self.id),
            _ => format!("/archives/{}", self.slug),
        }
    }

    pub fn admin_url(&self) -> String {
        format!("/admin/{}/{}/edit", kind_to_path(&self.kind), self.id)
    }

    pub fn status_label(&self) -> &'static str {
        status_label(&self.status)
    }

    pub fn review_label(&self) -> &'static str {
        review_label(&self.review_status)
    }

    pub fn kind_label(&self) -> &'static str {
        kind_label(&self.kind)
    }

    pub fn visibility_label(&self) -> &'static str {
        visibility_label(&self.visibility)
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Term {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub parent_id: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct TermWithCount {
    #[sqlx(flatten)]
    pub term: Term,
    pub count: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Comment {
    pub id: i64,
    pub content_id: i64,
    pub parent_id: i64,
    pub author: String,
    pub email: Option<String>,
    pub url: Option<String>,
    pub body: String,
    pub status: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct CommentWithContent {
    #[sqlx(flatten)]
    pub comment: Comment,
    pub content_title: String,
    pub content_slug: String,
    pub content_kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentNode {
    pub comment: Comment,
    pub children: Vec<CommentNode>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Revision {
    pub id: i64,
    pub content_id: i64,
    pub title: String,
    pub body: String,
    pub status: String,
    pub actor: String,
    pub note: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Media {
    pub id: i64,
    pub storage_key: String,
    pub original_name: String,
    pub mime: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub uploader_id: Option<i64>,
    pub created_at: i64,
}

pub const PROPOSAL_STATUSES: &[&str] = &["proposed", "applied", "rejected"];

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ContentProposal {
    pub id: i64,
    pub version: i64,
    pub content_id: i64,
    pub base_revision_id: Option<i64>,
    pub base_updated_at: i64,
    pub title: String,
    pub body: String,
    pub excerpt: Option<String>,
    pub summary: Option<String>,
    pub status: String,
    pub author: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub decision_note: Option<String>,
    pub applied_revision_id: Option<i64>,
}

pub fn proposal_status_label(status: &str) -> &'static str {
    match status {
        "proposed" => "待审版本",
        "applied" => "已应用",
        "rejected" => "已拒绝",
        _ => "未知",
    }
}

pub const ANNOTATION_KINDS: &[&str] = &["inline", "general"];
pub const ANNOTATION_STATUSES: &[&str] = &["open", "addressed", "resolved", "outdated"];

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Annotation {
    pub id: i64,
    pub content_id: i64,
    pub kind: String,
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_offset: Option<i64>,
    pub end_offset: Option<i64>,
    pub comment: String,
    pub status: String,
    pub author: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct AnnotationReply {
    pub id: i64,
    pub annotation_id: i64,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationWithReplies {
    pub annotation: Annotation,
    pub replies: Vec<AnnotationReply>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationWithContent {
    pub annotation: Annotation,
    pub replies: Vec<AnnotationReply>,
    pub content_title: String,
    pub content_slug: String,
    pub content_kind: String,
    pub content_status: String,
}

pub fn annotation_status_label(status: &str) -> &'static str {
    match status {
        "open" => "待处理",
        "addressed" => "AI 已改",
        "resolved" => "已确认",
        "outdated" => "原文已变化",
        _ => "未知",
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    pub token_hash: String,
    pub scopes: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelinePost {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub published_ts: i64,
    pub published_label: String,
    pub published_month_day: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentSummary {
    pub content: Content,
    pub author_name: String,
    pub categories: Vec<Term>,
    pub tags: Vec<Term>,
    pub excerpt_text: String,
    pub published_label: String,
    pub updated_label: String,
    pub promoted_id: Option<i64>,
    pub promoted_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkView {
    pub title: String,
    pub target_id: Option<i64>,
    pub target_kind: Option<String>,
    pub target_slug: Option<String>,
    pub target_admin_url: Option<String>,
    pub promoted_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentView {
    pub content: Content,
    pub author_name: String,
    pub categories: Vec<Term>,
    pub tags: Vec<Term>,
    pub html: String,
    #[serde(skip)]
    pub headings: Vec<crate::markdown::ArticleHeading>,
    pub excerpt_text: String,
    pub reading_minutes: i64,
    pub has_mermaid: bool,
    pub created_label: String,
    pub published_label: String,
    pub updated_label: String,
    pub promoted_slug: Option<String>,
    pub promoted_title: Option<String>,
    pub source_note_title: Option<String>,
    pub backlinks: Vec<ContentSummary>,
    pub outgoing: Vec<LinkView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageLink {
    pub number: i64,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pagination {
    pub current: i64,
    pub total_pages: i64,
    pub total_items: i64,
    pub prev_url: Option<String>,
    pub next_url: Option<String>,
    pub pages: Vec<PageLink>,
}

impl Pagination {
    pub fn build(
        base_path: &str,
        query: Option<&str>,
        page: i64,
        total_items: i64,
        page_size: i64,
    ) -> Self {
        let page_size = page_size.max(1);
        let total_pages = ((total_items + page_size - 1) / page_size).max(1);
        let current = page.clamp(1, total_pages);
        let url = |number: i64| {
            let mut url = format!("{}?page={}", base_path, number);
            if let Some(query) = query
                && !query.is_empty()
            {
                url.push('&');
                url.push_str(query);
            }
            url
        };

        let start = (current - 3).max(1);
        let end = (current + 3).min(total_pages);
        let pages = (start..=end)
            .map(|number| PageLink {
                number,
                url: url(number),
                active: number == current,
            })
            .collect();

        Self {
            current,
            total_pages,
            total_items,
            prev_url: (current > 1).then(|| url(current - 1)),
            next_url: (current < total_pages).then(|| url(current + 1)),
            pages,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub posts: i64,
    pub published_posts: i64,
    pub drafts: i64,
    pub pages: i64,
    pub notes: i64,
    pub pending_comments: i64,
    pub pending_reviews: i64,
    pub total_comments: i64,
    pub media: i64,
}

pub fn kind_from_path(path: &str) -> Option<&'static str> {
    match path {
        "posts" => Some("post"),
        "pages" => Some("page"),
        "notes" => Some("note"),
        _ => None,
    }
}

pub fn kind_to_path(kind: &str) -> &'static str {
    match kind {
        "page" => "pages",
        "note" => "notes",
        _ => "posts",
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "page" => "独立页面",
        "note" => "笔记",
        _ => "文章",
    }
}

pub fn default_status(kind: &str) -> &'static str {
    if kind == "note" { "inbox" } else { "draft" }
}

pub fn statuses_for_kind(kind: &str) -> &'static [&'static str] {
    if kind == "note" {
        NOTE_STATUSES
    } else {
        POST_STATUSES
    }
}

pub fn is_valid_status(kind: &str, status: &str) -> bool {
    statuses_for_kind(kind).contains(&status)
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "draft" => "草稿",
        "published" => "已发布",
        "trash" => "回收站",
        "inbox" => "收集箱",
        "developing" => "整理中",
        "evergreen" => "常青笔记",
        "archived" => "已归档",
        _ => "未知",
    }
}

pub fn review_label(status: &str) -> &'static str {
    match status {
        "pending" => "待 review",
        "approved" => "已通过",
        "changes_requested" => "需修改",
        _ => "未提交",
    }
}

pub fn comment_status_label(status: &str) -> &'static str {
    match status {
        "approved" => "已通过",
        "spam" => "垃圾评论",
        "trash" => "已删除",
        _ => "待审核",
    }
}

pub fn visibility_label(visibility: &str) -> &'static str {
    if visibility == "private" {
        "私密"
    } else {
        "公开"
    }
}
