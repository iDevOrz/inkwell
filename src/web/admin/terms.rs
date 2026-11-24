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
    let ctx = admin_ctx(
        &state,
        &session,
        "terms",
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await?;
    let terms = term::list(&state.db, &kind).await?;
    Ok(render(TermsTemplate {
        ctx,
        kind: kind.clone(),
        heading: if kind == "category" {
            "分类".to_string()
        } else {
            "标签".to_string()
        },
        terms,
        error: None,
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
            let ctx = admin_ctx(&state, &session, "terms", None, Some(error.message())).await?;
            let terms = term::list(&state.db, &form.kind).await?;
            Ok(render(TermsTemplate {
                ctx,
                kind: form.kind.clone(),
                heading: if form.kind == "category" {
                    "分类".to_string()
                } else {
                    "标签".to_string()
                },
                terms,
                error: Some(error.message()),
            })?
            .into_response())
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
