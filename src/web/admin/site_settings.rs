//! Site settings (admin only).

use super::*;

pub(super) async fn settings_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    session.require_admin()?;
    let ctx = admin_ctx(
        &state,
        &session,
        "settings",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let settings = settings::load(&state.db, &state.config.site_url).await?;
    Ok(render(SettingsTemplate {
        ctx,
        settings,
        error: None,
    })?
    .into_response())
}

pub(super) async fn settings_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<SettingsForm>,
) -> AppResult<Response> {
    session.require_admin()?;
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let page_size = form
        .page_size
        .parse::<i64>()
        .ok()
        .filter(|value| (1..=100).contains(value));
    let timezone_offset = form
        .timezone_offset
        .parse::<i32>()
        .ok()
        .filter(|value| (-12 * 60..=14 * 60).contains(value));

    let Some(page_size) = page_size else {
        return settings_error(&state, &session, &form, "每页文章数必须在 1 到 100 之间").await;
    };
    let Some(timezone_offset) = timezone_offset else {
        return settings_error(&state, &session, &form, "时区偏移不合法").await;
    };
    if !form.site_url.starts_with("http://") && !form.site_url.starts_with("https://") {
        return settings_error(
            &state,
            &session,
            &form,
            "站点地址必须以 http:// 或 https:// 开头",
        )
        .await;
    }

    let current = settings::load(&state.db, &state.config.site_url).await?;
    let avatar_url = form
        .avatar_url
        .as_deref()
        .unwrap_or(&current.avatar_url)
        .trim();
    if !avatar_url.is_empty() && !avatar_url.starts_with("https://") {
        return settings_error(&state, &session, &form, "头像链接必须以 https:// 开头").await;
    }
    let settings = SiteSettings {
        title: form.site_title.trim().to_string(),
        description: form.site_description.trim().to_string(),
        keywords: form.site_keywords.trim().to_string(),
        site_url: form.site_url.trim().trim_end_matches('/').to_string(),
        footer_text: form.footer_text.trim().to_string(),
        avatar_url: avatar_url.to_string(),
        home_quotes: form
            .home_quotes
            .as_deref()
            .unwrap_or(&current.home_quotes)
            .replace("\r\n", "\n")
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        icp_number: form
            .icp_number
            .as_deref()
            .unwrap_or(&current.icp_number)
            .trim()
            .to_string(),
        page_size,
        timezone_offset,
        comments_enabled: form.comments_enabled.is_some(),
        comments_moderation: form.comments_moderation.is_some(),
        comments_require_email: form.comments_require_email.is_some(),
        feed_full_text: form.feed_full_text.is_some(),
    };
    settings::update(&state.db, &settings).await?;
    Ok(Redirect::to("/admin/settings?notice=settings_saved").into_response())
}

pub(super) async fn settings_error(
    state: &AppState,
    session: &AuthSession,
    form: &SettingsForm,
    message: &str,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "settings", None, Some(message.to_string())).await?;
    let settings = SiteSettings {
        title: form.site_title.clone(),
        description: form.site_description.clone(),
        keywords: form.site_keywords.clone(),
        site_url: form.site_url.clone(),
        footer_text: form.footer_text.clone(),
        avatar_url: form
            .avatar_url
            .clone()
            .unwrap_or_else(|| ctx.site.avatar_url.clone()),
        home_quotes: form
            .home_quotes
            .clone()
            .unwrap_or_else(|| ctx.site.home_quotes.clone()),
        icp_number: form
            .icp_number
            .clone()
            .unwrap_or_else(|| ctx.site.icp_number.clone()),
        page_size: form.page_size.parse().unwrap_or(8),
        timezone_offset: form.timezone_offset.parse().unwrap_or(480),
        comments_enabled: form.comments_enabled.is_some(),
        comments_moderation: form.comments_moderation.is_some(),
        comments_require_email: form.comments_require_email.is_some(),
        feed_full_text: form.feed_full_text.is_some(),
    };
    Ok(render(SettingsTemplate {
        ctx,
        settings,
        error: Some(message.to_string()),
    })?
    .into_response())
}
