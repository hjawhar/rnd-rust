# Perception Engine — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a high-throughput Rust binary for real-time object detection, face recognition, and OCR — targeting edge devices (drones, surveillance units) with NVIDIA or Apple Silicon GPUs.

**Architecture:** Async pipeline with channel-based stages. Capture decoupled from inference via bounded channels with backpressure. Each pipeline stage (detection, face, OCR) runs as an independent tokio task. ByteTrack assigns stable object IDs across frames so expensive models only run on new objects. Storage is edge-first (SQLite WAL) with optional batch sync to Postgres.

**Tech Stack:** `ort` 2.0 (ONNX Runtime), `opencv` (capture + preview), `tokio` (async), `clap` (CLI), `sqlx` 0.8 (storage), `jamtrack-rs` (ByteTrack), `tracing` (observability), `serde` + `toml` (config).

---

## Concurrency & Threading Model

This is a **multi-threaded, async-concurrent** system. The design separates CPU-bound inference from async I/O, and ensures multiple frames can be in-flight simultaneously.

```
                            ┌──────────────────────────────────────────────────────┐
                            │              Tokio Async Runtime                     │
┌─────────────┐  bounded    │  ┌─────────────────────────────────────────────────┐ │
│ Capture     │  channel    │  │           Fan-Out Router (async task)           │ │
│ (dedicated  │────────────►│  │  clones frame to each enabled pipeline stage   │ │
│  OS thread) │  Frame      │  └───┬──────────────┬──────────────┬──────────────┘ │
│             │             │      │              │              │                │
│ std::thread │             │      ▼              ▼              ▼                │
│ ::spawn()   │             │  ┌────────┐    ┌────────┐    ┌────────┐             │
└─────────────┘             │  │Detector│    │  Face  │    │  OCR   │             │
                            │  │ Worker │    │ Worker │    │ Worker │             │
                            │  │        │    │        │    │        │             │
                            │  │ spawn_ │    │ spawn_ │    │ spawn_ │             │
                            │  │blocking│    │blocking│    │blocking│             │
                            │  └───┬────┘    └───┬────┘    └───┬────┘             │
                            │      │             │             │                  │
                            │      ▼             ▼             ▼                  │
                            │  ┌─────────────────────────────────────────────────┐ │
                            │  │        Event Collector (async task)             │ │
                            │  │  merges results, deduplicates via tracker       │ │
                            │  └──────────────────┬──────────────────────────────┘ │
                            │                     │                                │
                            │                     ▼                                │
                            │  ┌─────────────────────────────────────────────────┐ │
                            │  │     Storage Writer (async task, batched)        │ │
                            │  │  collects events, bulk-commits every 100ms     │ │
                            │  │  or every N events, whichever comes first       │ │
                            │  └─────────────────────────────────────────────────┘ │
                            └──────────────────────────────────────────────────────┘
```

### Thread Allocation

| Component | Thread Type | Why |
|-----------|-------------|-----|
| **Capture** | Dedicated OS thread (`std::thread::spawn`) | OpenCV `VideoCapture`/`Mat` are not `Send`. Frames are converted to owned `Frame` (bytes + metadata) before crossing the channel boundary. |
| **Router** | Tokio async task | Lightweight fan-out — receives frames, clones to each enabled pipeline channel. Pure async, no blocking. |
| **Detector/Face/OCR workers** | Tokio task + `spawn_blocking` per inference call | ONNX inference is CPU/GPU-bound. `spawn_blocking` offloads to tokio's blocking thread pool so the async runtime stays responsive. Multiple `spawn_blocking` calls run **in parallel** on separate OS threads. |
| **Event collector** | Tokio async task | Merges results from all pipeline workers, runs tracker update (lightweight CPU work). |
| **Storage writer** | Tokio async task | SQLite writes via `sqlx` are async. Batches events into transactions for throughput. |
| **Postgres sync** | Tokio async task (background) | Periodic poll + batch insert. Fully async I/O. |
| **Preview** | Dedicated OS thread | OpenCV `highgui` requires a GUI thread. Receives annotated frames via channel. |

### Parallelism Model

**Inter-stage parallelism:** All three pipeline stages (detection, face, OCR) process the same frame **concurrently**. A frame is fanned out to all enabled stages simultaneously.

