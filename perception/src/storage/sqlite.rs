//! SQLite-backed event storage using sqlx.

use std::path::Path;
use std::path::PathBuf;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::debug;
use uuid::Uuid;

use crate::error::{PerceptionError, Result};
use crate::types::{BBox, DetectionKind, Event, EventFilter};

use super::crops::CropWriter;
use super::StorageBackend;

/// SQLite event store with WAL journaling.
pub struct SqliteStorage {
    pool: SqlitePool,
    crop_writer: CropWriter,
}

impl SqliteStorage {
    /// Open (or create) a SQLite database at `path` with WAL mode enabled.
    ///
    /// Creates the `events` table and indices if they do not already exist.
    pub async fn new(path: &Path, crops_dir: &Path) -> Result<Self> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PerceptionError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;

        let storage = Self {
            pool,
            crop_writer: CropWriter::new(crops_dir),
        };
        storage.create_tables().await?;

        debug!("sqlite storage opened at {}", path.display());
        Ok(storage)
    }

    /// Variant for tests: wraps an already-connected pool.
    #[cfg(test)]
    async fn from_pool(pool: SqlitePool) -> Result<Self> {
        let storage = Self {
            pool,
            crop_writer: CropWriter::new(Path::new("/tmp/crops")),
        };
        storage.create_tables().await?;
        Ok(storage)
    }

    async fn create_tables(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id              TEXT PRIMARY KEY,
                timestamp       TEXT NOT NULL,
                frame_number    INTEGER NOT NULL,
                track_id        INTEGER,
                detection_kind  TEXT NOT NULL,
                label           TEXT,
                confidence      REAL NOT NULL,
                bbox_x1         REAL NOT NULL,
                bbox_y1         REAL NOT NULL,
                bbox_x2         REAL NOT NULL,
                bbox_y2         REAL NOT NULL,
                ocr_text        TEXT,
                face_identity   TEXT,
                face_similarity REAL,
                crop_path       TEXT,
                synced          INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Indices for common query patterns.
        for stmt in [
            "CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);",
            "CREATE INDEX IF NOT EXISTS idx_events_track_id ON events(track_id);",
            "CREATE INDEX IF NOT EXISTS idx_events_label ON events(label);",
            "CREATE INDEX IF NOT EXISTS idx_events_synced ON events(synced);",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }

        Ok(())
    }
}

/// Parse a `DetectionKind` from its stored text representation.
fn parse_detection_kind(s: &str) -> Result<DetectionKind> {
    match s {
        "object" => Ok(DetectionKind::Object),
        "face" => Ok(DetectionKind::Face),
        "text" => Ok(DetectionKind::Text),
        other => Err(PerceptionError::Storage(format!(
            "unknown detection_kind '{other}'"
        ))),
    }
}

