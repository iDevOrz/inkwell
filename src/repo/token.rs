use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::models::ApiToken;
use crate::util::{constant_time_eq, now_ts, random_token, sha256_hex};

pub const SCOPES: &[&str] = &["read", "write", "publish", "review", "admin"];

#[derive(Debug, Clone)]
pub struct McpPrincipal {
    pub name: String,
    pub scopes: Vec<String>,
}

impl McpPrincipal {
    pub fn local() -> Self {
        Self {
            name: "stdio".to_string(),
            scopes: SCOPES.iter().map(|scope| scope.to_string()).collect(),
        }
    }

    pub fn has(&self, scope: &str) -> bool {
        self.scopes.iter().any(|value| value == "admin")
            || self.scopes.iter().any(|value| value == scope)
    }

    pub fn actor(&self) -> String {
        format!("mcp:{}", self.name)
    }
}

pub async fn create(pool: &SqlitePool, name: &str, scopes: &[String]) -> AppResult<(i64, String)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request("Token 名称不能为空"));
    }

    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for scope in scopes {
        let scope = scope.trim();
        if scope.is_empty() {
            continue;
        }
        if !SCOPES.contains(&scope) {
            return Err(AppError::bad_request(format!("未知权限: {scope}")));
        }
        if seen.insert(scope.to_string()) {
            normalized.push(scope.to_string());
        }
    }
    if normalized.is_empty() {
        normalized.push("read".to_string());
    }

    let plaintext = format!("tcr_{}", random_token(32));
    let hash = sha256_hex(&plaintext);
    let result = sqlx::query(
        "INSERT INTO api_tokens(name, token_hash, scopes, created_at) VALUES(?, ?, ?, ?)",
    )
    .bind(name)
    .bind(&hash)
    .bind(normalized.join(","))
    .bind(now_ts())
    .execute(pool)
    .await?;

    Ok((result.last_insert_rowid(), plaintext))
}

pub async fn verify(pool: &SqlitePool, token: &str) -> AppResult<Option<McpPrincipal>> {
    let hash = sha256_hex(token);
    let row = sqlx::query_as::<_, ApiToken>(
        "SELECT id, name, token_hash, scopes, created_at, last_used_at, revoked_at \
         FROM api_tokens WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    if !constant_time_eq(&row.token_hash, &hash) {
        return Ok(None);
    }

    sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE id = ?")
        .bind(now_ts())
        .bind(row.id)
        .execute(pool)
        .await?;

    Ok(Some(McpPrincipal {
        name: row.name,
        scopes: row
            .scopes
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
    }))
}

pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ApiToken>> {
    Ok(sqlx::query_as::<_, ApiToken>(
        "SELECT id, name, token_hash, scopes, created_at, last_used_at, revoked_at \
         FROM api_tokens ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?)
}

/// Revokes a token. Revoking an already revoked token is a no-op; an unknown
/// id is reported as not found.
pub async fn revoke(pool: &SqlitePool, id: i64) -> AppResult<()> {
    let result =
        sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(now_ts())
            .bind(id)
            .execute(pool)
            .await?;
    if result.rows_affected() > 0 {
        return Ok(());
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM api_tokens WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    exists
        .map(|_| ())
        .ok_or_else(|| AppError::not_found("Token 不存在"))
}

pub fn scopes_from_env(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|scope| scope.trim().to_string())
        .filter(|scope| !scope.is_empty())
        .collect()
}