**Inter-frame parallelism:** While frame N is in the face recognition stage, frame N+1 can already be in the detection stage. The pipeline has depth = number of stages. With a channel capacity of 4, up to 4 frames can be in-flight at different stages.

**Intra-operator parallelism:** ONNX Runtime's own thread pool handles parallelism within a single inference call (multi-threaded CPU kernels, GPU stream scheduling). Configured via `ort::SessionBuilder::with_intra_threads()`.

**Batch inference (future optimization):** When multiple ROIs need the same model (e.g., 10 detected faces all need ArcFace embedding), batch them into a single inference call with batch dimension > 1. This maximizes GPU utilization.

### Highway Scenario (1000s of plates)

Concrete flow for the high-throughput case:

1. **Frame arrives** (30fps capture) → sent to router
2. **Detector** runs YOLO26 on full frame → finds 60 vehicles in ~15ms (GPU)
3. **Tracker** updates: 55 are existing tracks, 5 are new → only 5 need OCR
4. **OCR worker** receives 5 vehicle crops → batches into single inference (batch=5) → reads 5 plates in ~20ms
5. **Storage** batches 5 plate events → single SQLite transaction → ~0.05ms
6. While steps 3-5 happen, **next frame is already in step 2**

The tracker is the key throughput multiplier — without it, you'd run OCR on all 60 vehicles every frame (1800 OCR calls/sec). With it, you only run OCR on ~5 new vehicles per frame (~150 OCR calls/sec).

### Backpressure & Frame Dropping

The capture→router channel is **bounded** (default capacity: 4). When the pipeline can't keep up:

1. Capture thread calls `try_send()` on the channel
2. If channel is full, the oldest frame in the channel is replaced (ring buffer semantics) or the new frame is dropped
3. A `frames_dropped` counter increments (visible in metrics)
4. The system never OOMs, never builds an unbounded queue
5. Operator sees drop rate in logs and can tune: lower resolution, fewer pipeline stages, or faster hardware

---

## Module Map

```
src/
├── main.rs              # CLI parsing, pipeline assembly, run loop
├── config.rs            # TOML config deserialization + validation
├── types.rs             # Domain types: Detection, BBox, Event, Embedding, Frame
├── error.rs             # Error enum (thiserror)
├── capture/
│   ├── mod.rs           # FrameSource trait + factory function
│   ├── image.rs         # Single image file source
│   ├── video.rs         # Video file source (mp4, avi, mkv)
│   └── camera.rs        # Live camera (index) + RTSP stream
├── engine.rs            # ONNX session manager (shared env, EP selection)
├── pipeline/
│   ├── mod.rs           # Pipeline orchestrator (channel wiring, fan-out)
│   ├── detector.rs      # YOLO26 object detection + postprocessing
│   ├── face.rs          # SCRFD face detection + ArcFace embedding extraction
│   ├── ocr.rs           # PaddleOCR text detection (DBNet) + recognition (CRNN)
│   └── tracker.rs       # ByteTrack wrapper for stable cross-frame IDs
├── storage/
│   ├── mod.rs           # StorageBackend trait + factory
│   ├── sqlite.rs        # SQLite WAL local storage
│   ├── postgres.rs      # Optional batch sync to central Postgres (feature-gated)
│   └── crops.rs         # Image crop filesystem writer
├── preview.rs           # OpenCV highgui live preview (feature-gated)
├── face_db.rs           # Known face embedding DB + cosine similarity matching
└── download.rs          # Model auto-download from HuggingFace
tests/
├── fixtures/            # Test images, tiny ONNX models, sample config
├── test_config.rs       # Config parsing + validation
├── test_types.rs        # Domain type invariants
├── test_capture.rs      # Frame source tests (image files)
├── test_engine.rs       # ONNX session creation
├── test_detector.rs     # YOLO preprocessing + postprocessing
├── test_face.rs         # Face detection + embedding pipeline
├── test_ocr.rs          # OCR detection + recognition
├── test_tracker.rs      # ByteTrack ID assignment + age-out
├── test_storage.rs      # SQLite + crop writes
├── test_face_db.rs      # Embedding match/no-match
├── test_pipeline.rs     # End-to-end: image → detections → storage
└── test_download.rs     # Model download + cache validation
```

## Cargo Feature Flags

```toml
[features]
default = ["cpu"]
cpu = []
cuda = ["ort/cuda"]
tensorrt = ["ort/tensorrt"]
coreml = ["ort/coreml"]
preview = []               # enables OpenCV highgui window
postgres = ["sqlx/postgres"]
```

