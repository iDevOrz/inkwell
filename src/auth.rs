use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::RngCore;
use sqlx::{FromRow, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::User;
use crate::state::AppState;
use crate::util::{constant_time_eq, now_ts, random_token, sha256_hex};

pub const SESSION_COOKIE: &str = "typecho_session";
pub const LOGIN_CSRF_COOKIE: &str = "typecho_login_csrf";

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub user: User,
    pub csrf_token: String,
    pub token_hash: String,
}

#[derive(Debug, Clone)]
pub struct OptionalUser(pub Option<AuthSession>);

#[derive(Debug, Clone)]
pub struct AdminSession(pub AuthSession);

impl AdminSession {
    pub fn user(&self) -> &User {
        &self.0.user
    }
}

impl FromRequestParts<AppState> for OptionalUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = cookie_value(&parts.headers, SESSION_COOKIE) else {
            return Ok(Self(None));
        };

        let session = load_session(&state.db, &token).await?;
        Ok(Self(session))
    }
}

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let OptionalUser(session) = OptionalUser::from_request_parts(parts, state).await?;
        let Some(session) = session else {
            let next = parts
                .uri
                .path_and_query()
                .map(|value| value.as_str())
                .unwrap_or("/admin");
            let next = utf8_percent_encode(next, NON_ALPHANUMERIC);
            return Err(AppError::Redirect(format!("/admin/login?next={next}")));
        };

        if session.user.role != "admin" && session.user.role != "editor" {
            return Err(AppError::Forbidden("当前账号没有后台权限".to_string()));
        }

        Ok(Self(session))
    }
}

pub fn hash_password(password: &str) -> AppResult<String> {
    let mut salt_bytes = [0_u8; 16];
    rand::rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn create_session(
    pool: &SqlitePool,
    user_id: i64,
    days: i64,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> AppResult<(String, String)> {
    let token = random_token(32);
    let token_hash = sha256_hex(&token);
    let csrf_token = random_token(24);
    let now = now_ts();
    let expires_at = now + days.max(1) * 24 * 3600;

    sqlx::query(
        "INSERT INTO sessions(token_hash, user_id, csrf_token, created_at, expires_at, ip, user_agent) \
         VALUES(?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&token_hash)
    .bind(user_id)
    .bind(&csrf_token)
    .bind(now)
    .bind(expires_at)
    .bind(ip)
    .bind(user_agent)
    .execute(pool)
    .await?;

    Ok((token, csrf_token))
}

pub async fn delete_session(pool: &SqlitePool, token_hash: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn purge_expired_sessions(pool: &SqlitePool) -> AppResult<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now_ts())
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[derive(Debug, FromRow)]
struct SessionRow {
    id: i64,
    username: String,
    password_hash: String,
    display_name: String,
    email: Option<String>,
    role: String,
    created_at: i64,
    csrf_token: String,
    token_hash: String,
    expires_at: i64,
}

async fn load_session(pool: &SqlitePool, token: &str) -> AppResult<Option<AuthSession>> {
    let token_hash = sha256_hex(token);
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT u.id, u.username, u.password_hash, u.display_name, u.email, u.role, \
                u.created_at, s.csrf_token, s.token_hash, s.expires_at \
         FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = ?",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    if row.expires_at <= now_ts() {
        delete_session(pool, &row.token_hash).await?;
        return Ok(None);
    }

    Ok(Some(AuthSession {
        user: User {
            id: row.id,
            username: row.username,
            password_hash: row.password_hash,
            display_name: row.display_name,
            email: row.email,
            role: row.role,
            created_at: row.created_at,
        },
        csrf_token: row.csrf_token,
        token_hash: row.token_hash,
    }))
}

pub fn verify_csrf(session: &AuthSession, token: &str) -> AppResult<()> {
    if constant_time_eq(&session.csrf_token, token) {
        Ok(())
    } else {
        Err(AppError::forbidden("CSRF 校验失败，请刷新页面后重试"))
    }
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then(|| value.to_string())
        })
}

pub fn session_cookie(token: &str, max_age_seconds: i64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_seconds}{secure}"
    ))
    .expect("session cookie is valid")
}

pub fn clear_session_cookie(secure: bool) -> HeaderValue {
    session_cookie("", 0, secure)
}

pub fn login_csrf_cookie(token: &str, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{LOGIN_CSRF_COOKIE}={token}; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=900{secure}"
    ))
    .expect("login csrf cookie is valid")
}

pub fn clear_login_csrf_cookie(secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{LOGIN_CSRF_COOKIE}=; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=0{secure}"
    ))
    .expect("login csrf cookie is valid")
}

pub fn append_cookie(headers: &mut HeaderMap, value: HeaderValue) {
    headers.append(SET_COOKIE, value);
}

#[derive(Debug, Clone)]
struct Attempt {
    failures: u32,
    blocked_until: Option<Instant>,
}

#[derive(Default)]
pub struct LoginLimiter {
    attempts: Mutex<HashMap<String, Attempt>>,
}

impl LoginLimiter {
    pub fn allowed(&self, key: &str) -> bool {
        let mut attempts = self.attempts.lock().expect("login limiter lock");
        if attempts.len() > 2_000 {
            attempts.retain(|_, attempt| {
                attempt
                    .blocked_until
                    .map(|until| until > Instant::now())
                    .unwrap_or(false)
                    || attempt.failures > 0
            });
        }
        match attempts.get(key) {
            Some(attempt) => attempt
                .blocked_until
                .map(|until| until <= Instant::now())
                .unwrap_or(true),
            None => true,
        }
    }

    pub fn record_failure(&self, key: &str) {
        let mut attempts = self.attempts.lock().expect("login limiter lock");
        let entry = attempts.entry(key.to_string()).or_insert(Attempt {
            failures: 0,
            blocked_until: None,
        });
        entry.failures += 1;
        if entry.failures >= 5 {
            entry.blocked_until = Some(Instant::now() + Duration::from_secs(10 * 60));
        }
    }

    pub fn record_success(&self, key: &str) {
        self.attempts
            .lock()
            .expect("login limiter lock")
            .remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn limiter_blocks_after_failures() {
        let limiter = LoginLimiter::default();
        let key = "admin:127.0.0.1";
        assert!(limiter.allowed(key));
        for _ in 0..5 {
            limiter.record_failure(key);
        }
        assert!(!limiter.allowed(key));
    }
}
