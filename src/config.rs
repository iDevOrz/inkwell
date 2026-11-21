use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};

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

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            format!(
                "sqlite://{}/typecho.db",
                data_dir.to_string_lossy().trim_end_matches('/')
            )
        });

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

        let mcp_token_scopes = env::var("MCP_TOKEN_SCOPES")
            .unwrap_or_else(|_| "read,write,review".to_string())
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        let secure_cookies = env::var("COOKIE_SECURE")
            .ok()
            .map(|value| parse_bool(&value))
            .unwrap_or_else(|| site_url.starts_with("https://"));

        let trust_proxy_headers = env::var("TRUST_PROXY_HEADERS")
            .ok()
            .map(|value| parse_bool(&value))
            .unwrap_or(false);

        let session_days = env::var("SESSION_DAYS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(14);

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

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