## Configuration Format

```toml
[capture]
source = "camera"           # "image" | "video" | "camera" | "rtsp"
path = "/dev/video0"         # file path, camera index, or rtsp:// URL
fps_limit = 30

[pipeline]
detection = true
face_recognition = true
ocr = true
confidence_threshold = 0.5

[pipeline.detection]
model = "yolo26n"            # yolo26n | yolo26s | yolo26m
classes = ["car", "truck", "person", "bicycle"]

[pipeline.face]
detection_model = "scrfd_2.5g"
recognition_model = "arcface_r50"
known_faces_dir = "./faces/"
similarity_threshold = 0.6

[pipeline.ocr]
model = "ppocr_v5"
languages = ["en"]

[pipeline.tracker]
enabled = true
max_age = 30
iou_threshold = 0.3

[storage]
backend = "sqlite"
sqlite_path = "./perception.db"
crops_dir = "./crops/"
save_crops = true

[storage.sync]
enabled = false
postgres_url = "postgres://user:pass@host/perception"
interval_secs = 60

[preview]
enabled = false
window_width = 1280

[models]
cache_dir = "./models/"
auto_download = true
```

---

## Phase 1: Project Foundation

### Task 1.1: Scaffold Cargo Project

**Files:** Create `Cargo.toml`, `src/main.rs`, `src/error.rs`

- [ ] Initialize `cargo init --name perception`
- [ ] Set up `Cargo.toml` with all dependencies and feature flags:
  - `ort = { version = "2.0.0-rc.12", default-features = false }`
  - `opencv = { version = "0.94", default-features = false, features = ["videoio", "imgcodecs", "imgproc"] }`
  - `tokio = { version = "1", features = ["full"] }`
  - `clap = { version = "4", features = ["derive"] }`
  - `sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }`
  - `jamtrack = "0.1"` (verify crate name on crates.io)
  - `serde = { version = "1", features = ["derive"] }`
  - `toml = "0.8"`
  - `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`
  - `thiserror = "2"`, `anyhow = "1"`
  - `image = "0.25"` (for image crop encoding)
  - `ndarray = "0.16"` (for tensor manipulation)
  - `uuid = { version = "1", features = ["v4"] }` (event IDs)
  - `chrono = { version = "0.4", features = ["serde"] }` (timestamps)
  - `reqwest = { version = "0.12", features = ["stream"] }` (model download)
  - `tokio-util = { version = "0.7", features = ["io"] }` (download progress)
  - `indicatif = "0.17"` (progress bars for download)
  - `dirs = "6"` (XDG cache dir for models)
- [ ] Define `PerceptionError` enum in `src/error.rs` using `thiserror` with variants: `Config`, `Capture`, `Inference`, `Storage`, `Download`, `Io`
- [ ] Stub `main.rs` with tracing init + clap skeleton
- [ ] `cargo check` passes
- [ ] Commit: `"chore: scaffold perception project"`

### Task 1.2: Domain Types

**Files:** Create `src/types.rs`

- [ ] Define core domain types:
  - `Frame` — image data (OpenCV `Mat` wrapper or raw bytes) + source timestamp + frame number
  - `BBox` — `x1, y1, x2, y2: f32` + `confidence: f32` + `class_id: u32` + `label: String`
  - `Detection` — `bbox: BBox` + `kind: DetectionKind` (Object | Face | Text)
  - `FaceMatch` — `embedding: Vec<f32>` + `identity: Option<String>` + `similarity: f32`
  - `OcrResult` — `text: String` + `bbox: BBox` + `confidence: f32`
  - `Event` — `id: Uuid` + `timestamp: DateTime` + `frame_number: u64` + `detections: Vec<Detection>` + `track_id: Option<u64>`
- [ ] Implement `Display` for key types
- [ ] Write `tests/test_types.rs` — construction, serialization round-trip with serde
- [ ] Commit: `"feat: define domain types"`

### Task 1.3: Configuration

**Files:** Create `src/config.rs`, `tests/test_config.rs`

