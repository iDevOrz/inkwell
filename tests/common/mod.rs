//! Shared helpers for integration tests. Each test binary uses a different
//! subset, so unused helpers are expected.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Method, Request, Response, header};
use http_body_util::BodyExt;
use inkwell::{app, config::Config};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tempfile::TempDir;
use tower::ServiceExt;

pub const ENV_TOKEN: &str = "testtoken";

pub fn test_config(data_dir: &Path) -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        database_url: "sqlite::memory:".to_string(),
        data_dir: data_dir.to_path_buf(),
        site_url: "http://test.local".to_string(),
        admin_user: "admin".to_string(),
        admin_password: Some("testpass123".to_string()),
        mcp_token: Some(ENV_TOKEN.to_string()),
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

pub async fn body_string(response: Response<Body>) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn cookie_from(headers: &header::HeaderMap, name: &str) -> Option<String> {
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

pub fn hidden_value(html: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\" value=\"");
    let start = html.find(&needle).expect("hidden input") + needle.len();
    let end = html[start..].find('"').expect("hidden input end") + start;
    html[start..end].to_string()
}

pub fn encode(values: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in values {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

pub async fn mcp_call(router: &Router, payload: Value) -> Value {
    mcp_call_with_token(router, payload, ENV_TOKEN).await
}

pub async fn mcp_call_with_token(router: &Router, payload: Value, token: &str) -> Value {
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

pub async fn mcp_tool(router: &Router, name: &str, arguments: Value) -> Value {
    mcp_tool_with_token(router, name, arguments, ENV_TOKEN).await
}

pub async fn mcp_tool_with_token(
    router: &Router,
    name: &str,
    arguments: Value,
    token: &str,
) -> Value {
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

/// Text of a tool result's first content block (error messages live here).
pub fn tool_text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap_or_default()
}

/// A signed-in browser session created directly in the database, so tests
/// can act as any role without going through the login form.
#[derive(Debug, Clone)]
pub struct Session {
    pub cookie: String,
    pub csrf: String,
}

pub struct TestApp {
    pub router: Router,
    pub pool: SqlitePool,
    _data_dir: TempDir,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::build(|_| {}).await
    }

    /// Uses a WAL-mode database file with a real connection pool instead of
    /// the single-connection in-memory database, so concurrent requests
    /// actually overlap as they do in production.
    pub async fn with_file_database() -> Self {
        Self::build(|config| {
            config.database_url = format!("sqlite://{}/inkwell.db", config.data_dir.display());
        })
        .await
    }

    async fn build(customize: impl FnOnce(&mut Config)) -> Self {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let mut config = test_config(data_dir.path());
        customize(&mut config);
        let state = app::build_state(config).await.expect("build state");
        let pool = state.db.clone();
        let router =
            app::router(state).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
        Self {
            router,
            pool,
            _data_dir: data_dir,
        }
    }

    /// Creates a user with `role` and returns a live session for it.
    pub async fn session_for(&self, role: &str) -> Session {
        let username = format!("{role}-{}", inkwell::util::random_token(4));
        let hash = inkwell::auth::hash_password("unused-test-password").expect("hash");
        let user_id = inkwell::repo::user::create(&self.pool, &username, &hash, "", None, role)
            .await
            .expect("create user");
        let (token, csrf) = inkwell::auth::create_session(&self.pool, user_id, 1, None, None)
            .await
            .expect("create session");
        Session {
            cookie: format!("{}={token}", inkwell::auth::SESSION_COOKIE),
            csrf,
        }
    }

    pub fn data_dir(&self) -> &Path {
        self._data_dir.path()
    }

    /// GET a page and return its status and body text.
    pub async fn page(&self, uri: &str, session: Option<&Session>) -> (u16, String) {
        let response = self.get(uri, session).await;
        let status = response.status().as_u16();
        (status, body_string(response).await)
    }

    pub async fn token_with(&self, scopes: &[&str]) -> String {
        let scopes: Vec<String> = scopes.iter().map(|scope| scope.to_string()).collect();
        let (_, token) = inkwell::repo::token::create(&self.pool, "test", &scopes)
            .await
            .expect("create token");
        token
    }

    pub async fn get(&self, uri: &str, session: Option<&Session>) -> Response<Body> {
        let mut request = Request::builder().uri(uri);
        if let Some(session) = session {
            request = request.header(header::COOKIE, &session.cookie);
        }
        self.send(request.body(Body::empty()).expect("request"))
            .await
    }

    pub async fn post_form(
        &self,
        uri: &str,
        session: Option<&Session>,
        fields: &[(&str, &str)],
    ) -> Response<Body> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(session) = session {
            request = request.header(header::COOKIE, &session.cookie);
        }
        self.send(request.body(Body::from(encode(fields))).expect("request"))
            .await
    }

    /// POST a multipart form. Each part is (field name, optional file name, bytes).
    pub async fn post_multipart(
        &self,
        uri: &str,
        session: Option<&Session>,
        extra_headers: &[(&str, &str)],
        parts: &[(&str, Option<&str>, &[u8])],
    ) -> Response<Body> {
        const BOUNDARY: &str = "inkwell-test-boundary";
        let mut body = Vec::new();
        for (name, file_name, bytes) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            let disposition = match file_name {
                Some(file) => format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: application/octet-stream\r\n"
                ),
                None => format!("Content-Disposition: form-data; name=\"{name}\"\r\n"),
            };
            body.extend_from_slice(disposition.as_bytes());
            body.extend_from_slice(b"\r\n");
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());

        let mut request = Request::builder().method(Method::POST).uri(uri).header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        );
        if let Some(session) = session {
            request = request.header(header::COOKIE, &session.cookie);
        }
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }
        self.send(request.body(Body::from(body)).expect("request"))
            .await
    }

    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("response")
    }

    pub async fn mcp(&self, payload: Value) -> Value {
        mcp_call(&self.router, payload).await
    }

    pub async fn tool(&self, name: &str, arguments: Value) -> Value {
        mcp_tool(&self.router, name, arguments).await
    }

    pub async fn tool_as(&self, token: &str, name: &str, arguments: Value) -> Value {
        mcp_tool_with_token(&self.router, name, arguments, token).await
    }
}
