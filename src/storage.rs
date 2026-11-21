use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::error::AppError;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Vec<u8>>;
    async fn delete(&self, key: &str) -> Result<()>;
    fn url(&self, key: &str) -> String;
}

pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn safe_path(&self, key: &str) -> Result<PathBuf, AppError> {
        let path = Path::new(key);
        if key.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(AppError::bad_request("非法的文件路径"));
        }
        Ok(self.root.join(path))
    }
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self
            .safe_path(key)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create upload dir {}", parent.display()))?;
        }
        tokio::fs::write(&path, bytes)
            .await
            .with_context(|| format!("write upload {}", path.display()))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = self
            .safe_path(key)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        tokio::fs::read(&path)
            .await
            .with_context(|| format!("read upload {}", path.display()))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self
            .safe_path(key)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("delete upload {}", path.display())),
        }
    }

    fn url(&self, key: &str) -> String {
        format!("/media/{key}")
    }
}
