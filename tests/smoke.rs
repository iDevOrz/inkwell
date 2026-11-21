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
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::AUTHORIZATION, "Bearer testtoken")
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
    let response = mcp_call(
        router,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
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
    assert!(body_string(admin).await.contains("仪表盘"));

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

    // Promote the note into a draft post.
    let promoted = mcp_tool(
        &router,
        "note_promote",
        json!({"id": note_id, "status": "draft"}),
    )
    .await;
    assert_eq!(promoted["isError"], Value::Null);
    assert_eq!(promoted["structuredContent"]["created"], true);

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
    assert!(body_string(proposal_diff).await.contains("diff-add"));

    let applied = mcp_tool(&router, "proposal_apply", json!({"id": proposal_id})).await;
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
            .contains("第二段是新的版本。")
    );

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
}
