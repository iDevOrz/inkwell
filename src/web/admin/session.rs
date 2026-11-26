//! Login, logout and password change.

use super::*;

pub(super) async fn login_page(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Query(query): Query<LoginQuery>,
) -> AppResult<Response> {
    if session.is_some() {
        return Ok(Redirect::to("/admin").into_response());
    }
    render_login(
        &state,
        error_for(query.error.as_deref()),
        safe_next(query.next.as_deref()),
        String::new(),
    )
    .await
}

pub(super) async fn login_submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> AppResult<Response> {
    let next = safe_next(form.next.as_deref());
    let csrf_cookie = cookie_value(&headers, LOGIN_CSRF_COOKIE);
    let csrf_ok = match (csrf_cookie.as_deref(), form.csrf.as_deref()) {
        (Some(expected), Some(actual)) => constant_time_eq(expected, actual),
        _ => false,
    };
    if !csrf_ok {
        return render_login(
            &state,
            Some("页面已过期，请重新登录。".to_string()),
            next,
            form.username,
        )
        .await;
    }

    let ip = client_ip(&headers, peer, state.config.trust_proxy_headers);
    if !state.login_limiter.try_acquire(&ip) {
        return render_login(
            &state,
            Some("尝试次数过多，请 10 分钟后再试。".to_string()),
            next,
            form.username,
        )
        .await;
    }

    let username = form.username.trim();
    if username.is_empty() || username.chars().count() > 128 || form.password.len() > 1024 {
        return render_login(
            &state,
            Some("用户名或密码错误。".to_string()),
            next,
            form.username,
        )
        .await;
    };

    // The attempt was already counted by `try_acquire`. Unknown users still
    // run one Argon2 verification so timing does not reveal valid usernames.
    let user = user::find_by_username(&state.db, username).await?;
    let password_hash = user.as_ref().map(|user| user.password_hash.clone());
    let password_ok = check_login_password(form.password.clone(), password_hash).await;
    let Some(user) = user.filter(|_| password_ok) else {
        return render_login(
            &state,
            Some("用户名或密码错误。".to_string()),
            next,
            form.username,
        )
        .await;
    };

    let Some((token, _)) = create_verified_session(
        &state.db,
        &user,
        state.config.session_days,
        Some(&ip),
        user_agent(&headers).as_deref(),
    )
    .await?
    else {
        return render_login(
            &state,
            Some("用户名或密码错误。".to_string()),
            next,
            form.username,
        )
        .await;
    };
    state.login_limiter.record_success(&ip);

    let mut response = Redirect::to(&next).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        session_cookie(
            &token,
            state.config.session_days * 24 * 3600,
            state.config.secure_cookies,
        ),
    );
    response.headers_mut().append(
        SET_COOKIE,
        clear_login_csrf_cookie(state.config.secure_cookies),
    );
    response.headers_mut().append(
        SET_COOKIE,
        clear_legacy_session_cookie(state.config.secure_cookies),
    );
    Ok(response)
}

pub(super) async fn render_login(
    state: &AppState,
    error: Option<String>,
    next: String,
    username: String,
) -> AppResult<Response> {
    let site = settings::load(&state.db, &state.config.site_url).await?;
    let csrf = random_token(24);
    let template = LoginTemplate {
        site,
        error,
        next,
        csrf: csrf.clone(),
        username,
    };
    let mut response = render(template)?.into_response();
    response.headers_mut().append(
        SET_COOKIE,
        login_csrf_cookie(&csrf, state.config.secure_cookies),
    );
    Ok(response)
}

pub(super) async fn logout(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    delete_session(&state.db, &session.token_hash).await?;
    let mut response = Redirect::to("/admin/login").into_response();
    response.headers_mut().append(
        SET_COOKIE,
        clear_session_cookie(state.config.secure_cookies),
    );
    response.headers_mut().append(
        SET_COOKIE,
        clear_legacy_session_cookie(state.config.secure_cookies),
    );
    Ok(response)
}

pub(super) async fn profile_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let homepage = user::homepage(&state.db, session.user.id).await?;
    let ctx = admin_ctx(
        &state,
        &session,
        "profile",
        notice_for(query.notice.as_deref()),
        None,
    )
    .await?;
    Ok(render(ProfileTemplate {
        ctx,
        display_name: session.user.display_name,
        email: session.user.email.unwrap_or_default(),
        homepage: homepage.unwrap_or_default(),
    })?
    .into_response())
}

pub(super) async fn profile_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<ProfileForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let name = form.display_name.trim();
    let email = form.email.trim();
    let homepage = form.homepage.trim();
    let email_valid = email.is_empty()
        || (email.len() <= 254
            && !email
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
            && email.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty() && !domain.is_empty() && !domain.contains('@')
            }));
    let homepage_valid = homepage.is_empty()
        || (homepage.len() <= 2048
            && homepage.parse::<axum::http::Uri>().is_ok_and(|uri| {
                matches!(uri.scheme_str(), Some("https" | "http")) && uri.authority().is_some()
            }));
    let error =
        if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
            Some("请输入 1 到 128 个字符的昵称")
        } else if !email_valid {
            Some("请输入有效的邮件地址")
        } else if !homepage_valid {
            Some("个人主页必须是有效的 HTTP 或 HTTPS 地址")
        } else {
            None
        };
    if let Some(message) = error {
        let ctx = admin_ctx(&state, &session, "profile", None, Some(message.to_string())).await?;
        return Ok(render(ProfileTemplate {
            ctx,
            display_name: form.display_name,
            email: form.email,
            homepage: form.homepage,
        })?
        .into_response());
    }

    user::update_profile(
        &state.db,
        session.user.id,
        name,
        (!email.is_empty()).then_some(email),
        (!homepage.is_empty()).then_some(homepage),
    )
    .await?;
    Ok(Redirect::to("/admin/profile?notice=saved").into_response())
}

pub(super) async fn password_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let ctx = admin_ctx(
        &state,
        &session,
        "password",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    Ok(render(PasswordTemplate { ctx, error: None })?.into_response())
}

pub(super) async fn password_submit(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<PasswordForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    if !check_login_password(
        form.old_password.clone(),
        Some(session.user.password_hash.clone()),
    )
    .await
    {
        return password_error(&state, &session, "原密码不正确").await;
    }
    if form.new_password.chars().count() < 8 {
        return password_error(&state, &session, "新密码至少需要 8 个字符").await;
    }
    if form.new_password != form.confirm_password {
        return password_error(&state, &session, "两次输入的新密码不一致").await;
    }

    let hash = hash_password_async(form.new_password.clone()).await?;
    if !user::change_password(&state.db, &session.user, &hash, &session.token_hash).await? {
        return password_error(&state, &session, "密码已变更，请重新登录").await;
    }
    Ok(Redirect::to("/admin/password?notice=password_changed").into_response())
}

pub(super) async fn password_error(
    state: &AppState,
    session: &AuthSession,
    message: &str,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "password", None, Some(message.to_string())).await?;
    Ok(render(PasswordTemplate {
        ctx,
        error: Some(message.to_string()),
    })?
    .into_response())
}
