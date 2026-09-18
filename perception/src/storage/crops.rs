//! Crop image persistence to the local filesystem.
//!
//! Crops are organized by date: `{base_dir}/{YYYY-MM-DD}/{event_id}.jpg`.

use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::error::{PerceptionError, Result};

/// Writes detection crop images to a date-partitioned directory tree.
pub struct CropWriter {
    base_dir: PathBuf,
}

impl CropWriter {
    /// Create a new writer rooted at `base_dir`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Persist `image_data` for the given event, returning the **relative** path
    /// from `base_dir` (e.g. `2026-04-19/some-uuid.jpg`).
    ///
    /// Parent directories are created on demand.
    pub fn save(&self, event_id: &Uuid, image_data: &[u8]) -> Result<PathBuf> {
        let date_str = Utc::now().format("%Y-%m-%d").to_string();
        let relative = PathBuf::from(&date_str).join(format!("{event_id}.jpg"));
        let full_path = self.base_dir.join(&relative);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PerceptionError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        std::fs::write(&full_path, image_data).map_err(|e| PerceptionError::Io {
            path: full_path,
            source: e,
        })?;

        Ok(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_creates_file_at_expected_path() {
        let tmp = TempDir::new().unwrap();
        let writer = CropWriter::new(tmp.path());
        let id = Uuid::new_v4();
        let data = b"fake-jpeg-data";

        let rel = writer.save(&id, data).unwrap();

        // Relative path has date directory + uuid.jpg
        let today = Utc::now().format("%Y-%m-%d").to_string();
        assert_eq!(rel, PathBuf::from(&today).join(format!("{id}.jpg")));

        // File actually exists and contains the bytes we wrote.
        let full = tmp.path().join(&rel);
        assert!(full.exists());
        assert_eq!(std::fs::read(&full).unwrap(), data);
    }

    #[test]
    fn save_creates_nested_directories() {
        let tmp = TempDir::new().unwrap();
        // Use a subdirectory that doesn't exist yet.
        let nested = tmp.path().join("a").join("b");
        let writer = CropWriter::new(&nested);
        let id = Uuid::new_v4();

        let rel = writer.save(&id, b"x").unwrap();
        assert!(nested.join(&rel).exists());
    }
}
