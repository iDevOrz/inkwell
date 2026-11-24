use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::repo::token::SCOPES;

const DEFAULT_SESSION_DAYS: i64 = 14;
const MAX_SESSION_DAYS: i64 = 365;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub site_url: String,
    pub admin_user: String,
    pub admin_password: Option<String>,
    pub mcp_token: Option<String>,
    pub mcp_token_scopes: Vec<String>,
    pub secure_cookies: bool,
    pub trust_proxy_headers: bool,
    pub session_days: i64,
    pub max_upload_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
        let data_dir = PathBuf::from(data_dir);

        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| default_database_url(&data_dir));

        let bind_addr = env::var("BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
            .parse()
            .context("BIND_ADDR must be a valid socket address")?;

        let site_url = env::var("SITE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string())
            .trim_end_matches('/')
            .to_string();

        let admin_user = env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string());
        let admin_password = env::var("ADMIN_PASSWORD")
            .ok()
            .filter(|value| !value.is_empty());

        let mcp_token = env::var("MCP_TOKEN").ok().filter(|value| !value.is_empty());

        let mcp_token_scopes = parse_scopes(
            &env::var("MCP_TOKEN_SCOPES").unwrap_or_else(|_| "read,write,review".to_string()),
        )?;

        let secure_cookies = env::var("COOKIE_SECURE")
            .ok()
            .map(|value| parse_bool(&value))
            .unwrap_or_else(|| site_url.starts_with("https://"));

        let trust_proxy_headers = env::var("TRUST_PROXY_HEADERS")
            .ok()
            .map(|value| parse_bool(&value))
            .unwrap_or(false);

        let session_days = parse_session_days(env::var("SESSION_DAYS").ok().as_deref())?;

        let max_upload_bytes = env::var("MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10 * 1024 * 1024);

        Ok(Self {
            bind_addr,
            database_url,
            data_dir,
            site_url,
            admin_user,
            admin_password,
            mcp_token,
            mcp_token_scopes,
            secure_cookies,
            trust_proxy_headers,
            session_days,
            max_upload_bytes,
        })
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }
}

/// Rejects unknown scopes so a typo fails at startup instead of silently
/// producing a token without the intended permissions.
fn parse_scopes(value: &str) -> Result<Vec<String>> {
    let scopes: Vec<String> = value
        .split(',')
        .map(|scope| scope.trim().to_string())
        .filter(|scope| !scope.is_empty())
        .collect();
    if let Some(unknown) = scopes
        .iter()
        .find(|scope| !SCOPES.contains(&scope.as_str()))
    {
        bail!(
            "MCP_TOKEN_SCOPES contains unknown scope `{unknown}`; allowed: {}",
            SCOPES.join(",")
        );
    }
    Ok(scopes)
}

/// The same value feeds the session row and the cookie Max-Age, so keep it in
/// a range where both are valid (0 or negative would expire the cookie at once).
fn parse_session_days(value: Option<&str>) -> Result<i64> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(DEFAULT_SESSION_DAYS);
    };
    match value.parse::<i64>() {
        Ok(days) if (1..=MAX_SESSION_DAYS).contains(&days) => Ok(days),
        _ => bail!("SESSION_DAYS must be an integer between 1 and {MAX_SESSION_DAYS}"),
    }
}

const DATABASE_FILE: &str = "inkwell.db";
/// Default file name before the rename. Existing installs keep using it; it
/// is never renamed automatically because WAL/SHM files travel with it.
const LEGACY_DATABASE_FILE: &str = "typecho.db";

fn default_database_url(data_dir: &std::path::Path) -> String {
    let current = data_dir.join(DATABASE_FILE);
    let legacy = data_dir.join(LEGACY_DATABASE_FILE);
    let file = if !current.exists() && legacy.exists() {
        tracing::warn!(
            path = %legacy.display(),
            "using legacy database file name; stop the server and rename it (with any -wal/-shm files) to {DATABASE_FILE}"
        );
        LEGACY_DATABASE_FILE
    } else {
        DATABASE_FILE
    };
    format!(
        "sqlite://{}/{file}",
        data_dir.to_string_lossy().trim_end_matches('/')
    )
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_database_prefers_the_current_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let url = |dir: &std::path::Path| default_database_url(dir);

        assert!(url(dir.path()).ends_with("/inkwell.db"));

        std::fs::write(dir.path().join("typecho.db"), b"").unwrap();
        assert!(
            url(dir.path()).ends_with("/typecho.db"),
            "existing installs keep their legacy file"
        );

        std::fs::write(dir.path().join("inkwell.db"), b"").unwrap();
        assert!(url(dir.path()).ends_with("/inkwell.db"));
    }

    #[test]
    fn unknown_mcp_scope_is_rejected() {
        assert!(parse_scopes("read,wirte").is_err());
        assert_eq!(
            parse_scopes(" read, write ,").unwrap(),
            vec!["read", "write"]
        );
    }

    #[test]
    fn session_days_must_be_in_range() {
        assert_eq!(parse_session_days(None).unwrap(), DEFAULT_SESSION_DAYS);
        assert_eq!(parse_session_days(Some("30")).unwrap(), 30);
        assert!(parse_session_days(Some("0")).is_err());
        assert!(parse_session_days(Some("-1")).is_err());
        assert!(parse_session_days(Some("100000000000")).is_err());
        assert!(parse_session_days(Some("abc")).is_err());
    }
}
