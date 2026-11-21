use std::collections::HashMap;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::error::AppResult;
use crate::util::parse_bool;

#[derive(Debug, Clone, Serialize)]
pub struct SiteSettings {
    pub title: String,
    pub description: String,
    pub keywords: String,
    pub site_url: String,
    pub footer_text: String,
    pub page_size: i64,
    pub timezone_offset: i32,
    pub comments_enabled: bool,
    pub comments_moderation: bool,
    pub comments_require_email: bool,
    pub feed_full_text: bool,
}

impl SiteSettings {
    pub fn defaults(site_url: &str) -> Self {
        Self {
            title: "我的博客".to_string(),
            description: "记录想法、笔记与文章。".to_string(),
            keywords: "博客,笔记,Rust".to_string(),
            site_url: site_url.trim_end_matches('/').to_string(),
            footer_text: "由 inkwell 驱动".to_string(),
            page_size: 8,
            timezone_offset: 480,
            comments_enabled: true,
            comments_moderation: true,
            comments_require_email: true,
            feed_full_text: true,
        }
    }

    pub fn timezone(&self) -> chrono::FixedOffset {
        crate::util::timezone(self.timezone_offset)
    }
}

pub async fn ensure_defaults(pool: &SqlitePool, site_url: &str) -> AppResult<()> {
    let defaults = SiteSettings::defaults(site_url);
    let values = [
        ("site_title", defaults.title),
        ("site_description", defaults.description),
        ("site_keywords", defaults.keywords),
        ("site_url", defaults.site_url),
        ("footer_text", defaults.footer_text),
        ("page_size", defaults.page_size.to_string()),
        ("timezone_offset", defaults.timezone_offset.to_string()),
        ("comments_enabled", bool_value(defaults.comments_enabled)),
        (
            "comments_moderation",
            bool_value(defaults.comments_moderation),
        ),
        (
            "comments_require_email",
            bool_value(defaults.comments_require_email),
        ),
        ("feed_full_text", bool_value(defaults.feed_full_text)),
    ];

    for (name, value) in values {
        sqlx::query("INSERT INTO settings(name, value) VALUES(?, ?) ON CONFLICT(name) DO NOTHING")
            .bind(name)
            .bind(value)
            .execute(pool)
            .await?;
    }

    Ok(())
}

pub async fn load(pool: &SqlitePool, fallback_url: &str) -> AppResult<SiteSettings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, value FROM settings")
        .fetch_all(pool)
        .await?;
    let values: HashMap<String, String> = rows.into_iter().collect();
    let defaults = SiteSettings::defaults(fallback_url);

    let get = |name: &str| values.get(name).cloned();
    let get_bool = |name: &str, fallback: bool| {
        values
            .get(name)
            .map(|value| parse_bool(value))
            .unwrap_or(fallback)
    };

    Ok(SiteSettings {
        title: get("site_title").unwrap_or(defaults.title),
        description: get("site_description").unwrap_or(defaults.description),
        keywords: get("site_keywords").unwrap_or(defaults.keywords),
        site_url: get("site_url")
            .unwrap_or(defaults.site_url)
            .trim_end_matches('/')
            .to_string(),
        footer_text: get("footer_text").unwrap_or(defaults.footer_text),
        page_size: get("page_size")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.page_size)
            .clamp(1, 100),
        timezone_offset: get("timezone_offset")
            .and_then(|value| value.parse().ok())
            .unwrap_or(defaults.timezone_offset)
            .clamp(-12 * 60, 14 * 60),
        comments_enabled: get_bool("comments_enabled", defaults.comments_enabled),
        comments_moderation: get_bool("comments_moderation", defaults.comments_moderation),
        comments_require_email: get_bool("comments_require_email", defaults.comments_require_email),
        feed_full_text: get_bool("feed_full_text", defaults.feed_full_text),
    })
}

pub async fn update(pool: &SqlitePool, settings: &SiteSettings) -> AppResult<()> {
    let values = [
        ("site_title", settings.title.clone()),
        ("site_description", settings.description.clone()),
        ("site_keywords", settings.keywords.clone()),
        (
            "site_url",
            settings.site_url.trim_end_matches('/').to_string(),
        ),
        ("footer_text", settings.footer_text.clone()),
        ("page_size", settings.page_size.to_string()),
        ("timezone_offset", settings.timezone_offset.to_string()),
        ("comments_enabled", bool_value(settings.comments_enabled)),
        (
            "comments_moderation",
            bool_value(settings.comments_moderation),
        ),
        (
            "comments_require_email",
            bool_value(settings.comments_require_email),
        ),
        ("feed_full_text", bool_value(settings.feed_full_text)),
    ];

    let mut transaction = pool.begin().await?;
    for (name, value) in values {
        sqlx::query(
            "INSERT INTO settings(name, value) VALUES(?, ?) \
             ON CONFLICT(name) DO UPDATE SET value = excluded.value",
        )
        .bind(name)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

fn bool_value(value: bool) -> String {
    if value {
        "1".to_string()
    } else {
        "0".to_string()
    }
}
