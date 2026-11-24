use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::token::McpPrincipal;
use crate::state::AppState;

use super::tools;

pub const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
/// Requests run sequentially, so cap a batch to keep one HTTP call bounded.
pub const MAX_BATCH_REQUESTS: usize = 50;
const INVALID_REQUEST: i64 = -32600;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    pub jsonrpc: Option<String>,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub async fn handle_value(
    state: &AppState,
    principal: &McpPrincipal,
    value: Value,
) -> Option<Value> {
    match value {
        Value::Array(items) if items.is_empty() => Some(error_response(
            Value::Null,
            RpcError::new(INVALID_REQUEST, "批量请求不能为空"),
        )),
        Value::Array(items) if items.len() > MAX_BATCH_REQUESTS => Some(error_response(
            Value::Null,
            RpcError::new(
                INVALID_REQUEST,
                format!("批量请求最多 {MAX_BATCH_REQUESTS} 条"),
            ),
        )),
        Value::Array(items) => {
            let mut responses = Vec::new();
            for item in items {
                if let Some(response) = handle_request(state, principal, item).await {
                    responses.push(response);
                }
            }
            if responses.is_empty() {
                None
            } else {
                Some(Value::Array(responses))
            }
        }
        other => handle_request(state, principal, other).await,
    }
}

async fn handle_request(state: &AppState, principal: &McpPrincipal, value: Value) -> Option<Value> {
    // A notification is a request *without* an id member; `"id": null` is a
    // request that still expects a response, so check the raw object.
    let raw_id = value
        .as_object()
        .and_then(|object| object.get("id"))
        .cloned();
    let is_notification = raw_id.is_none();
    // The body already parsed as JSON, so a shape error is an invalid request.
    let request: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(error) => {
            return Some(error_response(
                raw_id.unwrap_or(Value::Null),
                RpcError::new(INVALID_REQUEST, format!("无效的 JSON-RPC 请求: {error}")),
            ));
        }
    };

    let id = raw_id;
    let params = request.params.unwrap_or_else(|| json!({}));

    let result = match request.method.as_str() {
        "initialize" => Ok(Some(initialize_result(&params))),
        "notifications/initialized" => Ok(None),
        "ping" => Ok(Some(json!({}))),
        "tools/list" => Ok(Some(json!({ "tools": tools::definitions() }))),
        "tools/call" => handle_tools_call(state, principal, &params).await.map(Some),
        "resources/list" => require_read(principal).map(|_| {
            Some(json!({
                "resources": [
                    {
                        "uri": "typecho://review/queue",
                        "name": "Review queue",
                        "description": "Content waiting for review and comments waiting for moderation",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "typecho://notes/recent",
                        "name": "Recent notes",
                        "description": "The most recently updated private notes",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "typecho://annotations/queue",
                        "name": "Open annotations",
                        "description": "Inline and general annotations waiting for AI edits",
                        "mimeType": "application/json"
                    }
                ]
            }))
        }),
        "resources/templates/list" => require_read(principal).map(|_| {
            Some(json!({
                "resourceTemplates": [
                    {
                        "uriTemplate": "typecho://content/{id}",
                        "name": "Content by id",
                        "description": "One post, page or note with body, metadata and revisions",
                        "mimeType": "application/json"
                    }
                ]
            }))
        }),
        "resources/read" => match require_read(principal) {
            Ok(()) => handle_resources_read(state, &params).await.map(Some),
            Err(error) => Err(error),
        },
        method if method.starts_with("notifications/") => Ok(None),
        method => Err(RpcError::new(
            -32601,
            format!("未知方法: {method}"),
        )),
    };

    match result {
        Ok(None) => None,
        Ok(Some(result)) => {
            if is_notification {
                None
            } else {
                Some(success_response(id.unwrap_or(Value::Null), result))
            }
        }
        Err(error) => {
            if is_notification {
                None
            } else {
                Some(error_response(id.unwrap_or(Value::Null), error))
            }
        }
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        DEFAULT_PROTOCOL_VERSION
    };

    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false }
        },
        "serverInfo": {
            "name": "inkwell",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Inkwell blog and notes server. AI can read and write drafts/notes, submit content for review, and moderate comments. For collaborative editing, use annotation_queue to pull open annotations, read the content with content_get, create a proposed version with proposal_create (do not create a separate draft article), then reply to each annotation with annotation_reply and the proposal id. The human reviews the proposal diff and applies or rejects the exact version shown. If proposal_get or proposal_update returns version N, pass N to proposal_apply or proposal_update; stale versions are rejected. Publishing requires an explicit publish-scoped token."
    })
}

fn require_read(principal: &McpPrincipal) -> Result<(), RpcError> {
    if principal.has("read") {
        Ok(())
    } else {
        Err(RpcError::new(-32001, "当前 token 缺少 read 权限"))
    }
}

async fn handle_tools_call(
    state: &AppState,
    principal: &McpPrincipal,
    params: &Value,
) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(-32602, "tools/call 缺少 name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = tools::call(state, principal, name, arguments).await;
    serde_json::to_value(result)
        .map_err(|error| RpcError::new(-32603, format!("序列化工具结果失败: {error}")))
}

async fn handle_resources_read(state: &AppState, params: &Value) -> Result<Value, RpcError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(-32602, "resources/read 缺少 uri"))?;

    let value = tools::resource_value(state, uri)
        .await
        .map_err(|error| RpcError::new(-32602, error.message()))?;
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| RpcError::new(-32603, format!("序列化资源失败: {error}")))?;

    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": text
        }]
    }))
}

fn success_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn error_response(id: Value, error: RpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": error.code,
            "message": error.message
        }
    })
}
