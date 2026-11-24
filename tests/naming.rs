//! Legacy `typecho` identifiers stay readable while new ones are issued
//! (see wiki/adr/0001-rename-legacy-typecho-identifiers.md).
mod common;

use axum::http::{StatusCode, header};
use common::{Session, TestApp};
use inkwell::auth::{LEGACY_SESSION_COOKIE, SESSION_COOKIE};
use serde_json::json;

fn as_legacy_cookie(session: &Session) -> Session {
    let token = session
        .cookie
        .strip_prefix(&format!("{SESSION_COOKIE}="))
        .expect("fixture uses the current cookie name");
    Session {
        cookie: format!("{LEGACY_SESSION_COOKIE}={token}"),
        csrf: session.csrf.clone(),
    }
}

fn set_cookies(response: &axum::http::Response<axum::body::Body>) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect()
}

#[test]
fn current_cookie_names_use_the_project_name() {
    assert_eq!(SESSION_COOKIE, "inkwell_session");
    assert_eq!(inkwell::auth::LOGIN_CSRF_COOKIE, "inkwell_login_csrf");
    assert_eq!(LEGACY_SESSION_COOKIE, "typecho_session");
}

#[tokio::test]
async fn sessions_issued_under_the_legacy_cookie_name_still_work() {
    let app = TestApp::new().await;
    let legacy = as_legacy_cookie(&app.session_for("editor").await);

    let response = app.get("/admin", Some(&legacy)).await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn logout_clears_both_cookie_names() {
    let app = TestApp::new().await;
    let legacy = as_legacy_cookie(&app.session_for("editor").await);

    let response = app
        .post_form("/admin/logout", Some(&legacy), &[("_csrf", &legacy.csrf)])
        .await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookies = set_cookies(&response);
    for name in [SESSION_COOKIE, LEGACY_SESSION_COOKIE] {
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.starts_with(&format!("{name}=;"))
                    && cookie.contains("Max-Age=0")),
            "{name} not cleared: {cookies:?}"
        );
    }
}

#[tokio::test]
async fn mcp_lists_inkwell_uris_and_still_reads_legacy_ones() {
    let app = TestApp::new().await;

    let listed = app
        .mcp(json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}))
        .await;
    let uris: Vec<&str> = listed["result"]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .filter_map(|resource| resource["uri"].as_str())
        .collect();
    assert!(!uris.is_empty());
    assert!(
        uris.iter().all(|uri| uri.starts_with("inkwell://")),
        "{uris:?}"
    );

    let templates = app
        .mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "resources/templates/list"}))
        .await;
    assert_eq!(
        templates["result"]["resourceTemplates"][0]["uriTemplate"],
        "inkwell://content/{id}"
    );

    for uri in [
        "inkwell://review/queue",
        "typecho://review/queue",
        "inkwell://content/1",
        "typecho://content/1",
    ] {
        let read = app
            .mcp(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": uri}}))
            .await;
        assert!(read.get("result").is_some(), "{uri}: {read}");
    }
}
