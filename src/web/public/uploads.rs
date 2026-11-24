//! Serving uploaded media.

use super::*;

pub async fn media(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    Path(path): Path<String>,
) -> AppResult<Response> {
    let key = path.trim_start_matches('/');
    let publicly_referenced = media::is_publicly_referenced(&state.db, key).await?;
    let can_view_private = session
        .as_ref()
        .is_some_and(AuthSession::can_manage_content);
    if !publicly_referenced && !can_view_private {
        return Ok((
            StatusCode::NOT_FOUND,
            [(CACHE_CONTROL, "private, no-store")],
        )
            .into_response());
    }

    let bytes = state
        .storage
        .get(key)
        .await
        .map_err(|_| AppError::not_found("文件不存在"))?;
    let mime = mime_guess::from_path(key)
        .first_or_octet_stream()
        .to_string();
    let cache_control = if publicly_referenced {
        "public, max-age=3600"
    } else {
        "private, no-store"
    };
    Response::builder()
        .header(CONTENT_TYPE, mime)
        .header(CACHE_CONTROL, cache_control)
        .body(Body::from(bytes))
        .map_err(|error| AppError::internal(error.to_string()))
}
