//! Sweep over every MCP tool: each one is classified by the scope it needs,
//! read tools succeed with valid arguments, and mutating tools refuse a
//! read-only token. Adding a tool without classifying it fails the test.
mod common;

use std::collections::BTreeSet;

use common::{TestApp, tool_text};
use inkwell::repo::comment::{self, NewComment};
use serde_json::{Value, json};

const INTERNAL_ERROR: &str = "服务器内部错误";
const MISSING_SCOPE: &str = "权限";

/// Ids of one object of each kind, so every tool has something real to act on.
struct Fixture {
    post: i64,
    note: i64,
    annotation: i64,
    proposal: i64,
    revision: i64,
    comment: i64,
}

fn id(result: &Value, path: &[&str]) -> i64 {
    let mut value = &result["structuredContent"];
    for key in path {
        value = &value[*key];
    }
    value
        .as_i64()
        .unwrap_or_else(|| panic!("missing {path:?} in {result}"))
}

async fn fixture(app: &TestApp) -> Fixture {
    let post = app
        .tool(
            "content_create",
            json!({"kind": "post", "title": "工具测试", "body": "正文 [[工具笔记]]"}),
        )
        .await;
    let post = id(&post, &["id"]);
    let note = app
        .tool(
            "note_create",
            json!({"title": "工具笔记", "body": "笔记正文"}),
        )
        .await;
    let note = id(&note, &["id"]);
    let annotation = app
        .tool(
            "annotation_create",
            json!({"content_id": post, "kind": "general", "comment": "改一下"}),
        )
        .await;
    let annotation = id(&annotation, &["id"]);
    let proposal = app
        .tool(
            "proposal_create",
            json!({"content_id": post, "body": "新版本正文"}),
        )
        .await;
    let proposal = id(&proposal, &["id"]);
    let revisions = app.tool("revision_list", json!({"content_id": post})).await;
    let revision = revisions["structuredContent"]["items"][0]["id"]
        .as_i64()
        .expect("revision id");
    let comment = comment::create(
        &app.pool,
        NewComment {
            content_id: post,
            parent_id: 0,
            author: "读者".to_string(),
            email: None,
            url: None,
            body: "评论".to_string(),
            status: "pending".to_string(),
            ip: None,
            user_agent: None,
        },
    )
    .await
    .expect("comment");
    Fixture {
        post,
        note,
        annotation,
        proposal,
        revision,
        comment,
    }
}

/// Tools that only read, with valid arguments.
fn read_calls(f: &Fixture) -> Vec<(&'static str, Value)> {
    vec![
        ("content_list", json!({})),
        ("content_get", json!({"id": f.post})),
        ("content_search", json!({"query": "工具"})),
        ("content_stats", json!({})),
        ("note_list", json!({})),
        ("note_get", json!({"id": f.note})),
        ("review_queue", json!({})),
        ("annotation_queue", json!({})),
        ("annotation_list", json!({"content_id": f.post})),
        ("annotation_get", json!({"id": f.annotation})),
        ("proposal_list", json!({"content_id": f.post})),
        ("proposal_get", json!({"id": f.proposal})),
        ("revision_list", json!({"content_id": f.post})),
        ("revision_get", json!({"id": f.revision})),
        ("comment_list", json!({})),
    ]
}

