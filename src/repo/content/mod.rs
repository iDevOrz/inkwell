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

mod links;
mod promotion;
mod queries;
mod views;
mod writes;

pub(in crate::repo) use links::*;
pub use promotion::*;
pub use queries::*;
pub use views::*;
pub use writes::*;

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

pub fn status_label_for(kind: &str, status: &str) -> &'static str {
    let _ = kind;
    crate::models::status_label(status)
}

pub fn content_kind_label(kind: &str) -> &'static str {
    kind_label(kind)
}
