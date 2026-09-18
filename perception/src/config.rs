//! Configuration loading and validation.
//!
//! Configuration is read from a TOML file. All sections have sensible defaults
//! so a minimal config (just `[capture]` with a source) is sufficient.

use crate::error::{PerceptionError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub capture: CaptureConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub preview: PreviewConfig,
    #[serde(default)]
    pub models: ModelsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub source: String,
    pub path: String,
    #[serde(default = "default_fps_limit")]
    pub fps_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    #[serde(default = "default_true")]
    pub detection: bool,
    #[serde(default)]
    pub face_recognition: bool,
    #[serde(default)]
    pub ocr: bool,
    #[serde(default = "default_confidence")]
    pub confidence_threshold: f32,
    #[serde(default)]
    pub detection_config: DetectionConfig,
    #[serde(default)]
    pub face_config: FaceConfig,
    #[serde(default)]
    pub ocr_config: OcrConfig,
    #[serde(default)]
    pub tracker_config: TrackerConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            detection: true,
            face_recognition: false,
            ocr: false,
            confidence_threshold: default_confidence(),
            detection_config: DetectionConfig::default(),
            face_config: FaceConfig::default(),
            ocr_config: OcrConfig::default(),
            tracker_config: TrackerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    #[serde(default = "default_detection_model")]
    pub model: String,
    #[serde(default)]
    pub classes: Vec<String>,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            model: default_detection_model(),
            classes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceConfig {
    #[serde(default = "default_face_det_model")]
    pub detection_model: String,
    #[serde(default = "default_face_rec_model")]
    pub recognition_model: String,
    #[serde(default = "default_faces_dir")]
    pub known_faces_dir: PathBuf,
    #[serde(default = "default_similarity")]
    pub similarity_threshold: f32,
}

impl Default for FaceConfig {
    fn default() -> Self {
        Self {
            detection_model: default_face_det_model(),
            recognition_model: default_face_rec_model(),
            known_faces_dir: default_faces_dir(),
            similarity_threshold: default_similarity(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrConfig {
    #[serde(default = "default_ocr_model")]
    pub model: String,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default = "default_batch_size")]
    pub max_batch_size: u32,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            model: default_ocr_model(),
            languages: default_languages(),
            max_batch_size: default_batch_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_age")]
    pub max_age: u32,
    #[serde(default = "default_iou")]
    pub iou_threshold: f32,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age: default_max_age(),
            iou_threshold: default_iou(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: PathBuf,
    #[serde(default = "default_crops_dir")]
    pub crops_dir: PathBuf,
    #[serde(default = "default_true")]
    pub save_crops: bool,
    #[serde(default)]
    pub sync: SyncConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            sqlite_path: default_sqlite_path(),
            crops_dir: default_crops_dir(),
            save_crops: true,
            sync: SyncConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub postgres_url: Option<String>,
    #[serde(default = "default_sync_interval")]
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_window_width")]
    pub window_width: u32,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            window_width: default_window_width(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
    #[serde(default = "default_true")]
    pub auto_download: bool,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            cache_dir: default_cache_dir(),
            auto_download: true,
        }
    }
}

fn default_true() -> bool { true }
fn default_fps_limit() -> u32 { 30 }
fn default_confidence() -> f32 { 0.5 }
fn default_detection_model() -> String { "yolo26n".into() }
fn default_face_det_model() -> String { "scrfd_2.5g".into() }
fn default_face_rec_model() -> String { "arcface_r50".into() }
fn default_faces_dir() -> PathBuf { PathBuf::from("./faces/") }
fn default_similarity() -> f32 { 0.6 }
fn default_ocr_model() -> String { "ppocr_v5".into() }
fn default_languages() -> Vec<String> { vec!["en".into()] }
fn default_batch_size() -> u32 { 32 }
fn default_max_age() -> u32 { 30 }
fn default_iou() -> f32 { 0.3 }
fn default_backend() -> String { "sqlite".into() }
fn default_sqlite_path() -> PathBuf { PathBuf::from("./perception.db") }
fn default_crops_dir() -> PathBuf { PathBuf::from("./crops/") }
fn default_sync_interval() -> u64 { 60 }
fn default_window_width() -> u32 { 1280 }
fn default_cache_dir() -> PathBuf { PathBuf::from("./models/") }

impl Config {
    /// Load configuration from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| PerceptionError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_str(&content)
    }

    /// Parse configuration from a TOML string.
    pub fn from_str(s: &str) -> Result<Self> {
        let config: Config =
            toml::from_str(s).map_err(|e| PerceptionError::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        match self.capture.source.as_str() {
            "image" | "video" | "camera" | "rtsp" => {}
            other => {
                return Err(PerceptionError::Config(format!(
                    "unknown capture source '{other}', expected: image, video, camera, rtsp"
                )));
            }
        }

        let c = self.pipeline.confidence_threshold;
        if !(0.0..=1.0).contains(&c) {
            return Err(PerceptionError::Config(format!(
                "confidence_threshold {c} must be between 0.0 and 1.0"
            )));
        }

        let s = self.pipeline.face_config.similarity_threshold;
        if !(0.0..=1.0).contains(&s) {
            return Err(PerceptionError::Config(format!(
                "similarity_threshold {s} must be between 0.0 and 1.0"
            )));
        }

        if !self.pipeline.detection && !self.pipeline.face_recognition && !self.pipeline.ocr {
            return Err(PerceptionError::Config(
                "at least one pipeline must be enabled (detection, face_recognition, or ocr)"
                    .into(),
            ));
        }

        match self.storage.backend.as_str() {
            "sqlite" | "postgres" => {}
            other => {
                return Err(PerceptionError::Config(format!(
                    "unknown storage backend '{other}', expected: sqlite, postgres"
                )));
            }
        }

        if self.storage.sync.enabled && self.storage.sync.postgres_url.is_none() {
            return Err(PerceptionError::Config(
                "storage.sync.enabled=true requires storage.sync.postgres_url".into(),
            ));
        }

        Ok(())
    }
}