- [ ] Define `Config` struct with nested sections matching the TOML format above
- [ ] All fields with sensible defaults via `#[serde(default)]`
- [ ] `Config::load(path: &Path) -> Result<Config>` — reads + deserializes + validates
- [ ] Validation: source path exists (for image/video), confidence in 0.0..=1.0, at least one pipeline enabled
- [ ] Write `tests/test_config.rs`:
  - Parse a complete config
  - Parse minimal config (all defaults)
  - Reject invalid confidence (negative, >1)
  - Reject config with all pipelines disabled
- [ ] Add a `perception.example.toml` at project root
- [ ] Commit: `"feat: TOML configuration with validation"`

---

## Phase 2: Capture Layer

### Task 2.1: FrameSource Trait + Image Source

**Files:** Create `src/capture/mod.rs`, `src/capture/image.rs`, `tests/test_capture.rs`, `tests/fixtures/test_640x480.jpg`

- [ ] Define `FrameSource` trait:
  ```rust
  pub trait FrameSource: Send {
      fn next_frame(&mut self) -> Result<Option<Frame>>;
      fn fps(&self) -> Option<f64>;
  }
  ```
- [ ] Factory function: `create_source(config: &CaptureConfig) -> Result<Box<dyn FrameSource>>`
- [ ] `ImageSource` — reads a single image file via `opencv::imgcodecs::imread`, yields it once, then returns `None`
- [ ] Place a small test JPEG in `tests/fixtures/`
- [ ] Tests: load fixture image, verify dimensions, verify second call returns None
- [ ] Commit: `"feat: frame source trait + image capture"`

### Task 2.2: Video + Camera Sources

**Files:** Create `src/capture/video.rs`, `src/capture/camera.rs`

- [ ] `VideoSource` — wraps `opencv::videoio::VideoCapture` opened on a file path. Yields frames sequentially. Returns `None` at EOF. Exposes FPS from file metadata.
- [ ] `CameraSource` — wraps `VideoCapture` opened on camera index or RTSP URL. Yields frames indefinitely. Supports `fps_limit` via frame timing.
- [ ] Update factory function to dispatch based on `source` config field
- [ ] Tests (video): use a tiny test video in fixtures (or generate one from test image via OpenCV `VideoWriter` in a test helper)
- [ ] Tests (camera): marked `#[ignore]` since CI won't have a camera — test construction + error on invalid index
- [ ] Commit: `"feat: video file + camera/RTSP capture sources"`

---

## Phase 3: Inference Engine

### Task 3.1: ONNX Session Manager

**Files:** Create `src/engine.rs`, `tests/test_engine.rs`

- [ ] `Engine` struct — holds `ort::Session`, provides `run(&self, inputs) -> Result<outputs>` wrapper
- [ ] `Engine::new(model_path: &Path, config: &EngineConfig) -> Result<Engine>` — builds session with EP selection:
  - Try CUDA/TensorRT if `cuda`/`tensorrt` feature enabled
  - Try CoreML if `coreml` feature enabled
  - Fallback to CPU always
  - Log which EP was activated via `tracing::info!`
- [ ] `EngineConfig` — number of threads, optimization level, EP preference
- [ ] **Batch inference support:** `run_batch(&self, inputs: Vec<Tensor>) -> Result<Vec<Output>>` — stacks N inputs along batch dimension (dim 0), runs single inference, splits output back. This is critical for burst scenarios (100 plates in one frame).
- [ ] **Worker pool:** `EnginePool` — holds `Vec<Arc<Engine>>` (N cloned sessions). `acquire() -> Arc<Engine>` round-robins across sessions. Allows N concurrent inference calls on the same model. Default N = number of CPU cores / 4, or 2 for GPU.
- [ ] Tests: create session from a minimal ONNX model, verify batch input/output shapes, verify pool round-robin
- [ ] Commit: `"feat: ONNX session manager with batch inference + worker pool"`

---

## Phase 4: Detection Pipelines

### Task 4.1: YOLO Object Detector

**Files:** Create `src/pipeline/mod.rs`, `src/pipeline/detector.rs`, `tests/test_detector.rs`

- [ ] Define `Detector` trait in `pipeline/mod.rs`:
  ```rust
  pub trait Detector: Send + Sync {
      fn detect(&self, frame: &Frame) -> Result<Vec<Detection>>;
  }
  ```
