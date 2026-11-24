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

// ------------------------------------------------------------- HTTP transport

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};

async fn raw_post(app: &TestApp, authorization: Option<&str>, body: &str) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    let response = app
        .send(request.body(Body::from(body.to_string())).expect("request"))
        .await;
    let status = response.status();
    (status, common::body_string(response).await)
}

const PING: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

#[tokio::test]
async fn http_requires_a_valid_unrevoked_bearer_token() {
    let app = TestApp::new().await;

    let missing = app
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .body(Body::from(PING))
                .expect("request"),
        )
        .await;
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        missing.headers()[header::WWW_AUTHENTICATE],
        "Bearer realm=\"inkwell\""
    );

    for authorization in ["Bearer ", "Basic dGVzdA==", "Bearer not-a-token"] {
        let (status, _) = raw_post(&app, Some(authorization), PING).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization}");
    }

    let (id, token) = inkwell::repo::token::create(&app.pool, "client", &["read".to_string()])
        .await
        .expect("token");
    let (status, _) = raw_post(&app, Some(&format!("Bearer {token}")), PING).await;
    assert_eq!(status, StatusCode::OK);
    inkwell::repo::token::revoke(&app.pool, id)
        .await
        .expect("revoke");
    let (status, _) = raw_post(&app, Some(&format!("Bearer {token}")), PING).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn http_reports_parse_errors_notifications_and_wrong_methods() {
    let app = TestApp::new().await;
    let auth = Some("Bearer testtoken");

    let (status, body) = raw_post(&app, auth, "{not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("-32700"));

    let (status, body) = raw_post(
        &app,
        auth,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_empty());

    let get = app.get("/mcp", None).await;
    assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(get.headers()[header::ALLOW], "POST");
}

#[tokio::test]
async fn protocol_errors_use_json_rpc_codes() {
    let app = TestApp::new().await;
    let call = |payload: Value| {
        let router = app.router.clone();
        async move { mcp_call(&router, payload).await }
    };

    let unknown = call(json!({"jsonrpc": "2.0", "id": 1, "method": "nope"})).await;
    assert_eq!(unknown["error"]["code"], -32601);

    let no_name =
        call(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {}})).await;
    assert_eq!(no_name["error"]["code"], -32602);

    let no_uri =
        call(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {}})).await;
    assert_eq!(no_uri["error"]["code"], -32602);

    let bad_uri = call(json!({
        "jsonrpc": "2.0", "id": 4, "method": "resources/read",
        "params": {"uri": "inkwell://content/not-a-number"}
    }))
    .await;
    assert_eq!(bad_uri["error"]["code"], -32602);

    for method in ["ping", "tools/list"] {
        let ok = call(json!({"jsonrpc": "2.0", "id": 5, "method": method})).await;
        assert!(ok.get("result").is_some(), "{method}: {ok}");
    }

    let init = call(json!({
        "jsonrpc": "2.0", "id": 6, "method": "initialize",
        "params": {"protocolVersion": "1999-01-01"}
    }))
    .await;
    assert_eq!(
        init["result"]["protocolVersion"],
        inkwell::mcp::protocol::DEFAULT_PROTOCOL_VERSION,
        "unknown versions fall back to the default"
    );
}

#[tokio::test]
async fn read_scope_is_required_for_resources() {
    let app = TestApp::new().await;
    let reviewer = app.token_with(&["review"]).await;
    for method in ["resources/list", "resources/templates/list"] {
        let response = common::mcp_call_with_token(
            &app.router,
            json!({"jsonrpc": "2.0", "id": 1, "method": method}),
            &reviewer,
        )
        .await;
        assert_eq!(response["error"]["code"], -32001, "{method}");
    }
}

// ------------------------------------------------------------ stdio transport

#[tokio::test]
async fn stdio_answers_each_line_and_skips_notifications() {
    let app = TestApp::new().await;
    // A second in-memory instance; stdio callers use the state directly.
    let state = inkwell::app::build_state(common::test_config(app.data_dir()))
        .await
        .expect("state");
    let input = [
        PING,
        "",
        "{broken",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"content_publish","arguments":{"id":1}}}"#,
    ]
    .join("\n");
    let mut output = Vec::new();

    inkwell::mcp::serve_lines(&state, input.as_bytes(), &mut output)
        .await
        .expect("serve");

    let responses: Vec<Value> = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect();
    assert_eq!(
        responses.len(),
        3,
        "ping, parse error, tool call; no reply to notifications"
    );
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[1]["error"]["code"], -32700);
    // stdio is local and fully trusted, so even publishing is allowed.
    assert_eq!(
        responses[2]["result"]["isError"],
        Value::Null,
        "{}",
        responses[2]
    );
}
