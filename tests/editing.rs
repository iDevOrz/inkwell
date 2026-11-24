//! Collaborative editing: annotations, revision diffs, AI proposals and
//! their versioning, and conflict handling for stale saves.
mod common;

use axum::http::{StatusCode, header};
use common::{Session, TestApp, body_string, hidden_value, mcp_tool};
use inkwell::repo::content::{self, ContentPatch};
use serde_json::{Value, json};

const ORIGINAL_BODY: &str = "第一段。\n\n第二段需要改进。\n\n第三段。";
const EDITED_BODY: &str = "第一段。\n\n第二段已经润色。\n\n第三段。";

async fn create_draft(app: &TestApp) -> i64 {
    let draft = app
        .tool(
            "content_create",
            json!({"kind": "post", "title": "批注测试", "body": ORIGINAL_BODY, "status": "draft"}),
        )
        .await;
    assert_eq!(draft["isError"], Value::Null, "{draft}");
    draft["structuredContent"]["id"].as_i64().expect("post id")
}

async fn annotate(app: &TestApp, post_id: i64) -> i64 {
    let annotation = app
        .tool(
            "annotation_create",
            json!({
                "content_id": post_id,
                "kind": "inline",
                "quote": "第二段需要改进。",
                "comment": "这段太生硬，润色一下"
            }),
        )
        .await;
    assert_eq!(annotation["isError"], Value::Null, "{annotation}");
    annotation["structuredContent"]["id"]
        .as_i64()
        .expect("annotation id")
}

#[tokio::test]
async fn annotation_is_queued_answered_and_shown_on_the_annotate_page() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let post_id = create_draft(&app).await;
    let annotation_id = annotate(&app, post_id).await;

    let queue = app
        .tool("annotation_queue", json!({"status": "open"}))
        .await;
    assert!(
        queue["structuredContent"]["items"]
            .as_array()
            .expect("queue")
            .iter()
            .any(|item| item["annotation"]["id"] == annotation_id)
    );

    let updated = app
        .tool(
            "content_update",
            json!({"id": post_id, "body": EDITED_BODY}),
        )
        .await;
    assert_eq!(updated["isError"], Value::Null);
    let replied = app
        .tool(
            "annotation_reply",
            json!({"id": annotation_id, "body": "已润色为更自然的表达。", "status": "addressed"}),
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

    let addressed = app
        .tool(
            "annotation_list",
            json!({"content_id": post_id, "status": "addressed"}),
        )
        .await;
    assert_eq!(addressed["structuredContent"]["total"], 1);

    let (status, html) = app
        .page(&format!("/admin/posts/{post_id}/annotate"), Some(&admin))
        .await;
    assert_eq!(status, 200);
    assert!(html.contains("第二段需要改进。"));
    assert!(html.contains("annotations.js"));
}

#[tokio::test]
async fn revision_diff_shows_the_edit() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let post_id = create_draft(&app).await;
    app.tool(
        "content_update",
        json!({"id": post_id, "body": EDITED_BODY}),
    )
    .await;
    let revisions = app
        .tool("revision_list", json!({"content_id": post_id, "limit": 5}))
        .await;
    let revision_id = revisions["structuredContent"]["items"][0]["id"]
        .as_i64()
        .expect("revision id");

    let (status, html) = app
        .page(
            &format!("/admin/posts/{post_id}/revisions/{revision_id}"),
            Some(&admin),
        )
        .await;

    assert_eq!(status, 200);
    assert!(html.contains("diff-add"));
    assert!(html.contains("第二段已经润色。"));
}

/// A draft (already edited once) with one proposal; returns (post, proposal).
async fn draft_with_proposal(app: &TestApp) -> (i64, i64) {
    let post_id = create_draft(app).await;
    app.tool(
        "content_update",
        json!({"id": post_id, "body": EDITED_BODY}),
    )
    .await;
    let proposal = app
        .tool(
            "proposal_create",
            json!({
                "content_id": post_id,
                "title": "批注测试（版本）",
                "body": "第一段。\n\n第二段是新的版本。\n\n第三段。",
                "summary": "压缩第二段"
            }),
        )
        .await;
    assert_eq!(proposal["isError"], Value::Null, "{proposal}");
    let proposal_id = proposal["structuredContent"]["id"]
        .as_i64()
        .expect("proposal id");
    (post_id, proposal_id)
}

