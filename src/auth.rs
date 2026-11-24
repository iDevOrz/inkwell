use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use sqlx::{FromRow, SqlitePool};
use tokio::sync::Semaphore;

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

impl AuthSession {
    /// Admins and editors may use the back office and see drafts/private content.
    pub fn can_manage_content(&self) -> bool {
        matches!(self.user.role.as_str(), "admin" | "editor")
    }

    pub fn is_admin(&self) -> bool {
        self.user.role == "admin"
    }

    /// Site settings and MCP tokens grant control over the whole instance.
    pub fn require_admin(&self) -> AppResult<()> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(AppError::forbidden("只有管理员可以执行此操作"))
        }
    }
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

        if !session.can_manage_content() {
            return Err(AppError::Forbidden("当前账号没有后台权限".to_string()));
        }

        Ok(Self(session))
    }
}

pub fn hash_password(password: &str) -> AppResult<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())?
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

/// Each Argon2 run costs ~19 MiB and tens of milliseconds; bound how many run
/// at once so a burst of logins cannot exhaust memory or the blocking pool.
const MAX_CONCURRENT_PASSWORD_CHECKS: usize = 4;
static PASSWORD_CHECK_SLOTS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_PASSWORD_CHECKS);

/// Verified against when the username does not exist, so unknown and known
/// usernames both cost one Argon2 run and cannot be told apart by timing.
static DUMMY_PASSWORD_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password(&random_token(32)).expect("hash dummy login password"));

/// Runs CPU-heavy password work on the blocking pool, never on async workers.
async fn run_password_work<T, F>(work: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let _permit = PASSWORD_CHECK_SLOTS
        .acquire()
        .await
        .map_err(|error| anyhow::anyhow!("password check semaphore closed: {error}"))?;
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| anyhow::anyhow!("password check task failed: {error}").into())
}

/// Checks a login password. Pass `None` when the user does not exist: a dummy
/// hash is still verified so the response time does not reveal usernames.
pub async fn check_login_password(password: String, hash: Option<String>) -> bool {
    let has_user = hash.is_some();
    let result = run_password_work(move || {
        let hash = hash.as_deref().unwrap_or(DUMMY_PASSWORD_HASH.as_str());
        verify_password(&password, hash)
    })
    .await;
    match result {
        Ok(matches) => has_user && matches,
        Err(error) => {
            tracing::error!(%error, "password verification failed");
            false
        }
    }
}

pub async fn hash_password_async(password: String) -> AppResult<String> {
    run_password_work(move || hash_password(&password)).await?
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
    /// Attempts are counted per fixed window, so occasional legitimate use
    /// (a reader leaving a comment now and then) never accumulates into a block.
    window_start: Instant,
}

const LOGIN_ATTEMPT_TTL: Duration = Duration::from_secs(20 * 60);
const LOGIN_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const MAX_LOGIN_KEYS: usize = 4_096;
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_BLOCK_DURATION: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
struct LoginLimiterState {
    attempts: HashMap<String, Attempt>,
    last_cleanup: Option<Instant>,
}

#[derive(Default)]
pub struct LoginLimiter {
    state: Mutex<LoginLimiterState>,
}

impl LoginLimiter {
    pub fn allowed(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut state = self.lock_and_clean(now);
        Self::allowed_locked(&mut state, key, now)
    }

    /// Atomically checks the block and counts this attempt before any slow
    /// work runs, so concurrent requests cannot all slip past the limit.
    /// Call `record_success` afterwards to clear the counter.
    pub fn try_acquire(&self, key: &str) -> bool {
        self.try_acquire_at(key, Instant::now())
    }

    fn try_acquire_at(&self, key: &str, now: Instant) -> bool {
        let mut state = self.lock_and_clean(now);
        if !Self::allowed_locked(&mut state, key, now) {
            return false;
        }
        Self::record_attempt_locked(&mut state, key, now);
        true
    }

    pub fn record_failure(&self, key: &str) {
        self.record_attempt(key);
    }

    pub fn record_attempt(&self, key: &str) {
        let now = Instant::now();
        let mut state = self.lock_and_clean(now);
        Self::record_attempt_locked(&mut state, key, now);
    }

