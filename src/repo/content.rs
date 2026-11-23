use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};

use crate::error::{AppError, AppResult};
use crate::markdown::{self, WikiTarget};
use crate::models::{
    Content, ContentSummary, ContentView, KINDS, LinkView, REVIEW_STATUSES, Term, TimelinePost,
    VISIBILITIES, default_status, is_valid_status, kind_label,
};
use crate::util::{format_date, format_datetime, like_pattern, now_ts, slugify, truncate_chars};

use super::{revision, term};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentOrder {
    PublishedDesc,
    UpdatedDesc,
    CreatedDesc,
}

#[derive(Debug, Clone)]
pub struct ContentFilter {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub query: Option<String>,
    pub term_id: Option<i64>,
    pub include_deleted: bool,
    pub include_private: bool,
    pub review_status: Option<String>,
    pub author_id: Option<i64>,
    pub published_after: Option<i64>,
    pub published_before: Option<i64>,
    pub limit: i64,
    pub offset: i64,
    pub order: ContentOrder,
}

impl Default for ContentFilter {
    fn default() -> Self {
        Self {
            kind: None,
            status: None,
            query: None,
            term_id: None,
            include_deleted: false,
            include_private: false,
            review_status: None,
            author_id: None,
            published_after: None,
            published_before: None,
            limit: 20,
            offset: 0,
            order: ContentOrder::PublishedDesc,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContentInput {
    pub kind: String,
    pub title: String,
    pub slug: Option<String>,
    pub body: String,
    pub excerpt: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub review_status: Option<String>,
    pub review_notes: Option<String>,
    pub allow_comments: Option<bool>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub author_id: i64,
    pub actor: String,
    pub source_note_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ContentPatch {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub body: Option<String>,
    pub excerpt: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub review_status: Option<String>,
    pub review_notes: Option<String>,
    pub allow_comments: Option<bool>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub actor: String,
    pub expected_updated_at: Option<i64>,
    pub revision_note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromoteOptions {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub status: String,
    pub sync: bool,
    pub can_publish: bool,
    pub actor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromoteResult {
    pub id: i64,
    pub slug: String,
    pub created: bool,
}

pub async fn list(
    pool: &SqlitePool,
    filter: &ContentFilter,
    timezone_offset: i32,
) -> AppResult<(Vec<ContentSummary>, i64)> {
    let mut count_query = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM contents c WHERE 1=1");
    push_filters(&mut count_query, filter);
    let total: i64 = count_query.build_query_scalar().fetch_one(pool).await?;

    let mut query = QueryBuilder::<Sqlite>::new("SELECT c.* FROM contents c WHERE 1=1");
    push_filters(&mut query, filter);
    match filter.order {
        ContentOrder::PublishedDesc => {
            query.push(" ORDER BY COALESCE(c.published_at, c.created_at) DESC, c.id DESC");
        }
        ContentOrder::UpdatedDesc => {
            query.push(" ORDER BY c.updated_at DESC, c.id DESC");
        }
        ContentOrder::CreatedDesc => {
            query.push(" ORDER BY c.created_at DESC, c.id DESC");
        }
    }
    query
        .push(" LIMIT ")
        .push_bind(filter.limit.clamp(1, 1000))
        .push(" OFFSET ")
        .push_bind(filter.offset.max(0));

    let rows = query.build_query_as::<Content>().fetch_all(pool).await?;
    let items = hydrate(pool, rows, timezone_offset).await?;
    Ok((items, total))
}

fn push_filters(query: &mut QueryBuilder<Sqlite>, filter: &ContentFilter) {
    if let Some(kind) = filter.kind.as_deref().filter(|value| *value != "all") {
        query.push(" AND c.kind = ").push_bind(kind.to_string());
    }
    if let Some(status) = filter.status.as_deref().filter(|value| *value != "all") {
        query.push(" AND c.status = ").push_bind(status.to_string());
    }
    if let Some(query_text) = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let pattern = like_pattern(query_text);
        query
            .push(" AND (c.title LIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR c.body LIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
    if let Some(term_id) = filter.term_id {
        query
            .push(" AND EXISTS (SELECT 1 FROM content_terms ct WHERE ct.content_id = c.id AND ct.term_id = ")
            .push_bind(term_id)
            .push(")");
    }
    if !filter.include_deleted {
        query.push(" AND c.deleted_at IS NULL AND c.status != 'trash'");
    }
    if !filter.include_private {
        query.push(" AND c.visibility = 'public'");
    }
    if let Some(review_status) = filter.review_status.as_deref() {
        query
            .push(" AND c.review_status = ")
            .push_bind(review_status.to_string());
    }
    if let Some(author_id) = filter.author_id {
        query.push(" AND c.author_id = ").push_bind(author_id);
    }
    if let Some(after) = filter.published_after {
        query
            .push(" AND COALESCE(c.published_at, c.created_at) >= ")
            .push_bind(after);
    }
    if let Some(before) = filter.published_before {
        query
            .push(" AND COALESCE(c.published_at, c.created_at) < ")
            .push_bind(before);
    }
}

#[derive(sqlx::FromRow)]
struct TimelineRow {
    id: i64,
    slug: String,
    title: String,
    published_ts: i64,
}

fn map_timeline(rows: Vec<TimelineRow>, timezone_offset: i32) -> Vec<TimelinePost> {
    rows.into_iter()
        .map(|row| TimelinePost {
            id: row.id,
            slug: row.slug,
            title: row.title,
            published_ts: row.published_ts,
            published_label: format_date(row.published_ts, timezone_offset, "%Y-%m-%d"),
            published_month_day: format_date(row.published_ts, timezone_offset, "%m-%d"),
        })
        .collect()
}

async fn fetch_timeline(
    pool: &SqlitePool,
    limit: Option<i64>,
    before: Option<(i64, i64)>,
) -> AppResult<Vec<TimelineRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, slug, title, COALESCE(published_at, created_at) AS published_ts \
         FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
           AND deleted_at IS NULL",
    );
    if let Some((published_ts, id)) = before {
        query
            .push(" AND (COALESCE(published_at, created_at) < ")
            .push_bind(published_ts)
            .push(" OR (COALESCE(published_at, created_at) = ")
            .push_bind(published_ts)
            .push(" AND id < ")
            .push_bind(id)
            .push("))");
    }
    query.push(" ORDER BY published_ts DESC, id DESC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit);
    }

    Ok(query
        .build_query_as::<TimelineRow>()
        .fetch_all(pool)
        .await?)
}

/// Lightweight homepage timeline: only the fields the timeline renders.
///
/// Avoids loading article bodies, generating excerpts, and hydrating terms for
/// every post, which would make the timeline scale with body size rather than
/// just with the number of titles.
pub async fn timeline(pool: &SqlitePool, timezone_offset: i32) -> AppResult<Vec<TimelinePost>> {
    let rows = fetch_timeline(pool, None, None).await?;
    Ok(map_timeline(rows, timezone_offset))
}

/// Fetch one keyset-paginated timeline page. Returns whether more posts exist.
pub async fn timeline_page(
    pool: &SqlitePool,
    limit: i64,
    before: Option<(i64, i64)>,
    timezone_offset: i32,
) -> AppResult<(Vec<TimelinePost>, bool)> {
    let limit = limit.clamp(1, 500);
    let mut rows = fetch_timeline(pool, Some(limit + 1), before).await?;
    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    Ok((map_timeline(rows, timezone_offset), has_more))
}

pub async fn count_timeline(pool: &SqlitePool) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
           AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?)
}

pub async fn count_by_status(pool: &SqlitePool, kind: &str) -> AppResult<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM contents WHERE kind = ? GROUP BY status")
            .bind(kind)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub async fn get_by_id(pool: &SqlitePool, id: i64) -> AppResult<Option<Content>> {
    Ok(sqlx::query_as::<_, Content>(
        "SELECT id, kind, title, slug, body, excerpt, status, visibility, author_id, created_at, \
                updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at \
         FROM contents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_by_slug(pool: &SqlitePool, slug: &str) -> AppResult<Option<Content>> {
    Ok(sqlx::query_as::<_, Content>(
        "SELECT id, kind, title, slug, body, excerpt, status, visibility, author_id, created_at, \
                updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at \
         FROM contents WHERE slug = ?",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await?)
}

/// Render a proposed body with the same wiki-link resolution as the live view.
pub async fn render_body(
    pool: &SqlitePool,
    content_id: i64,
    body: &str,
    for_admin: bool,
) -> AppResult<String> {
    let wiki = build_wiki_map(pool, content_id, for_admin).await?;
    Ok(markdown::render_markdown(body, &wiki))
}

pub async fn get_view(
    pool: &SqlitePool,
    id: i64,
    timezone_offset: i32,
    for_admin: bool,
) -> AppResult<ContentView> {
    let content = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;

    let author_name = author_name(pool, content.author_id).await?;
    let terms = term::terms_for_contents(pool, &[id]).await?;
    let terms = terms.get(&id).cloned().unwrap_or_default();
    let (categories, tags): (Vec<Term>, Vec<Term>) =
        terms.into_iter().partition(|term| term.kind == "category");

    let wiki = build_wiki_map(pool, id, for_admin).await?;
    let html = markdown::render_markdown(&content.body, &wiki);
    let excerpt_text = content
        .excerpt
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| markdown::excerpt(&content.body, 240));

    let (promoted_slug, promoted_title) = match content.promoted_to_id {
        Some(post_id) => match get_by_id(pool, post_id).await? {
            Some(post) => (Some(post.slug), Some(post.title)),
            None => (None, None),
        },
        None => (None, None),
    };

    let source_note_title = match content.source_note_id {
        Some(note_id) => get_by_id(pool, note_id).await?.map(|note| note.title),
        None => None,
    };

    let backlink_rows = sqlx::query_as::<_, Content>(
        "SELECT c.id, c.kind, c.title, c.slug, c.body, c.excerpt, c.status, c.visibility, \
                c.author_id, c.created_at, c.updated_at, c.published_at, c.review_status, \
                c.review_notes, c.allow_comments, c.source_note_id, c.promoted_to_id, \
                c.metadata, c.deleted_at \
         FROM links l JOIN contents c ON c.id = l.source_id \
         WHERE l.target_id = ? AND c.deleted_at IS NULL AND c.status != 'trash' \
         ORDER BY c.updated_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let backlinks = hydrate(pool, backlink_rows, timezone_offset).await?;

    let outgoing = outgoing_links(pool, id).await?;
    let reading_minutes = markdown::reading_minutes(&content.body);
    let has_mermaid = markdown::contains_mermaid(&content.body);
    let created_label = format_datetime(content.created_at, timezone_offset);
    let published_label = format_date(
        content.published_at.unwrap_or(content.created_at),
        timezone_offset,
        "%Y-%m-%d",
    );
    let updated_label = format_datetime(content.updated_at, timezone_offset);

    Ok(ContentView {
        content,
        author_name,
        categories,
        tags,
        html,
        excerpt_text,
        reading_minutes,
        has_mermaid,
        created_label,
        published_label,
        updated_label,
        promoted_slug,
        promoted_title,
        source_note_title,
        backlinks,
        outgoing,
    })
}

pub async fn adjacent_posts(
    pool: &SqlitePool,
    content: &Content,
) -> AppResult<(Option<(String, String)>, Option<(String, String)>)> {
    let timestamp = content.published_at.unwrap_or(content.created_at);
    let older = sqlx::query_as::<_, (String, String)>(
        "SELECT title, slug FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
           AND deleted_at IS NULL AND id != ? \
           AND COALESCE(published_at, created_at) < ? \
         ORDER BY COALESCE(published_at, created_at) DESC, id DESC LIMIT 1",
    )
    .bind(content.id)
    .bind(timestamp)
    .fetch_optional(pool)
    .await?;

    let newer = sqlx::query_as::<_, (String, String)>(
        "SELECT title, slug FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
           AND deleted_at IS NULL AND id != ? \
           AND COALESCE(published_at, created_at) > ? \
         ORDER BY COALESCE(published_at, created_at) ASC, id ASC LIMIT 1",
    )
    .bind(content.id)
    .bind(timestamp)
    .fetch_optional(pool)
    .await?;

    Ok((older, newer))
}

async fn outgoing_links(pool: &SqlitePool, source_id: i64) -> AppResult<Vec<LinkView>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        target_title: String,
        target_id: Option<i64>,
        target_kind: Option<String>,
        target_slug: Option<String>,
        target_promoted_to_id: Option<i64>,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT l.target_title, l.target_id, c.kind AS target_kind, c.slug AS target_slug, \
                c.promoted_to_id AS target_promoted_to_id \
         FROM links l LEFT JOIN contents c ON c.id = l.target_id \
         WHERE l.source_id = ? ORDER BY l.target_title COLLATE NOCASE",
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let promoted_slug = match row.target_promoted_to_id {
            Some(id) => get_by_id(pool, id).await?.map(|content| content.slug),
            None => None,
        };
        let target_admin_url = row.target_id.map(|id| {
            format!(
                "/admin/{}/{}/edit",
                crate::models::kind_to_path(row.target_kind.as_deref().unwrap_or("note")),
                id
            )
        });
        result.push(LinkView {
            title: row.target_title,
            target_id: row.target_id,
            target_kind: row.target_kind,
            target_slug: row.target_slug,
            target_admin_url,
            promoted_slug,
        });
    }
    Ok(result)
}

async fn build_wiki_map(
    pool: &SqlitePool,
    source_id: i64,
    for_admin: bool,
) -> AppResult<HashMap<String, WikiTarget>> {
    #[derive(sqlx::FromRow)]
    struct LinkRow {
        target_title: String,
        target_id: Option<i64>,
    }

    let rows = sqlx::query_as::<_, LinkRow>(
        "SELECT target_title, target_id FROM links WHERE source_id = ?",
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    let target_ids: Vec<i64> = rows.iter().filter_map(|row| row.target_id).collect();
    let targets = load_by_ids(pool, &target_ids).await?;
    let mut map = HashMap::new();

    for row in rows {
        let target = row.target_id.and_then(|id| targets.get(&id));
        let href = match target {
            Some(target) => target_href(pool, target, for_admin).await?,
            None => None,
        };
        let label = target
            .map(|target| target.title.clone())
            .unwrap_or_else(|| row.target_title.clone());
        let wiki = WikiTarget { label, href };
        map.insert(row.target_title.clone(), wiki.clone());
        map.insert(row.target_title.to_lowercase(), wiki);
    }

    Ok(map)
}

async fn target_href(
    pool: &SqlitePool,
    target: &Content,
    for_admin: bool,
) -> AppResult<Option<String>> {
    if for_admin {
        return Ok(Some(format!(
            "/admin/{}/{}/edit",
            crate::models::kind_to_path(&target.kind),
            target.id
        )));
    }

    if target.is_public() {
        return Ok(Some(match target.kind.as_str() {
            "page" => format!("/pages/{}", target.slug),
            _ => format!("/archives/{}", target.slug),
        }));
    }

    if let Some(post_id) = target.promoted_to_id
        && let Some(post) = get_by_id(pool, post_id).await?
        && post.is_public()
    {
        return Ok(Some(format!("/archives/{}", post.slug)));
    }

    Ok(None)
}

async fn author_name(pool: &SqlitePool, author_id: i64) -> AppResult<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id = ?")
            .bind(author_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or_else(|| "未知作者".to_string()),
    )
}

pub async fn hydrate(
    pool: &SqlitePool,
    contents: Vec<Content>,
    timezone_offset: i32,
) -> AppResult<Vec<ContentSummary>> {
    if contents.is_empty() {
        return Ok(Vec::new());
    }

    let content_ids: Vec<i64> = contents.iter().map(|content| content.id).collect();
    let author_ids: Vec<i64> = contents
        .iter()
        .map(|content| content.author_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut author_query =
        QueryBuilder::<Sqlite>::new("SELECT id, display_name FROM users WHERE id IN (");
    let mut separated = author_query.separated(", ");
    for id in &author_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    #[derive(sqlx::FromRow)]
    struct AuthorRow {
        id: i64,
        display_name: String,
    }

    let authors: HashMap<i64, String> = author_query
        .build_query_as::<AuthorRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row.display_name))
        .collect();

    let terms = term::terms_for_contents(pool, &content_ids).await?;

    let promoted_ids: Vec<i64> = contents
        .iter()
        .filter_map(|content| content.promoted_to_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let promoted = load_by_ids(pool, &promoted_ids).await?;

    let mut summaries = Vec::with_capacity(contents.len());
    for content in contents {
        let content_terms = terms.get(&content.id).cloned().unwrap_or_default();
        let (categories, tags): (Vec<Term>, Vec<Term>) = content_terms
            .into_iter()
            .partition(|term| term.kind == "category");
        let excerpt_text = content
            .excerpt
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| markdown::excerpt(&content.body, 200));
        let promoted_slug = content
            .promoted_to_id
            .and_then(|id| promoted.get(&id))
            .map(|post| post.slug.clone());
        let published_ts = content.published_at.unwrap_or(content.created_at);
        let promoted_id = content.promoted_to_id;

        summaries.push(ContentSummary {
            author_name: authors
                .get(&content.author_id)
                .cloned()
                .unwrap_or_else(|| "未知作者".to_string()),
            published_label: format_date(published_ts, timezone_offset, "%Y-%m-%d"),
            updated_label: format_datetime(content.updated_at, timezone_offset),
            categories,
            tags,
            excerpt_text,
            promoted_id,
            promoted_slug,
            content,
        });
    }

    Ok(summaries)
}

pub async fn create(pool: &SqlitePool, input: ContentInput) -> AppResult<i64> {
    if !KINDS.contains(&input.kind.as_str()) {
        return Err(AppError::bad_request("未知的内容类型"));
    }
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::bad_request("标题不能为空"));
    }

    let status = input
        .status
        .clone()
        .unwrap_or_else(|| default_status(&input.kind).to_string());
    if !is_valid_status(&input.kind, &status) {
        return Err(AppError::bad_request("非法的内容状态"));
    }
    let visibility = input
        .visibility
        .clone()
        .unwrap_or_else(|| "public".to_string());
    if !VISIBILITIES.contains(&visibility.as_str()) {
        return Err(AppError::bad_request("非法的可见性"));
    }
    let review_status = input
        .review_status
        .clone()
        .unwrap_or_else(|| "none".to_string());
    if !REVIEW_STATUSES.contains(&review_status.as_str()) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }

