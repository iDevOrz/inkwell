//! Back-office form submissions: the success paths and their validation
//! errors, checked against the resulting database state.
mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Session, TestApp, body_string, hidden_value};
use inkwell::repo::comment::{self, NewComment};
use inkwell::repo::content;
use serde_json::json;

fn location(response: &axum::http::Response<Body>) -> String {
    response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Submits the full post editor form; `extra` overrides or adds fields.
async fn submit_post(
    app: &TestApp,
    admin: &Session,
    uri: &str,
    extra: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut fields = vec![
        ("_csrf", admin.csrf.as_str()),
        ("title", "表单文章"),
        ("slug", "form-post"),
        ("body", "表单正文"),
        ("excerpt", ""),
        ("status", "draft"),
        ("visibility", "public"),
        ("review_status", "none"),
        ("review_notes", ""),
        ("allow_comments", "1"),
        ("category", "表单分类"),
        ("tags", "甲, 乙"),
        ("action", "save"),
    ];
    for (key, value) in extra {
        fields.retain(|(existing, _)| existing != key);
        fields.push((key, value));
    }
    app.post_form(uri, Some(admin), &fields).await
}

async fn create_post_via_form(app: &TestApp, admin: &Session) -> i64 {
    let response = submit_post(app, admin, "/admin/posts", &[]).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = location(&response);
    assert!(location.ends_with("/edit?notice=created"), "{location}");
    location
        .trim_start_matches("/admin/posts/")
        .split('/')
        .next()
        .and_then(|id| id.parse().ok())
        .expect("id in redirect")
}

async fn status_of(app: &TestApp, id: i64) -> Option<String> {
    content::get_by_id(&app.pool, id)
        .await
        .expect("get")
        .map(|content| content.status)
}

#[tokio::test]
async fn post_lifecycle_through_the_editor() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let id = create_post_via_form(&app, &admin).await;
    let created = content::get_by_id(&app.pool, id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(created.title, "表单文章");
    let (_, edit_html) = app
        .page(&format!("/admin/posts/{id}/edit"), Some(&admin))
        .await;
    assert!(edit_html.contains("表单正文"));
    assert!(edit_html.contains("甲"));

    let updated_at = hidden_value(&edit_html, "expected_updated_at");
    let update = submit_post(
        &app,
        &admin,
        &format!("/admin/posts/{id}"),
        &[("body", "改过的正文"), ("expected_updated_at", &updated_at)],
    )
    .await;
    assert_eq!(
        update.status(),
        StatusCode::SEE_OTHER,
        "{}",
        location(&update)
    );
    let updated = content::get_by_id(&app.pool, id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(updated.body, "改过的正文");

    let publish = app
        .post_form(
            &format!("/admin/posts/{id}/publish"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;
    assert!(location(&publish).ends_with("notice=published"));
    assert_eq!(status_of(&app, id).await.as_deref(), Some("published"));
    assert_eq!(
        app.get("/archives/form-post", None).await.status(),
        StatusCode::OK
    );

    let trash = app
        .post_form(
            &format!("/admin/posts/{id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;
    assert!(location(&trash).ends_with("notice=deleted"));
    assert_eq!(status_of(&app, id).await.as_deref(), Some("trash"));
    assert_eq!(
        app.get("/archives/form-post", None).await.status(),
        StatusCode::NOT_FOUND
    );

    let restore = app
        .post_form(
            &format!("/admin/posts/{id}/restore"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;
    assert!(location(&restore).ends_with("notice=restored"));
    assert_eq!(status_of(&app, id).await.as_deref(), Some("draft"));

    for _ in 0..2 {
        // First moves to trash, second (with permanent=1) deletes for good.
        app.post_form(
            &format!("/admin/posts/{id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf), ("permanent", "1")],
        )
        .await;
    }
    assert_eq!(status_of(&app, id).await, None);
}

#[tokio::test]
async fn invalid_editor_input_re_renders_the_form_with_an_error() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let response = submit_post(&app, &admin, "/admin/posts", &[("title", "  ")]).await;

    assert_eq!(response.status(), StatusCode::OK);
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contents WHERE slug = 'form-post'")
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(total, 0);
}

#[tokio::test]
async fn kind_in_the_url_must_match_the_content() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let id = create_post_via_form(&app, &admin).await;

    let response = app
        .post_form(
            &format!("/admin/pages/{id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(status_of(&app, id).await.as_deref(), Some("draft"));
}

#[tokio::test]
async fn content_lists_filter_by_status_and_search() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    create_post_via_form(&app, &admin).await;

    for (uri, expect_form_post) in [
        ("/admin/posts", true),
        ("/admin/posts?status=draft", true),
        ("/admin/posts?status=published", false),
        ("/admin/posts?q=表单", true),
        ("/admin/posts?q=不存在的词", false),
        ("/admin/posts?page=2", false),
    ] {
        let (status, html) = app.page(uri, Some(&admin)).await;
        assert_eq!(status, 200, "{uri}");
        assert_eq!(html.contains("表单文章"), expect_form_post, "{uri}");
    }
}

#[tokio::test]
async fn note_promotion_form_creates_a_post() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let note = app
        .tool(
            "note_create",
            json!({"title": "待转笔记", "body": "笔记正文"}),
        )
        .await;
    let note_id = note["structuredContent"]["id"].as_i64().expect("id");

    let response = app
        .post_form(
            &format!("/admin/notes/{note_id}/promote"),
            Some(&admin),
            &[("_csrf", &admin.csrf), ("status", "draft")],
        )
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(location(&response).starts_with("/admin/posts/"));
    let promoted = content::get_by_id(&app.pool, note_id)
        .await
        .expect("get")
        .expect("note");
    assert!(promoted.promoted_to_id.is_some());
}

async fn annotate_url(app: &TestApp, admin: &Session) -> (i64, String) {
    let id = create_post_via_form(app, admin).await;
    (id, format!("/admin/posts/{id}/annotations"))
}

async fn only_annotation(app: &TestApp) -> (i64, String) {
    sqlx::query_as("SELECT id, status FROM annotations")
        .fetch_one(&app.pool)
        .await
        .expect("annotation")
}

#[tokio::test]
async fn annotations_can_be_created_answered_resolved_and_reopened() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let (post_id, url) = annotate_url(&app, &admin).await;

    let empty = app
        .post_form(
            &url,
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("kind", "general"),
                ("comment", " "),
            ],
        )
        .await;
    assert!(location(&empty).ends_with("error=invalid"));

    let created = app
        .post_form(
            &url,
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("kind", "inline"),
                ("quote", "表单正文"),
                ("start_offset", "0"),
                ("end_offset", "4"),
                ("comment", "改一下"),
            ],
        )
        .await;
    assert!(location(&created).ends_with("notice=annotation_created"));
    let (annotation_id, status) = only_annotation(&app).await;
    assert_eq!(status, "open");

    let reply_url = format!("/admin/annotations/{annotation_id}/reply");
    let empty_reply = app
        .post_form(
            &reply_url,
            Some(&admin),
            &[("_csrf", &admin.csrf), ("body", "")],
        )
        .await;
    assert!(location(&empty_reply).ends_with("error=invalid"));
    let reply = app
        .post_form(
            &reply_url,
            Some(&admin),
            &[("_csrf", &admin.csrf), ("body", "好的")],
        )
        .await;
    assert!(location(&reply).ends_with("notice=annotation_replied"));

    for (action, expected, notice) in [
        ("resolve", "resolved", "annotation_resolved"),
        ("reopen", "open", "annotation_reopened"),
    ] {
        let response = app
            .post_form(
                &format!("/admin/annotations/{annotation_id}/{action}"),
                Some(&admin),
                &[("_csrf", &admin.csrf)],
            )
            .await;
        assert_eq!(
            location(&response),
            format!("/admin/posts/{post_id}/annotate?notice={notice}")
        );
        assert_eq!(only_annotation(&app).await.1, expected);
    }

    for uri in ["/admin/annotations", "/admin/annotations?status=all"] {
        let (status, html) = app.page(uri, Some(&admin)).await;
        assert_eq!(status, 200);
        assert!(html.contains("改一下"), "{uri}");
    }
}

#[tokio::test]
async fn terms_can_be_created_and_deleted() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let created = app
        .post_form(
            "/admin/terms",
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("kind", "tag"),
                ("name", "新标签"),
                ("slug", "new-tag"),
            ],
        )
        .await;
    assert!(location(&created).ends_with("notice=created"));
    let (_, html) = app.page("/admin/terms?status=tag", Some(&admin)).await;
    assert!(html.contains("新标签"));

    let invalid = app
        .post_form(
            "/admin/terms",
            Some(&admin),
            &[("_csrf", &admin.csrf), ("kind", "tag"), ("name", " ")],
        )
        .await;
    assert_eq!(
        invalid.status(),
        StatusCode::OK,
        "re-rendered with an error"
    );

    let term_id: i64 = sqlx::query_scalar("SELECT id FROM terms WHERE slug = 'new-tag'")
        .fetch_one(&app.pool)
        .await
        .expect("term");
    let deleted = app
        .post_form(
            &format!("/admin/terms/{term_id}/delete"),
            Some(&admin),
            &[("_csrf", &admin.csrf)],
        )
        .await;
    assert!(location(&deleted).ends_with("notice=deleted"));
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM terms WHERE slug = 'new-tag'")
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(remaining, 0);
}

fn settings_form<'a>(csrf: &'a str, title: &'a str, page_size: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("_csrf", csrf),
        ("site_title", title),
        ("site_description", "描述"),
        ("site_keywords", "关键词"),
        ("site_url", "http://test.local"),
        ("footer_text", "页脚"),
        ("page_size", page_size),
        ("timezone_offset", "480"),
        ("comments_enabled", "1"),
    ]
}

