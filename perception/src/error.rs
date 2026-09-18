//! Centralized error types for the perception pipeline.

use std::path::PathBuf;

/// Top-level error type for all perception operations.
#[derive(Debug, thiserror::Error)]
pub enum PerceptionError {
    /// Configuration file could not be loaded or is invalid.
    #[error("config error: {0}")]
    Config(String),

    /// Frame capture failed (camera unavailable, file not found, decode error).
    #[error("capture error: {0}")]
    Capture(String),

    /// ONNX inference failed (session creation, model load, runtime error).
    #[error("inference error: {0}")]
    Inference(String),

    /// Storage operation failed (SQLite, Postgres, filesystem).
    #[error("storage error: {0}")]
    Storage(String),

    /// Model download failed (network, checksum mismatch, I/O).
    #[error("download error for model '{model}': {reason}")]
    Download { model: String, reason: String },

    /// File or path I/O error.
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    /// OpenCV operation failed.
    #[error("opencv error: {0}")]
    OpenCv(String),
}

impl From<opencv::Error> for PerceptionError {
    fn from(e: opencv::Error) -> Self {
        PerceptionError::OpenCv(e.to_string())
    }
}

impl From<sqlx::Error> for PerceptionError {
    fn from(e: sqlx::Error) -> Self {
        PerceptionError::Storage(e.to_string())
    }
}

impl From<ort::Error> for PerceptionError {
    fn from(e: ort::Error) -> Self {
        PerceptionError::Inference(e.to_string())
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, PerceptionError>;
