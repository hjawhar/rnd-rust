# makana — agent guide

Vehicle telemetry platform. A Rust backend brokers `Sample`s from any source
(ELM327 dongles, ESP32 firmware over WebSocket, recorded logs, synthetic
mock) and fans them out to WebSocket clients. The frontend is a dashboard that
consumes the WS feed. The firmware end (future) runs on an ESP32 with a CAN
transceiver and speaks the same wire format.

## Project scope

- **What this is**: a personal platform for viewing your own car's data through
  firmware and software you control end-to-end.
- **What this is not**: a diagnostic tool clone (that's what ELM327 + off-the-shelf
  apps already do), an OEM-certified product, a safety-critical system.
- **Threat model / audience**: the author. No multi-tenant, no auth, no PII.

## Architecture

```
                      ┌── ELM327 emulator (dev, macOS-friendly)
                      │
    CarDataSource ────┼── ELM327 dongle over serial (prod, passive)
                      │
                      ├── ESP32 firmware over WS (prod, CAN direct)
                      │
                      ├── slcan / CANable USB dongle (prod, CAN direct)
                      │
                      └── Mock / replay log (test)
            │
            │ Sample stream
            ▼
       backend (Axum) ── broadcast::Sender<Sample> ──▶ WS clients
                                                          │
                                                          ▼
                                                   frontend dashboard
```

Every producer implements `CarDataSource`. The backend doesn't know or care
where a `Sample` came from — firmware is just another transport whose wire is
WebSocket instead of a pty. This is the load-bearing abstraction; keep it
honest.

## Layout

| Path                       | Role                                                        |
| -------------------------- | ----------------------------------------------------------- |
| `Cargo.toml`               | Workspace root; `[workspace.dependencies]` pin versions once |
| `crates/common/`           | `CanFrame`, `ObdPid`, `ObdValue`, `Sample`, `TransportError` |
| `crates/transport/`        | `CarDataSource` trait + `MockSource`, `Elm327Source`         |
| `crates/backend/`          | `makana-backend` binary — Axum server, WS broadcast         |
| `crates/backend/web/`      | Embedded static dashboard (`include_str!`)                  |
| `firmware/README.md`       | ESP32 firmware plan (not a cargo member — different target) |
| `README.md`                | User-facing quickstart                                      |

New crates go under `crates/`. Firmware will live in `firmware/esp32/` as a
separate cargo project when started — different toolchain (`espup`), different
target triple (`xtensa-esp32-none-elf` or `riscv32imc-unknown-none-elf`).

## Toolchain

- **Rust**: edition 2024, resolver 3, MSRV 1.90.
- **Target platform**: macOS (darwin) primary. Linux supported.
- **No SocketCAN on macOS** — `socketcan` crate is Linux-only. For raw CAN on
  macOS, use a CANable dongle via `slcan` over serial, or run the Linux path
  inside Docker.
- **Async runtime**: Tokio multi-thread. No blocking calls in async contexts.
- **HTTP**: Axum 0.8. Tower middleware. `tower-http` for CORS and tracing.
- **Errors**: `thiserror` for typed errors in library crates. `Box<dyn Error>`
  in the binary. Never `unwrap()` on external IO.
- **Tracing**: `tracing` + `tracing-subscriber` with `EnvFilter`. Control via
  `RUST_LOG`.

## Build & run

```sh
cargo build                          # all crates
cargo build -p makana-backend        # single crate
cargo test --workspace               # unit + integration
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt                            # format

cargo run -p makana-backend -- --transport mock --mock-tick-ms 50
cargo run -p makana-backend -- --transport elm327 --device /dev/ttys003
```

### Dev loop: mock source (zero setup)

```sh
cargo run -p makana-backend -- --transport mock
# open http://127.0.0.1:8080/ — dashboard with live gauges
# raw feed: ws://127.0.0.1:8080/ws — JSON Sample per tick
curl http://127.0.0.1:8080/health   # "ok"
```

### Dev loop: ELM327 emulator (realistic protocol, no hardware)

```sh
# First-time install (ELM327-emulator needs setuptools<70):
uv venv .venv
.venv/bin/python -m pip install 'setuptools<70'
.venv/bin/python -m pip install --no-build-isolation ELM327-emulator

# Launch emulator — batch-mode writes the pty path as its first line:
.venv/bin/elm -b /tmp/elm.txt &
PTY=$(head -1 /tmp/elm.txt)

# Point backend at the pty:
cargo run -p makana-backend -- --transport elm327 --device "$PTY"
```

