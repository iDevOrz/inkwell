//! Route × role access matrix: who may see which pages, and that every
//! state-changing back-office route rejects requests without a CSRF token.
mod common;

use axum::http::{StatusCode, header};
use common::{Session, TestApp, body_string};
use serde_json::json;

const SEE_OTHER: u16 = 303;
const FORBIDDEN: u16 = 403;
const NOT_FOUND: u16 = 404;
const OK: u16 = 200;

struct Roles {
    subscriber: Session,
    editor: Session,
    admin: Session,
}

impl Roles {
    async fn new(app: &TestApp) -> Self {
        Self {
            subscriber: app.session_for("subscriber").await,
            editor: app.session_for("editor").await,
            admin: app.session_for("admin").await,
        }
    }

    /// (label, session) in the column order used by the expectation tables.
    fn columns(&self) -> [(&'static str, Option<&Session>); 4] {
        [
            ("anonymous", None),
            ("subscriber", Some(&self.subscriber)),
            ("editor", Some(&self.editor)),
            ("admin", Some(&self.admin)),
        ]
    }
}

/// Hidden content created through MCP; titles carry markers so leaks are easy
/// to spot in listings and feeds.
struct Hidden {
    draft_post_id: i64,
    draft_post_slug: String,
    private_post_slug: String,
    private_page_slug: String,
}

const DRAFT_MARKER: &str = "LEAKDRAFT";
const PRIVATE_MARKER: &str = "LEAKPRIVATE";

async fn create_hidden_content(app: &TestApp) -> Hidden {
    let publisher = app.token_with(&["read", "write", "publish"]).await;
    let create = |kind: &'static str, title: String, status: &'static str, visibility| {
        let publisher = publisher.clone();
        async move {
            let created = app
                .tool_as(
                    &publisher,
                    "content_create",
                    json!({
                        "kind": kind,
                        "title": title,
                        "body": format!("{title} 正文"),
                        "status": status,
                        "visibility": visibility,
                    }),
                )
                .await;
            assert_eq!(created["isError"], serde_json::Value::Null, "{created}");
            let id = created["structuredContent"]["id"].as_i64().expect("id");
            let slug = created["structuredContent"]["slug"]
                .as_str()
                .expect("slug")
                .to_string();
            (id, slug)
        }
    };

    let (draft_post_id, draft_post_slug) =
        create("post", format!("{DRAFT_MARKER} 草稿"), "draft", "public").await;
    let (_, private_post_slug) = create(
        "post",
        format!("{PRIVATE_MARKER} 私密文章"),
        "published",
        "private",
    )
    .await;
    let (_, private_page_slug) = create(
        "page",
        format!("{PRIVATE_MARKER} 私密页面"),
        "published",
        "private",
    )
    .await;
    Hidden {
        draft_post_id,
        draft_post_slug,
        private_post_slug,
        private_page_slug,
    }
}

async fn assert_get_matrix(app: &TestApp, roles: &Roles, uri: &str, expected: [u16; 4]) {
    for ((role, session), expected) in roles.columns().into_iter().zip(expected) {
        let response = app.get(uri, session).await;
        let status = response.status().as_u16();
        assert_eq!(status, expected, "GET {uri} as {role}");
        if status == SEE_OTHER {
            let location = response.headers()[header::LOCATION].to_str().unwrap_or("");
            assert!(
                location.starts_with("/admin/login"),
                "GET {uri} as {role} redirected to {location}"
            );
        }
    }
}

#[tokio::test]
async fn back_office_pages_require_the_right_role() {
    let app = TestApp::new().await;
    let roles = Roles::new(&app).await;
    let hidden = create_hidden_content(&app).await;
    let edit = format!("/admin/posts/{}/edit", hidden.draft_post_id);

    let content_roles = [SEE_OTHER, FORBIDDEN, OK, OK];
    for uri in [
        "/admin",
        "/admin/posts",
        "/admin/pages",
        "/admin/notes",
        "/admin/posts/new",
        "/admin/review",
        "/admin/annotations",
        "/admin/comments",
        "/admin/terms",
        "/admin/media",
        "/admin/password",
        edit.as_str(),
    ] {
        assert_get_matrix(&app, &roles, uri, content_roles).await;
    }

    let admin_only = [SEE_OTHER, FORBIDDEN, FORBIDDEN, OK];
    for uri in ["/admin/settings", "/admin/tokens"] {
        assert_get_matrix(&app, &roles, uri, admin_only).await;
    }
}

#[tokio::test]
async fn drafts_and_private_content_are_hidden_from_readers() {
    let app = TestApp::new().await;
    let roles = Roles::new(&app).await;
    let hidden = create_hidden_content(&app).await;

    let staff_only = [NOT_FOUND, NOT_FOUND, OK, OK];
    for uri in [
        format!("/archives/{}", hidden.draft_post_slug),
        format!("/archives/{}", hidden.private_post_slug),
        format!("/pages/{}", hidden.private_page_slug),
    ] {
        assert_get_matrix(&app, &roles, &uri, staff_only).await;
    }

    // Listings, feeds and search never mention hidden content, even when the
    // reader is signed in without a back-office role.
    for session in [None, Some(&roles.subscriber)] {
        for uri in [
            "/",
            "/timeline/all",
            "/feed.xml",
            "/atom.xml",
            "/sitemap.xml",
            "/search?q=LEAK",
        ] {
            let response = app.get(uri, session).await;
            assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
            let body = body_string(response).await;
            for marker in [DRAFT_MARKER, PRIVATE_MARKER] {
                assert!(!body.contains(marker), "GET {uri} leaked {marker}");
            }
        }
    }
}

/// One form body that satisfies every admin form's required fields, so a
/// rejection can only come from the CSRF check, not from a 422.
fn complete_form(csrf: Option<&str>) -> Vec<(&str, &str)> {
    let mut fields = vec![
        ("title", "x"),
        ("body", "x"),
        ("verdict", "approved"),
        ("kind", "tag"),
        ("name", "x"),
        ("action", "approve"),
        ("comment", "x"),
        ("site_title", "x"),
        ("site_description", "x"),
        ("site_keywords", "x"),
        ("site_url", "http://test.local"),
        ("footer_text", "x"),
        ("page_size", "10"),
        ("timezone_offset", "0"),
        ("old_password", "unused-test-password"),
        ("new_password", "another-password"),
        ("confirm_password", "another-password"),
    ];
    if let Some(csrf) = csrf {
        fields.push(("_csrf", csrf));
    }
    fields
}

fn state_changing_routes(post: i64) -> Vec<String> {
    vec![
        "/admin/posts".to_string(),
        format!("/admin/posts/{post}"),
        format!("/admin/posts/{post}/publish"),
        format!("/admin/review/{post}"),
        format!("/admin/posts/{post}/annotations"),
        "/admin/terms".to_string(),
        "/admin/comments/1/status".to_string(),
        "/admin/settings".to_string(),
        "/admin/tokens".to_string(),
        "/admin/tokens/1/revoke".to_string(),
        "/admin/password".to_string(),
        format!("/admin/posts/{post}/delete"),
        // Last: it ends the session used by the positive control.
        "/admin/logout".to_string(),
    ]
}

const CSRF_ERROR: &str = "CSRF 校验失败";

#[tokio::test]
async fn state_changing_routes_reject_missing_or_wrong_csrf() {
    let app = TestApp::new().await;
    let roles = Roles::new(&app).await;
    let hidden = create_hidden_content(&app).await;
    let post = hidden.draft_post_id;

    for uri in state_changing_routes(post) {
        let anonymous = app.post_form(&uri, None, &complete_form(None)).await;
        assert_eq!(
            anonymous.status().as_u16(),
            SEE_OTHER,
            "POST {uri} anonymous"
        );

        let subscriber = app
            .post_form(
                &uri,
                Some(&roles.subscriber),
                &complete_form(Some(&roles.subscriber.csrf)),
            )
            .await;
        assert_eq!(
            subscriber.status().as_u16(),
            FORBIDDEN,
            "POST {uri} subscriber"
        );

        for (role, session) in [("editor", &roles.editor), ("admin", &roles.admin)] {
            for csrf in [None, Some("wrong-token")] {
                let response = app
                    .post_form(&uri, Some(session), &complete_form(csrf))
                    .await;
                assert_eq!(
                    response.status().as_u16(),
                    FORBIDDEN,
                    "POST {uri} as {role} with csrf {csrf:?}"
                );
            }
        }
    }

    // The draft survived every rejected request untouched.
    let draft = app.tool("content_get", json!({"id": post})).await;
    assert_eq!(draft["structuredContent"]["content"]["status"], "draft");
    assert_eq!(
        draft["structuredContent"]["content"]["title"],
        format!("{DRAFT_MARKER} 草稿")
    );
}

#[tokio::test]
async fn valid_csrf_passes_the_check_on_every_route() {
    // Positive control: the same requests with the session's token are never
    // rejected for CSRF, so the 403s above really come from the CSRF check.
    let app = TestApp::new().await;
    let admin = app.session_for("admin").await;
    let hidden = create_hidden_content(&app).await;

    for uri in state_changing_routes(hidden.draft_post_id) {
        let response = app
            .post_form(&uri, Some(&admin), &complete_form(Some(&admin.csrf)))
            .await;
        let status = response.status().as_u16();
        let body = body_string(response).await;
        assert!(
            !body.contains(CSRF_ERROR),
            "POST {uri} rejected for CSRF ({status})"
        );
    }

    // Logout (last route) really ended the session.
    let after = app.get("/admin", Some(&admin)).await;
    assert_eq!(after.status().as_u16(), SEE_OTHER);
}
