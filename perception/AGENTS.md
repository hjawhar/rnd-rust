# Agent Instructions for Perception

## Project Overview

Perception is a Rust binary for real-time computer vision. It processes images, video, and camera feeds through configurable pipelines: object detection (YOLO26), face recognition (SCRFD + ArcFace), and OCR (PaddleOCR). All inference runs through ONNX Runtime via the `ort` crate.

## Architecture Rules

- **Single binary.** This is not a library. There is one entry point (`main.rs`). No `lib.rs`.
- **Pipeline architecture.** Capture is decoupled from inference via bounded async channels. Pipeline stages run as concurrent tokio tasks. CPU/GPU-bound inference uses `spawn_blocking`.
- **Edge-first storage.** SQLite WAL is the primary store. Postgres sync is optional and feature-gated.
- **Feature flags control hardware backends.** `cuda`, `tensorrt`, `coreml`, `preview`, `postgres` are all opt-in Cargo features.
- **Batch inference.** All per-crop pipelines (OCR, face recognition) support batched inference. When N crops need the same model, they are stacked into a single batched tensor. This is critical for burst scenarios (100+ objects in one frame).
- **Worker pools.** `EnginePool` holds multiple ONNX sessions for the same model, enabling N concurrent inference calls. Combined with batching, this handles burst loads without dropping to single-digit FPS.

## Code Conventions

### Language & Tooling

- **Rust 1.82+**, edition 2021
- **Error handling:** `thiserror` for error types, `anyhow` in `main.rs` only. Pipeline code uses typed errors.
- **Async:** `tokio` runtime. All I/O-bound code is async. CPU/GPU-bound inference calls use `tokio::task::spawn_blocking`.
- **Logging:** `tracing` crate with structured spans. No `println!` or `eprintln!` in production code.
- **CLI:** `clap` with derive API.
- **Serialization:** `serde` for all config and data types. Config format is TOML.

### Code Style

- `cargo fmt` with default settings
- `cargo clippy` must pass with no warnings
- Every public type, function, and module has a `///` or `//!` doc comment
- No `unwrap()` or `expect()` in production code paths. Tests may use `unwrap()`.
- Prefer `Result<T, PerceptionError>` over panics

### File Organization

- `src/` contains the application code, organized by module (see PLAN.md for full map)
- `tests/` contains integration tests, one file per module
- `tests/fixtures/` contains test images, sample configs, and tiny ONNX models
- `benches/` contains criterion benchmarks
- `migrations/` contains SQLx migration files
- Unit tests go in `#[cfg(test)] mod tests` blocks within source files
- Integration tests in `tests/` are for tests that need multiple modules working together

### Dependencies

Before adding a new dependency, check if an existing one already covers the need. Key crate choices:

| Need | Crate | Notes |
|------|-------|-------|
| ONNX inference | `ort` | v2.0.0-rc.12. Do not use `onnxruntime` (deprecated) |
| Image/video I/O | `opencv` | Capture, resize, color convert, preview |
| Async runtime | `tokio` | Full features. No `async-std`. |
| SQL | `sqlx` | With sqlite feature. No Diesel. |
| Object tracking | `jamtrack` | ByteTrack in pure Rust |
| Tensor math | `ndarray` | For model I/O tensors |
| CLI | `clap` | Derive API |
| Config | `toml` + `serde` | |
| Error types | `thiserror` | |
| Logging | `tracing` | |

### Concurrency

- OpenCV types (`Mat`, `VideoCapture`) are **not `Send`**. They stay on their dedicated OS thread. Convert to owned `Frame` before crossing channel boundaries.
- ONNX `Session` is `Send + Sync`. It can be shared across tasks via `Arc`.
- Pipeline channels are **bounded**. Never use unbounded channels.
- `spawn_blocking` for inference calls. Never block the tokio runtime with model inference.
- **Batch inference** is the primary throughput mechanism. Always batch per-crop work (OCR, face embedding) into a single inference call with batch dimension > 1.
- **EnginePool** enables parallel inference. Multiple sessions for the same model can run concurrently across `spawn_blocking` threads.

### Testing

- **TDD preferred.** Write the test first, then the implementation.
- **No mocks for inference.** Test preprocessing/postprocessing with hand-crafted tensors. Real model tests are integration tests gated with `#[ignore]`.
- **SQLite tests** use in-memory databases (`:memory:`).
- **Camera tests** are `#[ignore]` (require hardware).
- Run only tests you modified unless asked otherwise: `cargo test <test_name>`

## Implementation Plan

See [PLAN.md](PLAN.md) for the phased implementation plan with task breakdown, dependency graph, and design decisions.

## Build & Test Commands

```bash
# Check compilation
cargo check

# Run unit tests
cargo test

# Run integration tests (needs models)
cargo test -- --ignored

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt --check

# Build release binary
cargo build --release

# Build with CUDA
cargo build --release --features cuda
```