The transport auto-detects pty paths (`/dev/ttys*` on macOS, `/dev/pts/*` on
Linux) and opens them as plain files instead of through `tokio-serial`.
`tokio-serial` calls `tcsetattr` to set baud rate, which ptys reject with
ENOTTY. Real serial devices (`/dev/cu.*`, `/dev/ttyUSB*`) go through
`tokio-serial` as usual so baud rate works.

### Dev loop: real car

Plug an ELM327-compatible USB or Bluetooth dongle into the OBD-II port. On
macOS the dongle appears as `/dev/cu.usbserial-*` or `/dev/cu.Bluetooth-*`.
Same command, different device path.

## CLI surface (makana-backend)

All flags also read from env with `MAKANA_` prefix.

| Flag              | Env                    | Default            | Purpose                                                |
| ----------------- | ---------------------- | ------------------ | ------------------------------------------------------ |
| `--transport`     | `MAKANA_TRANSPORT`     | `mock`             | `mock`, `elm327`, or `logfile`                         |
| `--device`        | `MAKANA_DEVICE`        | —                  | Serial/pty for `elm327`; JSONL path for `logfile`      |
| `--baud`          | `MAKANA_BAUD`          | `38400`            | ELM327 serial baud (ignored for ptys)                  |
| `--pids`          | `MAKANA_PIDS`          | (built-in default) | Comma-separated 4-hex-digit PIDs, e.g. `010C,010D`     |
| `--speed`         | `MAKANA_SPEED`         | `1.0`              | Logfile replay multiplier (`>1` faster, `<1` slower)   |
| `--repeat`        | `MAKANA_REPEAT`        | `false`            | Logfile loops back to start at EOF                     |
| `--record`        | `MAKANA_RECORD`        | —                  | Append every broadcast `Sample` as JSONL to this path  |
| `--no-discover`   | `MAKANA_NO_DISCOVER`   | absent (on)        | Disable ELM327 supported-PID auto-discovery            |
| `--record-rotate-mb` | `MAKANA_RECORD_ROTATE_MB` | —              | Byte-count rotation threshold for `--record`           |
| `--record-keep`   | `MAKANA_RECORD_KEEP`   | `5`                | Rotated files to keep (requires `--record-rotate-mb`)  |
| `--bind`          | `MAKANA_BIND`          | `127.0.0.1:8080`   | HTTP/WS listen address                                 |
| `--mock-tick-ms`  | `MAKANA_MOCK_TICK_MS`  | `100`              | Mock source cadence                                    |

## Wire format

Every WS message is a JSON-encoded `Sample`:

```json
{
  "source": "elm327:/dev/ttys003",
  "timestamp_ms": 1760000000000,
  "payload": { "type": "obd", "kind": "rpm", "value": 3500 }
}
```

`payload.type` is `obd`, `can`, or `event`. `ObdValue` variants use adjacent
tagging (`kind` + `value`) because serde internal tagging doesn't support
newtype-over-primitive variants. Keep it that way — the frontend depends on
this shape.

Events (`connected`, `disconnected`, `error`) are first-class payloads so the
UI can show source liveness.

## Conventions

### Data model

- All timestamps are `u64` milliseconds since the Unix epoch (`Millis` in
  `makana-common`). Portable across desktop and embedded, `serde`-cheap.
- `CanFrame.id` is `u32`. `extended` flag distinguishes 11-bit from 29-bit.
- `ObdValue` variants encode units in the name (`SpeedKph`, `CoolantTempC`).
  The frontend never guesses units.
- Unknown PIDs become `ObdValue::Raw { pid, bytes }` — preserve rather than
  drop. Add a decoder arm when you learn the formula.

### Transport contract

- `CarDataSource::start` consumes `Box<Self>`. A source is **single-shot** —
  reconnection means constructing a new instance. This keeps lifecycle honest
  and eliminates "is it started" state machines.
- Sources own their retry/timeout policy. Transient errors on individual
  queries do **not** kill the stream (engine-off is normal). Fatal errors
  surface as `Err` items and end the stream.
- First item is usually `SourceEvent::Connected`. Consumers rely on this.
- `SampleStream` is `Pin<Box<dyn Stream<...> + Send>>` — type-erased so the
  backend can multiplex sources without generics.

### Adding a new transport

1. Create `crates/transport/src/mynew.rs`.
2. Implement `CarDataSource` with `async fn start(self: Box<Self>)`.
3. Add a variant to `TransportKind` in `crates/backend/src/main.rs` and wire
   the CLI branch.
4. Write at least one decoder test in `mynew.rs` if you're parsing wire bytes.
5. If it's a long-running poll/read loop, spawn it in a tokio task driven by a
   `mpsc::channel`; wrap with `ReceiverStream::new(rx).boxed()`.

### Adding a new OBD PID decoder

