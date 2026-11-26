//! Categories and tags.

use super::*;

pub(super) async fn terms_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    let kind = query
        .status
        .clone()
        .filter(|value| value == "category" || value == "tag")
        .unwrap_or_else(|| "category".to_string());
    let editing = match query.edit {
        Some(id) => Some(
            term::get_by_id(&state.db, id)
                .await?
                .filter(|term| term.kind == kind)
                .ok_or_else(|| AppError::not_found("分类或标签不存在"))?,
        ),
        None => None,
    };
    terms_response(
        &state,
        &session,
        &kind,
        editing,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await
}

async fn terms_response(
    state: &AppState,
    session: &AuthSession,
    kind: &str,
    editing: Option<crate::models::Term>,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "terms", notice, error.clone()).await?;
    let terms = term::list(&state.db, kind).await?;
    Ok(render(TermsTemplate {
        ctx,
        kind: kind.to_string(),
        heading: if kind == "category" {
            "分类"
        } else {
            "标签"
        }
        .to_string(),
        terms,
        editing,
        error,
    })?
    .into_response())
}

pub(super) async fn term_create(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Form(form): Form<TermForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let result = term::create(
        &state.db,
        &form.kind,
        &form.name,
        form.slug.as_deref(),
        form.description.as_deref(),
    )
    .await;

    match result {
        Ok(_) => Ok(
            Redirect::to(&format!("/admin/terms?status={}&notice=created", form.kind))
                .into_response(),
        ),
        Err(error) => {
            terms_response(
                &state,
                &session,
                &form.kind,
                None,
                None,
                Some(error.message()),
            )
            .await
        }
    }
}

pub(super) async fn term_update(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<TermForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let mut existing = term::get_by_id(&state.db, id)
        .await?
        .filter(|term| term.kind == form.kind)
        .ok_or_else(|| AppError::not_found("分类或标签不存在"))?;
    match term::update(
        &state.db,
        &existing,
        &form.name,
        form.slug.as_deref(),
        form.description.as_deref(),
    )
    .await
    {
        Ok(()) => Ok(Redirect::to(&format!(
            "/admin/terms?status={}&notice=saved",
            existing.kind
        ))
        .into_response()),
        Err(error) => {
            existing.name = form.name;
            existing.slug = form.slug.unwrap_or(existing.slug);
            existing.description = form.description;
            terms_response(
                &state,
                &session,
                &form.kind,
                Some(existing),
                None,
                Some(error.message()),
            )
            .await
        }
    }
}

pub(super) async fn term_delete(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    let kind = term::get_by_id(&state.db, id)
        .await?
        .map(|term| term.kind)
        .unwrap_or_else(|| "category".to_string());
    term::delete(&state.db, id).await?;
    Ok(Redirect::to(&format!("/admin/terms?status={kind}&notice=deleted")).into_response())
}
