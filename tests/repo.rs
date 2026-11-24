//! Data-layer tests that call the repositories directly against SQLite,
//! without going through HTTP or MCP.
mod common;

use common::TestApp;
use inkwell::error::AppError;
use inkwell::models::{Comment, CommentNode};
use inkwell::repo::comment::{self, NewComment};
use inkwell::repo::token;
use serde_json::json;

async fn post_id(app: &TestApp) -> i64 {
    let created = app
        .tool(
            "content_create",
            json!({"kind": "post", "title": "仓储测试", "body": "正文"}),
        )
        .await;
    created["structuredContent"]["id"]
        .as_i64()
        .expect("post id")
}

fn new_comment(content_id: i64, parent_id: i64) -> NewComment {
    NewComment {
        content_id,
        parent_id,
        author: "读者".to_string(),
        email: None,
        url: None,
        body: "评论内容".to_string(),
        status: "pending".to_string(),
        ip: None,
        user_agent: None,
    }
}

fn is_bad_request<T: std::fmt::Debug>(result: Result<T, AppError>) -> bool {
    matches!(result, Err(AppError::BadRequest(_)))
}

// ---------------------------------------------------------------- comments

#[tokio::test]
async fn comment_create_validates_input() {
    let app = TestApp::new().await;
    let post = post_id(&app).await;
    let pool = &app.pool;

    let cases = [
        NewComment {
            author: "  ".to_string(),
            ..new_comment(post, 0)
        },
        NewComment {
            author: "长".repeat(81),
            ..new_comment(post, 0)
        },
        NewComment {
            body: " ".to_string(),
            ..new_comment(post, 0)
        },
        NewComment {
            body: "长".repeat(10_001),
            ..new_comment(post, 0)
        },
        NewComment {
            status: "published".to_string(),
            ..new_comment(post, 0)
        },
        NewComment {
            email: Some("not-an-email".to_string()),
            ..new_comment(post, 0)
        },
    ];
    for input in cases {
        let description = format!("{input:?}");
        assert!(
            is_bad_request(comment::create(pool, input).await),
            "{description}"
        );
    }
}

#[tokio::test]
async fn comment_create_normalizes_optional_fields() {
    let app = TestApp::new().await;
    let post = post_id(&app).await;

    let id = comment::create(
        &app.pool,
        NewComment {
            author: "  读者  ".to_string(),
            email: Some("  ".to_string()),
            url: Some("javascript:alert(1)".to_string()),
            parent_id: -5,
            ..new_comment(post, 0)
        },
    )
    .await
    .expect("create");
    let stored = comment::get(&app.pool, id)
        .await
        .expect("get")
        .expect("row");

    assert_eq!(stored.author, "读者");
    assert_eq!(stored.email, None);
    assert_eq!(stored.url, None, "non-http URLs are dropped");
    assert_eq!(stored.parent_id, 0, "negative parents become top-level");
}

