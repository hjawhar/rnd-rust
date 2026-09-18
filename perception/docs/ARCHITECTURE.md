# Architecture

## Pipeline Overview

```
                         bounded channel (cap=4)
  [Capture Thread] ──────────────────────────────> [Async Pipeline Loop]
   (OS thread)              Frame                    (tokio runtime)
   OpenCV Mat                                             │
   -> owned Frame                                         ▼
                                                   ┌─────────────┐
                                                   │  Detection   │
                                                   │  (YOLO26)    │
                                                   └──────┬──────┘
                                                          │ Vec<Detection>
                                                          ▼
                                                   ┌─────────────┐
                                                   │  Tracking    │
                                                   │  (IoU SORT)  │
                                                   └──────┬──────┘
                                                          │ Vec<TrackedDetection>
                                            ┌─────────────┼─────────────┐
                                            ▼             │             ▼
                                     ┌────────────┐      │      ┌────────────┐
                                     │ Face Recog  │      │      │    OCR     │
                                     │ (new person)│      │      │(new vehicle│
                                     │ SCRFD+Arc   │      │      │ DBNet+CRNN)│
                                     └──────┬─────┘      │      └─────┬──────┘
                                            │             │            │
                                            └─────────────┼────────────┘
                                                          ▼
                                                   ┌─────────────┐
                                                   │   Events     │
                                                   │  (build +    │
                                                   │   persist)   │
                                                   └──────┬──────┘
                                                          │
                                              ┌───────────┴───────────┐
                                              ▼                       ▼
                                       ┌────────────┐         ┌────────────┐
                                       │   SQLite    │         │   Crops    │
                                       │   (WAL)     │         │ (fs, by    │
                                       │             │         │  date)     │
                                       └────────────┘         └────────────┘
```

## Concurrency Model

Two execution contexts:

1. **Capture thread** (dedicated `std::thread`). OpenCV `Mat` is not `Send`, so capture runs on a native OS thread. Each frame is copied into an owned `Frame` (BGR byte vector) and sent over a bounded `tokio::sync::mpsc` channel (capacity 4). If the pipeline falls behind, frames are dropped at the sender via `try_send` and counted.

2. **Async pipeline loop** (tokio runtime, single-task). Receives frames from the channel and runs detection, tracking, recognition, OCR, and storage sequentially per frame. All ONNX inference uses `Engine::run()` which locks a `Mutex<ort::Session>`. The `EnginePool` provides round-robin access to multiple sessions for concurrent inference on the same model, but the current pipeline processes stages sequentially within each frame.

Shutdown is cooperative: Ctrl+C breaks the `tokio::select!` loop, remaining channel frames are drained, and the capture thread's channel sender drop causes it to exit.

## Module Responsibilities

### `main` (`src/main.rs`)
CLI entry point. Parses arguments via `clap`, initializes `tracing`, and dispatches to `pipeline::run`, `download::ensure_models`, `face_db` management, or `engine::print_info`.

### `config` (`src/config.rs`)
TOML configuration loading and validation. All sections except `[capture]` have defaults, so a minimal config with just a source type and path is sufficient. Validates source type, confidence thresholds, backend selection, and ensures at least one pipeline stage is enabled.

### `types` (`src/types.rs`)
Domain types shared across the system. `Frame` owns pixel data as `Vec<u8>` (BGR) so it is `Send`. `Detection` carries a bounding box, confidence, class, and label. `TrackedDetection` wraps a `Detection` with a stable `track_id` and `is_new` flag. `Event` is the persistence unit, aggregating detection metadata, optional face identity, optional OCR text, and crop path.

### `error` (`src/error.rs`)
Unified error enum (`PerceptionError`) with `thiserror` derivation. Variants: `Config`, `Capture`, `Inference`, `Storage`, `Download`, `Io`, `OpenCv`. Converts from `opencv::Error`, `sqlx::Error`, and `ort::Error`.

### `engine` (`src/engine.rs`)
ONNX Runtime session wrapper. `Engine` holds a `Mutex<ort::Session>` and selects execution providers at construction (CUDA > TensorRT > CoreML > CPU, via compile-time features). `EnginePool` manages multiple `Engine` instances with atomic round-robin for concurrent access. `default_pool_size()` returns 2 for GPU builds, `num_cpus/4` (clamped 1..8) for CPU.

### `download` (`src/download.rs`)
Model registry and download manager. Maintains a static registry of model URLs and SHA256 checksums. `ensure_models()` checks the local cache directory, downloads missing models with progress bars (`indicatif`), and verifies integrity. Returns `ModelPaths` indicating which models are available.

