use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::Router;

use crate::auth::{LoginLimiter, purge_expired_sessions};
use crate::config::Config;
use crate::db;
use crate::seed;
use crate::state::AppState;
use crate::storage::{LocalStorage, Storage};

pub async fn build_state(config: Config) -> Result<AppState> {
    let uploads_dir = config.uploads_dir();
    let pool = db::connect(&config).await?;
    seed::run(&pool, &config).await?;

    let storage: Arc<dyn Storage> = Arc::new(LocalStorage::new(uploads_dir));
    Ok(AppState {
        db: pool,
        config: Arc::new(config),
        storage,
        login_limiter: Arc::new(LoginLimiter::default()),
        comment_limiter: Arc::new(LoginLimiter::default()),
    })
}

pub fn router(state: AppState) -> Router {
    crate::web::router(state)
}

pub fn spawn_maintenance(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = purge_expired_sessions(&state.db).await {
                tracing::warn!(error = ?error, "session cleanup failed");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}