#[tokio::test]
async fn comment_moderation_updates_status_and_rejects_missing_ids() {
    let app = TestApp::new().await;
    let post = post_id(&app).await;
    let pool = &app.pool;
    let id = comment::create(pool, new_comment(post, 0))
        .await
        .expect("create");

    comment::moderate(pool, id, "approved")
        .await
        .expect("approve");
    let approved = comment::list_for_content(pool, post, "approved")
        .await
        .expect("list");
    assert_eq!(approved.iter().map(|c| c.id).collect::<Vec<_>>(), vec![id]);
    assert!(
        comment::list_for_content(pool, post, "pending")
            .await
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        comment::list_for_content(pool, post, "all")
            .await
            .expect("list")
            .len(),
        1
    );

    assert!(is_bad_request(comment::moderate(pool, id, "bogus").await));
    assert!(matches!(
        comment::moderate(pool, 999_999, "approved").await,
        Err(AppError::NotFound(_))
    ));

    comment::delete(pool, id).await.expect("delete");
    assert!(comment::get(pool, id).await.expect("get").is_none());
    assert!(matches!(
        comment::delete(pool, id).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn comment_counts_group_by_status() {
    let app = TestApp::new().await;
    let post = post_id(&app).await;
    let pool = &app.pool;
    let before_pending = comment::count_pending(pool).await.expect("count");
    let before_total = comment::count_all(pool).await.expect("count");

    let first = comment::create(pool, new_comment(post, 0))
        .await
        .expect("create");
    comment::create(pool, new_comment(post, 0))
        .await
        .expect("create");
    comment::moderate(pool, first, "spam").await.expect("spam");

    assert_eq!(
        comment::count_pending(pool).await.expect("count"),
        before_pending + 1
    );
    assert_eq!(
        comment::count_all(pool).await.expect("count"),
        before_total + 2
    );
    let by_status = comment::count_by_status(pool).await.expect("count");
    assert_eq!(by_status.get("spam"), Some(&1));
}

#[tokio::test]
async fn reply_depth_counts_ancestors() {
    let app = TestApp::new().await;
    let post = post_id(&app).await;
    let pool = &app.pool;

    let root = comment::create(pool, new_comment(post, 0))
        .await
        .expect("root");
    let child = comment::create(pool, new_comment(post, root))
        .await
        .expect("child");
    let grandchild = comment::create(pool, new_comment(post, child))
        .await
        .expect("grand");

    assert_eq!(comment::reply_depth(pool, root).await.expect("depth"), 0);
    assert_eq!(
        comment::reply_depth(pool, grandchild).await.expect("depth"),
        2
    );
}

fn bare_comment(id: i64, parent_id: i64) -> Comment {
    Comment {
        id,
        content_id: 1,
        parent_id,
        author: format!("author-{id}"),
        email: None,
        url: None,
        body: String::new(),
        status: "approved".to_string(),
        ip: None,
        user_agent: None,
        created_at: id,
    }
}

fn shape(nodes: &[CommentNode]) -> Vec<(i64, Vec<i64>)> {
    nodes
        .iter()
        .map(|node| {
            (
                node.comment.id,
                node.children.iter().map(|child| child.comment.id).collect(),
            )
        })
        .collect()
}

fn count_nodes(nodes: &[CommentNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_nodes(&node.children))
        .sum()
}

#[test]
fn build_tree_nests_replies_in_order() {
    let tree = comment::build_tree(vec![
        bare_comment(1, 0),
        bare_comment(2, 1),
        bare_comment(3, 1),
        bare_comment(4, 0),
    ]);
    assert_eq!(shape(&tree), vec![(1, vec![2, 3]), (4, vec![])]);
}

#[test]
fn build_tree_promotes_orphans_and_self_parents_to_roots() {
    let tree = comment::build_tree(vec![bare_comment(1, 99), bare_comment(2, 2)]);
    assert_eq!(shape(&tree), vec![(1, vec![]), (2, vec![])]);
}

#[test]
fn build_tree_breaks_cycles_without_losing_comments() {
    // 1 -> 2 -> 3 -> 1 is malformed data; every comment must still appear once.
    let tree = comment::build_tree(vec![
        bare_comment(1, 3),
        bare_comment(2, 1),
        bare_comment(3, 2),
    ]);
    assert_eq!(count_nodes(&tree), 3);
    assert!(!tree.is_empty());
}

#[test]
fn build_tree_of_nothing_is_empty() {
    assert!(comment::build_tree(Vec::new()).is_empty());
}

// ------------------------------------------------------------------ tokens

fn scopes(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[tokio::test]
async fn token_create_validates_and_normalizes_scopes() {
    let app = TestApp::new().await;
    let pool = &app.pool;

    assert!(is_bad_request(
        token::create(pool, "  ", &scopes(&["read"])).await
    ));
    assert!(is_bad_request(
        token::create(pool, "t", &scopes(&["wirte"])).await
    ));

    let (_, plaintext) = token::create(pool, " ci ", &scopes(&["write", " write", "", "read"]))
        .await
        .expect("create");
    assert!(plaintext.starts_with("tcr_"));
    let principal = token::verify(pool, &plaintext)
        .await
        .expect("verify")
        .expect("valid");
    assert_eq!(principal.name, "ci");
    assert_eq!(principal.scopes, scopes(&["write", "read"]));

    let (_, defaulted) = token::create(pool, "empty", &[]).await.expect("create");
    let principal = token::verify(pool, &defaulted)
        .await
        .expect("verify")
        .expect("valid");
    assert_eq!(
        principal.scopes,
        scopes(&["read"]),
        "no scopes means read-only"
    );
}

#[tokio::test]
async fn token_verify_rejects_unknown_and_revoked_tokens() {
    let app = TestApp::new().await;
    let pool = &app.pool;
    let (id, plaintext) = token::create(pool, "temp", &scopes(&["read"]))
        .await
        .expect("create");

    assert!(
        token::verify(pool, "tcr_unknown")
            .await
            .expect("verify")
            .is_none()
    );
    assert!(
        token::verify(pool, &plaintext)
            .await
            .expect("verify")
            .is_some()
    );

    let listed = token::list(pool).await.expect("list");
    let row = listed.iter().find(|row| row.id == id).expect("listed");
    assert!(row.last_used_at.is_some(), "verify records last use");
    assert_ne!(row.token_hash, plaintext, "only the hash is stored");

    token::revoke(pool, id).await.expect("revoke");
    assert!(
        token::verify(pool, &plaintext)
            .await
            .expect("verify")
            .is_none()
    );
    let row = token::list(pool)
        .await
        .expect("list")
        .into_iter()
        .find(|row| row.id == id)
        .expect("still listed");
    assert!(row.revoked_at.is_some());
}

#[tokio::test]
async fn token_revoke_reports_missing_ids_and_is_idempotent() {
    let app = TestApp::new().await;
    let pool = &app.pool;
    let (id, _) = token::create(pool, "temp", &scopes(&["read"]))
        .await
        .expect("create");

    token::revoke(pool, id).await.expect("revoke");
    token::revoke(pool, id)
        .await
        .expect("revoking twice is a no-op");
    assert!(matches!(
        token::revoke(pool, 999_999).await,
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn admin_scope_implies_every_other_scope() {
    let admin = token::McpPrincipal {
        name: "a".to_string(),
        scopes: scopes(&["admin"]),
    };
    let reader = token::McpPrincipal {
        name: "r".to_string(),
        scopes: scopes(&["read"]),
    };
    for scope in token::SCOPES {
        assert!(admin.has(scope), "admin lacks {scope}");
    }
    assert!(reader.has("read"));
    assert!(!reader.has("write"));
    assert_eq!(reader.actor(), "mcp:r");
}
