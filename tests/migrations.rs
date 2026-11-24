//! Upgrade-path tests: a database created by an older release (only the first
//! N migrations applied, holding data in that release's schema) must start
//! cleanly on the current release with its data intact.
mod common;

use std::path::{Path, PathBuf};

use common::test_config;
use inkwell::app;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tempfile::TempDir;

const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

fn migration_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(MIGRATIONS_DIR)
        .expect("migrations dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    files
}

/// A database file that has only the first `applied` migrations, as an older
/// release would have left it.
struct LegacyDatabase {
    dir: TempDir,
    pool: SqlitePool,
}

impl LegacyDatabase {
    async fn at_version(applied: usize) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let old_migrations = dir.path().join("old-migrations");
        std::fs::create_dir(&old_migrations).expect("mkdir");
        for file in migration_files().iter().take(applied) {
            std::fs::copy(file, old_migrations.join(file.file_name().expect("name")))
                .expect("copy migration");
        }

        let options = SqliteConnectOptions::new()
            .filename(Self::db_path(dir.path()))
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(options).await.expect("connect");
        Migrator::new(old_migrations.as_path())
            .await
            .expect("load old migrations")
            .run(&pool)
            .await
            .expect("apply old migrations");
        Self { dir, pool }
    }

    fn db_path(dir: &Path) -> PathBuf {
        dir.join("legacy.db")
    }

    async fn exec(&self, sql: &'static str) {
        sqlx::query(sql).execute(&self.pool).await.expect(sql);
    }

    /// Data every schema version has had since 0001.
    async fn seed_core_rows(&self) {
        self.exec(
            "INSERT INTO users(id, username, password_hash, display_name, role, created_at) \
             VALUES(1, 'legacy', 'x', 'Legacy', 'admin', 1)",
        )
        .await;
        self.exec(
            "INSERT INTO contents(id, kind, title, slug, body, status, author_id, created_at, updated_at) \
             VALUES(10, 'post', '旧文章', 'old-post', '旧正文 [[旧笔记]]', 'published', 1, 1, 2), \
                   (11, 'note', '旧笔记', 'old-note', '笔记', 'inbox', 1, 1, 2)",
        )
        .await;
        self.exec(
            "INSERT INTO revisions(id, content_id, title, body, status, actor, created_at) \
             VALUES(100, 10, '旧文章', '旧正文 [[旧笔记]]', 'published', 'human:1', 2)",
        )
        .await;
        self.exec("INSERT INTO links(source_id, target_title, target_id) VALUES(10, '旧笔记', 11)")
            .await;
    }

    /// Starts the current release on this database (migrations + seed).
    async fn upgrade(self) -> (TempDir, SqlitePool) {
        self.pool.close().await;
        let mut config = test_config(self.dir.path());
        config.database_url = format!("sqlite://{}", Self::db_path(self.dir.path()).display());
        let state = app::build_state(config).await.expect("upgrade starts");
        (self.dir, state.db)
    }
}

async fn scalar_i64(pool: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

async fn scalar_text(pool: &SqlitePool, sql: &'static str) -> String {
    sqlx::query_scalar(sql).fetch_one(pool).await.expect(sql)
}

#[tokio::test]
async fn every_older_schema_upgrades_with_core_data_intact() {
    let total = migration_files().len();
    for applied in 1..total {
        let legacy = LegacyDatabase::at_version(applied).await;
        legacy.seed_core_rows().await;
        let (_dir, pool) = legacy.upgrade().await;

        let label = format!("upgrade from {applied} of {total} migrations");
        assert_eq!(
            scalar_i64(&pool, "SELECT COUNT(*) FROM _sqlx_migrations").await,
            total as i64,
            "{label}"
        );
        assert_eq!(
            scalar_text(&pool, "SELECT body FROM contents WHERE id = 10").await,
            "旧正文 [[旧笔记]]",
            "{label}"
        );
        assert_eq!(
            scalar_i64(
                &pool,
                "SELECT COUNT(*) FROM revisions WHERE content_id = 10"
            )
            .await,
            1,
            "{label}"
        );
        assert_eq!(
            scalar_i64(&pool, "SELECT target_id FROM links WHERE source_id = 10").await,
            11,
            "{label}"
        );
        // The existing user is kept and seeding does not add a second admin.
        assert_eq!(
            scalar_i64(&pool, "SELECT COUNT(*) FROM users").await,
            1,
            "{label}"
        );
        let violations = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .expect("foreign_key_check");
        assert!(violations.is_empty(), "{label}: foreign key violations");
    }
}

#[tokio::test]
async fn proposals_from_before_versioning_get_a_first_version() {
    // 0004 introduced proposals; 0005 added immutable versions and must
    // backfill version 1 for proposals that already existed.
    let legacy = LegacyDatabase::at_version(4).await;
    legacy.seed_core_rows().await;
    legacy
        .exec(
            "INSERT INTO content_proposals(id, content_id, base_revision_id, base_updated_at, \
                 title, body, summary, status, author, created_at) \
             VALUES(7, 10, 100, 2, '旧文章', 'AI 改写', '压缩', 'proposed', 'mcp:env', 3)",
        )
        .await;
    let (_dir, pool) = legacy.upgrade().await;

    assert_eq!(
        scalar_i64(&pool, "SELECT version FROM content_proposals WHERE id = 7").await,
        1
    );
    assert_eq!(
        scalar_text(
            &pool,
            "SELECT body FROM content_proposal_versions WHERE proposal_id = 7 AND version = 1"
        )
        .await,
        "AI 改写"
    );
}

#[tokio::test]
async fn foreign_key_indexes_exist_after_upgrade() {
    let legacy = LegacyDatabase::at_version(5).await;
    legacy.seed_core_rows().await;
    let (_dir, pool) = legacy.upgrade().await;

    for index in [
        "contents_promoted_to",
        "content_proposals_base_revision",
        "content_proposals_applied_revision",
        "links_pending_title",
    ] {
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(index)
        .fetch_one(&pool)
        .await
        .expect("index lookup");
        assert_eq!(found, 1, "missing index {index}");
    }
}
