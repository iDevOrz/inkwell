//! Public listings (search, archives, taxonomy) and reader comments.
mod common;

use axum::http::{StatusCode, header};
use common::TestApp;
use inkwell::repo::comment::{self, NewComment};
use inkwell::settings::{self, SiteSettings};
use inkwell::util::{format_date, path_segment};

const WELCOME_SLUG: &str = "hello-inkwell";
const WELCOME_TITLE: &str = "欢迎使用 inkwell";

async fn site(app: &TestApp) -> SiteSettings {
    settings::load(&app.pool, "http://test.local")
        .await
        .expect("settings")
}

async fn set_comment_policy(app: &TestApp, enabled: bool, moderation: bool, require_email: bool) {
    let updated = SiteSettings {
        comments_enabled: enabled,
        comments_moderation: moderation,
        comments_require_email: require_email,
        ..site(app).await
    };
    settings::update(&app.pool, &updated)
        .await
        .expect("save settings");
}

async fn welcome_id(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT id FROM contents WHERE slug = ?")
        .bind(WELCOME_SLUG)
        .fetch_one(&app.pool)
        .await
        .expect("welcome post")
}

#[tokio::test]
async fn search_finds_published_posts_only_by_query() {
    let app = TestApp::new().await;
    for (uri, should_find) in [
        ("/search?q=inkwell", true),
        ("/search?q=nothing-matches-this", false),
        ("/search?q=", true), // an empty query lists every published post
        ("/search?q=inkwell&page=2", false),
    ] {
        let (status, html) = app.page(uri, None).await;
        assert_eq!(status, 200, "{uri}");
        assert_eq!(html.contains(WELCOME_TITLE), should_find, "{uri}");
    }
}