    let slug_base = input.slug.as_deref().unwrap_or(&title);
    let slug = unique_slug(pool, slug_base, None).await?;
    let now = now_ts();
    let published_at = (status == "published").then_some(now);
    let excerpt = input
        .excerpt
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| markdown::excerpt(&input.body, 240));
    let allow_comments = input.allow_comments.unwrap_or(true) as i64;

    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO contents(kind, title, slug, body, excerpt, status, visibility, author_id, \
                created_at, updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, '{}', NULL)",
    )
    .bind(&input.kind)
    .bind(&title)
    .bind(&slug)
    .bind(&input.body)
    .bind(&excerpt)
    .bind(&status)
    .bind(&visibility)
    .bind(input.author_id)
    .bind(now)
    .bind(now)
    .bind(published_at)
    .bind(&review_status)
    .bind(input.review_notes.as_deref())
    .bind(allow_comments)
    .bind(input.source_note_id)
    .execute(&mut *transaction)
    .await;

    let result = match result {
        Ok(result) => result,
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(AppError::conflict("缩略名或来源笔记已被使用"));
        }
        Err(error) => return Err(error.into()),
    };

    let id = result.last_insert_rowid();
    term::replace_terms_tx(&mut transaction, id, input.category.as_deref(), &input.tags).await?;
    sync_links_tx(&mut transaction, id, &input.body).await?;
    revision::insert_tx(
        &mut transaction,
        id,
        &title,
        &input.body,
        &status,
        &input.actor,
        Some("创建"),
        now,
    )
    .await?;
    transaction.commit().await?;

    if input.kind == "note" {
        resolve_pending_links(pool, id, &title, &slug).await?;
    }

    Ok(id)
}

