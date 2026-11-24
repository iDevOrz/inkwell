use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::repo::token::SCOPES;

const DEFAULT_SESSION_DAYS: i64 = 14;
const DEFAULT_MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
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
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Builds the configuration from any key/value source; `from_env` passes
    /// the process environment, tests pass a map.
    pub fn from_lookup(var: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let data_dir = var("DATA_DIR").unwrap_or_else(|| "data".to_string());
        let data_dir = PathBuf::from(data_dir);

        let database_url = var("DATABASE_URL").unwrap_or_else(|| default_database_url(&data_dir));

        let bind_addr = var("BIND_ADDR")
            .unwrap_or_else(|| "127.0.0.1:3000".to_string())
            .parse()
            .context("BIND_ADDR must be a valid socket address")?;

        let site_url = var("SITE_URL")
            .unwrap_or_else(|| "http://127.0.0.1:3000".to_string())
            .trim_end_matches('/')
            .to_string();

        let admin_user = var("ADMIN_USER").unwrap_or_else(|| "admin".to_string());
        let admin_password = var("ADMIN_PASSWORD").filter(|value| !value.is_empty());

        let mcp_token = var("MCP_TOKEN").filter(|value| !value.is_empty());

        let mcp_token_scopes = parse_scopes(
            &var("MCP_TOKEN_SCOPES").unwrap_or_else(|| "read,write,review".to_string()),
        )?;

        let secure_cookies = var("COOKIE_SECURE")
            .map(|value| parse_bool(&value))
            .unwrap_or_else(|| site_url.starts_with("https://"));

        let trust_proxy_headers = var("TRUST_PROXY_HEADERS")
            .map(|value| parse_bool(&value))
            .unwrap_or(false);

        let session_days = parse_session_days(var("SESSION_DAYS").as_deref())?;

        let max_upload_bytes = match var("MAX_UPLOAD_BYTES") {
            Some(value) => value
                .trim()
                .parse()
                .context("MAX_UPLOAD_BYTES must be a byte count")?,
            None => DEFAULT_MAX_UPLOAD_BYTES,
        };

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

    fn config_from(pairs: &[(&str, &str)]) -> Result<Config> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Config::from_lookup(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_from(&[("DATA_DIR", dir.path().to_str().unwrap())]).unwrap();

        assert_eq!(config.bind_addr.to_string(), "127.0.0.1:3000");
        assert_eq!(config.site_url, "http://127.0.0.1:3000");
        assert_eq!(config.admin_user, "admin");
        assert_eq!(config.admin_password, None);
        assert_eq!(config.mcp_token, None);
        assert_eq!(config.mcp_token_scopes, vec!["read", "write", "review"]);
        assert!(!config.secure_cookies);
        assert!(!config.trust_proxy_headers);
        assert_eq!(config.session_days, DEFAULT_SESSION_DAYS);
        assert_eq!(config.max_upload_bytes, DEFAULT_MAX_UPLOAD_BYTES);
        assert!(config.database_url.ends_with("/inkwell.db"));
    }

    #[test]
    fn explicit_values_are_parsed() {
        let config = config_from(&[
            ("BIND_ADDR", "0.0.0.0:8080"),
            ("SITE_URL", "https://blog.example/"),
            ("DATABASE_URL", "sqlite::memory:"),
            ("ADMIN_USER", "owner"),
            ("ADMIN_PASSWORD", "secret-pass"),
            ("MCP_TOKEN", "tok"),
            ("MCP_TOKEN_SCOPES", "read,publish"),
            ("TRUST_PROXY_HEADERS", "yes"),
            ("SESSION_DAYS", "30"),
            ("MAX_UPLOAD_BYTES", "2048"),
        ])
        .unwrap();

        assert_eq!(config.bind_addr.to_string(), "0.0.0.0:8080");
        assert_eq!(
            config.site_url, "https://blog.example",
            "trailing slash trimmed"
        );
        assert!(config.secure_cookies, "https site implies Secure cookies");
        assert_eq!(config.database_url, "sqlite::memory:");
        assert_eq!(config.admin_user, "owner");
        assert_eq!(config.admin_password.as_deref(), Some("secret-pass"));
        assert_eq!(config.mcp_token.as_deref(), Some("tok"));
        assert_eq!(config.mcp_token_scopes, vec!["read", "publish"]);
        assert!(config.trust_proxy_headers);
        assert_eq!(config.session_days, 30);
        assert_eq!(config.max_upload_bytes, 2048);
    }

    #[test]
    fn empty_secrets_are_treated_as_unset_and_cookie_flag_overrides() {
        let config = config_from(&[
            ("ADMIN_PASSWORD", ""),
            ("MCP_TOKEN", ""),
            ("SITE_URL", "https://blog.example"),
            ("COOKIE_SECURE", "false"),
            ("DATABASE_URL", "sqlite::memory:"),
        ])
        .unwrap();

        assert_eq!(config.admin_password, None);
        assert_eq!(config.mcp_token, None);
        assert!(!config.secure_cookies);
    }

    #[test]
    fn invalid_values_fail_fast() {
        for (key, value) in [
            ("BIND_ADDR", "not-an-address"),
            ("MCP_TOKEN_SCOPES", "read,wirte"),
            ("SESSION_DAYS", "0"),
            ("MAX_UPLOAD_BYTES", "ten megabytes"),
        ] {
            let result = config_from(&[(key, value), ("DATABASE_URL", "sqlite::memory:")]);
            assert!(result.is_err(), "{key}={value} should be rejected");
        }
    }

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
