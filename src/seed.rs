use sqlx::SqlitePool;

use crate::auth::hash_password;
use crate::config::Config;
use crate::error::AppResult;
use crate::repo::{content, term, user};
use crate::settings;
use crate::util::random_token;

pub async fn run(pool: &SqlitePool, config: &Config) -> AppResult<()> {
    settings::ensure_defaults(pool, &config.site_url).await?;

    if user::count(pool).await? == 0 {
        let password = config
            .admin_password
            .clone()
            .unwrap_or_else(|| random_token(12));
        let password_hash = hash_password(&password)?;
        user::create(
            pool,
            &config.admin_user,
            &password_hash,
            &config.admin_user,
            None,
            "admin",
        )
        .await?;

        eprintln!();
        eprintln!("============================================================");
        eprintln!("  已创建管理员账号");
        eprintln!("  用户名: {}", config.admin_user);
        if config.admin_password.is_some() {
            eprintln!("  密码: 使用 ADMIN_PASSWORD 环境变量中的值");
        } else {
            eprintln!("  密码: {password}");
            eprintln!("  请登录后立即修改密码。");
        }
        eprintln!("============================================================");
        eprintln!();
    }

    if term::list(pool, "category").await?.is_empty() {
        term::create(
            pool,
            "category",
            "未分类",
            Some("uncategorized"),
            Some("默认分类"),
        )
        .await?;
    }

    let content_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contents")
        .fetch_one(pool)
        .await?;

    if content_count == 0 {
        let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
            .fetch_one(pool)
            .await?;

        content::create(
            pool,
            content::ContentInput {
                kind: "post".to_string(),
                title: "欢迎使用 inkwell".to_string(),
                slug: Some("hello-inkwell".to_string()),
                body: r#"# 欢迎

这是你的第一篇文章。你可以在后台写文章、记录笔记，也可以让 AI 通过 MCP 帮你整理和 review。

<!--more-->

## 下一步

- 打开 `/admin` 登录后台
- 在「笔记」里随手记录想法
- 笔记成熟后，点「转为文章」
- 在「MCP Token」里创建一个 token，让 AI 客户端接入

```rust
fn main() {
    println!("hello, inkwell");
}
```
"#
                .to_string(),
                excerpt: None,
                status: Some("published".to_string()),
                visibility: Some("public".to_string()),
                review_status: Some("none".to_string()),
                review_notes: None,
                allow_comments: Some(true),
                category: Some("未分类".to_string()),
                tags: vec!["开始".to_string()],
                author_id: admin_id,
                actor: "system:seed".to_string(),
                source_note_id: None,
            },
        )
        .await?;
    }

    Ok(())
}