pub async fn update(pool: &SqlitePool, id: i64, patch: ContentPatch) -> AppResult<Content> {
    update_with_publish_permission(pool, id, patch, true).await
}

pub async fn update_with_publish_permission(
    pool: &SqlitePool,
    id: i64,
    patch: ContentPatch,
    can_publish: bool,
) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;

    if current.status == "published" && !can_publish {
        return Err(AppError::forbidden("修改已发布文章需要 publish 权限"));
    }

    if let Some(expected) = patch.expected_updated_at
        && current.updated_at != expected
    {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    let title = patch
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| current.title.clone());

    let body = patch.body.clone().unwrap_or_else(|| current.body.clone());
    let status = patch
        .status
        .clone()
        .unwrap_or_else(|| current.status.clone());
    if !is_valid_status(&current.kind, &status) {
        return Err(AppError::bad_request("非法的内容状态"));
    }
    let visibility = patch
        .visibility
        .clone()
        .unwrap_or_else(|| current.visibility.clone());
    if !VISIBILITIES.contains(&visibility.as_str()) {
        return Err(AppError::bad_request("非法的可见性"));
    }
    let review_status = patch
        .review_status
        .clone()
        .unwrap_or_else(|| current.review_status.clone());
    if !REVIEW_STATUSES.contains(&review_status.as_str()) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }
    let review_notes = patch
        .review_notes
        .clone()
        .or_else(|| current.review_notes.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let slug = match patch
        .slug
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => unique_slug(pool, value, Some(id)).await?,
        None => current.slug.clone(),
    };

    let excerpt = match patch.excerpt.as_deref() {
        Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Some(_) => Some(markdown::excerpt(&body, 240)),
        None if body != current.body => Some(markdown::excerpt(&body, 240)),
        None => current.excerpt.clone(),
    };

    let now = now_ts().max(current.updated_at.saturating_add(1));
    let published_at = if status == "published" {
        current.published_at.or(Some(now))
    } else {
        current.published_at
    };
    let deleted_at = if status == "trash" {
        current.deleted_at.or(Some(now))
    } else {
        None
    };
    let allow_comments = patch
        .allow_comments
        .map(|value| value as i64)
        .unwrap_or(current.allow_comments);

    let mut transaction = pool.begin().await?;
    let update = sqlx::query(
        "UPDATE contents SET title = ?, slug = ?, body = ?, excerpt = ?, status = ?, \
                visibility = ?, updated_at = ?, published_at = ?, review_status = ?, \
                review_notes = ?, allow_comments = ?, deleted_at = ? \
         WHERE id = ? AND updated_at = ? AND (? OR status != 'published')",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&body)
    .bind(&excerpt)
    .bind(&status)
    .bind(&visibility)
    .bind(now)
    .bind(published_at)
    .bind(&review_status)
    .bind(&review_notes)
    .bind(allow_comments)
    .bind(deleted_at)
    .bind(id)
    .bind(current.updated_at)
    .bind(can_publish)
    .execute(&mut *transaction)
    .await?;
    if update.rows_affected() != 1 {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    if let Some(tags) = &patch.tags {
        term::set_tags_tx(&mut transaction, id, tags).await?;
    }
    if let Some(category) = &patch.category {
        term::set_category_tx(&mut transaction, id, Some(category)).await?;
    }
    if body != current.body {
        sync_links_tx(&mut transaction, id, &body).await?;
    }
    revision::insert_tx(
        &mut transaction,
        id,
        &title,
        &body,
        &status,
        &patch.actor,
        patch.revision_note.as_deref().or(Some("更新")),
        now,
    )
    .await?;
    transaction.commit().await?;

    if current.kind == "note" && (title != current.title || slug != current.slug) {
        resolve_pending_links(pool, id, &title, &slug).await?;
    }

    get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("更新后读取内容失败"))
}

