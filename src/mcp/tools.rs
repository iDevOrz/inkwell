use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::models::{
    ANNOTATION_KINDS, ANNOTATION_STATUSES, ContentSummary, ContentView, KINDS, PROPOSAL_STATUSES,
};
use crate::repo::annotation::{self, AnnotationInput};
use crate::repo::content::{
    self, ContentFilter, ContentInput, ContentOrder, ContentPatch, PromoteOptions,
};
use crate::repo::proposal::{self, ProposalInput};
use crate::repo::token::McpPrincipal;
use crate::repo::{comment, revision, stats};
use crate::settings;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct CallToolResult {
    pub content: Vec<TextContent>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

fn ok(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
    CallToolResult {
        content: vec![TextContent { kind: "text", text }],
        structured_content: Some(value),
        is_error: None,
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult {
        content: vec![TextContent {
            kind: "text",
            text: message.into(),
        }],
        structured_content: None,
        is_error: Some(true),
    }
}

pub fn definitions() -> Value {
    let tools = vec![
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
            "Record a review verdict. Publishing requires the publish scope.",
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
            "Reply to an annotation and optionally change its status. Use addressed after applying the requested edit.",
            json!({
                "id": {"type": "integer"},
                "body": {"type": "string"},
                "status": {"type": "string", "enum": ["open", "addressed", "resolved", "outdated"]}
            }),
            &["id", "body"],
        ),
        tool(
            "annotation_resolve",
            "Mark an annotation as resolved, optionally with a final note.",
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
                "base_revision_id": {"type": "integer"}
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
            "Update a proposed version in place before the human reviews it. Does not change the live content.",
            json!({
                "id": {"type": "integer"},
                "title": {"type": "string"},
                "body": {"type": "string"},
                "excerpt": {"type": "string"},
                "summary": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "proposal_apply",
            "Apply a proposed version to the live content, creating a normal revision. Requires publish for already-published content.",
            json!({
                "id": {"type": "integer"},
                "force": {"type": "boolean", "default": false}
            }),
            &["id"],
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
            "Moderate or delete a comment.",
            json!({
                "id": {"type": "integer"},
                "action": {"type": "string", "enum": ["approve", "pending", "spam", "trash", "delete"]}
            }),
            &["id", "action"],
        ),
    ];

    Value::Array(tools)
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
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

pub async fn call(
    state: &AppState,
    principal: &McpPrincipal,
    name: &str,
    args: Value,
) -> CallToolResult {
    let result = match name {
        "content_list" => content_list(state, principal, &args).await,
        "content_get" => content_get(state, principal, &args).await,
        "content_create" => content_create(state, principal, &args).await,
        "content_update" => content_update(state, principal, &args).await,
        "content_delete" => content_delete(state, principal, &args).await,
        "content_publish" => content_publish(state, principal, &args).await,
        "content_search" => content_search(state, principal, &args).await,
        "content_stats" => content_stats(state, principal).await,
        "note_list" => note_list(state, principal, &args).await,
        "note_get" => note_get(state, principal, &args).await,
        "note_create" => note_create(state, principal, &args).await,
        "note_update" => note_update(state, principal, &args).await,
        "note_promote" => note_promote(state, principal, &args).await,
        "review_queue" => review_queue(state, principal, &args).await,
        "review_submit" => review_submit(state, principal, &args).await,
        "review_decide" => review_decide(state, principal, &args).await,
        "annotation_queue" => annotation_queue(state, principal, &args).await,
        "annotation_list" => annotation_list(state, principal, &args).await,
        "annotation_get" => annotation_get(state, principal, &args).await,
        "annotation_create" => annotation_create(state, principal, &args).await,
        "annotation_reply" => annotation_reply(state, principal, &args).await,
        "annotation_resolve" => annotation_resolve(state, principal, &args).await,
        "proposal_create" => proposal_create(state, principal, &args).await,
        "proposal_list" => proposal_list(state, principal, &args).await,
        "proposal_get" => proposal_get(state, principal, &args).await,
        "proposal_update" => proposal_update(state, principal, &args).await,
        "proposal_apply" => proposal_apply(state, principal, &args).await,
        "proposal_reject" => proposal_reject(state, principal, &args).await,
        "revision_list" => revision_list(state, principal, &args).await,
        "revision_get" => revision_get(state, principal, &args).await,
        "comment_list" => comment_list(state, principal, &args).await,
        "comment_moderate" => comment_moderate(state, principal, &args).await,
        _ => Err(AppError::bad_request(format!("未知工具: {name}"))),
    };

    match result {
        Ok(value) => ok(value),
        Err(error) => tool_error(error.to_string()),
    }
}

async fn content_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 100);
    let offset = arg_i64(args, "offset").unwrap_or(0).max(0);
    let status = arg_str(args, "status");
    let filter = ContentFilter {
        kind: arg_str(args, "kind"),
        status: status.clone(),
        query: arg_str(args, "query"),
        include_private: arg_bool(args, "include_private").unwrap_or(true),
        include_deleted: arg_bool(args, "include_deleted")
            .unwrap_or_else(|| status.as_deref() == Some("trash")),
        limit,
        offset,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total,
        "limit": limit,
        "offset": offset
    }))
}

