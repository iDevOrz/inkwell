use serde::Serialize;

use crate::auth::AuthSession;
use crate::error::AppResult;
use crate::models::{Comment, CommentNode, UserView};
use crate::repo::user;
use crate::settings::{self, SiteSettings};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct PublicCtx {
    pub site: SiteSettings,
    pub user: Option<UserView>,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub current_path: String,
}

impl PublicCtx {
    pub async fn load(
        state: &AppState,
        session: Option<&AuthSession>,
        current_path: &str,
        notice: Option<String>,
        error: Option<String>,
    ) -> AppResult<Self> {
        let site = settings::load(&state.db, &state.config.site_url).await?;
        Ok(Self {
            site,
            user: session.map(|session| UserView::from(&session.user)),
            notice,
            error,
            current_path: current_path.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminCtx {
    pub site: SiteSettings,
    pub user: UserView,
    pub csrf: String,
    pub active: String,
    pub notice: Option<String>,
    pub error: Option<String>,
}

impl AdminCtx {
    pub async fn load(
        state: &AppState,
        session: &AuthSession,
        active: &str,
        notice: Option<String>,
        error: Option<String>,
    ) -> AppResult<Self> {
        let site = settings::load(&state.db, &state.config.site_url).await?;
        Ok(Self {
            site,
            user: UserView::from(&session.user),
            csrf: session.csrf_token.clone(),
            active: active.to_string(),
            notice,
            error,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentItem {
    pub comment: Comment,
    pub depth: i64,
    pub created_label: String,
}

pub fn flatten_comments(
    nodes: Vec<CommentNode>,
    depth: i64,
    timezone_offset: i32,
) -> Vec<CommentItem> {
    let mut result = Vec::new();
    let mut stack: Vec<(CommentNode, i64)> =
        nodes.into_iter().rev().map(|node| (node, depth)).collect();
    while let Some((node, node_depth)) = stack.pop() {
        let CommentNode { comment, children } = node;
        result.push(CommentItem {
            created_label: crate::util::format_datetime(comment.created_at, timezone_offset),
            comment,
            depth: node_depth,
        });
        for child in children.into_iter().rev() {
            stack.push((child, node_depth + 1));
        }
    }
    result
}

#[derive(Debug, Clone, Serialize)]
pub struct PostLink {
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

pub fn status_options(kind: &str, selected: &str) -> Vec<StatusOption> {
    crate::models::statuses_for_kind(kind)
        .iter()
        .map(|value| StatusOption {
            value: value.to_string(),
            label: crate::models::status_label(value).to_string(),
            selected: *value == selected,
        })
        .collect()
}

pub fn visibility_options(selected: &str) -> Vec<StatusOption> {
    crate::models::VISIBILITIES
        .iter()
        .map(|value| StatusOption {
            value: value.to_string(),
            label: crate::models::visibility_label(value).to_string(),
            selected: *value == selected,
        })
        .collect()
}

pub fn review_options(selected: &str) -> Vec<StatusOption> {
    crate::models::REVIEW_STATUSES
        .iter()
        .map(|value| StatusOption {
            value: value.to_string(),
            label: crate::models::review_label(value).to_string(),
            selected: *value == selected,
        })
        .collect()
}

pub fn actor_label(actor: &str) -> String {
    if let Some(name) = actor.strip_prefix("mcp:") {
        if name == "stdio" {
            "AI · stdio".to_string()
        } else {
            format!("AI · {name}")
        }
    } else if actor.starts_with("human:") {
        "作者".to_string()
    } else {
        actor.to_string()
    }
}

pub fn notice_for(code: Option<&str>) -> Option<String> {
    let code = code?.trim();
    let message = match code {
        "created" => "已创建",
        "saved" => "已保存",
        "deleted" => "已移入回收站",
        "permanently_deleted" => "已彻底删除",
        "restored" => "已恢复",
        "published" => "已发布",
        "promoted" => "已转为文章",
        "synced" => "已同步到文章",
        "moderated" => "评论状态已更新",
        "settings_saved" => "设置已保存",
        "uploaded" => "文件已上传",
        "token_created" => "Token 已创建",
        "token_revoked" => "Token 已吊销",
        "review_saved" => "Review 状态已保存",
        "annotation_created" => "批注已添加",
        "annotation_replied" => "回复已保存",
        "annotation_resolved" => "批注已确认",
        "annotation_reopened" => "批注已重新打开",
        "proposal_applied" => "AI 版本已应用",
        "proposal_rejected" => "AI 版本已拒绝",
        "comment_pending" => "评论已提交，等待审核",
        "comment_approved" => "评论已提交",
        "password_changed" => "密码已修改",
        _ => return None,
    };
    Some(message.to_string())
}

pub fn error_for(code: Option<&str>) -> Option<String> {
    let code = code?.trim();
    let message = match code {
        "not_found" => "内容不存在",
        "invalid" => "提交的内容不完整",
        "duplicate" => "缩略名已存在",
        "csrf" => "页面已过期，请重试",
        "login" => "用户名或密码错误",
        "rate_limited" => "尝试次数过多，请稍后再试",
        "upload" => "上传失败",
        "too_large" => "文件超过大小限制",
        "forbidden" => "没有权限",
        _ => return None,
    };
    Some(message.to_string())
}

pub async fn user_display_name(state: &AppState, id: i64) -> AppResult<String> {
    Ok(user::find_by_id(&state.db, id)
        .await?
        .map(|user| user.display_name)
        .unwrap_or_else(|| "未知作者".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Comment;
    use crate::repo::comment;

    #[test]
    fn deeply_nested_comments_are_traversed_without_recursion() {
        let comments = (1..=10_000)
            .map(|id| Comment {
                id,
                content_id: 1,
                parent_id: id - 1,
                author: "reader".to_string(),
                email: None,
                url: None,
                body: format!("comment {id}"),
                status: "approved".to_string(),
                ip: None,
                user_agent: None,
                created_at: id,
            })
            .collect();

        let flat = flatten_comments(comment::build_tree(comments), 0, 0);
        assert_eq!(flat.len(), 10_000);
        assert_eq!(flat.last().map(|item| item.depth), Some(9_999));
    }
}
