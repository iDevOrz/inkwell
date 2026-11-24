//! Assembling the full view of one piece of content.

use super::*;

/// Render a proposed body with the same wiki-link resolution as the live view.
pub async fn render_body(pool: &SqlitePool, body: &str, for_admin: bool) -> AppResult<String> {
    // Resolve the proposal's own [[links]] (not the live body's links table),
    // so the preview matches what applying the proposal will render.
    let mut links = Vec::new();
    for title in markdown::wiki_titles(body) {
        let target_id = resolve_note_id(pool, &title).await?;
        links.push((title, target_id));
    }
    let wiki = wiki_map_for_links(pool, links, for_admin).await?;
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

pub(super) async fn outgoing_links(pool: &SqlitePool, source_id: i64) -> AppResult<Vec<LinkView>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        target_title: String,
        target_id: Option<i64>,
        target_kind: Option<String>,
        target_slug: Option<String>,
        promoted_slug: Option<String>,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT l.target_title, l.target_id, c.kind AS target_kind, c.slug AS target_slug, \
                p.slug AS promoted_slug \
         FROM links l \
         LEFT JOIN contents c ON c.id = l.target_id \
         LEFT JOIN contents p ON p.id = c.promoted_to_id \
         WHERE l.source_id = ? ORDER BY l.target_title COLLATE NOCASE",
    )
    .bind(source_id)
    .fetch_all(pool)
    .await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
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
            promoted_slug: row.promoted_slug,
        });
    }
    Ok(result)
}

pub(super) async fn author_name(pool: &SqlitePool, author_id: i64) -> AppResult<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT display_name FROM users WHERE id = ?")
            .bind(author_id)
            .fetch_optional(pool)
            .await?
            .unwrap_or_else(|| "未知作者".to_string()),
    )
}
