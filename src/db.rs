use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::config::Config;

pub async fn connect(config: &Config) -> Result<SqlitePool> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("create data dir {}", config.data_dir.display()))?;
    std::fs::create_dir_all(config.uploads_dir())
        .with_context(|| format!("create uploads dir {}", config.uploads_dir().display()))?;
    restrict_data_directory(&config.data_dir)?;
    restrict_data_directory(&config.uploads_dir())?;

    let options = SqliteConnectOptions::from_str(&config.database_url)
        .with_context(|| format!("invalid DATABASE_URL: {}", config.database_url))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));

    let max_connections = if config.database_url.contains(":memory:") {
        1
    } else {
        5
    };

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .context("connect to sqlite")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("run database migrations")?;

    Ok(pool)
}

/// Starts a transaction that will write. `BEGIN IMMEDIATE` takes the write
/// lock up front, so concurrent writers wait (up to the busy timeout) instead
/// of failing with SQLITE_BUSY when a read snapshot cannot be upgraded.
pub async fn begin_write(pool: &SqlitePool) -> sqlx::Result<Transaction<'static, Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE").await
}

#[cfg(unix)]
fn restrict_data_directory(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("restrict data directory permissions {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_data_directory(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
