# irc-proto fuzz harness

Libfuzzer targets for the `irc-proto` crate. Kept out of the workspace
so the main `cargo build` never compiles them — libfuzzer needs
nightly.

## Targets

| Target     | Exercises                                           |
|------------|-----------------------------------------------------|
| `decoder`  | `Message::parse` against arbitrary byte slices      |
| `casemap`  | `Casemap::fold` + reflexive equality on any bytes   |
| `codec`    | `IrcCodec::decode` with a split-buffer feed pattern |

## Setup

```sh
rustup install nightly
cargo install cargo-fuzz --locked
```

## Running

```sh
cd crates/irc-proto/fuzz
cargo +nightly fuzz run decoder -- -max_total_time=60
cargo +nightly fuzz run casemap -- -max_total_time=60
cargo +nightly fuzz run codec   -- -max_total_time=60
```

CI runs a short smoke sweep on push and a longer sweep nightly via
`.github/workflows/fuzz.yml` (added in Phase 13).

Corpus artifacts live in `corpus/` and `artifacts/` — both gitignored.
Promote any recurring crash into a regression test in the target
crate under `#[cfg(test)]` before discarding it.