#[tokio::test]
async fn site_settings_are_saved_and_validated() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let saved = app
        .post_form(
            "/admin/settings",
            Some(&admin),
            &settings_form(&admin.csrf, "新的站点名", "10"),
        )
        .await;
    assert!(location(&saved).ends_with("notice=settings_saved"));
    let (_, home) = app.page("/", None).await;
    assert!(home.contains("新的站点名"));

    let invalid = app
        .post_form(
            "/admin/settings",
            Some(&admin),
            &settings_form(&admin.csrf, "另一个名字", "abc"),
        )
        .await;
    assert_eq!(
        invalid.status(),
        StatusCode::OK,
        "re-rendered with an error"
    );
    let (_, home) = app.page("/", None).await;
    assert!(!home.contains("另一个名字"));
}

#[tokio::test]
async fn comments_are_moderated_from_the_admin() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let comment_id = comment::create(
        &app.pool,
        NewComment {
            content_id: 1,
            parent_id: 0,
            author: "读者".to_string(),
            email: None,
            url: None,
            body: "待审核的评论".to_string(),
            status: "pending".to_string(),
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("comment");

    let (_, pending) = app
        .page("/admin/comments?status=pending", Some(&admin))
        .await;
    assert!(pending.contains("待审核的评论"));

    let response = app
        .post_form(
            &format!("/admin/comments/{comment_id}/status"),
            Some(&admin),
            &[("_csrf", &admin.csrf), ("action", "approve")],
        )
        .await;
    assert!(location(&response).ends_with("notice=moderated"));
    let stored = comment::get(&app.pool, comment_id)
        .await
        .expect("get")
        .expect("comment");
    assert_eq!(stored.status, "approved");
}

#[tokio::test]
async fn review_form_records_verdicts_and_can_publish() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let id = create_post_via_form(&app, &admin).await;
    app.tool("review_submit", json!({"id": id})).await;
    let (_, queue) = app.page("/admin/review", Some(&admin)).await;
    assert!(queue.contains("表单文章"));

    let changes = app
        .post_form(
            &format!("/admin/review/{id}"),
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("verdict", "changes_requested"),
                ("notes", "再改改"),
            ],
        )
        .await;
    assert!(location(&changes).ends_with("notice=review_saved"));
    let post = content::get_by_id(&app.pool, id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(post.review_status, "changes_requested");
    assert_eq!(post.status, "draft");

    app.post_form(
        &format!("/admin/review/{id}"),
        Some(&admin),
        &[
            ("_csrf", &admin.csrf),
            ("verdict", "approved"),
            ("publish", "1"),
        ],
    )
    .await;
    let post = content::get_by_id(&app.pool, id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(post.review_status, "approved");
    assert_eq!(post.status, "published");
    assert_eq!(post.review_notes, None, "approval clears stale notes");
}

#[tokio::test]
async fn password_change_checks_the_old_password_and_ends_other_sessions() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;

    let user_id: i64 = sqlx::query_scalar("SELECT user_id FROM sessions")
        .fetch_one(&app.pool)
        .await
        .expect("session owner");
    inkwell::auth::create_session(&app.pool, user_id, 1, None, None)
        .await
        .expect("second browser");

    let wrong = app
        .post_form(
            "/admin/password",
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("old_password", "not-it"),
                ("new_password", "brand-new-password"),
                ("confirm_password", "brand-new-password"),
            ],
        )
        .await;
    assert!(body_string(wrong).await.contains("原密码不正确"));

    for (new, confirm, message) in [
        ("short", "short", "至少需要 8 个字符"),
        (
            "brand-new-password",
            "different-password",
            "两次输入的新密码不一致",
        ),
    ] {
        let response = app
            .post_form(
                "/admin/password",
                Some(&admin),
                &[
                    ("_csrf", &admin.csrf),
                    ("old_password", "unused-test-password"),
                    ("new_password", new),
                    ("confirm_password", confirm),
                ],
            )
            .await;
        assert!(body_string(response).await.contains(message), "{message}");
    }

    let changed = app
        .post_form(
            "/admin/password",
            Some(&admin),
            &[
                ("_csrf", &admin.csrf),
                ("old_password", "unused-test-password"),
                ("new_password", "brand-new-password"),
                ("confirm_password", "brand-new-password"),
            ],
        )
        .await;
    assert!(location(&changed).ends_with("notice=password_changed"));
    assert_eq!(
        app.get("/admin", Some(&admin)).await.status(),
        StatusCode::OK,
        "own session kept"
    );
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(sessions, 1, "the other browser was signed out");
}

