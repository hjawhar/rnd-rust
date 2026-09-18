# firmware

The firmware end of makana: runs on a microcontroller, reads CAN directly from
the car, forwards decoded `Sample`s to the backend over WebSocket/WiFi.

## Target: ESP32 (TWAI peripheral + WiFi + Rust)

Picked because it's the only hobby-priced MCU that has CAN, WiFi, and a
production-grade Rust ecosystem in one chip.

- **Chip**: ESP32 (classic) or ESP32-S3. Both have the TWAI (CAN) peripheral.
  Avoid ESP32-C3 — no CAN.
- **Rust**: `esp-rs/esp-hal` + `embassy` for async. No RTOS.
- **Transceiver**: MCP2551 or TJA1050 module ($2 on AliExpress).
- **Physical**: MCU TWAI pins → transceiver CHIP side → CAN_H/CAN_L to the
  car's OBD-II port (pins 6 and 14 for CAN).

## Architecture

```
  Car OBD-II ──CAN_H/CAN_L──▶ TJA1050 ──TX/RX──▶ ESP32 TWAI
                                                    │
                                    decode (same crate as backend?)
                                                    │
                                     WiFi ──WS──▶ makana-backend /ingest
```

The firmware is just another `CarDataSource` — on the backend side. On the MCU
side it's the "producer end" of the same wire format:

- Firmware serializes `Sample` as JSON (or CBOR to save bytes) over WS.
- Backend exposes an `/ingest` WS endpoint, wraps incoming frames as
  `WsIngestSource: CarDataSource`, and fans them out the same way as any other
  source.

## Milestones

1. **Bring-up**: blink + WiFi join + `println!` over USB.
2. **CAN echo**: TWAI loopback test. No car yet.
3. **CAN sniff**: parse raw frames on a known bus (ICSim or the car at idle).
4. **OBD polling**: request mode 01 PIDs over CAN, decode into `Sample`.
5. **Uplink**: WS client to `makana-backend`, send `Sample`s as JSON.
6. **Resilience**: reconnect loop, ring buffer when offline, OTA later.

## Why not just use an ELM327 dongle forever?

ELM327 is request-response and slow (~50 PIDs/sec realistically). It also only
exposes standard OBD-II. Manufacturer-specific PIDs (oil temp, transmission
data, turbo boost on BMWs, charging state on EVs) are on separate buses or
require vendor protocols. A direct TWAI attach gives you:

- Every frame the ECU broadcasts, passively, at bus line rate.
- Ability to query manufacturer-specific UDS services when you know the IDs.
- Physical independence from the laptop — firmware runs in the car 24/7.

## When to start

After the backend + mock-source + ELM327 path works end-to-end and you have a
frontend consuming the WS feed. The firmware will slot in as a drop-in source
with no backend changes if the trait boundary holds.

## Not in this repo yet

The firmware crate isn't in the workspace because it uses a different target
(`xtensa-esp32-none-elf` or `riscv32imc-unknown-none-elf`) and a different
toolchain (`espup`). It'll live in `firmware/esp32/` as a separate cargo project
when started. Shared types from `crates/common` will be pulled in as a git
path dep with `default-features = false` for `no_std` use.
