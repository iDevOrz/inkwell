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

mod annotations;
mod args;
mod comments;
mod contents;
mod definitions;
mod instance;
mod notes;
mod proposals;
mod resources;
mod reviews;
mod revisions;
mod serialize;

use annotations::*;
use args::*;
use comments::*;
use contents::*;
pub use definitions::definitions;
use instance::*;
use notes::*;
use proposals::*;
use resources::*;
pub use resources::{RESOURCE_SCHEME, resource_value};
use reviews::*;
use revisions::*;
use serialize::*;

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

pub async fn call(
    state: &AppState,
    principal: &McpPrincipal,
    name: &str,
    args: Value,
) -> CallToolResult {
    let result = match name {
        "instance_info" => instance_info(state, principal, &args).await,
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
        Err(error) => tool_error(error.message()),
    }
}
