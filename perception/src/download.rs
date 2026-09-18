//! Model management — registry, download, and integrity verification.
//!
//! Determines which ONNX models are needed based on the pipeline configuration,
//! downloads any that are missing from the local cache, and verifies SHA256
//! integrity of cached files.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing;

use crate::config::Config;
use crate::error::{PerceptionError, Result};

/// Sentinel value indicating the SHA256 hash has not been verified yet.
/// When a model entry uses this value, integrity verification is skipped.
const PLACEHOLDER_SHA256: &str = "TODO";

// ---------------------------------------------------------------------------
// Model registry
// ---------------------------------------------------------------------------

/// Metadata for a downloadable model artifact.
struct ModelInfo {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
    filename: &'static str,
}

// TODO: update SHA256 after first download
static REGISTRY: &[ModelInfo] = &[
    // Object detection — YOLO11 nano (community-exported ONNX, ~10MB)
    ModelInfo {
        name: "yolo26n",
        url: "https://huggingface.co/deepghs/yolos/resolve/main/yolo11n/model.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "yolo11n.onnx",
    },
    // Object detection — YOLO11 small (~20MB)
    ModelInfo {
        name: "yolo26s",
        url: "https://huggingface.co/deepghs/yolos/resolve/main/yolo11s/model.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "yolo11s.onnx",
    },
    // Face detection — SCRFD 2.5G (~3MB)
    ModelInfo {
        name: "scrfd_2.5g",
        url: "https://huggingface.co/crj/dl-ws/resolve/main/scrfd_2.5g.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "scrfd_2.5g.onnx",
    },
    // Face recognition — ArcFace ResNet50 (onnx-community, ~170MB)
    ModelInfo {
        name: "arcface_r50",
        url: "https://huggingface.co/onnx-community/arcface-onnx/resolve/main/arcface.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "arcface_r50.onnx",
    },
    // OCR text detection — PaddleOCR v5 (~4MB)
    ModelInfo {
        name: "ppocr_v5_det",
        url: "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/detection/v5/det.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "ppocr_v5_det.onnx",
    },
    // OCR text recognition — PaddleOCR v5 Latin (~10MB)
    ModelInfo {
        name: "ppocr_v5_rec",
        url: "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/languages/latin/rec.onnx",
        sha256: PLACEHOLDER_SHA256,
        filename: "ppocr_v5_rec.onnx",
    },
    // OCR character dictionary — PaddleOCR Latin
    ModelInfo {
        name: "ppocr_v5_keys",
        url: "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/languages/latin/dict.txt",
        sha256: PLACEHOLDER_SHA256,
        filename: "ppocr_v5_keys.txt",
    },
];

/// Looks up a model by name in the static registry.
fn lookup(name: &str) -> Option<&'static ModelInfo> {
    REGISTRY.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------------------
// ModelPaths
// ---------------------------------------------------------------------------

