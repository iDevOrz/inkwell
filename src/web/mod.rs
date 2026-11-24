pub mod admin;
pub mod public;
pub mod templates;
pub mod views;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use include_dir::{Dir, include_dir};
use serde_json::json;
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
        .route(crate::health::HEALTH_PATH, get(healthz))
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

/// Liveness probe for Docker and CI: 200 when the database answers, else 503.
async fn healthz(State(state): State<AppState>) -> Response {
    let (status, label) = match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
    {
        Ok(_) => (StatusCode::OK, "ok"),
        Err(error) => {
            tracing::warn!(%error, "health check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    };
    (
        status,
        [(CACHE_CONTROL, "no-store")],
        Json(json!({ "status": label })),
    )
        .into_response()
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

/// The hash allows Vditor's icon sprite: the editor fetches
/// `static/vditor/dist/js/icons/ant.js` with XHR and injects it as an inline
/// script, so it must match that file exactly. A unit test recomputes it.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     img-src 'self' data: https:; \
     style-src 'self' 'unsafe-inline'; \
     script-src 'self' 'sha256-qR4U4J3Ne5n0m3uNzGMB/tZ3TWJUf89OlxdXqjqALDM='; \
     frame-ancestors 'none'; \
     base-uri 'self'; \
     form-action 'self'";

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
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    if private_response {
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    response
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    use super::CONTENT_SECURITY_POLICY;

    #[test]
    fn csp_allows_the_inlined_vditor_icon_script() {
        let script = include_bytes!("../../static/vditor/dist/js/icons/ant.js");
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(script));
        assert!(
            CONTENT_SECURITY_POLICY.contains(&format!("'sha256-{hash}'")),
            "update the CSP hash after upgrading Vditor: sha256-{hash}"
        );
    }
}
