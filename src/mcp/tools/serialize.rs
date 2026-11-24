//! JSON shapes returned by tools.

use super::*;

pub(super) fn summary_json(item: &ContentSummary) -> Value {
    json!({
        "id": item.content.id,
        "kind": item.content.kind,
        "title": item.content.title,
        "slug": item.content.slug,
        "status": item.content.status,
        "visibility": item.content.visibility,
        "review_status": item.content.review_status,
        "review_notes": item.content.review_notes,
        "excerpt": item.excerpt_text,
        "created_at": item.content.created_at,
        "updated_at": item.content.updated_at,
        "published_at": item.content.published_at,
        "source_note_id": item.content.source_note_id,
        "promoted_to_id": item.content.promoted_to_id,
        "promoted_id": item.promoted_id,
        "promoted_slug": item.promoted_slug,
        "categories": item.categories.iter().map(|term| json!({
            "id": term.id, "name": term.name, "slug": term.slug
        })).collect::<Vec<_>>(),
        "tags": item.tags.iter().map(|term| json!({
            "id": term.id, "name": term.name, "slug": term.slug
        })).collect::<Vec<_>>()
    })
}

pub(super) fn content_view_json(view: &ContentView) -> Value {
    json!({
        "content": {
            "id": view.content.id,
            "kind": view.content.kind,
            "title": view.content.title,
            "slug": view.content.slug,
            "body": view.content.body,
            "excerpt": view.content.excerpt,
            "status": view.content.status,
            "visibility": view.content.visibility,
            "author_id": view.content.author_id,
            "author_name": view.author_name,
            "created_at": view.content.created_at,
            "updated_at": view.content.updated_at,
            "published_at": view.content.published_at,
            "review_status": view.content.review_status,
            "review_notes": view.content.review_notes,
            "allow_comments": view.content.allow_comments != 0,
            "source_note_id": view.content.source_note_id,
            "promoted_to_id": view.content.promoted_to_id
        },
        "html": view.html,
        "excerpt_text": view.excerpt_text,
        "reading_minutes": view.reading_minutes,
        "has_mermaid": view.has_mermaid,
        "published_label": view.published_label,
        "updated_label": view.updated_label,
        "categories": view.categories.iter().map(|term| json!({
            "id": term.id, "name": term.name, "slug": term.slug
        })).collect::<Vec<_>>(),
        "tags": view.tags.iter().map(|term| json!({
            "id": term.id, "name": term.name, "slug": term.slug
        })).collect::<Vec<_>>(),
        "promoted_slug": view.promoted_slug,
        "promoted_title": view.promoted_title,
        "source_note_title": view.source_note_title,
        "backlinks": view.backlinks.iter().map(summary_json).collect::<Vec<_>>(),
        "outgoing": view.outgoing.iter().map(|link| json!({
            "title": link.title,
            "target_id": link.target_id,
            "target_kind": link.target_kind,
            "target_slug": link.target_slug,
            "promoted_slug": link.promoted_slug
        })).collect::<Vec<_>>()
    })
}

pub(super) fn comment_json(comment: &crate::models::Comment) -> Value {
    json!({
        "id": comment.id,
        "content_id": comment.content_id,
        "parent_id": comment.parent_id,
        "author": comment.author,
        "email": comment.email,
        "url": comment.url,
        "body": comment.body,
        "status": comment.status,
        "created_at": comment.created_at
    })
}

pub(super) fn comment_with_content_json(row: &crate::models::CommentWithContent) -> Value {
    let mut value = comment_json(&row.comment);
    value["content_title"] = json!(row.content_title);
    value["content_slug"] = json!(row.content_slug);
    value["content_kind"] = json!(row.content_kind);
    value
}
