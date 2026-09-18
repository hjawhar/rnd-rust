//! Pluggable event persistence layer.
//!
//! All storage backends implement [`StorageBackend`]. Use [`create_storage`]
//! to instantiate the backend described in [`StorageConfig`].

pub mod crops;
pub mod postgres;
pub mod sqlite;

use std::path::PathBuf;

use uuid::Uuid;

use crate::config::StorageConfig;
use crate::error::{PerceptionError, Result};
use crate::types::{Event, EventFilter};

/// Trait implemented by every storage backend (SQLite, Postgres, etc.).
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    /// Persist a batch of events. An empty slice is a no-op.
    async fn store_events(&self, events: &[Event]) -> Result<()>;

    /// Save a crop image for `event_id`, returning the relative path to the file.
    async fn store_crop(&self, event_id: &Uuid, image_data: &[u8]) -> Result<PathBuf>;

    /// Query persisted events matching `filter`.
    async fn query_events(&self, filter: &EventFilter) -> Result<Vec<Event>>;
}

/// Construct the storage backend described by `config`.
pub async fn create_storage(config: &StorageConfig) -> Result<Box<dyn StorageBackend>> {
    match config.backend.as_str() {
        "sqlite" => {
            let storage =
                sqlite::SqliteStorage::new(&config.sqlite_path, &config.crops_dir).await?;
            Ok(Box::new(storage))
        }
        other => Err(PerceptionError::Config(format!(
            "unsupported storage backend '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_storage_rejects_unknown_backend() {
        let config = StorageConfig {
            backend: "redis".into(),
            ..Default::default()
        };
        let result = create_storage(&config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("unsupported storage backend"));
    }
}
