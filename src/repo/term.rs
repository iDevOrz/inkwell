use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::{AppError, AppResult};
use crate::models::{Term, TermWithCount};
use crate::util::{now_ts, slugify};

pub async fn list(pool: &SqlitePool, kind: &str) -> AppResult<Vec<TermWithCount>> {
    let rows = sqlx::query_as::<_, TermWithCount>(
        "SELECT t.id, t.kind, t.name, t.slug, t.description, t.parent_id, t.created_at, \
                COUNT(ct.content_id) AS count \
         FROM terms t LEFT JOIN content_terms ct ON ct.term_id = t.id \
         WHERE t.kind = ? GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
    )
    .bind(kind)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_public(pool: &SqlitePool, kind: &str) -> AppResult<Vec<TermWithCount>> {
    Ok(sqlx::query_as::<_, TermWithCount>(
        "SELECT t.id, t.kind, t.name, t.slug, t.description, t.parent_id, t.created_at, \
                COUNT(DISTINCT c.id) AS count \
         FROM terms t \
         JOIN content_terms ct ON ct.term_id = t.id \
         JOIN contents c ON c.id = ct.content_id \
         WHERE t.kind = ? AND c.kind = 'post' AND c.status = 'published' \
           AND c.visibility = 'public' AND c.deleted_at IS NULL \
         GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
    )
    .bind(kind)
    .fetch_all(pool)
    .await?)
}

pub async fn get_by_slug(pool: &SqlitePool, kind: &str, slug: &str) -> AppResult<Option<Term>> {
    Ok(sqlx::query_as::<_, Term>(
        "SELECT id, kind, name, slug, description, parent_id, created_at \
         FROM terms WHERE kind = ? AND slug = ?",
    )
    .bind(kind)
    .bind(slug)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_public_by_slug(
    pool: &SqlitePool,
    kind: &str,
    slug: &str,
) -> AppResult<Option<Term>> {
    Ok(sqlx::query_as::<_, Term>(
        "SELECT t.id, t.kind, t.name, t.slug, t.description, t.parent_id, t.created_at \
         FROM terms t \
         WHERE t.kind = ? AND t.slug = ? AND EXISTS ( \
             SELECT 1 FROM content_terms ct \
             JOIN contents c ON c.id = ct.content_id \
             WHERE ct.term_id = t.id AND c.kind = 'post' AND c.status = 'published' \
               AND c.visibility = 'public' AND c.deleted_at IS NULL \
         )",
    )
    .bind(kind)
    .bind(slug)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_by_id(pool: &SqlitePool, id: i64) -> AppResult<Option<Term>> {
    Ok(sqlx::query_as::<_, Term>(
        "SELECT id, kind, name, slug, description, parent_id, created_at FROM terms WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn create(
    pool: &SqlitePool,
    kind: &str,
    name: &str,
    slug: Option<&str>,
    description: Option<&str>,
) -> AppResult<Term> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("名称不能为空"));
    }
    if kind != "category" && kind != "tag" {
        return Err(AppError::bad_request("未知的术语类型"));
    }
    let slug = slug
        .map(slugify)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| slugify(name));

    let result = sqlx::query(
        "INSERT INTO terms(kind, name, slug, description, created_at) VALUES(?, ?, ?, ?, ?)",
    )
    .bind(kind)
    .bind(name)
    .bind(&slug)
    .bind(description)
    .bind(now_ts())
    .execute(pool)
    .await;

    match result {
        Ok(_) => get_by_slug(pool, kind, &slug)
            .await?
            .ok_or_else(|| AppError::internal("创建术语后读取失败")),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(AppError::conflict("该名称或缩略名已存在"))
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn delete(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM terms WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_or_create_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    kind: &str,
    name: &str,
) -> AppResult<Option<Term>> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let slug = slugify(name);

    if let Some(term) = sqlx::query_as::<_, Term>(
        "SELECT id, kind, name, slug, description, parent_id, created_at \
         FROM terms WHERE kind = ? AND (slug = ? OR name = ? COLLATE NOCASE) \
         ORDER BY id LIMIT 1",
    )
    .bind(kind)
    .bind(&slug)
    .bind(name)
    .fetch_optional(&mut **transaction)
    .await?
    {
        return Ok(Some(term));
    }

    sqlx::query(
        "INSERT INTO terms(kind, name, slug, description, created_at) VALUES(?, ?, ?, NULL, ?)",
    )
    .bind(kind)
    .bind(name)
    .bind(&slug)
    .bind(now_ts())
    .execute(&mut **transaction)
    .await?;

    let term = sqlx::query_as::<_, Term>(
        "SELECT id, kind, name, slug, description, parent_id, created_at \
         FROM terms WHERE kind = ? AND slug = ?",
    )
    .bind(kind)
    .bind(&slug)
    .fetch_one(&mut **transaction)
    .await?;

    Ok(Some(term))
}

pub async fn replace_terms_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    content_id: i64,
    category: Option<&str>,
    tags: &[String],
) -> AppResult<()> {
    sqlx::query("DELETE FROM content_terms WHERE content_id = ?")
        .bind(content_id)
        .execute(&mut **transaction)
        .await?;

    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty())
        && let Some(term) = find_or_create_tx(transaction, "category", category).await?
    {
        sqlx::query("INSERT OR IGNORE INTO content_terms(content_id, term_id) VALUES(?, ?)")
            .bind(content_id)
            .bind(term.id)
            .execute(&mut **transaction)
            .await?;
    }

    for tag in tags {
        if let Some(term) = find_or_create_tx(transaction, "tag", tag).await? {
            sqlx::query("INSERT OR IGNORE INTO content_terms(content_id, term_id) VALUES(?, ?)")
                .bind(content_id)
                .bind(term.id)
                .execute(&mut **transaction)
                .await?;
        }
    }

    Ok(())
}

pub async fn set_tags_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    content_id: i64,
    tags: &[String],
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM content_terms WHERE content_id = ? AND term_id IN \
         (SELECT id FROM terms WHERE kind = 'tag')",
    )
    .bind(content_id)
    .execute(&mut **transaction)
    .await?;

    for tag in tags {
        if let Some(term) = find_or_create_tx(transaction, "tag", tag).await? {
            sqlx::query("INSERT OR IGNORE INTO content_terms(content_id, term_id) VALUES(?, ?)")
                .bind(content_id)
                .bind(term.id)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

pub async fn set_category_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    content_id: i64,
    category: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "DELETE FROM content_terms WHERE content_id = ? AND term_id IN \
         (SELECT id FROM terms WHERE kind = 'category')",
    )
    .bind(content_id)
    .execute(&mut **transaction)
    .await?;

    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty())
        && let Some(term) = find_or_create_tx(transaction, "category", category).await?
    {
        sqlx::query("INSERT OR IGNORE INTO content_terms(content_id, term_id) VALUES(?, ?)")
            .bind(content_id)
            .bind(term.id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

pub async fn terms_for_contents(
    pool: &SqlitePool,
    content_ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, Vec<Term>>> {
    if content_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut builder = sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT ct.content_id, t.id, t.kind, t.name, t.slug, t.description, t.parent_id, t.created_at \
         FROM content_terms ct JOIN terms t ON t.id = ct.term_id WHERE ct.content_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in content_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY t.kind, t.name");

    #[derive(sqlx::FromRow)]
    struct Row {
        content_id: i64,
        id: i64,
        kind: String,
        name: String,
        slug: String,
        description: Option<String>,
        parent_id: Option<i64>,
        created_at: i64,
    }

    let rows = builder.build_query_as::<Row>().fetch_all(pool).await?;
    let mut result: std::collections::HashMap<i64, Vec<Term>> = std::collections::HashMap::new();
    for row in rows {
        result.entry(row.content_id).or_default().push(Term {
            id: row.id,
            kind: row.kind,
            name: row.name,
            slug: row.slug,
            description: row.description,
            parent_id: row.parent_id,
            created_at: row.created_at,
        });
    }
    Ok(result)
}
