use axum::Json;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::util::html_escape;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("redirect to {0}")]
    Redirect(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(anyhow::anyhow!(message.into()))
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Redirect(_) => StatusCode::SEE_OTHER,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Internal(error) => {
                tracing::error!(error = ?error, "internal error");
                "服务器内部错误，请稍后再试。".to_string()
            }
            other => other.to_string(),
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => Self::NotFound("记录不存在".to_string()),
            other => Self::Internal(other.into()),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(error.into())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::BadRequest(format!("JSON 格式错误: {error}"))
    }
}

impl From<askama::Error> for AppError {
    fn from(error: askama::Error) -> Self {
        Self::Internal(error.into())
    }
}

impl From<argon2::password_hash::Error> for AppError {
    fn from(error: argon2::password_hash::Error) -> Self {
        Self::Internal(anyhow::anyhow!("password hash error: {error}"))
    }
}

impl From<axum::extract::multipart::MultipartError> for AppError {
    fn from(error: axum::extract::multipart::MultipartError) -> Self {
        Self::BadRequest(format!("上传失败: {error}"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let Self::Redirect(location) = self {
            return (
                StatusCode::SEE_OTHER,
                [(axum::http::header::LOCATION, location)],
            )
                .into_response();
        }

        let status = self.status();
        let message = self.message();

        if status == StatusCode::UNAUTHORIZED {
            return (status, Json(serde_json::json!({ "error": message }))).into_response();
        }

        let title = match status {
            StatusCode::NOT_FOUND => "页面不存在",
            StatusCode::BAD_REQUEST => "请求有误",
            StatusCode::FORBIDDEN => "没有权限",
            StatusCode::CONFLICT => "内容冲突",
            _ => "出错了",
        };

        let body = format!(
            r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#f6f5f2;color:#25231f;font-family:ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:34rem;padding:2rem}}
h1{{font-size:1.6rem;margin:0 0 .75rem}}
p{{line-height:1.8;color:#5c574f}}
a{{color:#2f6f5e}}
</style>
</head>
<body><main><h1>{title}</h1><p>{message}</p><p><a href="/">返回首页</a></p></main></body>
</html>"#,
            title = html_escape(title),
            message = html_escape(&message),
        );

        (status, Html(body)).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