pub(super) async fn get_by_id_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> AppResult<Option<Content>> {
    Ok(sqlx::query_as::<_, Content>(
        "SELECT id, kind, title, slug, body, excerpt, status, visibility, author_id, created_at, \
                updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at \
         FROM contents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?)
}

pub(super) struct ProposalContentUpdate<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub excerpt: Option<&'a str>,
    pub actor: &'a str,
    pub revision_note: &'a str,
    pub can_publish: bool,
}

pub(super) async fn apply_proposal_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    current: &Content,
    update: ProposalContentUpdate<'_>,
) -> AppResult<(Content, i64)> {
    if current.status == "published" && !update.can_publish {
        return Err(AppError::forbidden("修改已发布文章需要 publish 权限"));
    }

    let title = if update.title.trim().is_empty() {
        current.title.clone()
    } else {
        update.title.trim().to_string()
    };
    let body = update.body.trim();
    if body.is_empty() {
        return Err(AppError::bad_request("版本正文不能为空"));
    }
    let excerpt = match update.excerpt {
        Some(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => Some(markdown::excerpt(body, 240)),
    };
    let now = now_ts().max(current.updated_at.saturating_add(1));

    let result = sqlx::query(
        "UPDATE contents SET title = ?, body = ?, excerpt = ?, updated_at = ? \
         WHERE id = ? AND updated_at = ? AND (? OR status != 'published')",
    )
    .bind(&title)
    .bind(body)
    .bind(&excerpt)
    .bind(now)
    .bind(current.id)
    .bind(current.updated_at)
    .bind(update.can_publish)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::conflict("内容已被其他人修改，请重新读取后再提交"));
    }

    if body != current.body {
        sync_links_tx(transaction, current.id, body).await?;
    }
    let revision_id = revision::insert_tx(
        transaction,
        current.id,
        &title,
        body,
        &current.status,
        update.actor,
        Some(update.revision_note),
        now,
    )
    .await?;
    let updated = get_by_id_in_transaction(transaction, current.id)
        .await?
        .ok_or_else(|| AppError::internal("应用版本后读取内容失败"))?;
    Ok((updated, revision_id))
}

