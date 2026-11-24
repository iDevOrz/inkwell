//! Read queries: listing, timeline, counts, lookups and summary hydration.

use super::*;

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

pub(super) fn push_filters(query: &mut QueryBuilder<Sqlite>, filter: &ContentFilter) {
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
pub(super) struct TimelineRow {
    id: i64,
    slug: String,
    title: String,
    published_ts: i64,
}

pub(super) fn map_timeline(rows: Vec<TimelineRow>, timezone_offset: i32) -> Vec<TimelinePost> {
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

pub(super) async fn fetch_timeline(
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

pub(super) async fn load_by_ids(
    pool: &SqlitePool,
    ids: &[i64],
) -> AppResult<HashMap<i64, Content>> {
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
