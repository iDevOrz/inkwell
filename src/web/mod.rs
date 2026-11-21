pub mod admin;
pub mod public;
pub mod templates;
pub mod views;

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use include_dir::{Dir, include_dir};
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

static VEDITOR_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static/vditor");

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(public::home))
        .route("/timeline/all", get(public::home_all))
        .route("/timeline/more", get(public::timeline_more))
        .route("/archives/{slug}", get(public::post))
        .route("/pages/{slug}", get(public::page))
        .route("/categories/{slug}", get(public::category))
        .route("/tags/{slug}", get(public::tag))
        .route("/archive/{year}/{month}", get(public::archive))
        .route("/search", get(public::search))
        .route("/feed.xml", get(public::feed_rss))
        .route("/atom.xml", get(public::feed_atom))
        .route("/sitemap.xml", get(public::sitemap))
        .route("/robots.txt", get(public::robots))
        .route("/media/{*path}", get(public::media))
        .route("/comments/{id}", post(public::comment))
        .route("/mcp", post(crate::mcp::http).get(crate::mcp::http_get))
        .merge(admin::router())
        .route("/static/vditor/{*path}", get(vditor_asset))
        .route("/static/{*path}", get(static_asset))
        .fallback(public::not_found)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn vditor_asset(Path(path): Path<String>) -> Response {
    let path = path.trim_start_matches('/');
    let Some(file) = VEDITOR_DIR.get_file(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    Response::builder()
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(file.contents().to_vec()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn static_asset(Path(path): Path<String>) -> Response {
    let path = path.trim_start_matches('/');
    let (content_type, body) = match path {
        "theme.css" => (
            "text/css; charset=utf-8",
            include_str!("../../static/theme.css"),
        ),
        "theme-init.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/theme-init.js"),
        ),
        "site.css" => (
            "text/css; charset=utf-8",
            include_str!("../../static/site.css"),
        ),
        "admin.css" => (
            "text/css; charset=utf-8",
            include_str!("../../static/admin.css"),
        ),
        "app.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/app.js"),
        ),
        "mermaid-init.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/mermaid-init.js"),
        ),
        "timeline.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/timeline.js"),
        ),
        "annotations.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/annotations.js"),
        ),
        "vditor-editor.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/vditor-editor.js"),
        ),
        "vendor/mermaid.min.js" => (
            "application/javascript; charset=utf-8",
            include_str!("../../static/vendor/mermaid.min.js"),
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    (
        [
            (CONTENT_TYPE, content_type),
            (CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path();
    let has_session =
        crate::auth::cookie_value(request.headers(), crate::auth::SESSION_COOKIE).is_some();
    let public_asset = path.starts_with("/static/") || path.starts_with("/media/");
    let private_response =
        !public_asset && (path.starts_with("/admin") || path == "/mcp" || has_session);
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; \
             img-src 'self' data: https:; \
             style-src 'self' 'unsafe-inline'; \
             script-src 'self' 'sha256-qR4U4J3Ne5n0m3uNzGMB/tZ3TWJUf89OlxdXqjqALDM='; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self'",
        ),
    );
    if private_response {
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    response
}