async fn content_get(state: &AppState, principal: &McpPrincipal, args: &Value) -> AppResult<Value> {
    require(principal, "read")?;
    let view = load_view(state, args).await?;
    Ok(content_view_json(&view))
}

async fn content_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let kind = required_str(args, "kind")?;
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::bad_request("kind 必须是 post、page 或 note"));
    }
    check_write_fields(principal, args)?;
    let title = required_str(args, "title")?;
    let body = required_str(args, "body")?;
    let author_id = admin_user_id(&state.db).await?;
    let is_note = kind == "note";
    let id = content::create(
        &state.db,
        ContentInput {
            kind,
            title,
            slug: arg_str(args, "slug"),
            body,
            excerpt: arg_str(args, "excerpt"),
            status: arg_str(args, "status"),
            visibility: arg_str(args, "visibility")
                .or_else(|| is_note.then(|| "private".to_string())),
            review_status: arg_str(args, "review_status"),
            review_notes: None,
            allow_comments: arg_bool(args, "allow_comments").or_else(|| is_note.then_some(false)),
            category: arg_str(args, "category"),
            tags: arg_string_array(args, "tags"),
            author_id,
            actor: principal.actor(),
            source_note_id: None,
        },
    )
    .await?;
    let content = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::internal("创建后读取内容失败"))?;
    Ok(json!({
        "id": content.id,
        "kind": content.kind,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "review_status": content.review_status,
        "updated_at": content.updated_at
    }))
}

async fn content_update(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    check_write_fields(principal, args)?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "published" {
        require(principal, "publish")?;
    }
    let patch = build_patch(args, principal);
    let content = content::update(&state.db, id, patch).await?;
    Ok(json!({
        "id": content.id,
        "kind": content.kind,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "review_status": content.review_status,
        "review_notes": content.review_notes,
        "updated_at": content.updated_at,
        "previous_updated_at": current.updated_at
    }))
}

async fn content_delete(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    let permanent = arg_bool(args, "permanent").unwrap_or(false);
    if permanent {
        require(principal, "admin")?;
        if current.status != "trash" {
            return Err(AppError::bad_request("请先移入回收站，再彻底删除"));
        }
        content::delete_permanent(&state.db, id).await?;
        Ok(json!({"id": id, "permanent": true}))
    } else {
        if current.status == "published" {
            require(principal, "publish")?;
        }
        let content = content::trash(&state.db, id, &principal.actor()).await?;
        Ok(json!({"id": content.id, "status": content.status, "permanent": false}))
    }
}

async fn content_publish(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "publish")?;
    let id = required_i64(args, "id")?;
    let content = content::publish(&state.db, id, &principal.actor()).await?;
    Ok(json!({
        "id": content.id,
        "slug": content.slug,
        "status": content.status,
        "published_at": content.published_at
    }))
}

async fn content_search(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let query = required_str(args, "query")?;
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 100);
    let filter = ContentFilter {
        kind: arg_str(args, "kind"),
        query: Some(query),
        include_private: true,
        include_deleted: false,
        limit,
        offset: 0,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total
    }))
}