async fn json_post(
    app: &TestApp,
    uri: &str,
    session: &Session,
    csrf: Option<&str>,
    body: serde_json::Value,
) -> (u16, String) {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &session.cookie);
    if let Some(csrf) = csrf {
        request = request.header("x-csrf-token", csrf);
    }
    let response = app
        .send(request.body(Body::from(body.to_string())).expect("request"))
        .await;
    let status = response.status().as_u16();
    (status, body_string(response).await)
}

#[tokio::test]
async fn live_preview_renders_markdown_with_csrf() {
    let app = TestApp::new().await;
    let editor = app.session_for("editor").await;

    let (status, html) = json_post(
        &app,
        "/admin/preview",
        &editor,
        Some(&editor.csrf),
        json!({"body": "# 标题"}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(html.contains("<h1>标题</h1>"));

    let (status, _) = json_post(
        &app,
        "/admin/preview",
        &editor,
        None,
        json!({"body": "# 标题"}),
    )
    .await;
    assert_eq!(status, 403);

    let (status, body) = json_post(
        &app,
        "/admin/vditor/preview",
        &editor,
        Some(&editor.csrf),
        json!({"markdownText": "**粗体**"}),
    )
    .await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(json["code"], 0);
    assert!(
        json["data"]
            .as_str()
            .unwrap_or_default()
            .contains("<strong>粗体</strong>")
    );
}
