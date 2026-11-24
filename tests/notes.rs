//! MCP handshake, instance identity, and the note workflow: create, edit,
//! promote to a post, sync into a published post, and submit for review.
mod common;

use common::TestApp;
use inkwell::repo::{content, revision};
use serde_json::{Value, json};

const NOTE_BODY: &str = "这是一篇测试笔记。参见 [[另一篇]]。";

async fn create_note(app: &TestApp) -> i64 {
    let note = app
        .tool(
            "note_create",
            json!({"title": "测试笔记", "body": NOTE_BODY, "tags": ["test", "笔记"]}),
        )
        .await;
    assert_eq!(note["isError"], Value::Null, "{note}");
    note["structuredContent"]["id"].as_i64().expect("note id")
}

async fn setting(app: &TestApp, name: &'static str) -> String {
    sqlx::query_scalar("SELECT value FROM settings WHERE name = ?")
        .bind(name)
        .fetch_one(&app.pool)
        .await
        .expect(name)
}

#[tokio::test]
async fn initialize_and_instance_info_identify_this_server() {
    let app = TestApp::new().await;

    let init = app
        .mcp(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": "2024-11-05"}
        }))
        .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "inkwell");

    let info = app.tool("instance_info", json!({})).await;
    assert_eq!(info["isError"], Value::Null);
    assert_eq!(
        info["structuredContent"]["installation_id"],
        setting(&app, "installation_id").await
    );
    assert_eq!(
        info["structuredContent"]["runtime_id"],
        setting(&app, "runtime_id").await
    );
}

#[tokio::test]
async fn instance_info_confirms_an_import_probe() {
    // The importer writes a probe row directly to the database, then asks the
    // MCP server whether it sees it, proving both point at the same instance.
    let app = TestApp::new().await;
    let probe = "smoke-test-probe";
    sqlx::query("INSERT INTO settings(name, value) VALUES('__inkwell_import_target_probe__', ?)")
        .bind(probe)
        .execute(&app.pool)
        .await
        .expect("insert probe");

    let info = app.tool("instance_info", json!({"probe": probe})).await;
    assert_eq!(info["structuredContent"]["probe_matches"], true);

    let other = app.tool("instance_info", json!({"probe": "other"})).await;
    assert_ne!(other["structuredContent"]["probe_matches"], true);
}

#[tokio::test]
async fn note_editor_offers_promotion() {
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let note_id = create_note(&app).await;

    let (status, html) = app
        .page(&format!("/admin/notes/{note_id}/edit"), Some(&admin))
        .await;

    assert_eq!(status, 200);
    assert!(html.contains("转为文章"));
}

#[tokio::test]
async fn default_token_cannot_publish_or_delete_comments() {
    let app = TestApp::new().await;
    let note_id = create_note(&app).await;

    let publish = app.tool("content_publish", json!({"id": note_id})).await;
    assert_eq!(publish["isError"], true);
    let delete = app
        .tool("comment_moderate", json!({"id": 1, "action": "delete"}))
        .await;
    assert_eq!(delete["isError"], true);
}

#[tokio::test]
async fn promoting_a_note_creates_a_draft_post() {
    let app = TestApp::new().await;
    let note_id = create_note(&app).await;

    let promoted = app
        .tool("note_promote", json!({"id": note_id, "status": "draft"}))
        .await;

    assert_eq!(promoted["isError"], Value::Null, "{promoted}");
    assert_eq!(promoted["structuredContent"]["created"], true);
}

async fn link_note_to_published_post(app: &TestApp, note_id: i64) -> i64 {
    let post_id: i64 = sqlx::query_scalar(
        "SELECT id FROM contents \
         WHERE kind = 'post' AND status = 'published' AND visibility = 'public' \
         ORDER BY id LIMIT 1",
    )
    .fetch_one(&app.pool)
    .await
    .expect("seeded published post");
    sqlx::query("UPDATE contents SET promoted_to_id = ? WHERE id = ?")
        .bind(post_id)
        .bind(note_id)
        .execute(&app.pool)
        .await
        .expect("link note");
    post_id
}

#[tokio::test]
async fn syncing_into_a_published_post_requires_publish_scope() {
    let app = TestApp::new().await;
    let note_id = create_note(&app).await;
    let post_id = link_note_to_published_post(&app, note_id).await;
    let before = content::get_by_id(&app.pool, post_id)
        .await
        .expect("get")
        .expect("post");
    let revisions_before = revision::list(&app.pool, post_id, 100)
        .await
        .expect("revisions");

    let denied = app
        .tool("note_promote", json!({"id": note_id, "sync": true}))
        .await;

    assert_eq!(denied["isError"], true);
    let after = content::get_by_id(&app.pool, post_id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(after.body, before.body);
    assert_eq!(
        revision::list(&app.pool, post_id, 100)
            .await
            .expect("revisions")
            .len(),
        revisions_before.len()
    );
}

#[tokio::test]
async fn publisher_token_syncs_note_into_published_post() {
    let app = TestApp::new().await;
    let note_id = create_note(&app).await;
    let post_id = link_note_to_published_post(&app, note_id).await;
    let publisher = app.token_with(&["read", "write", "publish"]).await;

    let synced = app
        .tool_as(
            &publisher,
            "note_promote",
            json!({"id": note_id, "sync": true}),
        )
        .await;

    assert_eq!(synced["isError"], Value::Null, "{synced}");
    assert_eq!(synced["structuredContent"]["created"], false);
    let post = content::get_by_id(&app.pool, post_id)
        .await
        .expect("get")
        .expect("post");
    assert_eq!(post.body, NOTE_BODY);
}

#[tokio::test]
async fn submitted_content_appears_in_the_review_queue() {
    let app = TestApp::new().await;
    let note_id = create_note(&app).await;

    let submit = app
        .tool(
            "review_submit",
            json!({"id": note_id, "notes": "请检查结构"}),
        )
        .await;
    assert_eq!(submit["structuredContent"]["review_status"], "pending");

    let queue = app.tool("review_queue", json!({})).await;
    assert!(
        queue["structuredContent"]["content"]
            .as_array()
            .expect("queue")
            .iter()
            .any(|item| item["id"] == note_id)
    );
}