async fn content_stats(state: &AppState, principal: &McpPrincipal) -> AppResult<Value> {
    require(principal, "read")?;
    let stats = stats::load(&state.db).await?;
    Ok(serde_json::to_value(stats)?)
}

async fn note_list(state: &AppState, principal: &McpPrincipal, args: &Value) -> AppResult<Value> {
    require(principal, "read")?;
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 100);
    let offset = arg_i64(args, "offset").unwrap_or(0).max(0);
    let status = arg_str(args, "status");
    let filter = ContentFilter {
        kind: Some("note".to_string()),
        status: status.clone(),
        query: arg_str(args, "query"),
        include_private: true,
        include_deleted: status.as_deref() == Some("trash"),
        limit,
        offset,
        order: ContentOrder::UpdatedDesc,
        ..Default::default()
    };
    let (items, total) = content::list(&state.db, &filter, site.timezone_offset).await?;
    Ok(json!({
        "items": items.iter().map(summary_json).collect::<Vec<_>>(),
        "total": total
    }))
}

async fn note_get(state: &AppState, principal: &McpPrincipal, args: &Value) -> AppResult<Value> {
    require(principal, "read")?;
    let view = load_view(state, args).await?;
    if view.content.kind != "note" {
        return Err(AppError::bad_request("该内容不是笔记"));
    }
    Ok(content_view_json(&view))
}

async fn note_create(state: &AppState, principal: &McpPrincipal, args: &Value) -> AppResult<Value> {
    require(principal, "write")?;
    let title = required_str(args, "title")?;
    let body = required_str(args, "body")?;
    let author_id = admin_user_id(&state.db).await?;
    let id = content::create(
        &state.db,
        ContentInput {
            kind: "note".to_string(),
            title,
            slug: arg_str(args, "slug"),
            body,
            excerpt: None,
            status: Some(arg_str(args, "status").unwrap_or_else(|| "inbox".to_string())),
            visibility: Some("private".to_string()),
            review_status: Some("none".to_string()),
            review_notes: None,
            allow_comments: Some(false),
            category: arg_str(args, "category"),
            tags: arg_string_array(args, "tags"),
            author_id,
            actor: principal.actor(),
            source_note_id: None,
        },
    )
    .await?;
    let content = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::internal("创建笔记后读取失败"))?;
    Ok(json!({
        "id": content.id,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "updated_at": content.updated_at
    }))
}

async fn note_update(state: &AppState, principal: &McpPrincipal, args: &Value) -> AppResult<Value> {
    require(principal, "write")?;
    check_write_fields(principal, args)?;
    let id = required_i64(args, "id")?;
    let current = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("笔记不存在"))?;
    if current.kind != "note" {
        return Err(AppError::bad_request("该内容不是笔记"));
    }
    let content = content::update(&state.db, id, build_patch(args, principal)).await?;
    Ok(json!({
        "id": content.id,
        "title": content.title,
        "slug": content.slug,
        "status": content.status,
        "updated_at": content.updated_at
    }))
}

async fn note_promote(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "draft".to_string());
    if status != "draft" && status != "published" {
        return Err(AppError::bad_request("status 必须是 draft 或 published"));
    }
    if status == "published" {
        require(principal, "publish")?;
    }
    let result = content::promote_note(
        &state.db,
        id,
        PromoteOptions {
            title: arg_str(args, "title"),
            slug: arg_str(args, "slug"),
            status,
            sync: arg_bool(args, "sync").unwrap_or(false),
            actor: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(result)?)
}

async fn review_queue(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 100);
    review_queue_value(state, limit).await
}

pub async fn resource_value(state: &AppState, uri: &str) -> AppResult<Value> {
    let principal = McpPrincipal::local();
    if uri == "typecho://review/queue" {
        review_queue_value(state, 50).await
    } else if uri == "typecho://annotations/queue" {
        annotation_queue(state, &principal, &json!({"status": "open", "limit": 50})).await
    } else if uri == "typecho://notes/recent" {
        note_list(state, &principal, &json!({"limit": 20})).await
    } else if let Some(id) = uri.strip_prefix("typecho://content/") {
        let id = id
            .parse::<i64>()
            .map_err(|_| AppError::bad_request("无效的内容 URI"))?;
        content_get(state, &principal, &json!({"id": id})).await
    } else {
        Err(AppError::not_found("未知资源"))
    }
}