### `capture` (`src/capture/`)
Frame source abstraction over OpenCV. `FrameSource` trait with `next_frame() -> Option<Frame>`. Implementations: `ImageSource` (single image, yields once), `VideoSource` (video file), `CameraSource` (live camera or RTSP stream). `mat_to_frame()` copies pixel data out of OpenCV `Mat` to produce a `Send`-safe `Frame`.

### `pipeline` (`src/pipeline/`)
Orchestrator. `Pipeline::new()` conditionally constructs detector, face pipeline, OCR pipeline, tracker, storage backend, and preview window based on config flags and available models. `Pipeline::process_frame()` runs the per-frame logic. `pipeline::run()` is the top-level entry point that spawns the capture thread and runs the async processing loop.

### `pipeline::detector` (`src/pipeline/detector.rs`)
YOLO26 object detection. Letterbox-resizes the input frame, runs ONNX inference, and decodes output tensors into `Vec<Detection>` with bounding boxes mapped back to original coordinates. Supports class filtering and confidence thresholding. Uses COCO 80-class labels.

### `pipeline::tracker` (`src/pipeline/tracker.rs`)
IoU-based multi-object tracker (simplified SORT/ByteTrack). Maintains active tracks across frames. Incoming detections are matched to tracks via greedy IoU assignment (sorted by descending IoU). Unmatched detections create new tracks (`is_new = true`). Unmatched tracks age out after `max_age` frames.

### `pipeline::face` (`src/pipeline/face.rs`)
Two-stage face recognition. Stage 1: SCRFD face detection produces bounding boxes and 5-point landmarks. Stage 2: ArcFace extracts a 512-dimensional embedding from each detected face crop. Embeddings are matched against the `FaceDb` via cosine similarity.

### `pipeline::ocr` (`src/pipeline/ocr.rs`)
Two-stage OCR. Stage 1: DBNet text detection produces a probability map thresholded into text-region bounding boxes. Stage 2: CRNN recognition crops each region, batches them (up to `max_batch_size`), and decodes via CTC greedy search against a character dictionary loaded from a keys file.

### `face_db` (`src/face_db.rs`)
In-memory database of known face embeddings. Loads serialized embeddings from a directory of files. Supports add, remove, list operations. `find_match()` computes cosine similarity against all stored embeddings and returns the best match above the configured threshold.

### `storage` (`src/storage/`)
Pluggable persistence. `StorageBackend` trait with `store_events()`, `store_crop()`, `query_events()`. `create_storage()` dispatches on the backend string ("sqlite" currently). PostgreSQL sync is stubbed behind a feature flag.

### `storage::sqlite` (`src/storage/sqlite.rs`)
SQLite backend using `sqlx` with WAL journaling. Batch-inserts events in a transaction. Supports filtered queries with dynamic parameter binding.

### `storage::crops` (`src/storage/crops.rs`)
Writes detection crop images to `{base_dir}/{YYYY-MM-DD}/{event_id}.jpg`. Returns relative paths for storage in the events table.

### `preview` (`src/preview.rs`)
Optional live display window (feature-gated behind `preview`). Uses OpenCV highgui to render annotated frames with bounding boxes, labels, and tracking IDs.

## Data Flow

```
Frame                     Raw pixel data (BGR Vec<u8>) with dimensions and timestamp.
  │                       Produced by capture thread, sent over bounded channel.
  ▼
Detection                 Bounding box + confidence + class_id + label + kind.
  │                       Produced by YoloDetector from a single frame.
  ▼
TrackedDetection          Detection + stable track_id + is_new flag.
  │                       Produced by ObjectTracker. track_id persists across frames
  │                       for the same physical object. is_new=true only on first appearance.
  │
  ├── [if is_new && person]  ──> FacePipeline ──> FaceDb.find_match() ──> (identity, similarity)
  ├── [if is_new && vehicle] ──> OcrPipeline  ──> (plate text)
  │
  ▼
Event                     Persistence record. UUID + timestamp + frame_number + track_id
                          + detection metadata + optional face_identity + optional ocr_text
                          + optional crop_path. Batch-inserted into SQLite.
```

The tracker is the key optimization boundary. Face recognition and OCR only run on **new** tracked detections (`is_new = true`), not on every frame. A person visible for 300 frames triggers recognition once, not 300 times.

## Storage Schema

