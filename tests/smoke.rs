use std::net::SocketAddr;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use inkwell::{app, config::Config};
use serde_json::{Value, json};
use tower::ServiceExt;

fn test_config(data_dir: &Path) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        database_url: "sqlite::memory:".to_string(),
        data_dir: data_dir.to_path_buf(),
        site_url: "http://test.local".to_string(),
        admin_user: "admin".to_string(),
        admin_password: Some("testpass123".to_string()),
        mcp_token: Some("testtoken".to_string()),
        mcp_token_scopes: vec![
            "read".to_string(),
            "write".to_string(),
            "review".to_string(),
        ],
        secure_cookies: false,
        trust_proxy_headers: false,
        session_days: 14,
        max_upload_bytes: 1024 * 1024,
    }
}

async fn body_string(response: Response<Body>) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

fn cookie_from(headers: &header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let first = value.split(';').next()?;
            let (key, value) = first.split_once('=')?;
            (key == name).then(|| value.to_string())
        })
}

fn hidden_value(html: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\" value=\"");
    let start = html.find(&needle).expect("hidden input") + needle.len();
    let end = html[start..].find('"').expect("hidden input end") + start;
    html[start..end].to_string()
}

fn encode(values: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in values {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

async fn mcp_call(router: &Router, payload: Value) -> Value {
    mcp_call_with_token(router, payload, "testtoken").await
}

async fn mcp_call_with_token(router: &Router, payload: Value, token: &str) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .expect("request"),
        )
        .await
        .expect("mcp response");
    assert_eq!(response.status(), 200);
    serde_json::from_str(&body_string(response).await).expect("mcp json")
}

async fn mcp_tool(router: &Router, name: &str, arguments: Value) -> Value {
    mcp_tool_with_token(router, name, arguments, "testtoken").await
}

async fn mcp_tool_with_token(router: &Router, name: &str, arguments: Value, token: &str) -> Value {
    let response = mcp_call_with_token(
        router,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
        token,
    )
    .await;
    response["result"].clone()
}