pub async fn review_queue_value(state: &AppState, limit: i64) -> AppResult<Value> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let (reviews, _) = content::list(
        &state.db,
        &ContentFilter {
            review_status: Some("pending".to_string()),
            include_private: true,
            include_deleted: false,
            limit,
            offset: 0,
            order: ContentOrder::UpdatedDesc,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    let (comments, _) = comment::list_admin(&state.db, "pending", limit, 0).await?;
    Ok(json!({
        "content": reviews.iter().map(summary_json).collect::<Vec<_>>(),
        "comments": comments.iter().map(comment_with_content_json).collect::<Vec<_>>()
    }))
}

async fn review_submit(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let notes = arg_str(args, "notes");
    let content = content::set_review(
        &state.db,
        id,
        "pending",
        notes.as_deref(),
        &principal.actor(),
    )
    .await?;
    Ok(json!({
        "id": content.id,
        "review_status": content.review_status,
        "review_notes": content.review_notes
    }))
}

async fn review_decide(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "review")?;
    let id = required_i64(args, "id")?;
    let verdict = required_str(args, "verdict")?;
    if verdict != "approved" && verdict != "changes_requested" {
        return Err(AppError::bad_request(
            "verdict 必须是 approved 或 changes_requested",
        ));
    }
    let publish = arg_bool(args, "publish").unwrap_or(false);
    if publish {
        require(principal, "publish")?;
        let existing = content::get_by_id(&state.db, id)
            .await?
            .ok_or_else(|| AppError::not_found("内容不存在"))?;
        if existing.kind == "note" {
            return Err(AppError::bad_request("笔记请先 promote 为文章再发布"));
        }
    }
    let notes = arg_str(args, "notes");
    content::set_review(
        &state.db,
        id,
        &verdict,
        notes.as_deref(),
        &principal.actor(),
    )
    .await?;

    if publish {
        content::publish(&state.db, id, &principal.actor()).await?;
    }

    let updated = content::get_by_id(&state.db, id)
        .await?
        .ok_or_else(|| AppError::internal("review 后读取失败"))?;
    Ok(json!({
        "id": updated.id,
        "review_status": updated.review_status,
        "review_notes": updated.review_notes,
        "status": updated.status,
        "published_at": updated.published_at
    }))
}

async fn annotation_queue(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "open".to_string());
    if !ANNOTATION_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 200);
    let items = annotation::queue(&state.db, &status, limit).await?;
    let total = items.len();
    Ok(json!({
        "items": items,
        "total": total,
        "status": status
    }))
}

async fn annotation_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "open".to_string());
    if !ANNOTATION_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    let status = (status != "all").then_some(status.as_str());
    let items = annotation::list_for_content(&state.db, content_id, status).await?;
    let total = items.len();
    Ok(json!({
        "items": items,
        "total": total
    }))
}

async fn annotation_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}

