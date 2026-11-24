//! Media library: uploads (form and editor), validation, and deletion.
mod common;

use axum::http::{StatusCode, header};
use common::{Session, TestApp, body_string};
use serde_json::Value;

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n not really an image";

async fn media_rows(app: &TestApp) -> Vec<(i64, String, String)> {
    sqlx::query_as("SELECT id, storage_key, original_name FROM media ORDER BY id")
        .fetch_all(&app.pool)
        .await
        .expect("media rows")
}

async fn upload(app: &TestApp, admin: &Session, file_name: &str, bytes: &[u8]) -> (u16, String) {
    let response = app
        .post_multipart(
            "/admin/media",
            Some(admin),
            &[],
            &[
                ("_csrf", None, admin.csrf.as_bytes()),
                ("file", Some(file_name), bytes),
            ],
        )
        .await;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = body_string(response).await;
    (status, location.unwrap_or(body))
}

#[tokio::test]
async fn uploaded_image_is_stored_listed_and_served_to_staff() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let (status, location) = upload(&app, &admin, "photo.png", PNG_BYTES).await;

    assert_eq!(status, 303, "{location}");
    assert_eq!(location, "/admin/media?notice=uploaded");
    let rows = media_rows(&app).await;
    assert_eq!(rows.len(), 1);
    let (_, key, name) = &rows[0];
    assert_eq!(name, "photo.png");
    assert!(
        key.ends_with(".png"),
        "random storage key keeps the extension"
    );
    assert_eq!(
        std::fs::read(app.data_dir().join("uploads").join(key)).expect("stored file"),
        PNG_BYTES
    );

    let (_, page) = app.page("/admin/media", Some(&admin)).await;
    assert!(page.contains("photo.png"));
    let served = app.get(&format!("/media/{key}"), Some(&admin)).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()[header::CONTENT_TYPE], "image/png");
}

#[tokio::test]
async fn unsafe_empty_and_oversized_uploads_are_rejected() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let oversized = vec![b'x'; 1024 * 1024 + 1]; // test config allows 1 MiB

    for (file_name, bytes, message) in [
        ("vector.svg", PNG_BYTES, "不支持的文件类型"),
        ("page.html", PNG_BYTES, "不支持的文件类型"),
        ("no-extension", PNG_BYTES, "不支持的文件类型"),
        ("empty.png", &b""[..], "文件为空"),
        ("big.png", &oversized[..], "文件超过大小限制"),
    ] {
        let (status, body) = upload(&app, &admin, file_name, bytes).await;
        assert_eq!(status, 200, "{file_name}");
        assert!(body.contains(message), "{file_name}: expected {message}");
    }
    assert!(media_rows(&app).await.is_empty());
}

#[tokio::test]
async fn upload_without_a_file_asks_for_one() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let response = app
        .post_multipart(
            "/admin/media",
            Some(&admin),
            &[],
            &[("_csrf", None, admin.csrf.as_bytes())],
        )
        .await;

    assert!(body_string(response).await.contains("请选择要上传的文件"));
}

#[tokio::test]
async fn deleting_media_removes_the_row_and_the_file() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    upload(&app, &admin, "photo.png", PNG_BYTES).await;
    let (id, key, _) = media_rows(&app).await.remove(0);

    let response = app
        .post_form(
            &format!("/admin/media/{id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(media_rows(&app).await.is_empty());
    assert!(!app.data_dir().join("uploads").join(&key).exists());
}

async fn editor_upload(app: &TestApp, session: &Session, csrf: &str, file_name: &str) -> Value {
    let response = app
        .post_multipart(
            "/admin/vditor/upload",
            Some(session),
            &[("x-csrf-token", csrf)],
            &[("file[]", Some(file_name), PNG_BYTES)],
        )
        .await;
    serde_json::from_str(&body_string(response).await).expect("json")
}

#[tokio::test]
async fn editor_upload_returns_vditor_json() {
    let app = TestApp::new().await;
    let editor = app.session_for("editor").await;

    let ok = editor_upload(&app, &editor, &editor.csrf, "inline.png").await;
    assert_eq!(ok["code"], 0, "{ok}");
    let url = ok["data"]["succMap"]["inline.png"].as_str().expect("url");
    assert!(url.starts_with("/media/"));

    let bad_csrf = editor_upload(&app, &editor, "wrong", "inline.png").await;
    assert_eq!(bad_csrf["code"], 1);
    assert!(
        bad_csrf["msg"]
            .as_str()
            .unwrap_or_default()
            .contains("CSRF")
    );

    let rejected = editor_upload(&app, &editor, &editor.csrf, "script.html").await;
    assert_eq!(rejected["code"], 1, "{rejected}");
}

#[tokio::test]
async fn editor_upload_requires_a_back_office_role() {
    let app = TestApp::new().await;
    let subscriber = app.session_for("subscriber").await;

    let result = editor_upload(&app, &subscriber, &subscriber.csrf, "inline.png").await;

    assert_eq!(result["code"], 1);
    assert!(media_rows(&app).await.is_empty());
}