#[async_trait::async_trait]
impl StorageBackend for SqliteStorage {
    async fn store_events(&self, events: &[Event]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        for event in events {
            sqlx::query(
                r#"
                INSERT INTO events (
                    id, timestamp, frame_number, track_id, detection_kind,
                    label, confidence, bbox_x1, bbox_y1, bbox_x2, bbox_y2,
                    ocr_text, face_identity, face_similarity, crop_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                "#,
            )
            .bind(event.id.to_string())
            .bind(event.timestamp.to_rfc3339())
            .bind(event.frame_number as i64)
            .bind(event.track_id.map(|v| v as i64))
            .bind(event.detection_kind.to_string())
            .bind(event.label.as_deref())
            .bind(event.confidence)
            .bind(event.bbox.x1)
            .bind(event.bbox.y1)
            .bind(event.bbox.x2)
            .bind(event.bbox.y2)
            .bind(event.ocr_text.as_deref())
            .bind(event.face_identity.as_deref())
            .bind(event.face_similarity)
            .bind(event.crop_path.as_deref())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn store_crop(&self, event_id: &Uuid, image_data: &[u8]) -> Result<PathBuf> {
        self.crop_writer.save(event_id, image_data)
    }

    async fn query_events(&self, filter: &EventFilter) -> Result<Vec<Event>> {
        let mut sql = String::from("SELECT * FROM events WHERE 1=1");
        struct BindValue {
            text: Option<String>,
            integer: Option<i64>,
            kind: BindKind,
        }
        #[derive(Clone, Copy)]
        enum BindKind {
            Text,
            Integer,
        }

        let mut binds: Vec<BindValue> = Vec::new();
        let mut param_idx = 0u32;

        if let Some(kind) = &filter.kind {
            param_idx += 1;
            sql.push_str(&format!(" AND detection_kind = ?{param_idx}"));
            binds.push(BindValue {
                text: Some(kind.to_string()),
                integer: None,
                kind: BindKind::Text,
            });
        }

        if let Some(label) = &filter.label {
            param_idx += 1;
            sql.push_str(&format!(" AND label = ?{param_idx}"));
            binds.push(BindValue {
                text: Some(label.clone()),
                integer: None,
                kind: BindKind::Text,
            });
        }

        if let Some(track_id) = filter.track_id {
            param_idx += 1;
            sql.push_str(&format!(" AND track_id = ?{param_idx}"));
            binds.push(BindValue {
                text: None,
                integer: Some(track_id as i64),
                kind: BindKind::Integer,
            });
        }

        if let Some(after) = &filter.after {
            param_idx += 1;
            sql.push_str(&format!(" AND timestamp > ?{param_idx}"));
            binds.push(BindValue {
                text: Some(after.to_rfc3339()),
                integer: None,
                kind: BindKind::Text,
            });
        }

        if let Some(before) = &filter.before {
            param_idx += 1;
            sql.push_str(&format!(" AND timestamp < ?{param_idx}"));
            binds.push(BindValue {
                text: Some(before.to_rfc3339()),
                integer: None,
                kind: BindKind::Text,
            });
        }

        sql.push_str(" ORDER BY timestamp DESC");

        if let Some(limit) = filter.limit {
            param_idx += 1;
            sql.push_str(&format!(" LIMIT ?{param_idx}"));
            binds.push(BindValue {
                text: None,
                integer: Some(limit as i64),
                kind: BindKind::Integer,
            });
        }

        let mut query = sqlx::query(&sql);
        for b in &binds {
            match b.kind {
                BindKind::Text => {
                    query = query.bind(b.text.as_deref());
                }
                BindKind::Integer => {
                    query = query.bind(b.integer);
                }
            }
        }

        let rows = query.fetch_all(&self.pool).await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in &rows {
            let id_str: String = row.get("id");
            let ts_str: String = row.get("timestamp");
            let kind_str: String = row.get("detection_kind");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| PerceptionError::Storage(format!("invalid uuid: {e}")))?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(&ts_str)
                .map_err(|e| PerceptionError::Storage(format!("invalid timestamp: {e}")))?
                .with_timezone(&chrono::Utc);

            events.push(Event {
                id,
                timestamp,
                frame_number: row.get::<i64, _>("frame_number") as u64,
                track_id: row
                    .get::<Option<i64>, _>("track_id")
                    .map(|v| v as u64),
                detection_kind: parse_detection_kind(&kind_str)?,
                label: row.get("label"),
                confidence: row.get("confidence"),
                bbox: BBox {
                    x1: row.get("bbox_x1"),
                    y1: row.get("bbox_y1"),
                    x2: row.get("bbox_x2"),
                    y2: row.get("bbox_y2"),
                },
                ocr_text: row.get("ocr_text"),
                face_identity: row.get("face_identity"),
                face_similarity: row.get("face_similarity"),
                crop_path: row.get("crop_path"),
            });
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Helper: create an in-memory SqliteStorage.
    async fn mem_storage() -> SqliteStorage {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        SqliteStorage::from_pool(pool).await.unwrap()
    }

    /// Helper: build a minimal Event for testing.
    fn make_event(kind: DetectionKind, label: Option<&str>) -> Event {
        Event {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            frame_number: 1,
            track_id: None,
            detection_kind: kind,
            label: label.map(String::from),
            confidence: 0.9,
            bbox: BBox::new(0.0, 0.0, 100.0, 100.0),
            ocr_text: None,
            face_identity: None,
            face_similarity: None,
            crop_path: None,
        }
    }

    #[tokio::test]
    async fn store_and_query_single_event() {
        let store = mem_storage().await;
        let event = make_event(DetectionKind::Object, Some("person"));

        store.store_events(&[event.clone()]).await.unwrap();

        let results = store
            .query_events(&EventFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, event.id);
        assert_eq!(results[0].label.as_deref(), Some("person"));
    }

    #[tokio::test]
    async fn store_batch_of_100() {
        let store = mem_storage().await;
        let events: Vec<Event> = (0..100)
            .map(|i| {
                let mut e = make_event(DetectionKind::Object, Some("car"));
                e.frame_number = i;
                e
            })
            .collect();

        store.store_events(&events).await.unwrap();

        let results = store
            .query_events(&EventFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 100);
    }

    #[tokio::test]
    async fn store_empty_slice_is_noop() {
        let store = mem_storage().await;
        store.store_events(&[]).await.unwrap();

        let results = store
            .query_events(&EventFilter::default())
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn filter_by_kind() {
        let store = mem_storage().await;
        store
            .store_events(&[
                make_event(DetectionKind::Object, Some("person")),
                make_event(DetectionKind::Face, None),
                make_event(DetectionKind::Text, None),
            ])
            .await
            .unwrap();

        let filter = EventFilter {
            kind: Some(DetectionKind::Face),
            ..Default::default()
        };
        let results = store.query_events(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].detection_kind, DetectionKind::Face);
    }

    #[tokio::test]
    async fn filter_by_label() {
        let store = mem_storage().await;
        store
            .store_events(&[
                make_event(DetectionKind::Object, Some("person")),
                make_event(DetectionKind::Object, Some("car")),
                make_event(DetectionKind::Object, Some("person")),
            ])
            .await
            .unwrap();

        let filter = EventFilter {
            label: Some("car".into()),
            ..Default::default()
        };
        let results = store.query_events(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].label.as_deref(), Some("car"));
    }

    #[tokio::test]
    async fn empty_filter_returns_all() {
        let store = mem_storage().await;
        store
            .store_events(&[
                make_event(DetectionKind::Object, Some("a")),
                make_event(DetectionKind::Face, Some("b")),
            ])
            .await
            .unwrap();

        let results = store
            .query_events(&EventFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn filter_with_limit() {
        let store = mem_storage().await;
        let events: Vec<Event> = (0..10)
            .map(|_| make_event(DetectionKind::Object, None))
            .collect();
        store.store_events(&events).await.unwrap();

        let filter = EventFilter {
            limit: Some(3),
            ..Default::default()
        };
        let results = store.query_events(&filter).await.unwrap();
        assert_eq!(results.len(), 3);
    }
}
