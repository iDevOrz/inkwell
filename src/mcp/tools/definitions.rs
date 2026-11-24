//! Tool names, descriptions and JSON input schemas returned by tools/list.

use super::*;

pub fn definitions() -> Value {
    let tools = vec![
        tool(
            "instance_info",
            "Return opaque installation and runtime IDs so migration tools can verify their SQLite target matches this MCP server. An optional probe checks the exact database file. Requires write scope.",
            json!({
                "probe": {"type": "string", "description": "Optional one-time migration target probe"}
            }),
            &[],
        ),
        tool(
            "content_list",
            "List posts, pages and notes. Returns summaries without the full body.",
            json!({
                "kind": {"type": "string", "enum": ["post", "page", "note"]},
                "status": {"type": "string", "description": "draft, published, inbox, developing, evergreen, archived, trash"},
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
                "offset": {"type": "integer", "minimum": 0, "default": 0},
                "include_private": {"type": "boolean", "default": true},
                "include_deleted": {"type": "boolean", "default": false}
            }),
            &[],
        ),
        tool(
            "content_get",
            "Get one piece of content by id or slug, including body, HTML, tags, links and revisions metadata.",
            json!({
                "id": {"type": "integer"},
                "slug": {"type": "string"}
            }),
            &[],
        ),
        tool(
            "content_create",
            "Create a post, page or note. Defaults to draft/inbox and does not publish.",
            json!({
                "kind": {"type": "string", "enum": ["post", "page", "note"]},
                "title": {"type": "string"},
                "body": {"type": "string", "description": "Markdown source"},
                "slug": {"type": "string"},
                "excerpt": {"type": "string"},
                "status": {"type": "string"},
                "visibility": {"type": "string", "enum": ["public", "private"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "category": {"type": "string"},
                "review_status": {"type": "string", "enum": ["none", "pending", "approved", "changes_requested"]},
                "allow_comments": {"type": "boolean", "default": true}
            }),
            &["kind", "title", "body"],
        ),
        tool(
            "content_update",
            "Update content. Use expected_updated_at for optimistic locking. All writes create a revision.",
            json!({
                "id": {"type": "integer"},
                "title": {"type": "string"},
                "body": {"type": "string"},
                "slug": {"type": "string"},
                "excerpt": {"type": "string"},
                "status": {"type": "string"},
                "visibility": {"type": "string", "enum": ["public", "private"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "category": {"type": "string"},
                "review_status": {"type": "string", "enum": ["none", "pending", "approved", "changes_requested"]},
                "review_notes": {"type": "string"},
                "allow_comments": {"type": "boolean"},
                "expected_updated_at": {"type": "integer", "description": "Unix timestamp from a previous read"}
            }),
            &["id"],
        ),
        tool(
            "content_delete",
            "Move content to trash, or permanently delete content that is already in trash.",
            json!({
                "id": {"type": "integer"},
                "permanent": {"type": "boolean", "default": false}
            }),
            &["id"],
        ),
        tool(
            "content_publish",
            "Publish a post or page. Notes must be promoted instead.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "content_search",
            "Search titles and bodies across content.",
            json!({
                "query": {"type": "string"},
                "kind": {"type": "string", "enum": ["post", "page", "note"]},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
            }),
            &["query"],
        ),
        tool(
            "content_stats",
            "Get blog content and moderation statistics.",
            json!({}),
            &[],
        ),
        tool(
            "note_list",
            "List private notes.",
            json!({
                "status": {"type": "string", "enum": ["inbox", "developing", "evergreen", "archived", "trash"]},
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
                "offset": {"type": "integer", "minimum": 0, "default": 0}
            }),
            &[],
        ),
        tool(
            "note_get",
            "Get a note by id or slug, including backlinks and outgoing wiki links.",
            json!({"id": {"type": "integer"}, "slug": {"type": "string"}}),
            &[],
        ),
        tool(
            "note_create",
            "Create a private note. Defaults to inbox.",
            json!({
                "title": {"type": "string"},
                "body": {"type": "string"},
                "slug": {"type": "string"},
                "status": {"type": "string", "enum": ["inbox", "developing", "evergreen", "archived"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "category": {"type": "string"}
            }),
            &["title", "body"],
        ),
        tool(
            "note_update",
            "Update a private note.",
            json!({
                "id": {"type": "integer"},
                "title": {"type": "string"},
                "body": {"type": "string"},
                "slug": {"type": "string"},
                "status": {"type": "string", "enum": ["inbox", "developing", "evergreen", "archived", "trash"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "category": {"type": "string"},
                "review_status": {"type": "string", "enum": ["none", "pending", "approved", "changes_requested"]},
                "review_notes": {"type": "string"},
                "expected_updated_at": {"type": "integer"}
            }),
            &["id"],
        ),
        tool(
            "note_promote",
            "Convert a note into a blog post. The note is kept and linked to the post. Use sync=true to update an existing post.",
            json!({
                "id": {"type": "integer"},
                "title": {"type": "string"},
                "slug": {"type": "string"},
                "status": {"type": "string", "enum": ["draft", "published"], "default": "draft"},
                "sync": {"type": "boolean", "default": false}
            }),
            &["id"],
        ),
        tool(
            "review_queue",
            "List content waiting for review and comments waiting for moderation.",
            json!({"limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50}}),
            &[],
        ),
        tool(
            "review_submit",
            "Mark content as pending review with optional review notes.",
            json!({
                "id": {"type": "integer"},
                "notes": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "review_decide",
            "Record a review verdict. publish=true requires the publish scope and verdict approved; approval and publishing happen in one write.",
            json!({
                "id": {"type": "integer"},
                "verdict": {"type": "string", "enum": ["approved", "changes_requested"]},
                "notes": {"type": "string"},
                "publish": {"type": "boolean", "default": false}
            }),
            &["id", "verdict"],
        ),
        tool(
            "annotation_queue",
            "List annotations waiting for AI editing. Returns the selected text, the user's instruction, replies and content metadata. Pull these before rewriting a draft.",
            json!({
                "status": {"type": "string", "enum": ["open", "addressed", "resolved", "all"], "default": "open"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
            }),
            &[],
        ),
        tool(
            "annotation_list",
            "List annotations for one piece of content.",
            json!({
                "content_id": {"type": "integer"},
                "status": {"type": "string", "enum": ["open", "addressed", "resolved", "outdated", "all"], "default": "open"}
            }),
            &["content_id"],
        ),
        tool(
            "annotation_get",
            "Get one annotation with its replies.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "annotation_create",
            "Create an inline or general annotation on content. Inline annotations require quote.",
            json!({
                "content_id": {"type": "integer"},
                "kind": {"type": "string", "enum": ["inline", "general"]},
                "comment": {"type": "string"},
                "quote": {"type": "string"},
                "prefix": {"type": "string"},
                "suffix": {"type": "string"},
                "start_offset": {"type": "integer"},
                "end_offset": {"type": "integer"}
            }),
            &["content_id", "kind", "comment"],
        ),
        tool(
            "annotation_reply",
            "Reply to an annotation and optionally change its status. Use addressed after applying the requested edit; resolved requires the review scope.",
            json!({
                "id": {"type": "integer"},
                "body": {"type": "string"},
                "status": {"type": "string", "enum": ["open", "addressed", "resolved", "outdated"]}
            }),
            &["id", "body"],
        ),
        tool(
            "annotation_resolve",
            "Mark an annotation as resolved, optionally with a final note. Requires the review scope.",
            json!({
                "id": {"type": "integer"},
                "note": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "proposal_create",
            "Attach a proposed new version to an existing article or note. This does not change the live content and does not create a new article. Use it after reading annotations.",
            json!({
                "content_id": {"type": "integer"},
                "title": {"type": "string", "description": "Defaults to the current title"},
                "body": {"type": "string", "description": "Full proposed Markdown body"},
                "excerpt": {"type": "string"},
                "summary": {"type": "string", "description": "What changed in this version"},
                "base_revision_id": {"type": "integer", "description": "Latest revision id you read; rejected with a conflict if the content has a newer revision"},
                "expected_updated_at": {"type": "integer", "description": "updated_at from content_get; rejected with a conflict if the content changed since. Strongly recommended"}
            }),
            &["content_id", "body"],
        ),
        tool(
            "proposal_list",
            "List proposed versions attached to one piece of content.",
            json!({
                "content_id": {"type": "integer"},
                "status": {"type": "string", "enum": ["proposed", "applied", "rejected", "all"], "default": "proposed"}
            }),
            &["content_id"],
        ),
        tool(
            "proposal_get",
            "Get one proposed version.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "proposal_update",
            "Create a new immutable version of a proposed edit. Supply the version returned by proposal_get; does not change live content.",
            json!({
                "id": {"type": "integer"},
                "version": {"type": "integer", "description": "Current proposal version returned by proposal_get"},
                "title": {"type": "string"},
                "body": {"type": "string"},
                "excerpt": {"type": "string"},
                "summary": {"type": "string"}
            }),
            &["id", "version"],
        ),
        tool(
            "proposal_apply",
            "Apply exactly the proposal version you reviewed. Supply its version from proposal_get; stale versions are rejected. Requires publish for already-published content.",
            json!({
                "id": {"type": "integer"},
                "version": {"type": "integer", "description": "Exact proposal version reviewed"},
                "force": {"type": "boolean", "default": false}
            }),
            &["id", "version"],
        ),
        tool(
            "proposal_reject",
            "Reject a proposed version without changing the content.",
            json!({
                "id": {"type": "integer"},
                "note": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "revision_list",
            "List recent revisions for a piece of content.",
            json!({"content_id": {"type": "integer"}, "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20}}),
            &["content_id"],
        ),
        tool(
            "revision_get",
            "Get a revision including its Markdown body.",
            json!({"id": {"type": "integer"}}),
            &["id"],
        ),
        tool(
            "comment_list",
            "List comments. Use content_id to scope to one piece of content.",
            json!({
                "content_id": {"type": "integer"},
                "status": {"type": "string", "enum": ["all", "pending", "approved", "spam", "trash"], "default": "all"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50},
                "offset": {"type": "integer", "minimum": 0, "default": 0}
            }),
            &[],
        ),
        tool(
            "comment_moderate",
            "Moderate a comment. Permanently deleting a comment requires the admin scope.",
            json!({
                "id": {"type": "integer"},
                "action": {"type": "string", "enum": ["approve", "pending", "spam", "trash", "delete"]}
            }),
            &["id", "action"],
        ),
    ];

    Value::Array(tools)
}

pub(super) fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}