#[tokio::test]
async fn month_archive_lists_posts_and_rejects_invalid_months() {
    let app = TestApp::new().await;
    let offset = site(&app).await.timezone_offset;
    let published_at: i64 = sqlx::query_scalar("SELECT published_at FROM contents WHERE slug = ?")
        .bind(WELCOME_SLUG)
        .fetch_one(&app.pool)
        .await
        .expect("published_at");
    let month = format_date(published_at, offset, "%Y/%m");

    let (status, html) = app.page(&format!("/archive/{month}"), None).await;
    assert_eq!(status, 200);
    assert!(html.contains(WELCOME_TITLE));

    let (status, html) = app.page("/archive/1999/01", None).await;
    assert_eq!(status, 200);
    assert!(!html.contains(WELCOME_TITLE));

    assert_eq!(
        app.get("/archive/2026/13", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn seeded_category_and_tag_pages_list_the_welcome_post() {
    let app = TestApp::new().await;
    for uri in [
        "/categories/uncategorized".to_string(),
        format!("/tags/{}", path_segment("开始")),
    ] {
        let (status, html) = app.page(&uri, None).await;
        assert_eq!(status, 200, "{uri}");
        assert!(html.contains(WELCOME_TITLE), "{uri}");
    }
    assert_eq!(
        app.get("/tags/does-not-exist", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn feeds_and_timeline_fragments_render() {
    let app = TestApp::new().await;
    for (uri, content_type) in [
        ("/feed.xml", "application/rss+xml"),
        ("/atom.xml", "application/atom+xml"),
        ("/sitemap.xml", "application/xml"),
        ("/robots.txt", "text/plain"),
        ("/timeline/more", "text/html"),
    ] {
        let response = app.get(uri, None).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let actual = response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap_or("");
        assert!(actual.starts_with(content_type), "{uri}: {actual}");
    }
    let (_, rss) = app.page("/feed.xml", None).await;
    assert!(rss.contains(WELCOME_TITLE));
}

fn location(response: &axum::http::Response<axum::body::Body>) -> String {
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap_or_default()
        .to_string()
}

async fn post_comment(
    app: &TestApp,
    post_id: i64,
    fields: &[(&str, &str)],
) -> axum::http::Response<axum::body::Body> {
    app.post_form(&format!("/comments/{post_id}"), None, fields)
        .await
}

async fn comment_count(app: &TestApp, post_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE content_id = ?")
        .bind(post_id)
        .fetch_one(&app.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn default_policy_requires_email_and_holds_comments_for_moderation() {
    let app = TestApp::new().await;
    let post_id = welcome_id(&app).await;

    let no_email = post_comment(&app, post_id, &[("author", "读者"), ("body", "你好")]).await;
    assert!(location(&no_email).contains("comment=error"));
    assert_eq!(comment_count(&app, post_id).await, 0);

    let pending = post_comment(
        &app,
        post_id,
        &[
            ("author", "读者"),
            ("email", "r@example.com"),
            ("body", "你好"),
        ],
    )
    .await;
    assert!(location(&pending).contains("comment=pending"));
    let stored: String = sqlx::query_scalar("SELECT status FROM comments WHERE content_id = ?")
        .bind(post_id)
        .fetch_one(&app.pool)
        .await
        .expect("comment");
    assert_eq!(stored, "pending");
    let (_, page) = app.page(&format!("/archives/{WELCOME_SLUG}"), None).await;
    assert!(!page.contains("你好"), "pending comments are not shown");
}

#[tokio::test]
async fn honeypot_submissions_look_successful_but_are_dropped() {
    let app = TestApp::new().await;
    let post_id = welcome_id(&app).await;

    let response = post_comment(
        &app,
        post_id,
        &[
            ("author", "bot"),
            ("email", "b@example.com"),
            ("body", "spam"),
            ("website", "http://spam"),
        ],
    )
    .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(comment_count(&app, post_id).await, 0);
}

#[tokio::test]
async fn open_policy_publishes_comments_and_threads_replies() {
    let app = TestApp::new().await;
    set_comment_policy(&app, true, false, false).await;
    let post_id = welcome_id(&app).await;

    let first = post_comment(&app, post_id, &[("author", "甲"), ("body", "第一条评论")]).await;
    assert!(location(&first).contains("comment=ok"));
    let parent: i64 = sqlx::query_scalar("SELECT id FROM comments WHERE content_id = ?")
        .bind(post_id)
        .fetch_one(&app.pool)
        .await
        .expect("parent");

    let parent_field = parent.to_string();
    post_comment(
        &app,
        post_id,
        &[
            ("author", "乙"),
            ("body", "回复"),
            ("parent_id", &parent_field),
        ],
    )
    .await;
    post_comment(
        &app,
        post_id,
        &[
            ("author", "丙"),
            ("body", "孤儿回复"),
            ("parent_id", "999999"),
        ],
    )
    .await;

    let parents: Vec<(String, i64)> =
        sqlx::query_as("SELECT body, parent_id FROM comments WHERE content_id = ? ORDER BY id")
            .bind(post_id)
            .fetch_all(&app.pool)
            .await
            .expect("comments");
    assert_eq!(parents[1], ("回复".to_string(), parent));
    assert_eq!(
        parents[2],
        ("孤儿回复".to_string(), 0),
        "unknown parents become top-level"
    );

    let (_, page) = app.page(&format!("/archives/{WELCOME_SLUG}"), None).await;
    assert!(page.contains("第一条评论") && page.contains("回复"));
}

#[tokio::test]
async fn reply_depth_is_limited() {
    let app = TestApp::new().await;
    set_comment_policy(&app, true, false, false).await;
    let post_id = welcome_id(&app).await;
    let mut parent = 0;
    for depth in 0..9 {
        parent = comment::create(
            &app.pool,
            NewComment {
                content_id: post_id,
                parent_id: parent,
                author: format!("层{depth}"),
                email: None,
                url: None,
                body: "嵌套".to_string(),
                status: "approved".to_string(),
                ip: None,
                user_agent: None,
            },
        )
        .await
        .expect("comment");
    }

    let parent_field = parent.to_string();
    let response = post_comment(
        &app,
        post_id,
        &[
            ("author", "深"),
            ("body", "太深了"),
            ("parent_id", &parent_field),
        ],
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn comments_are_refused_where_they_are_not_allowed() {
    let app = TestApp::new().await;
    set_comment_policy(&app, true, false, false).await;
    let post_id = welcome_id(&app).await;
    let fields = [("author", "读者"), ("body", "评论")];

    assert_eq!(
        app.post_form("/comments/999999", None, &fields)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    sqlx::query("UPDATE contents SET allow_comments = 0 WHERE id = ?")
        .bind(post_id)
        .execute(&app.pool)
        .await
        .expect("close comments");
    assert_eq!(
        post_comment(&app, post_id, &fields).await.status(),
        StatusCode::BAD_REQUEST
    );

    sqlx::query("UPDATE contents SET allow_comments = 1 WHERE id = ?")
        .bind(post_id)
        .execute(&app.pool)
        .await
        .expect("reopen comments");
    set_comment_policy(&app, false, false, false).await;
    assert_eq!(
        post_comment(&app, post_id, &fields).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(comment_count(&app, post_id).await, 0);
}