1. Add the PID constant to `ObdPid` in `crates/common/src/lib.rs`.
2. Add the typed variant to `ObdValue` with units in the name.
3. Add the decoding arm in `elm327::decode_response`.
4. Add a decoder unit test with a known-good reply.
5. If it's a common PID, add it to `DEFAULT_PIDS` in `elm327.rs`.

Formulas come from the [SAE J1979 / OBD-II PID reference](https://en.wikipedia.org/wiki/OBD-II_PIDs).
Paste the formula into a comment next to the decoding arm.

### Vendor-specific PIDs

Standard OBD-II is the floor, not the ceiling. BMW, VW, Toyota etc. expose
extra data on manufacturer-specific modes (0x22, UDS services) or on separate
buses (PT-CAN, K-CAN). When you start decoding these:

- Add a `ObdValue::Vendor { vendor, pid, value }` variant, or introduce a
  parallel `VendorValue` type if the shapes diverge significantly.
- Keep vendor decoders in their own module (`elm327/bmw.rs` etc.) — don't
  pollute the standard decoder.
- Document the source (forum post, Leaf Spy, BimmerCode DB, repo) next to
  every formula. These are reverse-engineered, not specified.

## Testing

- **Unit tests**: co-located in `#[cfg(test)] mod tests`. Decoder tests MUST
  use literal hex replies from real adapters, not synthesized strings.
- **Integration**:
  - `crates/backend/tests/ws_smoke.rs` — mock source end-to-end. Runs in the
    default `cargo test` pass.
  - `crates/backend/tests/elm327_emulator.rs` — spawns the real Python
    `ELM327-emulator`, opens its pty, drives the ELM327 transport through a
    full init sequence + polling loop, asserts decoded OBD values arrive
    over WS. Marked `#[ignore]` so `cargo test` doesn't require the emulator
    installed; run it explicitly:
    ```sh
    cargo test -p makana-backend --test elm327_emulator -- --ignored --nocapture
    ```
    Install the emulator once with:
    ```sh
    uv venv .venv
    .venv/bin/python -m pip install 'setuptools<70'
    .venv/bin/python -m pip install --no-build-isolation ELM327-emulator
    ```
    (ELM327-emulator 3.0.5 requires `setuptools<70` because of a
    `pkg_resources` build-time dependency that newer setuptools removed.)
  - Add one of these for any new transport that can be exercised without
    real hardware.
- **No mocks**: the `MockSource` is a real implementation, not a mock. Don't
  introduce mock trait implementations for testing — if you need to test
  against a fake serial device, use `tokio::io::duplex` or a pty pair.

Run tests with `cargo test --workspace`. Current tests: 59 default + 1 ignored
end-to-end (serde round-trips + DTC decoding + ELM327 decoder + pty-path
detection + PID-bitmap decoding + mock stream + logfile replay + backoff +
recorder rotation + CLI parsers + WS smoke + ELM327 emulator end-to-end).

## Observability

- `tracing::info!` for lifecycle events (source start, client connect/disconnect).
- `tracing::warn!` for recoverable errors (lagged WS client, transient transport
  error, unknown PID).
- `tracing::debug!` for per-sample noise.
- Filter via `RUST_LOG=makana_backend=debug,makana_transport=debug`.

## What's deliberately not here

- **Auth / TLS**: not needed for a single-user localhost tool. When exposing
  across a network (firmware → cloud backend), add `rustls` at the edge.
- **Persistence**: the backend is a broker, not a database. Log replay lives
  in `LogSource` (JSONL file-in, `Sample`-out); recording lives behind
  `--record` (append-only JSONL tail of the broadcast feed). No DB, no
  SQL, no binary log format.
- **Writes to the ECU**: UDS write services, actuator tests, flashing —
  explicitly out of scope. This tool is read-only. DTC *read* (Mode 03) is
  in. DTC *clear* (Mode 04) is not.
- **Frontend framework**: the dashboard is a single `index.html` embedded with
  `include_str!`. No build step, no node_modules, no framework. If the data
  model outgrows this — charts, history, complex state — move to Vite+Svelte
  or Angular. Not before.

## Workflow rules

- **Don't add dependencies for one-line helpers.** Check if `std` or existing
  deps cover it.
- **Don't introduce abstraction layers without a second call site.** Two copies
  of a pattern is when you extract.
- **Don't add backwards-compat shims.** Change the type, fix all callers.
- **Don't commit `.env` files or real vehicle logs.** They leak location and
  VIN. Fake data only in the repo.
- **Run `cargo clippy -- -D warnings` and `cargo test --workspace` before
  claiming done.** The `ws_smoke` test is the only thing that proves the
  system is wired; a green `cargo check` means nothing.