pub async fn publish(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.kind == "note" {
        return Err(AppError::bad_request("笔记请使用 promote 转为文章"));
    }
    if current.status == "trash" || current.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站中的内容不能直接发布"));
    }

    update(
        pool,
        id,
        ContentPatch {
            status: Some("published".to_string()),
            review_status: (current.review_status == "pending").then(|| "approved".to_string()),
            actor: actor.to_string(),
            revision_note: Some("发布".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn trash(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    update(
        pool,
        id,
        ContentPatch {
            status: Some("trash".to_string()),
            actor: actor.to_string(),
            revision_note: Some("移入回收站".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn restore(pool: &SqlitePool, id: i64, actor: &str) -> AppResult<Content> {
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let status = if current.kind == "note" {
        "inbox"
    } else {
        "draft"
    };
    update(
        pool,
        id,
        ContentPatch {
            status: Some(status.to_string()),
            actor: actor.to_string(),
            revision_note: Some("恢复".to_string()),
            ..Default::default()
        },
    )
    .await
}

pub async fn delete_permanent(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM contents WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_review(
    pool: &SqlitePool,
    id: i64,
    review_status: &str,
    review_notes: Option<&str>,
    actor: &str,
) -> AppResult<Content> {
    if !REVIEW_STATUSES.contains(&review_status) {
        return Err(AppError::bad_request("非法的 review 状态"));
    }
    let current = get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let now = now_ts().max(current.updated_at.saturating_add(1));
    let notes = review_notes
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE contents SET review_status = ?, review_notes = ?, updated_at = ? WHERE id = ?",
    )
    .bind(review_status)
    .bind(notes)
    .bind(now)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    revision::insert_tx(
        &mut transaction,
        id,
        &current.title,
        &current.body,
        &current.status,
        actor,
        Some(&format!("review: {review_status}")),
        now,
    )
    .await?;
    transaction.commit().await?;

    get_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("review 后读取内容失败"))
}

pub async fn promote_note(
    pool: &SqlitePool,
    note_id: i64,
    options: PromoteOptions,
) -> AppResult<PromoteResult> {
    let note = get_by_id(pool, note_id)
        .await?
        .ok_or_else(|| AppError::not_found("笔记不存在"))?;
    if note.kind != "note" {
        return Err(AppError::bad_request("只有笔记可以转为文章"));
    }
    if note.status == "trash" || note.deleted_at.is_some() {
        return Err(AppError::bad_request("回收站中的笔记不能转为文章"));
    }

    if let Some(post_id) = note.promoted_to_id {
        let post = get_by_id(pool, post_id)
            .await?
            .ok_or_else(|| AppError::not_found("关联文章不存在"))?;
        if options.sync {
            let terms = term::terms_for_contents(pool, &[note.id]).await?;
            let terms = terms.get(&note.id).cloned().unwrap_or_default();
            let tags: Vec<String> = terms
                .iter()
                .filter(|term| term.kind == "tag")
                .map(|term| term.name.clone())
                .collect();
            let category = terms
                .iter()
                .find(|term| term.kind == "category")
                .map(|term| term.name.clone());
            update_with_publish_permission(
                pool,
                post.id,
                ContentPatch {
                    title: Some(note.title.clone()),
                    body: Some(note.body.clone()),
                    excerpt: note.excerpt.clone(),
                    tags: Some(tags),
                    category: Some(category.unwrap_or_default()),
                    actor: options.actor,
                    revision_note: Some("从笔记同步".to_string()),
                    ..Default::default()
                },
                options.can_publish,
            )
            .await?;
        }
        return Ok(PromoteResult {
            id: post.id,
            slug: post.slug,
            created: false,
        });
    }

    let status = if options.status == "published" {
        "published"
    } else {
        "draft"
    };
    if status == "published" && !options.can_publish {
        return Err(AppError::forbidden("发布文章需要 publish 权限"));
    }
    let terms = term::terms_for_contents(pool, &[note.id]).await?;
    let terms = terms.get(&note.id).cloned().unwrap_or_default();
    let tags: Vec<String> = terms
        .iter()
        .filter(|term| term.kind == "tag")
        .map(|term| term.name.clone())
        .collect();
    let category = terms
        .iter()
        .find(|term| term.kind == "category")
        .map(|term| term.name.clone());

    let title = options.title.unwrap_or_else(|| note.title.clone());
    let result = create(
        pool,
        ContentInput {
            kind: "post".to_string(),
            title,
            slug: options.slug,
            body: note.body.clone(),
            excerpt: note.excerpt.clone(),
            status: Some(status.to_string()),
            visibility: Some("public".to_string()),
            review_status: Some(note.review_status.clone()),
            review_notes: note.review_notes.clone(),
            allow_comments: Some(note.allow_comments != 0),
            category,
            tags,
            author_id: note.author_id,
            actor: options.actor,
            source_note_id: Some(note.id),
        },
    )
    .await;

    let post_id = match result {
        Ok(id) => id,
        Err(error) => {
            // If another request won the race, return the existing promoted post.
            if let Some(existing) =
                sqlx::query_scalar::<_, i64>("SELECT id FROM contents WHERE source_note_id = ?")
                    .bind(note.id)
                    .fetch_optional(pool)
                    .await?
            {
                existing
            } else {
                return Err(error);
            }
        }
    };

    let post = get_by_id(pool, post_id)
        .await?
        .ok_or_else(|| AppError::internal("转为文章后读取失败"))?;

    let now = now_ts().max(note.updated_at.saturating_add(1));
    sqlx::query("UPDATE contents SET promoted_to_id = ?, updated_at = ? WHERE id = ?")
        .bind(post_id)
        .bind(now)
        .bind(note.id)
        .execute(pool)
        .await?;

    Ok(PromoteResult {
        id: post.id,
        slug: post.slug,
        created: true,
    })
}

pub async fn unique_slug(
    pool: &SqlitePool,
    base: &str,
    exclude_id: Option<i64>,
) -> AppResult<String> {
    let base = slugify(base);
    let mut candidate = base.clone();
    let mut counter = 2_i64;

    loop {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT id FROM contents WHERE slug = ? AND (? IS NULL OR id != ?)")
                .bind(&candidate)
                .bind(exclude_id)
                .bind(exclude_id)
                .fetch_optional(pool)
                .await?;

        if exists.is_none() {
            return Ok(candidate);
        }

        candidate = format!("{}-{}", truncate_chars(&base, 110), counter);
        counter += 1;
        if counter > 10_000 {
            return Err(AppError::conflict("无法生成唯一的缩略名"));
        }
    }
}

async fn sync_links_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    source_id: i64,
    body: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM links WHERE source_id = ?")
        .bind(source_id)
        .execute(&mut **transaction)
        .await?;

    for title in markdown::wiki_titles(body) {
        let target_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM contents WHERE kind = 'note' AND deleted_at IS NULL \
             AND (title = ? COLLATE NOCASE OR slug = ? COLLATE NOCASE) \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(&title)
        .bind(&title)
        .fetch_optional(&mut **transaction)
        .await?;

        sqlx::query(
            "INSERT OR REPLACE INTO links(source_id, target_title, target_id) VALUES(?, ?, ?)",
        )
        .bind(source_id)
        .bind(&title)
        .bind(target_id)
        .execute(&mut **transaction)
        .await?;

        if let Some(target_id) = target_id {
            sqlx::query(
                "UPDATE links SET target_id = ? WHERE target_id IS NULL AND target_title = ? COLLATE NOCASE",
            )
            .bind(target_id)
            .bind(&title)
            .execute(&mut **transaction)
            .await?;
        }
    }

    Ok(())
}

pub(super) async fn resolve_pending_links(
    pool: &SqlitePool,
    target_id: i64,
    title: &str,
    slug: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE links SET target_id = ? WHERE target_id IS NULL \
         AND (target_title = ? COLLATE NOCASE OR target_title = ? COLLATE NOCASE)",
    )
    .bind(target_id)
    .bind(title)
    .bind(slug)
    .execute(pool)
    .await?;
    Ok(())
}

async fn load_by_ids(pool: &SqlitePool, ids: &[i64]) -> AppResult<HashMap<i64, Content>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, kind, title, slug, body, excerpt, status, visibility, author_id, created_at, \
                updated_at, published_at, review_status, review_notes, allow_comments, \
                source_note_id, promoted_to_id, metadata, deleted_at \
         FROM contents WHERE id IN (",
    );
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let rows = query.build_query_as::<Content>().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|content| (content.id, content))
        .collect())
}

pub fn status_label_for(kind: &str, status: &str) -> &'static str {
    let _ = kind;
    crate::models::status_label(status)
}

pub fn content_kind_label(kind: &str) -> &'static str {
    kind_label(kind)
}