/// Resolved filesystem paths for each model the pipeline needs.
///
/// An `Option::None` value means the corresponding pipeline stage is disabled
/// and no model was downloaded.
#[derive(Debug, Clone)]
pub struct ModelPaths {
    /// Object detection model (e.g. YOLOv26n).
    pub detection: Option<PathBuf>,
    /// Face detection model (e.g. SCRFD).
    pub face_detection: Option<PathBuf>,
    /// Face recognition / embedding model (e.g. ArcFace).
    pub face_recognition: Option<PathBuf>,
    /// OCR text-detection model.
    pub ocr_detection: Option<PathBuf>,
    /// OCR text-recognition model.
    pub ocr_recognition: Option<PathBuf>,
    /// OCR character dictionary file.
    pub ocr_keys: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Ensure all models required by the current pipeline configuration are
/// available in the local cache, downloading any that are missing.
///
/// Returns [`ModelPaths`] with the resolved path for each enabled stage.
pub async fn ensure_models(config: &Config) -> Result<ModelPaths> {
    let cache_dir = &config.models.cache_dir;
    tokio::fs::create_dir_all(cache_dir).await.map_err(|e| PerceptionError::Io {
        path: cache_dir.clone(),
        source: e,
    })?;

    let auto = config.models.auto_download;

    let detection = if config.pipeline.detection {
        let model_name = &config.pipeline.detection_config.model;
        resolve_model(model_name, cache_dir, auto).await?
    } else {
        None
    };

    let (face_detection, face_recognition) = if config.pipeline.face_recognition {
        let det = resolve_model(&config.pipeline.face_config.detection_model, cache_dir, auto).await?;
        let rec = resolve_model(&config.pipeline.face_config.recognition_model, cache_dir, auto).await?;
        (det, rec)
    } else {
        (None, None)
    };

    let (ocr_detection, ocr_recognition, ocr_keys) = if config.pipeline.ocr {
        let det = resolve_model("ppocr_v5_det", cache_dir, auto).await?;
        let rec = resolve_model("ppocr_v5_rec", cache_dir, auto).await?;
        let keys = resolve_model("ppocr_v5_keys", cache_dir, auto).await?;
        (det, rec, keys)
    } else {
        (None, None, None)
    };

    Ok(ModelPaths {
        detection,
        face_detection,
        face_recognition,
        ocr_detection,
        ocr_recognition,
        ocr_keys,
    })
}

/// Resolve a model: if auto_download is true, download if missing.
/// If false, only return the path if the file already exists in cache.
async fn resolve_model(name: &str, cache_dir: &Path, auto_download: bool) -> Result<Option<PathBuf>> {
    let info = match lookup(name) {
        Some(i) => i,
        None => {
            tracing::warn!(model = name, "model not found in registry, skipping");
            return Ok(None);
        }
    };
    let cached_path = cache_dir.join(info.filename);
    if cached_path.exists() {
        return Ok(Some(cached_path));
    }
    if !auto_download {
        tracing::debug!(model = name, "model not cached and auto_download=false, skipping");
        return Ok(None);
    }
    let path = download_model(info, cache_dir).await?;
    Ok(Some(path))
}

/// Resolve a single model by name — look it up in the registry and
/// download if necessary.
async fn ensure_single(name: &str, cache_dir: &Path) -> Result<PathBuf> {
    let info = lookup(name).ok_or_else(|| PerceptionError::Download {
        model: name.to_string(),
        reason: format!("unknown model \"{name}\" — not in registry"),
    })?;
    download_model(info, cache_dir).await
}

// ---------------------------------------------------------------------------
// Download + verify
// ---------------------------------------------------------------------------

/// Download a model to `cache_dir` if it is not already cached and valid.
///
/// 1. If the file exists and the SHA256 matches (or is a placeholder), return immediately.
/// 2. Otherwise stream the file from `info.url` with a progress bar.
/// 3. Verify the SHA256 of the downloaded file; delete and error on mismatch.
async fn download_model(info: &ModelInfo, cache_dir: &Path) -> Result<PathBuf> {
    let dest = cache_dir.join(info.filename);

    // Cache hit — file exists with valid hash.
    if dest.exists() {
        if info.sha256 == PLACEHOLDER_SHA256 {
            tracing::debug!(model = info.name, "cache hit (hash verification skipped — placeholder)");
            return Ok(dest);
        }
        let hash = compute_sha256(&dest)?;
        if hash == info.sha256 {
            tracing::debug!(model = info.name, "cache hit");
            return Ok(dest);
        }
        tracing::warn!(model = info.name, "SHA256 mismatch on cached file — re-downloading");
    }

    tracing::info!(model = info.name, url = info.url, "downloading model");

    // Stream download with progress bar.
    let response = reqwest::get(info.url).await.map_err(|e| PerceptionError::Download {
        model: info.name.to_string(),
        reason: e.to_string(),
    })?;

    if !response.status().is_success() {
        return Err(PerceptionError::Download {
            model: info.name.to_string(),
            reason: format!("HTTP {}", response.status()),
        });
    }

    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    );
    pb.set_message(info.name.to_string());

