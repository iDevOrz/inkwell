//! The real browser login flow and admin-side media handling.
mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestApp, body_string, cookie_from, encode, hidden_value};
use inkwell::auth::{LOGIN_CSRF_COOKIE, SESSION_COOKIE};
use inkwell::repo::media::{self, NewMedia};

#[tokio::test]
async fn login_form_with_csrf_cookie_opens_the_dashboard() {
    let app = TestApp::new().await;

    let login = app.get("/admin/login", None).await;
    assert_eq!(login.status(), StatusCode::OK);
    let csrf_cookie = cookie_from(login.headers(), LOGIN_CSRF_COOKIE).expect("login csrf cookie");
    let csrf = hidden_value(&body_string(login).await, "csrf");

    let submit = app
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, format!("{LOGIN_CSRF_COOKIE}={csrf_cookie}"))
                .body(Body::from(encode(&[
                    ("username", "admin"),
                    ("password", "testpass123"),
                    ("next", "/admin"),
                    ("csrf", &csrf),
                ])))
                .expect("request"),
        )
        .await;
    assert_eq!(submit.status(), StatusCode::SEE_OTHER);
    let session = cookie_from(submit.headers(), SESSION_COOKIE).expect("session cookie");

    let dashboard = app
        .send(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, format!("{SESSION_COOKIE}={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await;
    assert_eq!(dashboard.status(), StatusCode::OK);
    assert_eq!(dashboard.headers()[header::CACHE_CONTROL], "no-store");
    assert!(body_string(dashboard).await.contains("仪表盘"));
}

#[tokio::test]
async fn login_without_the_csrf_cookie_is_rejected() {
    let app = TestApp::new().await;
    let login = app.get("/admin/login", None).await;
    let csrf = hidden_value(&body_string(login).await, "csrf");

    let submit = app
        .post_form(
            "/admin/login",
            None,
            &[
                ("username", "admin"),
                ("password", "testpass123"),
                ("csrf", &csrf),
            ],
        )
        .await;

    assert!(cookie_from(submit.headers(), SESSION_COOKIE).is_none());
}

#[tokio::test]
async fn failed_file_deletion_keeps_the_media_row_for_retry() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    // A directory where a file is expected makes the storage delete fail.
    let key = "delete-failure-directory";
    std::fs::create_dir(app.data_dir().join("uploads").join(key)).expect("mkdir");
    let media_id = media::insert(
        &app.pool,
        NewMedia {
            storage_key: key.to_string(),
            original_name: "delete-failure".to_string(),
            mime: "application/octet-stream".to_string(),
            size: 0,
            width: None,
            height: None,
            uploader_id: Some(1),
        },
    )
    .await
    .expect("insert media");

    let response = app
        .post_form(
            &format!("/admin/media/{media_id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        media::get(&app.pool, media_id)
            .await
            .expect("get")
            .is_some()
    );
}