/// Tools that change state, with arguments valid enough to reach the scope check.
fn mutating_calls(f: &Fixture) -> Vec<(&'static str, Value)> {
    vec![
        ("instance_info", json!({})),
        (
            "content_create",
            json!({"kind": "post", "title": "t", "body": "b"}),
        ),
        ("content_update", json!({"id": f.post, "body": "b"})),
        ("content_delete", json!({"id": f.post})),
        ("content_publish", json!({"id": f.post})),
        ("note_create", json!({"title": "t", "body": "b"})),
        ("note_update", json!({"id": f.note, "body": "b"})),
        ("note_promote", json!({"id": f.note})),
        ("review_submit", json!({"id": f.post})),
        (
            "review_decide",
            json!({"id": f.post, "verdict": "approved"}),
        ),
        (
            "annotation_create",
            json!({"content_id": f.post, "kind": "general", "comment": "c"}),
        ),
        ("annotation_reply", json!({"id": f.annotation, "body": "b"})),
        ("annotation_resolve", json!({"id": f.annotation})),
        (
            "proposal_create",
            json!({"content_id": f.post, "body": "b"}),
        ),
        (
            "proposal_update",
            json!({"id": f.proposal, "version": 1, "body": "b"}),
        ),
        ("proposal_apply", json!({"id": f.proposal, "version": 1})),
        ("proposal_reject", json!({"id": f.proposal})),
        (
            "comment_moderate",
            json!({"id": f.comment, "action": "approve"}),
        ),
    ]
}

async fn listed_tools(app: &TestApp) -> BTreeSet<String> {
    let listed = app
        .mcp(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .await;
    listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("name").to_string())
        .collect()
}

#[tokio::test]
async fn every_listed_tool_is_classified() {
    let app = TestApp::new().await;
    let f = fixture(&app).await;
    let classified: BTreeSet<String> = read_calls(&f)
        .into_iter()
        .chain(mutating_calls(&f))
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(listed_tools(&app).await, classified);
}

#[tokio::test]
async fn read_tools_succeed_with_valid_arguments() {
    let app = TestApp::new().await;
    let f = fixture(&app).await;
    let reader = app.token_with(&["read"]).await;
    for (name, arguments) in read_calls(&f) {
        let result = app.tool_as(&reader, name, arguments).await;
        assert_eq!(result["isError"], Value::Null, "{name}: {result}");
        assert!(result["structuredContent"].is_object(), "{name}: {result}");
    }
}

#[tokio::test]
async fn mutating_tools_refuse_a_read_only_token() {
    let app = TestApp::new().await;
    let f = fixture(&app).await;
    let reader = app.token_with(&["read"]).await;
    for (name, arguments) in mutating_calls(&f) {
        let result = app.tool_as(&reader, name, arguments).await;
        assert_eq!(result["isError"], true, "{name} accepted a read-only token");
        assert!(
            tool_text(&result).contains(MISSING_SCOPE),
            "{name} failed for another reason: {}",
            tool_text(&result)
        );
    }

    // Nothing changed: the post is still an unpublished draft with one proposal.
    let post = app.tool("content_get", json!({"id": f.post})).await;
    assert_eq!(post["structuredContent"]["content"]["status"], "draft");
    let proposal = app.tool("proposal_get", json!({"id": f.proposal})).await;
    assert_eq!(proposal["structuredContent"]["status"], "proposed");
}

#[tokio::test]
async fn tokens_without_read_scope_cannot_read() {
    let app = TestApp::new().await;
    let f = fixture(&app).await;
    let reviewer = app.token_with(&["review"]).await;
    for (name, arguments) in read_calls(&f) {
        let result = app.tool_as(&reviewer, name, arguments).await;
        assert_eq!(
            result["isError"], true,
            "{name} readable without read scope"
        );
    }
}

#[tokio::test]
async fn malformed_arguments_never_cause_internal_errors() {
    let app = TestApp::new().await;
    let names = listed_tools(&app).await;
    let malformed = [
        json!({}),
        json!({"id": "not-a-number", "content_id": -1, "limit": 100_000}),
        json!({"id": 999_999, "content_id": 999_999, "version": 999}),
    ];
    for name in &names {
        for arguments in &malformed {
            let result = app.tool(name, arguments.clone()).await;
            let text = tool_text(&result);
            assert!(!text.contains(INTERNAL_ERROR), "{name} {arguments}: {text}");
            assert!(
                !text.contains("未知工具"),
                "{name} is listed but not dispatched"
            );
        }
    }
}
