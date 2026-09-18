# Perception

A high-throughput Rust binary for real-time computer vision: object detection, face recognition, and OCR. Designed for edge deployment on surveillance systems, drones, and roadside units with powerful GPU/CPU hardware.

![Perception Demo](docs/demo.gif)

## What It Does

- **Object Detection** — Detects and classifies objects (vehicles, people, bicycles, etc.) using YOLO26 via ONNX Runtime
- **Face Recognition** — Detects faces (SCRFD) and matches them against a known faces database (ArcFace embeddings)
- **OCR** — Detects and reads text in the scene (license plates, signs) using PaddleOCR
- **Multi-Object Tracking** — ByteTrack assigns stable IDs across frames, avoiding redundant processing
- **High-Throughput Storage** — SQLite WAL for edge-local storage (~100k events/sec), optional Postgres batch sync
- **Flexible Input** — Single images, video files, live cameras, or RTSP streams

## Quick Start

```bash
# Build (CPU-only)
cargo build --release

# Download models
./target/release/perception download

# Run on an image
./target/release/perception run -c perception.toml --source image --path photo.jpg

# Run on a video file
./target/release/perception run -c perception.toml --source video --path highway.mp4

# Run on a live camera
./target/release/perception run -c perception.toml --source camera --path 0

# Run on an RTSP stream
./target/release/perception run -c perception.toml --source rtsp --path "rtsp://192.168.1.100:554/stream"
```

## Building with GPU Support

```bash
# NVIDIA CUDA
cargo build --release --features cuda

# NVIDIA TensorRT (maximum inference speed)
cargo build --release --features tensorrt

# Apple Silicon (CoreML)
cargo build --release --features coreml

# With live preview window
cargo build --release --features preview

# With Postgres sync support
cargo build --release --features postgres

# Kitchen sink
cargo build --release --features "cuda,preview,postgres"
```

### Build Dependencies

- **Rust** 1.82+
- **OpenCV** 4.x (with videoio, imgcodecs, imgproc modules)
- **CUDA Toolkit** 12.x (if using `cuda` or `tensorrt` features)
- **CMake** (for opencv-rust build)

#### macOS
```bash
brew install opencv
```

#### Ubuntu/Debian
```bash
sudo apt install libopencv-dev
```

## Configuration

Copy `perception.example.toml` to `perception.toml` and edit to your needs. See [PLAN.md](PLAN.md) for the full configuration reference.

Key configuration sections:

| Section | Purpose |
|---------|---------|
| `[capture]` | Input source type, path, FPS limit |
| `[pipeline]` | Enable/disable detection, face, OCR; confidence thresholds |
| `[pipeline.detection]` | Model variant, target classes |
| `[pipeline.face]` | Detection/recognition models, known faces directory, similarity threshold |
| `[pipeline.ocr]` | Model variant, languages |
| `[pipeline.tracker]` | ByteTrack parameters (max age, IOU threshold) |
| `[storage]` | Backend (sqlite/postgres), paths, crop saving |
| `[storage.sync]` | Optional Postgres batch sync interval |
| `[preview]` | Live preview window toggle |
| `[models]` | Model cache directory, auto-download toggle |

## CLI Reference

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

### Managing Known Faces

```bash
# Add a known face (provide name + photo with a clear face)
perception faces add "John Doe" john.jpg

# List all known faces
perception faces list

# Remove a known face
perception faces remove "John Doe"
```

## Architecture

See [PLAN.md](PLAN.md) for the full architecture, concurrency model, and implementation plan.

**Pipeline overview:**

```
Capture (OS thread) → Router (async) → Detection Workers (spawn_blocking) → Tracker → Storage (async batched)
```

Each pipeline stage (object detection, face recognition, OCR) runs concurrently. The tracker ensures expensive models only run on newly-appearing objects, not every frame. This is the key to handling thousands of objects at highway speeds.

## Supported Models

| Task | Model | Input Size | Speed |
|------|-------|------------|-------|
| Object Detection | YOLO26n | 640x640 | ~5ms (GPU) |
| Object Detection | YOLO26s | 640x640 | ~10ms (GPU) |
| Face Detection | SCRFD 2.5G | 640x640 | ~3ms (GPU) |
| Face Recognition | ArcFace R50 | 112x112 | ~2ms (GPU) |
| Text Detection | PaddleOCR DBNet | 640x640 | ~5ms (GPU) |
| Text Recognition | PaddleOCR CRNN | 48x320 | ~3ms (GPU) |

Models are automatically downloaded on first run (configurable via `[models]` section).

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `cpu` | yes | CPU inference (always available as fallback) |
| `cuda` | no | NVIDIA CUDA execution provider |
| `tensorrt` | no | NVIDIA TensorRT execution provider |
| `coreml` | no | Apple CoreML execution provider |
| `preview` | no | OpenCV highgui live preview window |
| `postgres` | no | PostgreSQL sync support |

## Storage Schema

Events are stored in SQLite with the following schema:

```sql
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    frame_number INTEGER NOT NULL,
    track_id INTEGER,
    detection_kind TEXT NOT NULL,    -- 'object', 'face', 'text'
    label TEXT,                      -- 'car', 'person', 'John Doe', etc.
    confidence REAL NOT NULL,
    bbox_x1 REAL, bbox_y1 REAL, bbox_x2 REAL, bbox_y2 REAL,
    ocr_text TEXT,                   -- recognized text (if OCR)
    face_identity TEXT,              -- matched face name (if face)
    face_similarity REAL,
    crop_path TEXT,                  -- path to saved image crop
    synced INTEGER DEFAULT 0         -- 0=pending, 1=synced to Postgres
);
```

## Testing

```bash
# Unit tests (no models needed)
cargo test

# Integration tests (requires downloaded models)
perception download
cargo test -- --ignored

# Benchmarks
cargo bench
```

## License

MIT. See [LICENSE](LICENSE).