#[tokio::test]
async fn blog_login_notes_and_mcp_smoke() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = app::build_state(test_config(temp.path()))
        .await
        .expect("build state");
    let pool = state.db.clone();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(temp.path())
                .expect("data dir metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(temp.path().join("uploads"))
                .expect("uploads dir metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let router =
        app::router(state).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));

    // Public home renders the seeded welcome post.
    let home = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("home");
    assert_eq!(home.status(), 200);
    let home_html = body_string(home).await;
    assert!(home_html.contains("欢迎使用 inkwell"));

    // Public comments are rate-limited by the socket peer, not spoofable headers.
    for attempt in 0..6 {
        let comment = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/comments/1")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(encode(&[
                        ("author", "Reader"),
                        ("body", &format!("测试评论 {attempt}")),
                    ])))
                    .expect("comment request"),
            )
            .await
            .expect("comment response");
        if attempt < 5 {
            assert_eq!(comment.status(), StatusCode::SEE_OTHER);
        } else {
            assert_eq!(comment.status(), StatusCode::TOO_MANY_REQUESTS);
        }
    }

    let css = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/static/site.css")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("css");
    assert_eq!(css.status(), 200);
    assert!(body_string(css).await.contains("post-card"));

    // Login with CSRF and cookie.
    let login = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/login")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("login page");
    assert_eq!(login.status(), 200);
    let login_csrf_cookie =
        cookie_from(login.headers(), "typecho_login_csrf").expect("login csrf cookie");
    let login_html = body_string(login).await;
    let login_csrf = hidden_value(&login_html, "csrf");

    let form = encode(&[
        ("username", "admin"),
        ("password", "testpass123"),
        ("next", "/admin"),
        ("csrf", &login_csrf),
    ]);
    let login_submit = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/admin/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(
                    header::COOKIE,
                    format!("typecho_login_csrf={login_csrf_cookie}"),
                )
                .body(Body::from(form))
                .expect("request"),
        )
        .await
        .expect("login submit");
    assert_eq!(login_submit.status(), 303);
    let session = cookie_from(login_submit.headers(), "typecho_session").expect("session cookie");

    let admin = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin")
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("admin");
    assert_eq!(admin.status(), 200);
    assert_eq!(
        admin.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert!(body_string(admin).await.contains("仪表盘"));

    // Failed file deletion must preserve the media row so it can be retried.
    let undeletable_key = "delete-failure-directory";
    std::fs::create_dir(temp.path().join("uploads").join(undeletable_key))
        .expect("create undeletable media path");
    let undeletable_media_id = inkwell::repo::media::insert(
        &pool,
        inkwell::repo::media::NewMedia {
            storage_key: undeletable_key.to_string(),
            original_name: "delete-failure".to_string(),
            mime: "application/octet-stream".to_string(),
            size: 0,
            width: None,
            height: None,
            uploader_id: Some(1),
        },
    )
    .await
    .expect("insert undeletable media");
    let media_page = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/media")
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("media page"),
        )
        .await
        .expect("media page response");
    let media_csrf = hidden_value(&body_string(media_page).await, "_csrf");
    let failed_delete = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/admin/media/{undeletable_media_id}/delete"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(encode(&[("_csrf", &media_csrf)])))
                .expect("media delete request"),
        )
        .await
        .expect("media delete response");
    assert_eq!(failed_delete.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        inkwell::repo::media::get(&pool, undeletable_media_id)
            .await
            .expect("read media after failed delete")
            .is_some()
    );

    // Private media and taxonomy stay private unless a public post references them.
    let author_id: i64 = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("seeded author");
    let storage_key = "2026/11/private-preview.png";
    let upload_path = temp.path().join("uploads").join(storage_key);
    std::fs::create_dir_all(upload_path.parent().expect("upload parent"))
        .expect("create upload directory");
    std::fs::write(&upload_path, b"private image bytes").expect("write test media");
    inkwell::repo::media::insert(
        &pool,
        inkwell::repo::media::NewMedia {
            storage_key: storage_key.to_string(),
            original_name: "private-preview.png".to_string(),
            mime: "image/png".to_string(),
            size: 19,
            width: None,
            height: None,
            uploader_id: Some(author_id),
        },
    )
    .await
    .expect("insert test media");
    inkwell::repo::content::create(
        &pool,
        inkwell::repo::content::ContentInput {
            kind: "note".to_string(),
            title: "Private media fixture".to_string(),
            slug: Some("private-media-fixture".to_string()),
            body: format!("![private](/media/{storage_key})"),
            excerpt: None,
            status: Some("inbox".to_string()),
            visibility: Some("private".to_string()),
            review_status: Some("none".to_string()),
            review_notes: None,
            allow_comments: Some(false),
            category: Some("secret-project".to_string()),
            tags: vec!["secret-project".to_string()],
            author_id,
            actor: "test".to_string(),
            source_note_id: None,
        },
    )
    .await
    .expect("create private note fixture");

    let private_media = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/media/{storage_key}"))
                .body(Body::empty())
                .expect("private media request"),
        )
        .await
        .expect("private media response");
    assert_eq!(private_media.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        private_media.headers().get(header::CACHE_CONTROL).unwrap(),
        "private, no-store"
    );

    let private_media_for_admin = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/media/{storage_key}"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("authenticated private media request"),
        )
        .await
        .expect("authenticated private media response");
    assert_eq!(private_media_for_admin.status(), StatusCode::OK);
    assert_eq!(
        private_media_for_admin
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap(),
        "private, no-store"
    );

    let private_tag = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tags/secret-project")
                .body(Body::empty())
                .expect("private tag request"),
        )
        .await
        .expect("private tag response");
    assert_eq!(private_tag.status(), StatusCode::NOT_FOUND);
    let private_category = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/categories/secret-project")
                .body(Body::empty())
                .expect("private category request"),
        )
        .await
        .expect("private category response");
    assert_eq!(private_category.status(), StatusCode::NOT_FOUND);
    let sitemap = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sitemap.xml")
                .body(Body::empty())
                .expect("sitemap request"),
        )
        .await
        .expect("sitemap response");
    let sitemap_xml = body_string(sitemap).await;
    assert!(!sitemap_xml.contains("/tags/secret-project"));
    assert!(!sitemap_xml.contains("/categories/secret-project"));

    let public_post_id = inkwell::repo::content::create(
        &pool,
        inkwell::repo::content::ContentInput {
            kind: "post".to_string(),
            title: "Public media fixture".to_string(),
            slug: Some("public-media-fixture".to_string()),
            body: format!("![public](/media/{storage_key})"),
            excerpt: None,
            status: Some("published".to_string()),
            visibility: Some("public".to_string()),
            review_status: Some("none".to_string()),
            review_notes: None,
            allow_comments: Some(true),
            category: Some("secret-project".to_string()),
            tags: vec!["secret-project".to_string()],
            author_id,
            actor: "test".to_string(),
            source_note_id: None,
        },
    )
    .await
    .expect("create public post fixture");

    let public_media = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/media/{storage_key}"))
                .body(Body::empty())
                .expect("public media request"),
        )
        .await
        .expect("public media response");
    assert_eq!(public_media.status(), StatusCode::OK);
    assert_eq!(
        public_media.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    let public_tag = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tags/secret-project")
                .body(Body::empty())
                .expect("public tag request"),
        )
        .await
        .expect("public tag response");
    assert_eq!(public_tag.status(), StatusCode::OK);
    let public_category = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/categories/secret-project")
                .body(Body::empty())
                .expect("public category request"),
        )
        .await
        .expect("public category response");
    assert_eq!(public_category.status(), StatusCode::OK);

    inkwell::repo::content::update(
        &pool,
        public_post_id,
        inkwell::repo::content::ContentPatch {
            status: Some("draft".to_string()),
            visibility: Some("private".to_string()),
            actor: "test".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("make media reference private");
    let private_again = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/media/{storage_key}"))
                .body(Body::empty())
                .expect("private media after unpublish request"),
        )
        .await
        .expect("private media after unpublish response");
    assert_eq!(private_again.status(), StatusCode::NOT_FOUND);
    let sitemap = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/sitemap.xml")
                .body(Body::empty())
                .expect("updated sitemap request"),
        )
        .await
        .expect("updated sitemap response");
    let sitemap_xml = body_string(sitemap).await;
    assert!(!sitemap_xml.contains("/tags/secret-project"));
    assert!(!sitemap_xml.contains("/categories/secret-project"));

    // MCP initialize and note creation.
    let init = mcp_call(
        &router,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        }),
    )
    .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "inkwell");
    let instance_info = mcp_tool(&router, "instance_info", json!({})).await;
    assert_eq!(instance_info["isError"], Value::Null);
    let database_instance_id: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE name = 'installation_id'")
            .fetch_one(&pool)
            .await
            .expect("installation id");
    assert_eq!(
        instance_info["structuredContent"]["installation_id"],
        database_instance_id
    );
    let database_runtime_id: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE name = 'runtime_id'")
            .fetch_one(&pool)
            .await
            .expect("runtime id");
    assert_eq!(
        instance_info["structuredContent"]["runtime_id"],
        database_runtime_id
    );
    let import_probe = "smoke-test-probe";
    sqlx::query("INSERT INTO settings(name, value) VALUES(?, ?)")
        .bind("__inkwell_import_target_probe__")
        .bind(import_probe)
        .execute(&pool)
        .await
        .expect("insert import probe");
    let probed_instance = mcp_tool(&router, "instance_info", json!({"probe": import_probe})).await;
    assert_eq!(probed_instance["structuredContent"]["probe_matches"], true);
    sqlx::query("DELETE FROM settings WHERE name = ? AND value = ?")
        .bind("__inkwell_import_target_probe__")
        .bind(import_probe)
        .execute(&pool)
        .await
        .expect("remove import probe");

    let note = mcp_tool(
        &router,
        "note_create",
        json!({
            "title": "测试笔记",
            "body": "这是一篇测试笔记。参见 [[另一篇]]。",
            "tags": ["test", "笔记"]
        }),
    )
    .await;
    assert_eq!(note["isError"], Value::Null);
    let note_id = note["structuredContent"]["id"].as_i64().expect("note id");

    // Note editor renders and offers promotion.
    let note_edit = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/notes/{note_id}/edit"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("note edit");
    assert_eq!(note_edit.status(), 200);
    assert!(body_string(note_edit).await.contains("转为文章"));

    // Default MCP token cannot publish.
    let publish = mcp_tool(&router, "content_publish", json!({"id": note_id})).await;
    assert_eq!(publish["isError"], true);
    let delete_comment = mcp_tool(
        &router,
        "comment_moderate",
        json!({"id": 1, "action": "delete"}),
    )
    .await;
    assert_eq!(delete_comment["isError"], true);

    // Promote the note into a draft post.
    let promoted = mcp_tool(
        &router,
        "note_promote",
        json!({"id": note_id, "status": "draft"}),
    )
    .await;
    assert_eq!(promoted["isError"], Value::Null);
    assert_eq!(promoted["structuredContent"]["created"], true);

    // A write-only MCP principal cannot sync a note into an already published post.
    let published_post_id: i64 = sqlx::query_scalar(
        "SELECT id FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
         ORDER BY id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("seeded published post");
    sqlx::query("UPDATE contents SET promoted_to_id = ? WHERE id = ?")
        .bind(published_post_id)
        .bind(note_id)
        .execute(&pool)
        .await
        .expect("associate note with published post");
    let published_before = inkwell::repo::content::get_by_id(&pool, published_post_id)
        .await
        .expect("read published post")
        .expect("published post exists");
    let revisions_before = inkwell::repo::revision::list(&pool, published_post_id, 100)
        .await
        .expect("read published post revisions");

    let sync_published = mcp_tool(
        &router,
        "note_promote",
        json!({"id": note_id, "sync": true}),
    )
    .await;
    assert_eq!(sync_published["isError"], true);

    let published_after = inkwell::repo::content::get_by_id(&pool, published_post_id)
        .await
        .expect("read published post after denied sync")
        .expect("published post remains");
    let revisions_after = inkwell::repo::revision::list(&pool, published_post_id, 100)
        .await
        .expect("read published post revisions after denied sync");
    assert_eq!(published_after.body, published_before.body);
    assert_eq!(revisions_after.len(), revisions_before.len());

    let (_, publisher_token) = inkwell::repo::token::create(
        &pool,
        "publish-test",
        &["read".into(), "write".into(), "publish".into()],
    )
    .await
    .expect("create publish-scoped token");
    let sync_with_publish = mcp_tool_with_token(
        &router,
        "note_promote",
        json!({"id": note_id, "sync": true}),
        &publisher_token,
    )
    .await;
    assert_eq!(sync_with_publish["isError"], Value::Null);
    assert_eq!(sync_with_publish["structuredContent"]["created"], false);
    let published_after_authorized_sync =
        inkwell::repo::content::get_by_id(&pool, published_post_id)
            .await
            .expect("read published post after authorized sync")
            .expect("published post remains");
    assert_eq!(
        published_after_authorized_sync.body,
        "这是一篇测试笔记。参见 [[另一篇]]。"
    );

    // Review workflow.
    let submit = mcp_tool(
        &router,
        "review_submit",
        json!({"id": note_id, "notes": "请检查结构"}),
    )
    .await;
    assert_eq!(submit["structuredContent"]["review_status"], "pending");
    let queue = mcp_tool(&router, "review_queue", json!({})).await;
    assert!(
        queue["structuredContent"]["content"]
            .as_array()
            .expect("queue")
            .iter()
            .any(|item| item["id"] == note_id)
    );

    // Collaborative annotation workflow: draft -> annotation -> AI edit -> reply -> diff.
    let draft = mcp_tool(
        &router,
        "content_create",
        json!({
            "kind": "post",
            "title": "批注测试",
            "body": "第一段。\n\n第二段需要改进。\n\n第三段。",
            "status": "draft"
        }),
    )
    .await;
    assert_eq!(draft["isError"], Value::Null);
    let post_id = draft["structuredContent"]["id"]
        .as_i64()
        .expect("draft post id");

    let annotation = mcp_tool(
        &router,
        "annotation_create",
        json!({
            "content_id": post_id,
            "kind": "inline",
            "quote": "第二段需要改进。",
            "comment": "这段太生硬，润色一下"
        }),
    )
    .await;
    assert_eq!(annotation["isError"], Value::Null);
    let annotation_id = annotation["structuredContent"]["id"]
        .as_i64()
        .expect("annotation id");

    let annotation_queue = mcp_tool(&router, "annotation_queue", json!({"status": "open"})).await;
    assert!(
        annotation_queue["structuredContent"]["items"]
            .as_array()
            .expect("annotation queue")
            .iter()
            .any(|item| item["annotation"]["id"] == annotation_id)
    );

    let updated = mcp_tool(
        &router,
        "content_update",
        json!({
            "id": post_id,
            "body": "第一段。\n\n第二段已经润色。\n\n第三段。"
        }),
    )
    .await;
    assert_eq!(updated["isError"], Value::Null);

    let replied = mcp_tool(
        &router,
        "annotation_reply",
        json!({
            "id": annotation_id,
            "body": "已润色为更自然的表达。",
            "status": "addressed"
        }),
    )
    .await;
    assert_eq!(
        replied["structuredContent"]["annotation"]["status"],
        "addressed"
    );
    assert_eq!(
        replied["structuredContent"]["replies"]
            .as_array()
            .expect("replies")
            .len(),
        1
    );

    let addressed = mcp_tool(
        &router,
        "annotation_list",
        json!({"content_id": post_id, "status": "addressed"}),
    )
    .await;
    assert_eq!(addressed["structuredContent"]["total"], 1);

    let annotate = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/posts/{post_id}/annotate"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("annotate page");
    assert_eq!(annotate.status(), 200);
    let annotate_html = body_string(annotate).await;
    assert!(annotate_html.contains("第二段需要改进。"));
    assert!(annotate_html.contains("annotations.js"));

    let revisions = mcp_tool(
        &router,
        "revision_list",
        json!({"content_id": post_id, "limit": 5}),
    )
    .await;
    let revision_id = revisions["structuredContent"]["items"][0]["id"]
        .as_i64()
        .expect("revision id");
    let diff = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/posts/{post_id}/revisions/{revision_id}"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("diff page");
    assert_eq!(diff.status(), 200);
    let diff_html = body_string(diff).await;
    assert!(diff_html.contains("diff-add"));
    assert!(diff_html.contains("第二段已经润色。"));

    // Proposed versions are attached to the same content, not new articles.
    let proposal = mcp_tool(
        &router,
        "proposal_create",
        json!({
            "content_id": post_id,
            "title": "批注测试（版本）",
            "body": "第一段。\n\n第二段是新的版本。\n\n第三段。",
            "summary": "压缩第二段"
        }),
    )
    .await;
    assert_eq!(proposal["isError"], Value::Null);
    let proposal_id = proposal["structuredContent"]["id"]
        .as_i64()
        .expect("proposal id");

    let proposals = mcp_tool(
        &router,
        "proposal_list",
        json!({"content_id": post_id, "status": "proposed"}),
    )
    .await;
    assert_eq!(proposals["structuredContent"]["total"], 1);

    let proposal_diff = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/posts/{post_id}/proposals/{proposal_id}"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("proposal diff page");
    assert_eq!(proposal_diff.status(), 200);
    let proposal_diff_html = body_string(proposal_diff).await;
    assert!(proposal_diff_html.contains("diff-add"));
    let reviewed_version = hidden_value(&proposal_diff_html, "version")
        .parse::<i64>()
        .expect("proposal version");

    let changed_proposal = mcp_tool(
        &router,
        "proposal_update",
        json!({
            "id": proposal_id,
            "version": reviewed_version,
            "body": "第一段。\n\n第二段是后来修改的版本。\n\n第三段。",
            "summary": "后来修改"
        }),
    )
    .await;
    assert_eq!(changed_proposal["isError"], Value::Null);
    let latest_version = changed_proposal["structuredContent"]["version"]
        .as_i64()
        .expect("latest proposal version");
    assert_eq!(latest_version, reviewed_version + 1);
    let proposal_snapshots: Vec<(i64, String)> = sqlx::query_as(
        "SELECT version, body FROM content_proposal_versions \
         WHERE proposal_id = ? ORDER BY version",
    )
    .bind(proposal_id)
    .fetch_all(&pool)
    .await
    .expect("read immutable proposal versions");
    assert_eq!(proposal_snapshots.len(), 2);
    assert_eq!(proposal_snapshots[0].0, reviewed_version);
    assert!(proposal_snapshots[0].1.contains("第二段是新的版本。"));
    assert_eq!(proposal_snapshots[1].0, latest_version);
    assert!(proposal_snapshots[1].1.contains("第二段是后来修改的版本。"));

    // A browser form must not apply a proposal revision different from the one it rendered.
    let stale_apply = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/admin/posts/{post_id}/proposals/{proposal_id}/apply"
                ))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::from(encode(&[
                    ("_csrf", &hidden_value(&proposal_diff_html, "_csrf")),
                    ("version", &reviewed_version.to_string()),
                ])))
                .expect("stale apply request"),
        )
        .await
        .expect("stale apply response");
    assert_eq!(stale_apply.status(), StatusCode::SEE_OTHER);
    let expected_location =
        format!("/admin/posts/{post_id}/proposals/{proposal_id}?error=proposal_changed");
    assert_eq!(
        stale_apply
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(expected_location.as_str())
    );
    let latest_proposal_page = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&expected_location)
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("latest proposal diff request"),
        )
        .await
        .expect("latest proposal diff response");
    assert_eq!(latest_proposal_page.status(), StatusCode::OK);
    let latest_proposal_html = body_string(latest_proposal_page).await;
    assert!(latest_proposal_html.contains("提案或文章在预览后发生了变化"));
    assert!(latest_proposal_html.contains("第二段是后来修改的版本。"));

    let latest = mcp_tool(&router, "proposal_get", json!({"id": proposal_id})).await;
    let latest_version = latest["structuredContent"]["version"]
        .as_i64()
        .expect("latest proposal version");
    let live_before_apply = inkwell::repo::content::get_by_id(&pool, post_id)
        .await
        .expect("read live content before apply")
        .expect("live content exists");
    assert!(live_before_apply.body.contains("第二段已经润色。"));

    let concurrent_edit = mcp_tool(
        &router,
        "content_update",
        json!({
            "id": post_id,
            "body": "第一段。\n\n作者在审核期间又修改了正文。\n\n第三段。",
            "expected_updated_at": live_before_apply.updated_at
        }),
    )
    .await;
    assert_eq!(concurrent_edit["isError"], Value::Null);
    let base_conflict = mcp_tool(
        &router,
        "proposal_apply",
        json!({"id": proposal_id, "version": latest_version}),
    )
    .await;
    assert_eq!(base_conflict["isError"], true);

    let applied = mcp_tool(
        &router,
        "proposal_apply",
        json!({"id": proposal_id, "version": latest_version, "force": true}),
    )
    .await;
    assert_eq!(applied["isError"], Value::Null);
    assert_eq!(
        applied["structuredContent"]["proposal"]["status"],
        "applied"
    );
    let content = mcp_tool(&router, "content_get", json!({"id": post_id})).await;
    assert!(
        content["structuredContent"]["content"]["body"]
            .as_str()
            .expect("body")
            .contains("第二段是后来修改的版本。")
    );
    let applied_revision_id = applied["structuredContent"]["proposal"]["applied_revision_id"]
        .as_i64()
        .expect("applied revision id");
    let latest_revision = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM revisions WHERE content_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(post_id)
    .fetch_one(&pool)
    .await
    .expect("revision created by apply");
    assert_eq!(applied_revision_id, latest_revision);

    let rejected = mcp_tool(
        &router,
        "proposal_create",
        json!({
            "content_id": post_id,
            "body": "第一段。\n\n这个版本会被拒绝。\n\n第三段。",
            "summary": "待拒绝"
        }),
    )
    .await;
    let rejected_id = rejected["structuredContent"]["id"]
        .as_i64()
        .expect("rejected proposal id");
    let rejected = mcp_tool(&router, "proposal_reject", json!({"id": rejected_id})).await;
    assert_eq!(rejected["structuredContent"]["status"], "rejected");

    // A stale admin edit must surface a conflict instead of overwriting a newer save.
    let concurrent_draft = mcp_tool(
        &router,
        "content_create",
        json!({
            "kind": "post",
            "title": "并发保存测试",
            "slug": "concurrent-save-test",
            "body": "初始正文。",
            "status": "draft"
        }),
    )
    .await;
    let concurrent_id = concurrent_draft["structuredContent"]["id"]
        .as_i64()
        .expect("concurrent draft id");
    let edit_page = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/posts/{concurrent_id}/edit"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .body(Body::empty())
                .expect("concurrent edit page"),
        )
        .await
        .expect("concurrent edit page response");
    let edit_html = body_string(edit_page).await;
    let stale_csrf = hidden_value(&edit_html, "_csrf");
    let stale_updated_at = hidden_value(&edit_html, "expected_updated_at");

    inkwell::repo::content::update(
        &pool,
        concurrent_id,
        inkwell::repo::content::ContentPatch {
            body: Some("服务器上的较新正文。".to_string()),
            actor: "test".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("save newer content");

    let stale_form = encode(&[
        ("_csrf", &stale_csrf),
        ("expected_updated_at", &stale_updated_at),
        ("title", "并发保存测试"),
        ("slug", "concurrent-save-test"),
        ("body", "旧页面提交的正文。"),
        ("excerpt", ""),
        ("status", "draft"),
        ("visibility", "public"),
        ("review_status", "none"),
        ("review_notes", ""),
        ("allow_comments", "1"),
        ("category", ""),
        ("tags", ""),
        ("action", "save"),
    ]);
    let stale_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/admin/posts/{concurrent_id}"))
                .header(header::COOKIE, format!("typecho_session={session}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(stale_form))
                .expect("stale admin save"),
        )
        .await
        .expect("stale admin response");
    assert_eq!(stale_response.status(), StatusCode::OK);
    let stale_html = body_string(stale_response).await;
    assert!(stale_html.contains("内容已被其他人修改"));
    assert!(stale_html.contains("旧页面提交的正文。"));
    let persisted = inkwell::repo::content::get_by_id(&pool, concurrent_id)
        .await
        .expect("read concurrent draft")
        .expect("concurrent draft exists");
    assert_eq!(persisted.body, "服务器上的较新正文。");
}