- [ ] `YoloDetector` struct wrapping an `Engine`
- [ ] Preprocessing: letterbox resize to model input size (640x640 default), normalize to 0..1, HWC→CHW, build `ndarray` tensor
- [ ] Postprocessing: decode YOLO26 output (NMS-free — just threshold + extract boxes), scale boxes back to original image coordinates
- [ ] Filter by configured class list
- [ ] Tests:
  - Preprocessing: verify letterbox dimensions, normalization range
  - Postprocessing: hand-craft a fake model output tensor, verify box decoding produces expected coordinates
  - Integration (gated by model availability): run real YOLO26n.onnx on test image, verify detections are non-empty
- [ ] Commit: `"feat: YOLO26 object detection pipeline"`

### Task 4.2: Face Detection + Recognition

**Files:** Create `src/pipeline/face.rs`, `src/face_db.rs`, `tests/test_face.rs`, `tests/test_face_db.rs`

- [ ] `FacePipeline` struct — holds two engines: SCRFD (detection) + ArcFace (recognition)
- [ ] SCRFD preprocessing: resize to 640x640, normalize
- [ ] SCRFD postprocessing: decode anchor-based face boxes + 5-point landmarks, NMS, threshold
- [ ] ArcFace preprocessing: crop + align face using landmarks, resize to 112x112, normalize
- [ ] ArcFace postprocessing: L2-normalize the 512-dim embedding vector
- [ ] `FaceDb` in `face_db.rs`:
  - Load known faces from a directory (one subdirectory per person, images inside)
  - On init: run ArcFace on each image, store `(name, Vec<f32>)` pairs
  - `match_face(embedding: &[f32]) -> Option<FaceMatch>` — cosine similarity against all known embeddings, return best match above threshold
- [ ] Tests:
  - Preprocessing dimensions and normalization
  - Cosine similarity: identical vectors → 1.0, orthogonal → 0.0
  - FaceDb: insert embeddings, verify match/no-match at threshold
  - Integration (gated): SCRFD on test image with faces → non-empty detections
- [ ] Commit: `"feat: SCRFD face detection + ArcFace recognition pipeline"`

### Task 4.3: OCR Pipeline

**Files:** Create `src/pipeline/ocr.rs`, `tests/test_ocr.rs`

- [ ] `OcrPipeline` struct — holds two engine pools: text detection (DBNet) + text recognition (CRNN)
- [ ] Text detection preprocessing: resize, normalize, build tensor
- [ ] Text detection postprocessing: threshold probability map → binary map → find contours → extract rotated bounding boxes
- [ ] Text recognition preprocessing: crop text region, resize to recognition input (e.g., 48x320), normalize
- [ ] Text recognition postprocessing: CTC decode — argmax along sequence, collapse repeats, map indices to characters via dictionary
- [ ] **Batch OCR:** `recognize_batch(crops: &[Mat]) -> Result<Vec<OcrResult>>` — stacks all text crops into a single batched tensor, runs one CRNN inference call. For 100 plates: 1 call (batch=100) instead of 100 calls.
- [ ] **Configurable batch size:** `[pipeline.ocr] max_batch_size = 32`. Crops exceeding batch size are split into multiple batched calls, processed across the engine pool in parallel.
- [ ] Tests:
  - CTC decode: hand-craft logit sequence, verify decoded text
  - Contour-to-box extraction with synthetic binary maps
  - Batch preprocessing: verify N crops produce tensor with shape [N, C, H, W]
  - Integration (gated): run on test image with visible text
- [ ] Commit: `"feat: PaddleOCR text detection + recognition with batch inference"`

### Task 4.4: Object Tracker

**Files:** Create `src/pipeline/tracker.rs`, `tests/test_tracker.rs`

- [ ] `ObjectTracker` wrapping `jamtrack` ByteTrack
- [ ] Interface: `update(detections: &[Detection]) -> Vec<TrackedDetection>` where `TrackedDetection` adds a `track_id: u64` and `is_new: bool`
- [ ] `is_new` flag indicates first frame this object was seen — triggers expensive processing (face recognition, OCR) only for new objects
- [ ] Track aging: objects not seen for `max_age` frames are dropped
- [ ] Tests:
  - Same box in consecutive frames → same track_id, `is_new` only on first
  - Box disappears for > max_age frames → new track_id when it reappears
  - Multiple non-overlapping boxes → distinct track_ids
- [ ] Commit: `"feat: ByteTrack object tracking wrapper"`

---

## Phase 5: Storage

### Task 5.1: SQLite Storage

