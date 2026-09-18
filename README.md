# rnd-rust

**Archive of my Rust R&D projects. No longer maintained.**

I've put a hold on working with Rust. I'm shifting careers and learning other
things, so everything Rust-related here is deprecated / decommissioned /
stopped. I'm currently focused on my **Master's in Applied AI**.

These twelve repositories were consolidated into this single archive and the
originals were removed. Nothing here is under active development, nothing will
receive fixes, and none of it should be treated as production-ready. Code is
kept public purely as a record of what was built.

---

## What's in here

| Project | What it was |
|---|---|
| [`perception`](perception/) | High-throughput real-time computer vision binary — object detection (YOLO26 / ONNX Runtime), face recognition, and OCR for edge deployment on surveillance systems, drones, and roadside units. |
| [`bambu-homelab`](bambu-homelab/) | Self-hosted fleet management for Bambu Lab 3D printers — a fully local replacement for Bambu Cloud and Bambu Handy. |
| [`makana`](makana/) | Vehicle telemetry platform. Reads car data from OBD-II dongles, ESP32 firmware, or recorded logs; serves it over WebSocket to a live dashboard. (*makana* — مَكَنَة — Arabic for "engine.") |
| [`monitored`](monitored/) | Docker and PM2 instance checker. Uses a `russh` client to inspect running services across many servers simultaneously. |
| [`distributed-system`](distributed-system/) | Distributed-systems playground: service discovery, pub/sub, request-reply, horizontal scaling, and distributed tracing over NATS, with Prometheus / Grafana / Jaeger. |
| [`downloader`](downloader/) | Simple concurrent file downloader. |
| [`localsend-rs`](localsend-rs/) | Rust implementation of the LocalSend v2 protocol with a Tauri + Angular GUI for peer-to-peer file transfer. |
| [`irc`](irc/) | Full modular IRC stack — standards-compliant server, mIRC-style GUI client, and a 24/7 bouncer, sharing one protocol crate. |
| [`dex-volume-app`](dex-volume-app/) | Volume Maker — Rust backend with an Angular 19 frontend for DEX volume generation. |
| [`solana-twitter-sniper`](solana-twitter-sniper/) | Monitored Twitter/X for token signals and acted on them on Solana. |
| [`solana-thunder`](solana-thunder/) | Solana DEX aggregator. Loaded 2M+ pools across 6 protocols (Raydium V4/CLMM, Meteora DAMM V1/V2, Meteora DLMM, Pumpfun AMM) for multi-hop routing and real-time pricing with no external APIs. |
| [`solana-rpc-tester`](solana-rpc-tester/) | Benchmarked Solana RPC endpoints — timed transactions via SWQoS or Jito bundles, confirmation latency tracked through a Yellowstone/Geyser gRPC subscription. |

---

## Status

Deprecated. Decommissioned. Not accepting issues or pull requests.

Each subdirectory is a source snapshot at the point work stopped; per-project
build instructions live in their own `README.md`. Expect bit-rotted
dependencies and toolchain drift.
