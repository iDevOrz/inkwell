//! JSON-RPC protocol edge cases and scope rules that span several tools.
mod common;

use common::{TestApp, mcp_call, mcp_tool, mcp_tool_with_token};
use serde_json::{Value, json};

#[tokio::test]
async fn mcp_protocol_and_scope_edges() {
    let app = TestApp::new().await;
    let pool = app.pool.clone();
    let router = app.router.clone();

    // JSON-RPC: empty and oversized batches are invalid requests.
    let empty = mcp_call(&router, json!([])).await;
    assert_eq!(empty["error"]["code"], -32600, "{empty}");
    let oversized: Vec<Value> = (0..=inkwell::mcp::protocol::MAX_BATCH_REQUESTS)
        .map(|index| json!({"jsonrpc": "2.0", "id": index, "method": "ping"}))
        .collect();
    let oversized = mcp_call(&router, Value::Array(oversized)).await;
    assert_eq!(oversized["error"]["code"], -32600, "{oversized}");

    // `"id": null` is a request, not a notification, so it gets a response.
    let null_id = mcp_call(
        &router,
        json!({"jsonrpc": "2.0", "id": null, "method": "ping"}),
    )
    .await;
    assert!(null_id.get("result").is_some(), "{null_id}");
    assert_eq!(null_id["id"], Value::Null);

    // A well-formed JSON value with the wrong shape is -32600 and keeps its id.
    let no_method = mcp_call(&router, json!({"jsonrpc": "2.0", "id": 7})).await;
    assert_eq!(no_method["error"]["code"], -32600, "{no_method}");
    assert_eq!(no_method["id"], 7);

    let templates = mcp_call(
        &router,
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/templates/list"}),
    )
    .await;
    assert_eq!(
        templates["result"]["resourceTemplates"][0]["uriTemplate"],
        "inkwell://content/{id}"
    );

    // Resolving annotations is the author's call: write-only tokens cannot.
    let (_, writer_token) =
        inkwell::repo::token::create(&pool, "writer", &["read".to_string(), "write".to_string()])
            .await
            .expect("writer token");
    let draft = mcp_tool(
        &router,
        "content_create",
        json!({"kind": "post", "title": "范围测试", "body": "正文。", "status": "draft"}),
    )
    .await;
    let post_id = draft["structuredContent"]["id"].as_i64().expect("post id");
    let annotation = mcp_tool(
        &router,
        "annotation_create",
        json!({"content_id": post_id, "kind": "general", "comment": "改一下"}),
    )
    .await;
    let annotation_id = annotation["structuredContent"]["id"]
        .as_i64()
        .expect("annotation id");
    let resolve = mcp_tool_with_token(
        &router,
        "annotation_resolve",
        json!({"id": annotation_id}),
        &writer_token,
    )
    .await;
    assert_eq!(resolve["isError"], true, "{resolve}");
    let reply_resolved = mcp_tool_with_token(
        &router,
        "annotation_reply",
        json!({"id": annotation_id, "body": "done", "status": "resolved"}),
        &writer_token,
    )
    .await;
    assert_eq!(reply_resolved["isError"], true, "{reply_resolved}");

    // A reply without a status must not reopen an annotation the author resolved.
    let resolved = mcp_tool(&router, "annotation_resolve", json!({"id": annotation_id})).await;
    assert_eq!(resolved["isError"], Value::Null, "{resolved}");
    let reply = mcp_tool_with_token(
        &router,
        "annotation_reply",
        json!({"id": annotation_id, "body": "补充说明"}),
        &writer_token,
    )
    .await;
    assert_eq!(reply["isError"], Value::Null, "{reply}");
    assert_eq!(
        reply["structuredContent"]["annotation"]["status"], "resolved",
        "{reply}"
    );

    // Publishing with a non-approved verdict is rejected and changes nothing.
    let decide = mcp_tool(
        &router,
        "review_decide",
        json!({"id": post_id, "verdict": "changes_requested", "publish": true}),
    )
    .await;
    assert_eq!(decide["isError"], true, "{decide}");

    // Moderating a comment that does not exist reports an error.
    let moderate = mcp_tool(
        &router,
        "comment_moderate",
        json!({"id": 999_999, "action": "approve"}),
    )
    .await;
    assert_eq!(moderate["isError"], true, "{moderate}");

    // Internal error details never reach MCP clients.
    for text in [&resolve, &reply_resolved, &decide, &moderate]
        .iter()
        .filter_map(|value| value["content"][0]["text"].as_str())
    {
        assert!(
            !text.contains("bad request:") && !text.contains("SQLITE"),
            "{text}"
        );
    }
}