**Files:** Create `src/storage/mod.rs`, `src/storage/sqlite.rs`, `src/storage/crops.rs`, `tests/test_storage.rs`

- [ ] Define `StorageBackend` trait:
  ```rust
  #[async_trait]
  pub trait StorageBackend: Send + Sync {
      async fn store_event(&self, event: &Event) -> Result<()>;
      async fn store_crop(&self, event_id: &Uuid, image: &[u8]) -> Result<PathBuf>;
      async fn query_events(&self, filter: &EventFilter) -> Result<Vec<Event>>;
  }
  ```
- [ ] SQLite schema (via sqlx migrations):
  ```sql
  CREATE TABLE events (
      id TEXT PRIMARY KEY,
      timestamp TEXT NOT NULL,
      frame_number INTEGER NOT NULL,
      track_id INTEGER,
      detection_kind TEXT NOT NULL,
      label TEXT,
      confidence REAL NOT NULL,
      bbox_x1 REAL, bbox_y1 REAL, bbox_x2 REAL, bbox_y2 REAL,
      ocr_text TEXT,
      face_identity TEXT,
      face_similarity REAL,
      crop_path TEXT,
      synced INTEGER DEFAULT 0
  );
  CREATE INDEX idx_events_timestamp ON events(timestamp);
  CREATE INDEX idx_events_track ON events(track_id);
  CREATE INDEX idx_events_label ON events(label);
  CREATE INDEX idx_events_synced ON events(synced) WHERE synced = 0;
  ```
- [ ] `SqliteStorage` — opens SQLite with WAL mode, batch inserts via transactions
- [ ] `CropWriter` — saves JPEG crops to `crops_dir/{date}/{event_id}.jpg`
- [ ] Tests (in-memory SQLite):
  - Store event → query returns it
  - Store 1000 events in batch → all retrievable
  - Filter by label, time range, track_id
  - Crop writer: write bytes → file exists at expected path
- [ ] Commit: `"feat: SQLite storage + image crop writer"`

### Task 5.2: Postgres Sync (Feature-Gated)

**Files:** Create `src/storage/postgres.rs`

- [ ] `PostgresSync` — background task that:
  - Polls SQLite for `synced = 0` events every `interval_secs`
  - Batch-inserts into Postgres (same schema)
  - Marks events as `synced = 1` in SQLite after successful insert
- [ ] Feature-gated behind `postgres` feature flag
- [ ] Tests: marked `#[ignore]` (requires running Postgres) — verify batch insert + sync flag update
- [ ] Commit: `"feat: optional Postgres batch sync"`

---

## Phase 6: Pipeline Orchestration

### Task 6.1: Pipeline Wiring

**Files:** Create full `src/pipeline/mod.rs` orchestrator

- [ ] `Pipeline` struct — owns all detector instances + tracker + storage
- [ ] `Pipeline::new(config: &Config) -> Result<Pipeline>` — conditionally creates detectors based on config flags, loads only needed models
- [ ] `Pipeline::process_frame(&self, frame: Frame) -> Result<Vec<Event>>`:
  1. Run object detection on full frame (single inference call — handles any number of objects)
  2. Update tracker with detections → get `Vec<TrackedDetection>` with `is_new` flags
  3. **Partition new detections by type** and batch-process:
     - Collect all new `person` detections → crop regions → batch face detection + recognition
     - Collect all new `vehicle` detections → crop regions → batch OCR (plate reading)
     - These two batches run **concurrently** via `tokio::join!`
  4. Build `Event` structs from results
  5. Store events + crops via storage backend (batched transaction)
  6. Return events (for preview / metrics)
- [ ] **Burst handling (100 cars scenario):**
  - Detection: 1 YOLO call finds all 100 vehicles (~15ms GPU). No per-car cost.
  - Tracker: all 100 are new → all need OCR
  - OCR: crops batched into groups of `max_batch_size` (e.g., 32). With engine pool of 4 workers: 100 crops = 4 batches of 25, processed in parallel across workers = ~8-15ms GPU total.
  - Storage: 100 events in one SQLite transaction = ~1ms
  - **Total burst frame time: ~30-40ms → still 25+ FPS**
- [ ] Backpressure: bounded channel between capture and pipeline. If pipeline falls behind, newest frames replace oldest (ring buffer semantics via `tokio::sync::watch` or bounded channel with `try_send` + drop).
- [ ] Tests:
  - Orchestrator with all pipelines disabled → empty events
  - Frame through detection-only pipeline → events produced
  - Backpressure: send frames faster than processing → no OOM, frames dropped gracefully
  - Batch path: 100 synthetic detections → verify all get processed, verify batching reduces call count
