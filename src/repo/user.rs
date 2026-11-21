use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::util::now_ts;

pub async fn count(pool: &SqlitePool) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?)
}

pub async fn find_by_username(pool: &SqlitePool, username: &str) -> AppResult<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, display_name, email, role, created_at \
         FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?)
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> AppResult<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, display_name, email, role, created_at \
         FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn create(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    display_name: &str,
    email: Option<&str>,
    role: &str,
) -> AppResult<i64> {
    let username = username.trim();
    if username.is_empty() {
        return Err(AppError::bad_request("用户名不能为空"));
    }
    let display_name = display_name.trim();
    let display_name = if display_name.is_empty() {
        username
    } else {
        display_name
    };

    let result = sqlx::query(
        "INSERT INTO users(username, password_hash, display_name, email, role, created_at) \
         VALUES(?, ?, ?, ?, ?, ?)",
    )
    .bind(username)
    .bind(password_hash)
    .bind(display_name)
    .bind(email)
    .bind(role)
    .bind(now_ts())
    .execute(pool)
    .await;

    match result {
        Ok(result) => Ok(result.last_insert_rowid()),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(AppError::conflict("用户名已存在"))
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn update_password(pool: &SqlitePool, id: i64, password_hash: &str) -> AppResult<()> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
