//! Liveness endpoint used by Docker HEALTHCHECK and CI.
mod common;

use std::future::IntoFuture;

use axum::http::{StatusCode, header};
use common::{TestApp, body_string};

#[tokio::test]
async fn healthz_reports_ok_when_database_is_reachable() {
    let app = TestApp::new().await;

    let response = app.get("/healthz", None).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: serde_json::Value =
        serde_json::from_str(&body_string(response).await).expect("json body");
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn healthz_reports_unavailable_when_database_is_closed() {
    let app = TestApp::new().await;
    app.pool.close().await;

    let response = app.get("/healthz", None).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value =
        serde_json::from_str(&body_string(response).await).expect("json body");
    assert_eq!(body["status"], "unavailable");
}

#[tokio::test]
async fn healthcheck_probe_succeeds_against_a_running_server() {
    let app = TestApp::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(axum::serve(listener, app.router.clone()).into_future());

    let probe = tokio::task::spawn_blocking(move || inkwell::health::probe(addr))
        .await
        .expect("probe task");

    server.abort();
    probe.expect("healthy server passes the probe");
}
