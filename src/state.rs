use std::sync::Arc;

use sqlx::SqlitePool;

use crate::auth::LoginLimiter;
use crate::config::Config;
use crate::storage::Storage;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub storage: Arc<dyn Storage>,
    pub login_limiter: Arc<LoginLimiter>,
    /// Separate from logins so comment traffic cannot evict or block login state.
    pub comment_limiter: Arc<LoginLimiter>,
}
