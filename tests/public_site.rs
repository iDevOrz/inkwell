//! Public site: home page, static assets, comment rate limiting, and media
//! and taxonomy visibility.
mod common;

use axum::http::{StatusCode, header};
use common::TestApp;
use inkwell::repo::content::{self, ContentInput, ContentPatch};
use inkwell::repo::media::{self, NewMedia};

#[tokio::test]
async fn data_directories_are_private_to_the_service_user() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let app = TestApp::new().await;
        for dir in [app.data_dir().to_path_buf(), app.data_dir().join("uploads")] {
            let mode = std::fs::metadata(&dir)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "{}", dir.display());
        }
    }
}

#[tokio::test]
async fn home_renders_the_seeded_welcome_post() {
    let app = TestApp::new().await;

    let (status, html) = app.page("/", None).await;

    assert_eq!(status, 200);
    assert!(html.contains("欢迎使用 inkwell"));
}

#[tokio::test]
async fn static_assets_are_served() {
    let app = TestApp::new().await;

    let (status, css) = app.page("/static/site.css", None).await;

    assert_eq!(status, 200);
    assert!(css.contains("post-card"));
}

#[tokio::test]
async fn comments_are_rate_limited_by_socket_peer() {
    // Every request comes from the same mocked peer; spoofable forwarding
    // headers are ignored because proxy trust is off.
    let app = TestApp::new().await;
    for attempt in 0..6 {
        let body = format!("测试评论 {attempt}");
        let response = app
            .post_form(
                "/comments/1",
                None,
                &[("author", "Reader"), ("body", &body)],
            )
            .await;
        let expected = if attempt < 5 {
            StatusCode::SEE_OTHER
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(response.status(), expected, "attempt {attempt}");
    }
}

const STORAGE_KEY: &str = "2026/11/private-preview.png";
const SECRET_TERM: &str = "secret-project";

async fn author_id(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("seeded author")
}

/// Stores an image that is referenced only by a private note tagged and
/// categorized with SECRET_TERM.
async fn private_media_fixture(app: &TestApp) {
    let author_id = author_id(app).await;
    let upload_path = app.data_dir().join("uploads").join(STORAGE_KEY);
    std::fs::create_dir_all(upload_path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&upload_path, b"private image bytes").expect("write media");
    media::insert(
        &app.pool,
        NewMedia {
            storage_key: STORAGE_KEY.to_string(),
            original_name: "private-preview.png".to_string(),
            mime: "image/png".to_string(),
            size: 19,
            width: None,
            height: None,
            uploader_id: Some(author_id),
        },
    )
    .await
    .expect("insert media");
    content::create(&app.pool, referencing_content(author_id, "note", "private"))
        .await
        .expect("private note");
}

fn referencing_content(author_id: i64, kind: &str, visibility: &str) -> ContentInput {
    let public = visibility == "public";
    ContentInput {
        kind: kind.to_string(),
        title: format!("{kind} media fixture"),
        slug: Some(format!("{kind}-media-fixture")),
        body: format!("![media](/media/{STORAGE_KEY})"),
        excerpt: None,
        status: Some(if public { "published" } else { "inbox" }.to_string()),
        visibility: Some(visibility.to_string()),
        review_status: Some("none".to_string()),
        review_notes: None,
        allow_comments: Some(public),
        category: Some(SECRET_TERM.to_string()),
        tags: vec![SECRET_TERM.to_string()],
        author_id,
        actor: "test".to_string(),
        source_note_id: None,
    }
}

async fn assert_taxonomy_hidden(app: &TestApp) {
    for uri in [
        format!("/tags/{SECRET_TERM}"),
        format!("/categories/{SECRET_TERM}"),
    ] {
        assert_eq!(
            app.get(&uri, None).await.status(),
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    let (_, sitemap) = app.page("/sitemap.xml", None).await;
    assert!(!sitemap.contains(&format!("/tags/{SECRET_TERM}")));
    assert!(!sitemap.contains(&format!("/categories/{SECRET_TERM}")));
}

#[tokio::test]
async fn media_and_terms_referenced_only_privately_stay_private() {
    let app = TestApp::new().await;
    private_media_fixture(&app).await;
    let admin = app.session_for("admin").await;
    let media_uri = format!("/media/{STORAGE_KEY}");

    let anonymous = app.get(&media_uri, None).await;
    assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        anonymous.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );

    let staff = app.get(&media_uri, Some(&admin)).await;
    assert_eq!(staff.status(), StatusCode::OK);
    assert_eq!(staff.headers()[header::CACHE_CONTROL], "private, no-store");

    assert_taxonomy_hidden(&app).await;
}

#[tokio::test]
async fn publishing_a_reference_makes_media_public_until_unpublished() {
    let app = TestApp::new().await;
    private_media_fixture(&app).await;
    let author_id = author_id(&app).await;
    let media_uri = format!("/media/{STORAGE_KEY}");

    let post_id = content::create(&app.pool, referencing_content(author_id, "post", "public"))
        .await
        .expect("public post");

    let public = app.get(&media_uri, None).await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(
        public.headers()[header::CACHE_CONTROL],
        "public, max-age=3600"
    );
    for uri in [
        format!("/tags/{SECRET_TERM}"),
        format!("/categories/{SECRET_TERM}"),
    ] {
        assert_eq!(app.get(&uri, None).await.status(), StatusCode::OK, "{uri}");
    }

    content::update(
        &app.pool,
        post_id,
        ContentPatch {
            status: Some("draft".to_string()),
            visibility: Some("private".to_string()),
            actor: "test".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("unpublish");

    assert_eq!(
        app.get(&media_uri, None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_taxonomy_hidden(&app).await;
}
