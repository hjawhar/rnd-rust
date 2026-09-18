# IRC Suite

A modern, modular IRC stack written entirely in Rust: a standards-compliant **server**, an **mIRC-style GUI client**, and a 24/7 **bouncer** — all built from a shared protocol crate.

## What's in the box

| Component | Crate | What it does |
|---|---|---|
| **Server** | `irc-server` | Single-node IRC daemon. RFC 1459/2812 + IRCv3 (CAP 302, SASL PLAIN/EXTERNAL, server-time, echo-message, account-notify, MONITOR). Account registration (REGISTER/VERIFY), HMAC cloaks, class-based operators, KLINE/KILL, flood control, connection limiter, TLS (rustls), PROXY protocol v2, Prometheus metrics, SQLite storage. |
| **GUI Client** | `irc-client-gui` | mIRC-style desktop client built on iced. Connect dialog, multi-server treebar, per-server status windows, channel/query windows, nick list, light/dark theme toggle, `/help` command with guidelines, desktop notifications, DCC, Rhai scripting. |
| **Bouncer** | `irc-bnc` | Persistent 24/7 upstream connections. Replays missed traffic with IRCv3 `server-time` tags. Multi-user, multi-network, per-target message buffering, `*status` admin pseudo-user. |
| **Client Core** | `irc-client-core` | Headless client library shared by the GUI and bouncer. Multi-network connection manager, state tracking, Rhai scripting engine. |
| **Protocol** | `irc-proto` | Wire parser/serializer (zero-copy, fuzzed), typed Command enum, ReplyCode numerics, casemap, newtypes, tokio codec, CAP/ISUPPORT, modes, CTCP, DCC, mIRC color codes. |
| **Test Kit** | `irc-testkit` | Test infrastructure: Clock, Store, SMTP sink, DNSBL resolver traits with in-memory implementations. |

## Architecture

```
┌──────────────┐      ┌──────────────┐      ┌──────────────┐
│ irc-client-  │      │ irc-bnc      │      │ irc-server   │
│ gui (iced)   │◀────▶│ (bouncer)    │◀────▶│ (daemon)     │
└──────┬───────┘      └──────┬───────┘      └──────┬───────┘
       │                     │                     │
       └─ irc-client-core ───┴──── irc-proto ──────┘
                 │                        │
                 └─ irc-testkit ◀─────────┘
                            │
                            └─ /metrics → Prometheus → Grafana
```

## Requirements

- **Rust** 1.85+ (pinned to 1.90 via `rust-toolchain.toml`)
- **Docker** 24+ and **Docker Compose v2** (optional; for the reference stack)
- **Linux/macOS/Windows** for the GUI client; **Linux/macOS** for server and bouncer

## Quick start

### Native (development)

```bash
# 1. Clone and build
git clone https://github.com/hjawhar/irc.git
cd irc
cargo build --workspace

# 2. Start the server
cargo run -p irc-server -- --config examples/server-config.toml

# 3. In another terminal, launch the GUI client
cargo run -p irc-client-gui
```

The GUI opens with a **Connect dialog** pre-filled with `127.0.0.1:6667`. Click **Connect**, and you'll see the welcome burst in the Status window. Type `/join #test` to open a channel, `/help` for all commands.

### With the bouncer

```bash
# Terminal 3: start the bouncer
cargo run -p irc-bnc -- --config examples/bnc-config.toml
```

Connect your IRC client to `localhost:6668` and authenticate with `PASS alice/localnet:hunter2`.

### Docker Compose (full stack)

```bash
docker compose -f ops/compose/docker-compose.yml up --build
```

| Service | Address |
|---|---|
| IRC server | `localhost:6667` |
| Bouncer | `localhost:6668` |
| Prometheus | `http://localhost:9090` |
| Grafana | `http://localhost:3000` (admin/admin) |
| MailHog | `http://localhost:8025` |

## GUI Client Features

- **Connect dialog** — host, port, nick, username, realname, TLS toggle
- **Multi-server** — connect to multiple servers simultaneously
- **Treebar** — server nodes with `●`/`○` status, channels and queries nested underneath
- **Status window** — per-server, shows numerics, MOTD, errors
- **Light/Dark theme** — toggle via the status bar button or `/theme` command
- **Commands** — `/join`, `/part`, `/nick`, `/msg`, `/topic`, `/list`, `/connect`, `/quit`, `/raw`, `/theme`, `/help`
- **Welcome guidelines** — shown when you join a channel
- **Desktop notifications** — PMs and nick highlights via `notify-rust`
- **Channel list browser** — `/list` opens a filterable grid with click-to-join
- **Rhai scripting** — aliases, event hooks, custom identifiers

## Server Features

- **RFC 1459/2812** compliant with **IRCv3** extensions
- **CAP 302** negotiation: `server-time`, `echo-message`, `account-notify`, `away-notify`, `extended-join`, `message-tags`, `multi-prefix`, `sasl`, `cap-notify`
- **SASL** PLAIN + EXTERNAL authentication
- **Account registration** — `REGISTER`/`VERIFY` commands with argon2id password hashing
- **HMAC-SHA256 cloaks** — IP masking for all users; account-based cloaks for registered users
- **Operators** — config-driven `[[oper]]` blocks with argon2 passwords, SASL account binding, hostmask restriction, class-based privileges
- **KLINE/UNKLINE** — persistent bans with glob matching and expiry
- **KILL** — disconnect users with audit logging
- **MONITOR** — online/offline notifications for watched nicks
- **Channel modes** — `+n`, `+t`, `+m`, `+i`, `+k`, `+l`, `+o`, `+v`
- **Flood control** — token-bucket rate limiter per connection
- **Connection limiter** — per-IP concurrent connection cap
- **Registration deadline** — drops slow-loris connections
- **TLS** — via `tokio-rustls` with `rustls` (ring backend)
- **PROXY protocol v2** — for HAProxy/nginx fronting
- **Prometheus metrics** — connections, messages, auth, flood kicks, klines
- **SQLite storage** — accounts, klines via `sqlx`

## Configuration

Configs are TOML. See the annotated examples:
- [`examples/server-config.toml`](examples/server-config.toml) — server
- [`examples/bnc-config.toml`](examples/bnc-config.toml) — bouncer

## Development

```bash
rustup show                                          # picks up pinned toolchain
cargo install cargo-nextest cargo-deny

cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo deny check
```

Fuzz the protocol parser (requires nightly):
```bash
cd crates/irc-proto/fuzz
cargo +nightly fuzz run decoder -- -max_total_time=60
```

## Project structure

```
irc/
├── crates/
│   ├── irc-proto/          # Protocol: parser, codec, types, fuzz harness
│   ├── irc-server/         # Server daemon
│   ├── irc-client-core/    # Headless client library
│   ├── irc-client-gui/     # iced GUI client
│   ├── irc-bnc/            # Bouncer
│   ├── irc-cli/            # TUI client (stub)
│   └── irc-testkit/        # Test infrastructure
├── ops/
│   ├── docker/             # Per-binary Dockerfiles
│   ├── compose/            # docker-compose dev stack
│   ├── prometheus/         # Scrape config
│   └── grafana/            # Dashboard JSON
├── examples/               # Reference TOML configs
├── docs/                   # Topic documentation
└── .github/workflows/      # CI (fmt, clippy, test, deny, fuzz, audit, release)
```

## License

MIT. See [`LICENSE-MIT`](LICENSE-MIT).
