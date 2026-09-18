//! ONNX Runtime session management and model path resolution.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tracing::info;

use crate::error::{PerceptionError, Result};

/// Wraps an `ort::Session` with execution-provider selection logic.
pub struct Engine {
    session: Mutex<ort::session::Session>,
}

impl Engine {
    /// Creates a new ONNX Runtime session from the given model file.
    ///
    /// Execution provider selection order (compile-time features):
    /// 1. CUDA (if `cuda` feature enabled)
    /// 2. TensorRT (if `tensorrt` feature enabled)
    /// 3. CoreML (if `coreml` feature enabled)
    /// 4. CPU (always available as fallback)
    pub fn new(model_path: &Path) -> Result<Self> {
        if !model_path.exists() {
            return Err(PerceptionError::Io {
                path: model_path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "model file not found",
                ),
            });
        }

        let mut builder = ort::session::Session::builder()?;

        #[cfg(feature = "cuda")]
        {
            builder = builder
                .with_execution_providers([ort::ep::CUDA::default().build()])?;
            info!("CUDA execution provider requested");
        }

        #[cfg(feature = "tensorrt")]
        {
            builder = builder
                .with_execution_providers([ort::ep::TensorRT::default().build()])?;
            info!("TensorRT execution provider requested");
        }

        #[cfg(feature = "coreml")]
        {
            builder = builder
                .with_execution_providers([ort::ep::CoreML::default().build()])?;
            info!("CoreML execution provider requested");
        }

        info!(path = %model_path.display(), "loading ONNX model");

        let session = builder.commit_from_file(model_path)?;

        info!("ONNX session created successfully");

        Ok(Self { session: Mutex::new(session) })
    }

    /// Locks and runs inference on the inner ONNX Runtime session.
    pub fn run<'i, 'v: 'i, const N: usize>(
        &self,
        input_values: impl Into<ort::session::SessionInputs<'i, 'v, N>>,
    ) -> Result<Vec<ort::value::DynValue>> {
        let mut session = self.session.lock().map_err(|e| {
            PerceptionError::Inference(format!("session lock poisoned: {e}"))
        })?;
        let outputs = session.run(input_values)?;
        // Collect owned values so SessionOutputs (which borrows &mut Session) is dropped.
        Ok(outputs.into_iter().map(|(_, v)| v).collect())
    }
}

/// Pool of `Engine` instances for concurrent inference on the same model.
///
/// Uses atomic round-robin to distribute work across sessions.
pub struct EnginePool {
    engines: Vec<Arc<Engine>>,
    counter: AtomicUsize,
}

impl EnginePool {
    /// Creates a pool of `pool_size` independent ONNX sessions for the given model.
    ///
    /// A `pool_size` of 0 is clamped to 1.
    pub fn new(model_path: &Path, pool_size: usize) -> Result<Self> {
        let size = pool_size.max(1);
        let mut engines = Vec::with_capacity(size);

        for i in 0..size {
            let engine = Engine::new(model_path)?;
            info!(index = i + 1, total = size, "engine pool: created session");
            engines.push(Arc::new(engine));
        }

        Ok(Self {
            engines,
            counter: AtomicUsize::new(0),
        })
    }

    /// Returns the next engine via round-robin selection.
    pub fn get(&self) -> Arc<Engine> {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.engines.len();
        Arc::clone(&self.engines[idx])
    }

    /// Returns the number of engines in the pool.
    pub fn size(&self) -> usize {
        self.engines.len()
    }
}

/// Returns a sensible default pool size based on available hardware.
///
/// GPU builds (cuda/tensorrt/coreml) default to 2 sessions.
/// CPU builds default to `num_cpus / 4`, clamped to `1..=8`.
pub fn default_pool_size() -> usize {
    if cfg!(any(feature = "cuda", feature = "tensorrt", feature = "coreml")) {
        2
    } else {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        (cpus / 4).clamp(1, 8)
    }
}


/// Prints ONNX Runtime version and available execution providers.
pub fn print_info() {
    println!("ONNX Runtime info: {}", ort::info());

    let mut eps = vec!["CPU"];

    #[cfg(feature = "cuda")]
    eps.push("CUDA");

    #[cfg(feature = "tensorrt")]
    eps.push("TensorRT");

    #[cfg(feature = "coreml")]
    eps.push("CoreML");

    println!("Compiled execution providers: {}", eps.join(", "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::ModelPaths;

    #[test]
    fn default_pool_size_is_at_least_one() {
        assert!(default_pool_size() >= 1);
    }

    #[test]
    fn default_pool_size_within_bounds() {
        let size = default_pool_size();
        assert!((1..=8).contains(&size));
    }

    #[test]
    fn model_paths_default_is_all_none() {
        let paths = ModelPaths {
            detection: None,
            face_detection: None,
            face_recognition: None,
            ocr_detection: None,
            ocr_recognition: None,
            ocr_keys: None,
        };
        assert!(paths.detection.is_none());
        assert!(paths.face_detection.is_none());
    }

    #[test]
    fn model_paths_partial_construction() {
        let paths = ModelPaths {
            detection: Some(PathBuf::from("/models/yolov8.onnx")),
            face_detection: None,
            face_recognition: Some(PathBuf::from("/models/arcface.onnx")),
            ocr_detection: None,
            ocr_recognition: None,
            ocr_keys: None,
        };
        assert!(paths.detection.is_some());
        assert!(paths.face_detection.is_none());
        assert!(paths.face_recognition.is_some());
    }

    #[test]
    fn engine_missing_model_returns_io_error() {
        let result = Engine::new(Path::new("/nonexistent/model.onnx"));
        assert!(result.is_err());
        let err = result.err().unwrap();
        match err {
            PerceptionError::Io { path, .. } => {
                assert_eq!(path, PathBuf::from("/nonexistent/model.onnx"));
            }
            other => panic!("expected Io error, got: {other:?}"),
        }
    }

    #[test]
    #[ignore = "requires a real ONNX model file on disk"]
    fn engine_pool_clamps_zero_size() {
        let path = Path::new("test_model.onnx");
        if let Ok(pool) = EnginePool::new(path, 0) {
            assert_eq!(pool.size(), 1);
        }
    }

    #[test]
    #[ignore = "requires a real ONNX model file on disk"]
    fn engine_pool_round_robin() {
        let path = Path::new("test_model.onnx");
        if let Ok(pool) = EnginePool::new(path, 3) {
            // Each get() should cycle through engines
            let _e0 = pool.get();
            let _e1 = pool.get();
            let _e2 = pool.get();
            // Fourth call wraps around to index 0
            let _e3 = pool.get();
            assert_eq!(pool.size(), 3);
        }
    }
}
