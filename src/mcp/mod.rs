pub mod protocol;
pub mod tools;

use anyhow::Context;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{ALLOW, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::repo::token::{self, McpPrincipal};
use crate::state::AppState;
use crate::util::constant_time_eq;

pub async fn http(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return unauthorized("缺少 Bearer token");
    };

    let principal = match authenticate(&state, &token).await {
        Ok(Some(principal)) => principal,
        Ok(None) => return unauthorized("Token 无效或已吊销"),
        Err(error) => {
            tracing::error!(error = ?error, "MCP authentication failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "authentication failed"})),
            )
                .into_response();
        }
    };

    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("JSON 解析失败: {error}")}
                })),
            )
                .into_response();
        }
    };

    match protocol::handle_value(&state, &principal, value).await {
        Some(response) => ([(CONTENT_TYPE, "application/json")], Json(response)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

pub async fn http_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(ALLOW, "POST")],
        Json(json!({"error": "MCP endpoint accepts POST requests"})),
    )
        .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn authenticate(state: &AppState, token: &str) -> anyhow::Result<Option<McpPrincipal>> {
    if let Some(env_token) = &state.config.mcp_token
        && constant_time_eq(env_token, token)
    {
        return Ok(Some(McpPrincipal {
            name: "env".to_string(),
            scopes: state.config.mcp_token_scopes.clone(),
        }));
    }

    Ok(token::verify(&state.db, token).await?)
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(WWW_AUTHENTICATE, "Bearer realm=\"inkwell\"")],
        Json(json!({"error": message})),
    )
        .into_response()
}

pub async fn run_stdio(state: AppState) -> anyhow::Result<()> {
    serve_lines(&state, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serves newline-delimited JSON-RPC from `input`, writing one response line
/// per request to `output`. Local (stdio) callers get full scopes.
pub async fn serve_lines<R, W>(state: &AppState, input: R, mut output: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(input).lines();
    let principal = McpPrincipal::local();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(line) {
            Ok(value) => protocol::handle_value(state, &principal, value).await,
            Err(error) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("JSON 解析失败: {error}")}
            })),
        };
        if let Some(response) = response {
            write_response(&mut output, &response).await?;
        }
    }

    Ok(())
}

async fn write_response<W: AsyncWrite + Unpin>(
    output: &mut W,
    response: &Value,
) -> anyhow::Result<()> {
    let text = serde_json::to_string(response).context("serialize MCP response")?;
    output.write_all(text.as_bytes()).await?;
    output.write_all(b"\n").await?;
    output.flush().await?;
    Ok(())
}
