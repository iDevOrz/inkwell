//! Request bodies for back-office forms.

use super::*;

#[derive(Debug, Deserialize, Default)]
pub struct AdminListQuery {
    pub status: Option<String>,
    pub edit: Option<i64>,
    pub q: Option<String>,
    pub page: Option<i64>,
    pub notice: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LoginQuery {
    pub next: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub display_name: String,
    pub email: String,
    pub homepage: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
    pub next: Option<String>,
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub permanent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContentForm {
    pub title: String,
    pub slug: Option<String>,
    pub body: String,
    pub expected_updated_at: Option<i64>,
    pub excerpt: Option<String>,
    pub status: Option<String>,
    pub visibility: Option<String>,
    pub review_status: Option<String>,
    pub review_notes: Option<String>,
    pub allow_comments: Option<String>,
    pub category: Option<String>,
    pub tags: Option<String>,
    pub action: Option<String>,
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromoteForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub status: Option<String>,
    pub sync: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReviewForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub verdict: String,
    pub notes: Option<String>,
    pub publish: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub kind: String,
    pub quote: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_offset: Option<String>,
    pub end_offset: Option<String>,
    pub comment: String,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationReplyForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct AnnotationActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProposalApplyForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub version: Option<i64>,
    pub force: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProposalRejectForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommentActionForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct TermForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub kind: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub site_title: String,
    pub site_description: String,
    pub site_keywords: String,
    pub site_url: String,
    pub footer_text: String,
    pub avatar_url: Option<String>,
    pub home_quotes: Option<String>,
    pub icp_number: Option<String>,
    pub page_size: String,
    pub timezone_offset: String,
    pub comments_enabled: Option<String>,
    pub comments_moderation: Option<String>,
    pub comments_require_email: Option<String>,
    pub feed_full_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub name: String,
    pub scope_read: Option<String>,
    pub scope_write: Option<String>,
    pub scope_publish: Option<String>,
    pub scope_review: Option<String>,
    pub scope_admin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PasswordForm {
    #[serde(rename = "_csrf")]
    pub csrf: Option<String>,
    pub old_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    pub body: String,
}
