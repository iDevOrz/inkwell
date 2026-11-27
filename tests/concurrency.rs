//! Races that single-request tests cannot see. These run on a multi-threaded
//! runtime against a file database with a real connection pool.
mod common;

use std::collections::HashSet;

use std::future::Future;

use common::{TestApp, body_string, cookie_from, hidden_value, mcp_tool, tool_text};
use inkwell::auth::LOGIN_CSRF_COOKIE;
use serde_json::{Value, json};
use tower::ServiceExt;

const CONCURRENT_REQUESTS: usize = 10;
const LOGIN_ATTEMPT_LIMIT: usize = 5;
const INTERNAL_ERROR: &str = "服务器内部错误";

/// Spawns every future as its own task so they run in parallel on the
/// multi-threaded runtime, then collects the results in order.
async fn race<T, F>(futures: impl IntoIterator<Item = F>) -> Vec<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await.expect("task panicked"));
    }
    results
}

/// The same tool call `count` times, each with its own arguments.
async fn race_tool(app: &TestApp, name: &'static str, args: Vec<Value>) -> Vec<Value> {
    race(args.into_iter().map(|arguments| {
        let router = app.router.clone();
        async move { mcp_tool(&router, name, arguments).await }
    }))
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_failed_logins_cannot_exceed_the_attempt_limit() {
    let app = TestApp::with_file_database().await;
    let login = app.get("/admin/login", None).await;
    let csrf_cookie = cookie_from(login.headers(), LOGIN_CSRF_COOKIE).expect("csrf cookie");
    let csrf = hidden_value(&body_string(login).await, "csrf");

    let attempts = (0..CONCURRENT_REQUESTS).map(|_| {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/admin/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", format!("{LOGIN_CSRF_COOKIE}={csrf_cookie}"))
            .body(axum::body::Body::from(common::encode(&[
                ("username", "admin"),
                ("password", "wrong-password"),
                ("csrf", &csrf),
            ])))
            .expect("request");
        let router = app.router.clone();
        async move { body_string(router.oneshot(request).await.expect("response")).await }
    });
    let pages = race(attempts).await;

    let checked = pages
        .iter()
        .filter(|page| page.contains("用户名或密码错误"))
        .count();
    let blocked = pages
        .iter()
        .filter(|page| page.contains("尝试次数过多"))
        .count();
    assert_eq!(checked, LOGIN_ATTEMPT_LIMIT, "password checks that ran");
    assert_eq!(blocked, CONCURRENT_REQUESTS - LOGIN_ATTEMPT_LIMIT);
}

async fn create_post(app: &TestApp, slug: &str) -> (i64, i64) {
    let created = app
        .tool(
            "content_create",
            json!({"kind": "post", "title": slug, "slug": slug, "body": "正文", "status": "draft"}),
        )
        .await;
    assert_eq!(created["isError"], Value::Null, "{created}");
    let content = &created["structuredContent"];
    (
        content["id"].as_i64().expect("id"),
        content["updated_at"].as_i64().expect("updated_at"),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_slug_claims_never_produce_duplicates_or_internal_errors() {
    let app = TestApp::with_file_database().await;
    let mut posts = Vec::new();
    for index in 0..CONCURRENT_REQUESTS {
        posts.push(create_post(&app, &format!("post-{index}")).await.0);
    }

    let args = posts
        .iter()
        .map(|id| json!({"id": id, "slug": "shared-slug"}))
        .collect();
    let results = race_tool(&app, "content_update", args).await;

    let mut slugs = HashSet::new();
    for result in &results {
        assert!(!tool_text(result).contains(INTERNAL_ERROR), "{result}");
        if result["isError"] == Value::Null {
            let slug = result["structuredContent"]["slug"].as_str().expect("slug");
            assert!(slugs.insert(slug.to_string()), "duplicate slug {slug}");
        }
    }
    assert!(
        slugs.contains("shared-slug"),
        "someone gets the requested slug"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_updates_from_the_same_base_let_exactly_one_win() {
    let app = TestApp::with_file_database().await;
    let (post_id, updated_at) = create_post(&app, "contended").await;

    let args = (0..CONCURRENT_REQUESTS)
        .map(|index| {
            json!({
                "id": post_id,
                "body": format!("版本 {index}"),
                "expected_updated_at": updated_at
            })
        })
        .collect();
    let results = race_tool(&app, "content_update", args).await;

    for result in &results {
        assert!(!tool_text(result).contains(INTERNAL_ERROR), "{result}");
    }
    let winners = results
        .iter()
        .filter(|r| r["isError"] == Value::Null)
        .count();
    assert_eq!(winners, 1);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE content_id = ?")
        .bind(post_id)
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(revisions, 2, "create + exactly one update");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_proposal_version_is_applied_at_most_once() {
    let app = TestApp::with_file_database().await;
    let (post_id, _) = create_post(&app, "proposal-race").await;
    let proposal = app
        .tool(
            "proposal_create",
            json!({"content_id": post_id, "body": "AI 版本"}),
        )
        .await;
    let proposal_id = proposal["structuredContent"]["id"].as_i64().expect("id");

    let args = vec![json!({"id": proposal_id, "version": 1}); CONCURRENT_REQUESTS];
    let results = race_tool(&app, "proposal_apply", args).await;

    for result in &results {
        assert!(!tool_text(result).contains(INTERNAL_ERROR), "{result}");
    }
    let applied = results
        .iter()
        .filter(|r| r["isError"] == Value::Null)
        .count();
    assert_eq!(applied, 1);
    let revisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE content_id = ?")
        .bind(post_id)
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(revisions, 2, "create + exactly one apply");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_note_promotions_create_and_link_a_post_once() {
    let app = TestApp::with_file_database().await;
    let note = app
        .tool(
            "note_create",
            json!({"title": "Concurrent promotion", "body": "A note"}),
        )
        .await;
    let note_id = note["structuredContent"]["id"].as_i64().expect("note id");
    let before: i64 = sqlx::query_scalar("SELECT updated_at FROM contents WHERE id = ?")
        .bind(note_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let results = race_tool(
        &app,
        "note_promote",
        vec![json!({"id": note_id}); CONCURRENT_REQUESTS],
    )
    .await;
    let (linked_post, after): (i64, i64) =
        sqlx::query_as("SELECT promoted_to_id, updated_at FROM contents WHERE id = ?")
            .bind(note_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let mut created = 0;
    for result in results {
        assert_eq!(result["structuredContent"]["id"], linked_post, "{result}");
        created += usize::from(result["structuredContent"]["created"] == true);
    }
    assert_eq!(created, 1);
    assert!(after > before);
}