async fn annotation_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let content_id = required_i64(args, "content_id")?;
    let kind = required_str(args, "kind")?;
    if !ANNOTATION_KINDS.contains(&kind.as_str()) {
        return Err(AppError::bad_request("kind 必须是 inline 或 general"));
    }
    let comment = required_str(args, "comment")?;
    let item = annotation::create(
        &state.db,
        AnnotationInput {
            content_id,
            kind,
            quote: arg_str(args, "quote"),
            prefix: arg_str(args, "prefix"),
            suffix: arg_str(args, "suffix"),
            start_offset: arg_i64(args, "start_offset"),
            end_offset: arg_i64(args, "end_offset"),
            comment,
            author: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

async fn annotation_reply(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let body = required_str(args, "body")?;
    let status = arg_str(args, "status");
    if let Some(status) = status.as_deref()
        && !ANNOTATION_STATUSES.contains(&status)
    {
        return Err(AppError::bad_request("非法的批注状态"));
    }
    annotation::reply(&state.db, id, &principal.actor(), &body, status.as_deref()).await?;
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}

async fn annotation_resolve(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    if let Some(note) = arg_str(args, "note").filter(|value| !value.trim().is_empty()) {
        annotation::reply(&state.db, id, &principal.actor(), &note, Some("resolved")).await?;
    } else {
        annotation::set_status(&state.db, id, "resolved").await?;
    }
    let item = annotation::get_with_replies(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("批注不存在"))?;
    Ok(serde_json::to_value(item)?)
}

async fn proposal_create(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let content_id = required_i64(args, "content_id")?;
    let body = required_str(args, "body")?;
    let item = proposal::create(
        &state.db,
        ProposalInput {
            content_id,
            base_revision_id: arg_i64(args, "base_revision_id"),
            title: arg_str(args, "title"),
            body,
            excerpt: arg_str(args, "excerpt"),
            summary: arg_str(args, "summary"),
            author: principal.actor(),
        },
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

async fn proposal_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "proposed".to_string());
    if !PROPOSAL_STATUSES.contains(&status.as_str()) && status != "all" {
        return Err(AppError::bad_request("非法的版本状态"));
    }
    let status = (status != "all").then_some(status.as_str());
    let items = proposal::list_for_content(&state.db, content_id, status).await?;
    let total = items.len();
    Ok(json!({"items": items, "total": total}))
}

async fn proposal_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let item = proposal::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    Ok(serde_json::to_value(item)?)
}

async fn proposal_update(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let item = proposal::update(
        &state.db,
        id,
        arg_str(args, "title"),
        arg_str(args, "body"),
        arg_str(args, "excerpt"),
        arg_str(args, "summary"),
    )
    .await?;
    Ok(serde_json::to_value(item)?)
}

async fn proposal_apply(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    let id = required_i64(args, "id")?;
    let proposal = proposal::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("版本不存在"))?;
    let current = content::get_by_id(&state.db, proposal.content_id)
        .await?
        .ok_or_else(|| AppError::not_found("内容不存在"))?;
    if current.status == "published" {
        require(principal, "publish")?;
    } else {
        require(principal, "write")?;
    }
    let force = arg_bool(args, "force").unwrap_or(false);
    let (proposal, content) = proposal::apply(&state.db, id, &principal.actor(), force).await?;
    Ok(json!({
        "proposal": proposal,
        "content": {
            "id": content.id,
            "title": content.title,
            "slug": content.slug,
            "status": content.status,
            "updated_at": content.updated_at
        }
    }))
}

async fn proposal_reject(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let id = required_i64(args, "id")?;
    let note = arg_str(args, "note");
    let item = proposal::reject(&state.db, id, note.as_deref()).await?;
    Ok(serde_json::to_value(item)?)
}

async fn revision_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let content_id = required_i64(args, "content_id")?;
    let limit = arg_i64(args, "limit").unwrap_or(20).clamp(1, 200);
    let revisions = revision::list(&state.db, content_id, limit).await?;
    Ok(json!({
        "items": revisions.iter().map(|revision| json!({
            "id": revision.id,
            "content_id": revision.content_id,
            "title": revision.title,
            "status": revision.status,
            "actor": revision.actor,
            "note": revision.note,
            "created_at": revision.created_at,
            "body": revision.body
        })).collect::<Vec<_>>()
    }))
}

async fn revision_get(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let id = required_i64(args, "id")?;
    let revision = revision::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("修订不存在"))?;
    Ok(serde_json::to_value(revision)?)
}

async fn comment_list(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "read")?;
    let status = arg_str(args, "status").unwrap_or_else(|| "all".to_string());
    if let Some(content_id) = arg_i64(args, "content_id") {
        let comments = comment::list_for_content(&state.db, content_id, &status).await?;
        return Ok(json!({
            "items": comments.iter().map(comment_json).collect::<Vec<_>>()
        }));
    }

    let limit = arg_i64(args, "limit").unwrap_or(50).clamp(1, 100);
    let offset = arg_i64(args, "offset").unwrap_or(0).max(0);
    let (comments, total) = comment::list_admin(&state.db, &status, limit, offset).await?;
    Ok(json!({
        "items": comments.iter().map(comment_with_content_json).collect::<Vec<_>>(),
        "total": total
    }))
}