- [ ] Commit: `"feat: pipeline orchestrator with batch processing + backpressure"`

### Task 6.2: Metrics + Tracing

- [ ] Add `tracing::instrument` to each pipeline stage
- [ ] Track and log every 100 frames:
  - Capture FPS (actual)
  - Processing FPS (inference throughput)
  - Detection counts by type
  - Queue depth (pending frames)
  - Dropped frame count
- [ ] Commit: `"feat: pipeline performance metrics"`

---

## Phase 7: CLI + Preview

### Task 7.1: CLI Interface

**Files:** Finalize `src/main.rs`

- [ ] Clap derive-based CLI:
  ```
  perception [OPTIONS] <COMMAND>
  
  Commands:
    run        Run the perception pipeline
    download   Download/update models
    faces      Manage known faces database
    info       Show system info (GPU, available EPs, loaded models)

  Options:
    -c, --config <PATH>    Config file path [default: perception.toml]
    -v, --verbose          Increase log verbosity (-v, -vv, -vvv)
  ```
- [ ] `run` subcommand: load config → init engine → build pipeline → capture loop → ctrl-c graceful shutdown
- [ ] `download` subcommand: download all models specified in config to cache dir
- [ ] `faces` subcommand: `faces add <name> <image_path>`, `faces list`, `faces remove <name>`
- [ ] `info` subcommand: print ONNX Runtime version, available EPs, GPU info
- [ ] Graceful shutdown: `tokio::signal::ctrl_c()` → stop capture → drain pipeline → flush storage
- [ ] Commit: `"feat: CLI interface with subcommands"`

### Task 7.2: Live Preview

**Files:** Create `src/preview.rs`

- [ ] Feature-gated behind `preview` feature
- [ ] `PreviewWindow` — opens OpenCV `highgui::named_window`
- [ ] Draws bounding boxes + labels + track IDs on frame
- [ ] Color-coded by detection kind: green=object, blue=face, red=text
- [ ] Shows FPS overlay in corner
- [ ] ESC key closes preview
- [ ] Commit: `"feat: optional live preview window"`

---

## Phase 8: Model Management

### Task 8.1: Model Auto-Download

**Files:** Create `src/download.rs`, `tests/test_download.rs`

- [ ] Model registry: hardcoded map of model name → HuggingFace URL + expected SHA256
  - `yolo26n` → `https://huggingface.co/ultralytics/yolo26/resolve/main/yolo26n.onnx`
  - `scrfd_2.5g` → `https://huggingface.co/insightface/...`
  - `arcface_r50` → `https://huggingface.co/onnx-community/arcface-onnx/...`
  - `ppocr_v5_det` → `https://huggingface.co/monkt/paddleocr-onnx/...`
  - `ppocr_v5_rec` → `https://huggingface.co/monkt/paddleocr-onnx/...`
  - `ppocr_v5_keys` → character dictionary file
- [ ] `download_model(name, cache_dir) -> Result<PathBuf>`:
  - Check cache: if file exists + SHA256 matches → return path
  - Otherwise: HTTP GET with progress bar (`indicatif`), verify SHA256, write to cache dir
- [ ] `ensure_models(config) -> Result<ModelPaths>` — downloads all models required by current config
- [ ] Tests:
  - Cache hit: file exists → no download
  - SHA256 mismatch: re-download
  - (HTTP tests: use `#[ignore]` or mock with a local file server)
- [ ] Commit: `"feat: model auto-download with SHA256 verification"`

---

## Phase 9: Documentation

### Task 9.1: Project Documentation

**Files:** Create `README.md`, `docs/ARCHITECTURE.md`, inline rustdoc

- [ ] `README.md`:
  - Project description + use cases
  - Quick start (install, download models, run on image/video/camera)
  - Configuration reference
  - Feature flags reference
  - Building from source (with CUDA, CoreML, etc.)
  - Supported models table
- [ ] `docs/ARCHITECTURE.md`:
  - Pipeline diagram (ASCII)
  - Module responsibilities
  - Data flow description
  - Performance characteristics + tuning guide
  - Storage schema
