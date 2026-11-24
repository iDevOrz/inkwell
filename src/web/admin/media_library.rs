//! Media library and editor uploads.

use super::*;

pub(super) async fn media_page(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Query(query): Query<AdminListQuery>,
) -> AppResult<Response> {
    media_response(
        &state,
        &session,
        notice_for(query.notice.as_deref()),
        error_for(query.error.as_deref()),
    )
    .await
}

pub(super) async fn media_response(
    state: &AppState,
    session: &AuthSession,
    notice: Option<String>,
    error: Option<String>,
) -> AppResult<Response> {
    let ctx = admin_ctx(state, session, "media", notice, error.clone()).await?;
    let site = ctx.site.clone();
    let media = media::list(&state.db, 200)
        .await?
        .iter()
        .map(|item| MediaView::from_media(item, site.timezone_offset))
        .collect();
    Ok(render(MediaTemplate { ctx, media, error })?.into_response())
}

pub(super) struct StoredUpload {
    url: String,
}

pub(super) async fn store_upload(
    state: &AppState,
    session: &AuthSession,
    original_name: String,
    declared_mime: String,
    bytes: Vec<u8>,
) -> AppResult<StoredUpload> {
    if bytes.is_empty() {
        return Err(AppError::bad_request("文件为空"));
    }
    if bytes.len() > state.config.max_upload_bytes {
        return Err(AppError::bad_request("文件超过大小限制"));
    }

    let original_name = if original_name.trim().is_empty() {
        "upload".to_string()
    } else {
        original_name
    };
    let extension = std::path::Path::new(&original_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "pdf" | "txt" | "md" | "zip"
            )
        })
        .ok_or_else(|| AppError::bad_request("不支持的文件类型"))?;

    let mime = mime_guess::from_path(&original_name)
        .first_or_octet_stream()
        .to_string();
    if declared_mime.contains("html")
        || declared_mime.contains("javascript")
        || mime.contains("html")
    {
        return Err(AppError::bad_request("不支持的文件类型"));
    }

    let now = chrono::Utc::now();
    let storage_key = format!(
        "{}/{:02}/{}.{}",
        now.format("%Y"),
        now.format("%m"),
        random_token(16),
        extension
    );
    let url = format!("/media/{storage_key}");

    state
        .storage
        .put(&storage_key, &bytes)
        .await
        .map_err(|error| AppError::internal(format!("文件保存失败: {error}")))?;

    media::insert(
        &state.db,
        media::NewMedia {
            storage_key,
            original_name,
            mime: mime.clone(),
            size: bytes.len() as i64,
            width: None,
            height: None,
            uploader_id: Some(session.user.id),
        },
    )
    .await?;

    Ok(StoredUpload { url })
}

pub(super) async fn media_upload(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    mut multipart: Multipart,
) -> AppResult<Response> {
    let mut csrf = None;
    let mut file: Option<(String, String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or("").to_string();
        if name == "_csrf" {
            csrf = Some(field.text().await?);
        } else if name == "file" || name == "file[]" {
            let original_name = field
                .file_name()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "upload".to_string());
            let mime = field
                .content_type()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let bytes = field.bytes().await?;
            file = Some((original_name, mime, bytes.to_vec()));
        }
    }

    verify_csrf(&session, csrf.as_deref().unwrap_or(""))?;
    let Some((original_name, declared_mime, bytes)) = file else {
        return media_response(
            &state,
            &session,
            None,
            Some("请选择要上传的文件".to_string()),
        )
        .await;
    };

    if let Err(error) = store_upload(&state, &session, original_name, declared_mime, bytes).await {
        return media_response(&state, &session, None, Some(error.message())).await;
    }

    Ok(Redirect::to("/admin/media?notice=uploaded").into_response())
}

#[derive(Debug, Deserialize)]
pub struct VditorPreviewRequest {
    #[serde(rename = "markdownText")]
    pub markdown_text: String,
}

pub(super) async fn vditor_preview(
    OptionalUser(session): OptionalUser,
    Json(request): Json<VditorPreviewRequest>,
) -> Response {
    if !session
        .as_ref()
        .is_some_and(AuthSession::can_manage_content)
    {
        return Json(json!({
            "code": 1,
            "msg": "登录已过期，请刷新页面",
            "data": ""
        }))
        .into_response();
    }

    let html = markdown::render_markdown(&request.markdown_text, &HashMap::new());
    Json(json!({
        "code": 0,
        "msg": "",
        "data": html
    }))
    .into_response()
}

pub(super) async fn vditor_upload(
    State(state): State<AppState>,
    OptionalUser(session): OptionalUser,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let Some(session) = session.filter(AuthSession::can_manage_content) else {
        return Json(json!({
            "code": 1,
            "msg": "登录已过期，请刷新页面",
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    };

    let mut csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_string();
                if name == "_csrf" {
                    if let Ok(value) = field.text().await {
                        csrf = Some(value);
                    }
                } else if name == "file" || name == "file[]" {
                    let original_name = field
                        .file_name()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "upload".to_string());
                    let mime = field
                        .content_type()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    match field.bytes().await {
                        Ok(bytes) => files.push((original_name, mime, bytes.to_vec())),
                        Err(error) => {
                            return Json(json!({
                                "code": 1,
                                "msg": format!("读取上传文件失败: {error}"),
                                "data": { "errFiles": [], "succMap": {} }
                            }))
                            .into_response();
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                return Json(json!({
                    "code": 1,
                    "msg": format!("上传失败: {error}"),
                    "data": { "errFiles": [], "succMap": {} }
                }))
                .into_response();
            }
        }
    }

    if let Err(error) = verify_csrf(&session, csrf.as_deref().unwrap_or("")) {
        return Json(json!({
            "code": 1,
            "msg": error.message(),
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    }

    if files.is_empty() {
        return Json(json!({
            "code": 1,
            "msg": "请选择要上传的文件",
            "data": { "errFiles": [], "succMap": {} }
        }))
        .into_response();
    }

    let mut succ_map = HashMap::new();
    let mut err_files = Vec::new();
    let mut first_error = String::new();

    for (original_name, declared_mime, bytes) in files {
        match store_upload(
            &state,
            &session,
            original_name.clone(),
            declared_mime,
            bytes,
        )
        .await
        {
            Ok(upload) => {
                succ_map.insert(original_name, upload.url);
            }
            Err(error) => {
                if first_error.is_empty() {
                    first_error = error.message();
                }
                err_files.push(original_name);
            }
        }
    }

    let code = if succ_map.is_empty() { 1 } else { 0 };
    Json(json!({
        "code": code,
        "msg": first_error,
        "data": {
            "errFiles": err_files,
            "succMap": succ_map
        }
    }))
    .into_response()
}

pub(super) async fn media_delete(
    State(state): State<AppState>,
    AdminSession(session): AdminSession,
    Path(id): Path<i64>,
    Form(form): Form<ActionForm>,
) -> AppResult<Response> {
    verify_csrf(&session, form.csrf.as_deref().unwrap_or(""))?;
    if let Some(item) = media::get(&state.db, id).await? {
        if let Err(error) = state.storage.delete(&item.storage_key).await {
            tracing::error!(error = ?error, media_id = id, "delete upload file failed");
            return Err(AppError::internal("文件删除失败，媒体记录已保留，请重试"));
        }
        media::delete(&state.db, id).await?;
    }
    Ok(Redirect::to("/admin/media?notice=deleted").into_response())
}