    fn lock_and_clean(&self, now: Instant) -> std::sync::MutexGuard<'_, LoginLimiterState> {
        let mut state = self.state.lock().expect("login limiter lock");
        Self::cleanup(&mut state, now);
        state
    }

    fn allowed_locked(state: &mut LoginLimiterState, key: &str, now: Instant) -> bool {
        if let Some(attempt) = state.attempts.get_mut(key) {
            if let Some(until) = attempt.blocked_until {
                if until > now {
                    return false;
                }
                attempt.blocked_until = None;
                Self::reset_window(attempt, now);
            } else if now.duration_since(attempt.window_start) >= LOGIN_ATTEMPT_TTL {
                Self::reset_window(attempt, now);
            }
        }
        true
    }

    fn reset_window(attempt: &mut Attempt, now: Instant) {
        attempt.failures = 0;
        attempt.window_start = now;
    }

    fn record_attempt_locked(state: &mut LoginLimiterState, key: &str, now: Instant) {
        if !state.attempts.contains_key(key) && state.attempts.len() >= MAX_LOGIN_KEYS {
            // Keep the unauthenticated limiter bounded. Prefer evicting an
            // unblocked entry; fall back to any entry only if all are blocked.
            let victim = state
                .attempts
                .iter()
                .find(|(_, attempt)| {
                    attempt
                        .blocked_until
                        .map(|until| until <= now)
                        .unwrap_or(true)
                })
                .map(|(key, _)| key.clone())
                .or_else(|| state.attempts.keys().next().cloned());
            if let Some(victim) = victim {
                state.attempts.remove(&victim);
            }
        }

        let entry = state.attempts.entry(key.to_string()).or_insert(Attempt {
            failures: 0,
            blocked_until: None,
            window_start: now,
        });
        entry.failures += 1;
        if entry.failures >= MAX_LOGIN_ATTEMPTS {
            entry.blocked_until = Some(now + LOGIN_BLOCK_DURATION);
        }
    }

    pub fn record_success(&self, key: &str) {
        self.state
            .lock()
            .expect("login limiter lock")
            .attempts
            .remove(key);
    }

    fn cleanup(state: &mut LoginLimiterState, now: Instant) {
        let should_cleanup = state
            .last_cleanup
            .map(|last| now.duration_since(last) >= LOGIN_CLEANUP_INTERVAL)
            .unwrap_or(true);
        if !should_cleanup {
            return;
        }

        state.attempts.retain(|_, attempt| {
            attempt
                .blocked_until
                .map(|until| until > now)
                .unwrap_or_else(|| now.duration_since(attempt.window_start) < LOGIN_ATTEMPT_TTL)
        });
        state.last_cleanup = Some(now);
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

    #[test]
    fn try_acquire_counts_attempts_before_they_finish() {
        // Concurrent logins must not all pass the check before any failure is
        // recorded, so the slot is taken up front.
        let limiter = LoginLimiter::default();
        let key = "127.0.0.1";
        for _ in 0..5 {
            assert!(limiter.try_acquire(key));
        }
        assert!(!limiter.try_acquire(key));
    }

    #[test]
    fn attempts_expire_after_the_window() {
        let limiter = LoginLimiter::default();
        let key = "comment:127.0.0.1";
        let start = Instant::now();
        for _ in 0..4 {
            assert!(limiter.try_acquire_at(key, start));
        }
        let later = start + LOGIN_ATTEMPT_TTL + Duration::from_secs(1);
        for _ in 0..4 {
            assert!(limiter.try_acquire_at(key, later));
        }
    }

    #[test]
    fn successful_login_resets_acquired_attempts() {
        let limiter = LoginLimiter::default();
        let key = "127.0.0.1";
        for _ in 0..4 {
            assert!(limiter.try_acquire(key));
        }
        limiter.record_success(key);
        for _ in 0..5 {
            assert!(limiter.try_acquire(key));
        }
    }

    #[tokio::test]
    async fn login_password_check_verifies_off_the_async_runtime() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(
            check_login_password("correct horse battery staple".into(), Some(hash.clone())).await
        );
        assert!(!check_login_password("wrong".into(), Some(hash)).await);
    }

    #[tokio::test]
    async fn login_password_check_rejects_unknown_users() {
        assert!(!check_login_password("anything".into(), None).await);
    }

    #[test]
    fn dummy_hash_uses_the_same_parameters_as_real_hashes() {
        let real = hash_password("x").unwrap();
        let params = |hash: &str| hash.rsplitn(3, '$').nth(2).unwrap_or_default().to_string();
        assert_eq!(params(&real), params(&DUMMY_PASSWORD_HASH));
    }

    #[test]
    fn limiter_storage_is_bounded_for_unique_keys() {
        let limiter = LoginLimiter::default();
        for index in 0..(MAX_LOGIN_KEYS + 100) {
            limiter.record_failure(&format!("user-{index}:127.0.0.1"));
        }
        let state = limiter.state.lock().expect("login limiter lock");
        assert!(state.attempts.len() <= MAX_LOGIN_KEYS);
    }
}
