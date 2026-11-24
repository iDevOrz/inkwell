//! Single post and page views.

use super::*;

/// Drafts and private content are visible only to back-office roles.
pub(super) fn can_view(content: &Content, session: Option<&AuthSession>) -> bool {
    content.is_public() || session.is_some_and(AuthSession::can_manage_content)
}

pub async fn post(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    let (notice, error) = query_messages(&query);
    let Some(content_row) = content::get_by_slug(&state.db, &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/archives/{slug}")).await;
    };
    if content_row.kind != "post" || !can_view(&content_row, session.as_ref()) {
        return not_found_page(&state, session.as_ref(), &format!("/archives/{slug}")).await;
    }

    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/archives/{slug}"),
        notice,
        error,
    )
    .await?;
    let site = ctx.site.clone();
    let post = content::get_view(
        &state.db,
        content_row.id,
        site.timezone_offset,
        session
            .as_ref()
            .is_some_and(AuthSession::can_manage_content),
    )
    .await?;

    let comment_rows = comment::list_for_content(&state.db, post.content.id, "approved").await?;
    let comment_count = comment_rows.len() as i64;
    let comment_tree = comment::build_tree(comment_rows);
    let comments = flatten_comments(comment_tree, 0, site.timezone_offset);

    let reply_to = query.reply.and_then(|reply_id| {
        comments
            .iter()
            .find(|item| item.comment.id == reply_id)
            .cloned()
    });
    let parent_id = reply_to.as_ref().map(|item| item.comment.id).unwrap_or(0);

    let comment_form = CommentFormView {
        content_id: post.content.id,
        parent_id,
        author: String::new(),
        email: String::new(),
        url: String::new(),
        body: String::new(),
        reply_author: reply_to.as_ref().map(|item| item.comment.author.clone()),
    };

    let (older, newer) = content::adjacent_posts(&state.db, &post.content).await?;
    let prev = older.map(|(title, slug)| PostLink { title, slug });
    let next = newer.map(|(title, slug)| PostLink { title, slug });

    let template = PostTemplate {
        ctx,
        post,
        comments,
        comment_count,
        comment_form,
        reply_to,
        canonical: format!("{}/archives/{}", site.site_url, slug),
        prev,
        next,
    };
    Ok(render(template)?.into_response())
}

pub async fn page(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(slug): Path<String>,
) -> AppResult<Response> {
    let Some(content_row) = content::get_by_slug(&state.db, &slug).await? else {
        return not_found_page(&state, session.as_ref(), &format!("/pages/{slug}")).await;
    };
    if content_row.kind != "page" || !can_view(&content_row, session.as_ref()) {
        return not_found_page(&state, session.as_ref(), &format!("/pages/{slug}")).await;
    }

    let ctx = PublicCtx::load(
        &state,
        session.as_ref(),
        &format!("/pages/{slug}"),
        None,
        None,
    )
    .await?;
    let site = ctx.site.clone();
    let page = content::get_view(
        &state.db,
        content_row.id,
        site.timezone_offset,
        session
            .as_ref()
            .is_some_and(AuthSession::can_manage_content),
    )
    .await?;
    let template = PageTemplate {
        ctx,
        page,
        canonical: format!("{}/pages/{}", site.site_url, slug),
    };
    Ok(render(template)?.into_response())
}
