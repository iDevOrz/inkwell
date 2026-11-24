//! MCP token management (admin only).

use super::*;

pub(super) async fn tokens_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    session.require_admin()?;
    tokens_response(
        &state,
        &session,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
        None,
        &["read", "write", "review"],
    )
    .await
}

pub(super) async fn tokens_response(
    state: &AppState,
    session: &AuthSession,
    notice: Option<String>,
    error: Option<String>,
    new_token: Option<String>,
    selected: &[&str],
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "tokens", notice, error.clone()).await?;
    let site = ctx.site.clone();
    let tokens = token::list(&state.db)
        .await?
        .iter()
        .map(|token| TokenView::from_token(token, site.timezone_offset))
        .collect();
    let scopes = token::SCOPES
        .iter()
        .map(|scope| ScopeOption {
            value: scope.to_string(),
            label: match *scope {
                "read" => "读取",
                "write" => "写入草稿/笔记",
                "publish" => "发布",
                "review" => "Review",
                "admin" => "全部权限",
                _ => scope,
            }
            .to_string(),
            checked: selected.contains(scope),
        })
        .collect();

    Ok(render(TokensTemplate {
        ctx,
        tokens,
        new_token,
        scopes,
        error,
    })?
    .into_response())
}

pub(super) async fn token_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<TokenForm>,
) -> AppResult<Response> {
    session.require_admin()?;
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let mut scopes = Vec::new();
    if form.scope_read.is_some() {
        scopes.push("read".to_string());
    }
    if form.scope_write.is_some() {
        scopes.push("write".to_string());
    }
    if form.scope_publish.is_some() {
        scopes.push("publish".to_string());
    }
    if form.scope_review.is_some() {
        scopes.push("review".to_string());
    }
    if form.scope_admin.is_some() {
        scopes.push("admin".to_string());
    }

    match token::create(&state.db, &form.name, &scopes).await {
        Ok((_, plaintext)) => {
            let selected: Vec<&str> = scopes.iter().map(String::as_str).collect();
            tokens_response(
                &state,
                &session,
                notice_for(Some("token_created")),
                None,
                Some(plaintext),
                &selected,
            )
            .await
        }
        Err(error) => {
            tokens_response(
                &state,
                &session,
                None,
                Some(error.message()),
                None,
                &["read", "write", "review"],
            )
            .await
        }
    }
}

pub(super) async fn token_revoke(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    session.require_admin()?;
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    token::revoke(&state.db, id).await?;
    Ok(Redirect::to("/admin/tokens?notice=token_revoked").into_response())
}
