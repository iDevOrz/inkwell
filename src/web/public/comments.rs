//! Reader comment submission.

use super::*;

pub async fn comment(
    State(state): State<AppState>,
    Path(content_id): Path<i64>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<NewCommentForm>,
) -> AppResult<Response> {
    const MAX_COMMENT_DEPTH: i64 = 8;

    let Some(content_row) = content::get_by_id(&state.db, content_id).await? else {
        return Err(AppError::not_found("内容不存在"));
    };
    if content_row.kind != "post" || !content_row.is_public() {
        return Err(AppError::not_found("内容不存在"));
    }
    if content_row.allow_comments == 0 {
        return Err(AppError::bad_request("该内容已关闭评论"));
    }
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    if !site.comments_enabled {
        return Err(AppError::bad_request("评论功能已关闭"));
    }

    let target = format!("{}#comments", content_row.public_url());
    if !form.website.as_deref().unwrap_or("").trim().is_empty() {
        // Honeypot filled: pretend success.
        return Ok(axum::response::Redirect::to(&target).into_response());
    }

    let ip = client_ip(&headers, peer, state.config.trust_proxy_headers);
    let rate_limit_key = format!("comment:{ip}");
    if !state.comment_limiter.try_acquire(&rate_limit_key) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            "评论提交过于频繁，请稍后再试。",
        )
            .into_response());
    }

    if site.comments_require_email
        && form
            .email
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Ok(axum::response::Redirect::to(&format!(
            "{}?comment=error#comments",
            content_row.public_url()
        ))
        .into_response());
    }

    let mut parent_id = form.parent_id.unwrap_or(0).max(0);
    if parent_id > 0 {
        match comment::get(&state.db, parent_id).await? {
            Some(parent) if parent.content_id == content_row.id && parent.status == "approved" => {
                if comment::reply_depth(&state.db, parent_id).await? >= MAX_COMMENT_DEPTH {
                    return Err(AppError::bad_request("评论回复层级不能超过 8 层"));
                }
            }
            _ => parent_id = 0,
        }
    }

    let status = if site.comments_moderation {
        "pending"
    } else {
        "approved"
    };

    let result = comment::create(
        &state.db,
        comment::NewComment {
            content_id: content_row.id,
            parent_id,
            author: form.author,
            email: form.email,
            url: form.url,
            body: form.body,
            status: status.to_string(),
            ip: Some(ip),
            user_agent: user_agent(&headers),
        },
    )
    .await;

    match result {
        Ok(_) => {
            let code = if site.comments_moderation {
                "pending"
            } else {
                "ok"
            };
            Ok(axum::response::Redirect::to(&format!(
                "{}?comment={}#comments",
                content_row.public_url(),
                code
            ))
            .into_response())
        }
        Err(error) => {
            tracing::warn!(error = ?error, "comment submission failed");
            Ok(axum::response::Redirect::to(&format!(
                "{}?comment=error#comments",
                content_row.public_url()
            ))
            .into_response())
        }
    }
}
