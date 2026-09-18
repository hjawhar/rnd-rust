# Claude Instructions

Read and follow [AGENTS.md](AGENTS.md) for all project conventions, architecture rules, dependencies, and coding standards.

## Key Points

- This is a **Rust binary** (not a library). Single entry point in `main.rs`.
- Implementation plan is in [PLAN.md](PLAN.md). Follow the phased task breakdown.
- All inference goes through `ort` (ONNX Runtime). No other ML frameworks.
- Concurrency is critical: capture on OS thread, inference via `spawn_blocking`, storage via async. Bounded channels only.
- **Batch inference** for all per-crop pipelines. When 100 cars appear at once, their plates are batched into grouped inference calls across a worker pool — not processed one at a time.
- Tests are separated: unit tests in-module, integration tests in `tests/`.
- No mocks for inference. Test math (pre/postprocessing) with synthetic data. Real model tests are `#[ignore]`.