async fn proposal_page(app: &TestApp, admin: &Session, post_id: i64, proposal_id: i64) -> String {
    let (status, html) = app
        .page(
            &format!("/admin/posts/{post_id}/proposals/{proposal_id}"),
            Some(admin),
        )
        .await;
    assert_eq!(status, 200);
    html
}

#[tokio::test]
async fn proposals_attach_to_the_same_content_with_immutable_versions() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let (post_id, proposal_id) = draft_with_proposal(&app).await;

    let listed = app
        .tool(
            "proposal_list",
            json!({"content_id": post_id, "status": "proposed"}),
        )
        .await;
    assert_eq!(listed["structuredContent"]["total"], 1);

    let html = proposal_page(&app, &admin, post_id, proposal_id).await;
    assert!(html.contains("diff-add"));
    let reviewed: i64 = hidden_value(&html, "version").parse().expect("version");

    let changed = app
        .tool(
            "proposal_update",
            json!({
                "id": proposal_id,
                "version": reviewed,
                "body": "第一段。\n\n第二段是后来修改的版本。\n\n第三段。",
                "summary": "后来修改"
            }),
        )
        .await;
    assert_eq!(changed["isError"], Value::Null, "{changed}");
    let latest = changed["structuredContent"]["version"]
        .as_i64()
        .expect("version");
    assert_eq!(latest, reviewed + 1);

    let snapshots: Vec<(i64, String)> = sqlx::query_as(
        "SELECT version, body FROM content_proposal_versions \
         WHERE proposal_id = ? ORDER BY version",
    )
    .bind(proposal_id)
    .fetch_all(&app.pool)
    .await
    .expect("versions");
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].0, reviewed);
    assert!(snapshots[0].1.contains("第二段是新的版本。"));
    assert_eq!(snapshots[1].0, latest);
    assert!(snapshots[1].1.contains("第二段是后来修改的版本。"));
}