- [ ] Rustdoc: `//!` module-level docs on every `mod.rs`, `///` on every public type and function
- [ ] Commit: `"docs: README, architecture guide, rustdoc"`

---

## Phase 10: Integration + Polish

### Task 10.1: End-to-End Integration Test

**Files:** Create `tests/test_pipeline.rs`

- [ ] Test: load test image → run full pipeline (detection + OCR) → verify events stored in SQLite → verify crops written to disk
- [ ] Test: load test video (5 frames) → verify tracker assigns consistent IDs → verify event count matches expectations
- [ ] Gated behind model availability (`#[ignore]` if models not downloaded)
- [ ] Commit: `"test: end-to-end integration tests"`

### Task 10.2: Benchmarks

**Files:** Create `benches/inference.rs`

- [ ] Criterion benchmarks:
  - YOLO26 preprocessing (letterbox + normalize)
  - YOLO26 inference (single frame)
  - Full pipeline (capture → detect → store)
  - SQLite batch insert throughput (1k, 10k events)
- [ ] Commit: `"bench: inference + storage benchmarks"`

### Task 10.3: CI Configuration

**Files:** Create `.github/workflows/ci.yml`

- [ ] Workflow: `cargo check`, `cargo test` (CPU only, no models needed for unit tests), `cargo clippy`, `cargo fmt --check`
- [ ] Separate job for integration tests (downloads models, runs gated tests)
- [ ] Commit: `"ci: GitHub Actions workflow"`

---

## Dependency Graph

```
Phase 1 (Foundation) ──┬── Phase 2 (Capture)
                       ├── Phase 3 (Engine)
                       └── Phase 5 (Storage)
                              │
Phase 3 (Engine) ─────────── Phase 4 (Pipelines) ── Phase 6 (Orchestration)
                                                          │
Phase 2 (Capture) ────────────────────────────────── Phase 6
Phase 5 (Storage) ────────────────────────────────── Phase 6
                                                          │
                                                     Phase 7 (CLI + Preview)
                                                          │
Phase 8 (Model Mgmt) ── can be done in parallel with Phases 4-6
Phase 9 (Docs) ───────── after Phase 7
Phase 10 (Integration) ── after everything
```

**Parallelizable:** Phases 2, 3, 5 are independent after Phase 1. Tasks 4.1-4.4 are independent of each other. Phase 8 is independent of Phases 4-6.

## Test Strategy

- **Unit tests:** Per-module, test preprocessing/postprocessing with synthetic data. No real models needed.
- **Integration tests:** `tests/` directory, require downloaded models, gated with `#[ignore]` or env var check.
- **No mocks for inference.** Test preprocessing math and postprocessing decoding with hand-crafted tensors. Real model tests are integration tests.
- **SQLite tests:** In-memory database (`:memory:`) for speed.
- **Camera tests:** `#[ignore]` — require physical hardware.
- **Benchmarks:** `criterion` in `benches/`, run manually or in dedicated CI job.

## Key Design Decisions

1. **Why `ort` over `tch-rs` or `tract`?** `ort` supports all target EPs (CUDA, TensorRT, CoreML, CPU) through one API. `tch-rs` locks you to PyTorch. `tract` has no GPU support.

2. **Why YOLO26 over YOLOv8?** 2x faster on CPU, NMS-free (no postprocessing latency), same accuracy. YOLOv8 is fallback if YOLO26 ONNX export has issues.

3. **Why SCRFD + ArcFace over YuNet?** YuNet detects faces but doesn't produce embeddings. SCRFD gives landmarks needed for face alignment before ArcFace embedding. This is the standard pipeline used by InsightFace.

4. **Why PaddleOCR over Tesseract?** PaddleOCR is GPU-accelerated, more accurate on scene text (license plates, signs), and exports to ONNX. Tesseract is CPU-only and designed for document text.

5. **Why SQLite over direct Postgres?** Edge devices have unreliable networks. SQLite WAL gives ~100k inserts/sec with zero network dependency. Postgres sync is a background concern, not on the hot path.

6. **Why ByteTrack over DeepSORT?** ByteTrack is faster (no appearance model needed), works well with high-confidence detectors like YOLO26, and has a pure Rust implementation (`jamtrack-rs`).

7. **Why channel-based pipeline over thread pool?** Channels give natural backpressure. When inference is slower than capture, the bounded channel drops frames instead of growing memory. Thread pools don't model this flow.