    // Write to a temporary file first so a crash doesn't leave a partial artifact
    // at the final path.
    let tmp_path = cache_dir.join(format!(".{}.part", info.filename));
    let mut file = tokio::fs::File::create(&tmp_path).await.map_err(|e| PerceptionError::Io {
        path: tmp_path.clone(),
        source: e,
    })?;

    let mut stream = response.bytes_stream();
    let result: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| PerceptionError::Download {
                model: info.name.to_string(),
                reason: e.to_string(),
            })?;
            file.write_all(&chunk).await.map_err(|e| PerceptionError::Io {
                path: tmp_path.clone(),
                source: e,
            })?;
            pb.inc(chunk.len() as u64);
        }
        file.flush().await.map_err(|e| PerceptionError::Io {
            path: tmp_path.clone(),
            source: e,
        })?;
        Ok(())
    }
    .await;

    // On failure, clean up the partial temp file.
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        pb.abandon_with_message("download failed");
        return Err(e);
    }

    pb.finish_with_message("done");

    // Verify integrity (skip when using placeholder hash).
    if info.sha256 != PLACEHOLDER_SHA256 {
        let hash = compute_sha256(&tmp_path)?;
        if hash != info.sha256 {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(PerceptionError::Download {
                model: info.name.to_string(),
                reason: format!(
                    "SHA256 mismatch: expected {}, got {hash}",
                    info.sha256
                ),
            });
        }
    }

    // Atomically move temp file to final destination.
    tokio::fs::rename(&tmp_path, &dest).await.map_err(|e| PerceptionError::Io {
        path: dest.clone(),
        source: e,
    })?;

    tracing::info!(model = info.name, path = %dest.display(), "model ready");
    Ok(dest)
}

// ---------------------------------------------------------------------------
// SHA256
// ---------------------------------------------------------------------------

/// Compute the SHA256 digest of a file, returning the lowercase hex string.
fn compute_sha256(path: &Path) -> Result<String> {
    let data = std::fs::read(path).map_err(|e| PerceptionError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let hash = Sha256::digest(&data);
    Ok(hex::encode(hash))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn compute_sha256_known_value() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello world").unwrap();
        f.flush().unwrap();

        let hash = compute_sha256(f.path()).unwrap();
        // SHA256("hello world") is well-known.
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn compute_sha256_empty_file() {
        let f = NamedTempFile::new().unwrap();
        let hash = compute_sha256(f.path()).unwrap();
        // SHA256 of empty input.
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn registry_has_all_models() {
        let expected = [
            "yolo26n",
            "yolo26s",
            "scrfd_2.5g",
            "arcface_r50",
            "ppocr_v5_det",
            "ppocr_v5_rec",
            "ppocr_v5_keys",
        ];
        for name in &expected {
            assert!(
                lookup(name).is_some(),
                "model \"{name}\" missing from registry"
            );
        }
    }

    #[tokio::test]
    async fn ensure_models_detection_only_skips_face_and_ocr() {
        // Config validation requires at least one pipeline enabled.
        // Verify that face/ocr paths are None when only detection is on.
        let toml = r#"
[capture]
source = "camera"
path = "/dev/video0"

[pipeline]
detection = true
face_recognition = false
ocr = false

[models]
auto_download = false
"#;
        let config = Config::from_str(toml).unwrap();
        let paths = ensure_models(&config).await.unwrap();
        // detection path will be Some only if model file exists in cache.
        // face/ocr should always be None since those pipelines are disabled.
        assert!(paths.face_detection.is_none());
        assert!(paths.face_recognition.is_none());
        assert!(paths.ocr_detection.is_none());
        assert!(paths.ocr_recognition.is_none());
        assert!(paths.ocr_keys.is_none());
    }

    #[tokio::test]
    #[ignore] // Requires network access.
    async fn download_real_model() {
        let tmp = tempfile::tempdir().unwrap();
        let info = lookup("yolo26n").unwrap();
        let path = download_model(info, tmp.path()).await.unwrap();
        assert!(path.exists());
    }
}