#[tokio::test]
async fn browser_cannot_apply_a_proposal_version_it_did_not_render() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let (post_id, proposal_id) = draft_with_proposal(&app).await;
    let html = proposal_page(&app, &admin, post_id, proposal_id).await;
    let reviewed = hidden_value(&html, "version");

    // The AI updates the proposal after the author opened it.
    app.tool(
        "proposal_update",
        json!({
            "id": proposal_id,
            "version": reviewed.parse::<i64>().expect("version"),
            "body": "第一段。\n\n第二段是后来修改的版本。\n\n第三段。"
        }),
    )
    .await;

    let response = app
        .post_form(
            &format!("/admin/posts/{post_id}/proposals/{proposal_id}/apply"),
            Some(&admin),
            &[
                ("_csrf", &hidden_value(&html, "_csrf")),
                ("version", &reviewed),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let expected = format!("/admin/posts/{post_id}/proposals/{proposal_id}?error=proposal_changed");
    assert_eq!(response.headers()[header::LOCATION], expected.as_str());

    let (status, latest_html) = app.page(&expected, Some(&admin)).await;
    assert_eq!(status, 200);
    assert!(latest_html.contains("提案或文章在预览后发生了变化"));
    assert!(latest_html.contains("第二段是后来修改的版本。"));
}

#[tokio::test]
async fn applying_after_a_concurrent_edit_needs_force_and_records_a_revision() {
    let app = TestApp::new().await;
    let (post_id, proposal_id) = draft_with_proposal(&app).await;
    let version =
        app.tool("proposal_get", json!({"id": proposal_id})).await["structuredContent"]["version"]
            .as_i64()
            .expect("version");
    let live = content::get_by_id(&app.pool, post_id)
        .await
        .expect("get")
        .expect("post");

    let edit = app
        .tool(
            "content_update",
            json!({
                "id": post_id,
                "body": "第一段。\n\n作者在审核期间又修改了正文。\n\n第三段。",
                "expected_updated_at": live.updated_at
            }),
        )
        .await;
    assert_eq!(edit["isError"], Value::Null);

    let conflict = app
        .tool(
            "proposal_apply",
            json!({"id": proposal_id, "version": version}),
        )
        .await;
    assert_eq!(conflict["isError"], true);

    let applied = app
        .tool(
            "proposal_apply",
            json!({"id": proposal_id, "version": version, "force": true}),
        )
        .await;
    assert_eq!(applied["isError"], Value::Null, "{applied}");
    assert_eq!(
        applied["structuredContent"]["proposal"]["status"],
        "applied"
    );
    let body = app.tool("content_get", json!({"id": post_id})).await;
    assert!(
        body["structuredContent"]["content"]["body"]
            .as_str()
            .expect("body")
            .contains("第二段是新的版本。")
    );
    let latest_revision: i64 = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE content_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(post_id)
    .fetch_one(&app.pool)
    .await
    .expect("revision");
    assert_eq!(
        applied["structuredContent"]["proposal"]["applied_revision_id"],
        latest_revision
    );
}

#[tokio::test]
async fn rejected_proposals_are_marked_rejected() {
    let app = TestApp::new().await;
    let (_, proposal_id) = draft_with_proposal(&app).await;

    let rejected = app
        .tool("proposal_reject", json!({"id": proposal_id}))
        .await;

    assert_eq!(rejected["structuredContent"]["status"], "rejected");
}

#[tokio::test]
async fn stale_admin_save_shows_a_conflict_instead_of_overwriting() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let created = app
        .tool(
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
    let post_id = created["structuredContent"]["id"].as_i64().expect("id");
    let (_, edit_html) = app
        .page(&format!("/admin/posts/{post_id}/edit"), Some(&admin))
        .await;

    content::update(
        &app.pool,
        post_id,
        ContentPatch {
            body: Some("服务器上的较新正文。".to_string()),
            actor: "test".to_string(),
            ..Default::default()
        },
    )
    .await
    .expect("newer save");

    let response = app
        .post_form(
            &format!("/admin/posts/{post_id}"),
            Some(&admin),
            &[
                ("_csrf", &hidden_value(&edit_html, "_csrf")),
                (
                    "expected_updated_at",
                    &hidden_value(&edit_html, "expected_updated_at"),
                ),
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
            ],
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains("内容已被其他人修改"));
    assert!(
        html.contains("旧页面提交的正文。"),
        "the author's text is kept in the form"
    );
    let persisted = content::get_by_id(&app.pool, post_id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(persisted.body, "服务器上的较新正文。");
}

#[tokio::test]
async fn proposal_create_rejects_stale_base() {
    let app = TestApp::new().await;
    let router = app.router.clone();

    let created = mcp_tool(
        &router,
        "content_create",
        json!({"kind": "post", "title": "冲突测试", "body": "原文。", "status": "draft"}),
    )
    .await;
    assert_eq!(created["isError"], Value::Null);
    let post_id = created["structuredContent"]["id"]
        .as_i64()
        .expect("post id");
    let created_updated_at = created["structuredContent"]["updated_at"]
        .as_i64()
        .expect("updated_at");

    let read_revisions = mcp_tool(&router, "revision_list", json!({"content_id": post_id})).await;
    let read_revision_id = read_revisions["structuredContent"]["items"][0]["id"]
        .as_i64()
        .expect("revision id");

    // The author edits the post after the AI read it.
    let edited = mcp_tool(
        &router,
        "content_update",
        json!({"id": post_id, "body": "作者改过的原文。"}),
    )
    .await;
    assert_eq!(edited["isError"], Value::Null);

    let stale_revision = mcp_tool(
        &router,
        "proposal_create",
        json!({"content_id": post_id, "body": "AI 版本。", "base_revision_id": read_revision_id}),
    )
    .await;
    assert_eq!(stale_revision["isError"], true, "{stale_revision}");

    let stale_timestamp = mcp_tool(
        &router,
        "proposal_create",
        json!({
            "content_id": post_id,
            "body": "AI 版本。",
            "expected_updated_at": created_updated_at - 1
        }),
    )
    .await;
    assert_eq!(stale_timestamp["isError"], true, "{stale_timestamp}");

    let latest = mcp_tool(&router, "revision_list", json!({"content_id": post_id})).await;
    let latest_revision_id = latest["structuredContent"]["items"][0]["id"]
        .as_i64()
        .expect("latest revision id");
    let current = mcp_tool(&router, "content_get", json!({"id": post_id})).await;
    let current_updated_at = current["structuredContent"]["content"]["updated_at"]
        .as_i64()
        .expect("current updated_at");
    let fresh = mcp_tool(
        &router,
        "proposal_create",
        json!({
            "content_id": post_id,
            "body": "AI 版本。",
            "base_revision_id": latest_revision_id,
            "expected_updated_at": current_updated_at
        }),
    )
    .await;
    assert_eq!(fresh["isError"], Value::Null, "{fresh}");
}