async fn comment_moderate(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "review")?;
    let id = required_i64(args, "id")?;
    let action = required_str(args, "action")?;
    match action.as_str() {
        "approve" => comment::moderate(&state.db, id, "approved").await?,
        "pending" => comment::moderate(&state.db, id, "pending").await?,
        "spam" => comment::moderate(&state.db, id, "spam").await?,
        "trash" => comment::moderate(&state.db, id, "trash").await?,
        "delete" => comment::delete(&state.db, id).await?,
        _ => return Err(AppError::bad_request("未知的评论操作")),
    }
    Ok(json!({"id": id, "action": action}))
}

async fn load_view(state: &AppState, args: &Value) -> AppResult<ContentView> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let content = if let Some(id) = arg_i64(args, "id") {
        content::get_by_id(&state.db, id).await?
    } else if let Some(slug) = arg_str(args, "slug") {
        content::get_by_slug(&state.db, &slug).await?
    } else {
        return Err(AppError::bad_request("请提供 id 或 slug"));
    }
    .ok_or_else(|| AppError::not_found("内容不存在"))?;

    content::get_view(&state.db, content.id, site.timezone_offset, true).await
}

fn build_patch(args: &Value, principal: &McpPrincipal) -> ContentPatch {
    ContentPatch {
        title: arg_str(args, "title"),
        slug: arg_str(args, "slug"),
        body: arg_str(args, "body"),
        excerpt: arg_str(args, "excerpt"),
        status: arg_str(args, "status"),
        visibility: arg_str(args, "visibility"),
        review_status: arg_str(args, "review_status"),
        review_notes: args
            .get("review_notes")
            .map(|_| arg_str(args, "review_notes").unwrap_or_default()),
        allow_comments: arg_bool(args, "allow_comments"),
        category: args
            .get("category")
            .map(|_| arg_str(args, "category").unwrap_or_default()),
        tags: args.get("tags").map(|_| arg_string_array(args, "tags")),
        actor: principal.actor(),
        expected_updated_at: arg_i64(args, "expected_updated_at"),
        revision_note: Some("MCP 更新".to_string()),
    }
}

async fn admin_user_id(pool: &SqlitePool) -> AppResult<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::internal("没有可用的管理员账号"))
}

fn require(principal: &McpPrincipal, scope: &str) -> AppResult<()> {
    if principal.has(scope) {
        Ok(())
    } else {
        Err(AppError::forbidden(format!("当前 token 缺少 {scope} 权限")))
    }
}

fn check_write_fields(principal: &McpPrincipal, args: &Value) -> AppResult<()> {
    if arg_str(args, "status").as_deref() == Some("published") {
        require(principal, "publish")?;
    }
    if matches!(
        arg_str(args, "review_status").as_deref(),
        Some("approved") | Some("changes_requested")
    ) {
        require(principal, "review")?;
    }
    Ok(())
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn required_str(args: &Value, key: &str) -> AppResult<String> {
    arg_str(args, key).ok_or_else(|| AppError::bad_request(format!("缺少参数 {key}")))
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn required_i64(args: &Value, key: &str) -> AppResult<i64> {
    arg_i64(args, key).ok_or_else(|| AppError::bad_request(format!("缺少参数 {key}")))
}

fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|value| {
        value
            .as_bool()
            .or_else(|| value.as_str().map(crate::util::parse_bool))
    })
}

fn arg_string_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn summary_json(item: &ContentSummary) -> Value {
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

fn content_view_json(view: &ContentView) -> Value {
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

fn comment_json(comment: &crate::models::Comment) -> Value {
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

fn comment_with_content_json(row: &crate::models::CommentWithContent) -> Value {
    let mut value = comment_json(&row.comment);
    value["content_title"] = json!(row.content_title);
    value["content_slug"] = json!(row.content_slug);
    value["content_kind"] = json!(row.content_kind);
    value
}