```sql
CREATE TABLE IF NOT EXISTS events (
    id              TEXT PRIMARY KEY,       -- UUID v4
    timestamp       TEXT NOT NULL,          -- RFC 3339
    frame_number    INTEGER NOT NULL,
    track_id        INTEGER,               -- stable tracker ID, NULL if tracking disabled
    detection_kind  TEXT NOT NULL,          -- "object" | "face" | "text"
    label           TEXT,                   -- COCO class label (e.g. "person", "car")
    confidence      REAL NOT NULL,
    bbox_x1         REAL NOT NULL,
    bbox_y1         REAL NOT NULL,
    bbox_x2         REAL NOT NULL,
    bbox_y2         REAL NOT NULL,
    ocr_text        TEXT,                   -- recognized text, NULL if not applicable
    face_identity   TEXT,                   -- matched face name, NULL if unknown/not applicable
    face_similarity REAL,                   -- cosine similarity score
    crop_path       TEXT,                   -- relative path to cropped image
    synced          INTEGER NOT NULL DEFAULT 0  -- 0/1 flag for Postgres sync
);

CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_track_id  ON events(track_id);
CREATE INDEX IF NOT EXISTS idx_events_label     ON events(label);
CREATE INDEX IF NOT EXISTS idx_events_synced    ON events(synced);
```

WAL journaling is enabled for concurrent read access during writes. The connection pool is capped at 4 connections. Events are batch-inserted within a single transaction per frame.

## Performance Characteristics

**Throughput bottleneck: ONNX inference.** Each frame passes through at least one ONNX model (YOLO26 detection). With face recognition and OCR enabled, a single frame may require up to 5 model invocations (detection + face detection + face embedding + OCR detection + OCR recognition). Inference dominates frame processing time.

**Bounded channel backpressure.** The capture-to-pipeline channel has capacity 4. When the pipeline cannot keep up, frames are dropped at the capture thread (`try_send` failure) rather than accumulating unbounded memory. Dropped frame counts are tracked and logged.

**Tracking reduces downstream load.** The tracker assigns stable IDs and marks only first-appearance detections as `is_new`. Face recognition and OCR only execute on new detections. Without tracking, every frame would trigger recognition for every visible object. With tracking, an object visible for N frames triggers recognition once. This is the primary throughput multiplier for multi-model pipelines.

**OCR batching.** The CRNN recognition stage batches multiple text regions into a single inference call (up to `max_batch_size`, default 32). This amortizes model invocation overhead when multiple text regions are detected in a single frame.

**Engine pooling.** `EnginePool` maintains multiple ONNX sessions for the same model. Round-robin distribution enables concurrent inference when the pipeline is extended to parallel processing. Current pipeline is sequential per-frame, so pool size > 1 prepares for future concurrency.

**SQLite WAL.** Write-ahead logging allows concurrent reads during writes. Events are batch-inserted per-frame in a single transaction, minimizing fsync overhead.

## Configuration Overview

Configuration is loaded from a TOML file (default: `perception.toml`). Every section except `[capture]` has defaults.

```toml
[capture]
source = "video"              # "image" | "video" | "camera" | "rtsp"
path = "./input.mp4"          # file path, device index, or RTSP URL
fps_limit = 30                # max frames/sec from source

[pipeline]
detection = true              # enable YOLO26 object detection
face_recognition = false      # enable SCRFD + ArcFace face pipeline
ocr = false                   # enable DBNet + CRNN text pipeline
confidence_threshold = 0.5    # minimum detection confidence (0.0-1.0)

[pipeline.detection_config]
model = "yolo26n"             # model variant from registry
classes = []                  # class filter; empty = all 80 COCO classes

[pipeline.face_config]
detection_model = "scrfd_2.5g"
recognition_model = "arcface_r50"
known_faces_dir = "./faces/"
similarity_threshold = 0.6    # cosine similarity threshold for identity match

[pipeline.ocr_config]
model = "ppocr_v5"
languages = ["en"]
max_batch_size = 32           # max text regions per CRNN inference batch

[pipeline.tracker_config]
enabled = true                # disable to skip tracking (all detections treated as new)
max_age = 30                  # frames before unmatched track is pruned
iou_threshold = 0.3           # minimum IoU for detection-track association

[storage]
backend = "sqlite"            # "sqlite" | "postgres" (postgres not yet implemented)
sqlite_path = "./perception.db"
crops_dir = "./crops/"
save_crops = true

[storage.sync]
enabled = false               # periodic sync to remote Postgres
postgres_url = ""             # required when sync.enabled = true
interval_secs = 60

[preview]
enabled = false               # live display window (requires "preview" feature)
window_width = 1280

[models]
cache_dir = "./models/"       # local directory for downloaded ONNX models
auto_download = true          # download missing models on startup
```

Config validation enforces: known source types, confidence/similarity thresholds in [0,1], at least one pipeline stage enabled, known storage backend, and `postgres_url` required when sync is enabled.
