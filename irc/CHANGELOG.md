# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-20

Initial release of the IRC Suite.

### Added

#### Protocol (`irc-proto`)
- RFC 1459 / RFC 2812 wire-format parser and serializer (zero-copy, `bytes`-backed).
- Typed `Message`, `Command` enum (17 variants), `ReplyCode` (~60 named numerics).
- IRCv3 message-tags with escape handling.
- Channel and user mode parser with ISUPPORT-aware `ModeSpec`.
- CTCP encode/decode with `ACTION` helper.
- DCC protocol types (CHAT, SEND) with IP encoding.
- mIRC formatting codes with `StyledSpan` output.
- Casemapping newtypes: `Nick`, `ChannelName`, `ServerName`, `AccountName`.
- Tokio codec with line-length limits and lenient `\n` decode.
- CAP 302 primitives and ISUPPORT parser with typed accessors.
- `proptest` roundtrip tests (512 cases) + panic-resistance test.
- `cargo-fuzz` targets: decoder, casemap, codec with seed corpus.

#### Server (`irc-server`)
- Tokio multi-threaded async runtime with per-connection tasks.
- TLS via `tokio-rustls` (rustls ring backend).
- PROXY protocol v2 support (AF_INET + AF_INET6 TCP).
- Registration state machine: CAP, NICK, USER, PASS with welcome burst (001-005, LUSERS, MOTD).
- Channel operations: JOIN (auto-op first joiner), PART, TOPIC, NAMES.
- Messaging: PRIVMSG, NOTICE to channels and users.
- Channel modes: `+n`, `+t`, `+m`, `+i`, `+k`, `+l`, `+o`, `+v` with op enforcement.
- User modes: `+i`, `+w`.
- PING/PONG keepalive, QUIT with broadcast to channel peers.
- SQLite storage (`sqlx`) behind a `Store` trait with `InMemoryStore` for tests.
- Account registration (`REGISTER`/`VERIFY`) with argon2id password hashing.
- SASL PLAIN and EXTERNAL authentication.
- HMAC-SHA256 IP cloaking; `user/<account>` cloaks for registered users.
- Operator system: config-driven blocks, argon2 passwords, SASL-account binding, hostmask restriction, class-based privileges.
- Operator commands: `OPER`, `KILL`, `KLINE`/`UNKLINE`, `SHOWHOST`.
- Audit logging via `tracing` on the `audit` target.
- IRCv3 CAP 302 with 9 caps: server-time, echo-message, account-notify, away-notify, extended-join, message-tags, multi-prefix, sasl, cap-notify.
- MONITOR command (online/offline notifications, 100-entry limit).
- Per-IP connection limiter with CAS-based acquire/release.
- Registration deadline (configurable timeout for slow-loris protection).
- Token-bucket per-connection flood control with configurable rate/burst.
- TOML configuration with validation, builder pattern for tests.
- Prometheus metrics: connections, messages, auth, flood kicks, klines.
- Event bus (broadcast channel) for future TS6 server-to-server linking.

#### Client GUI (`irc-client-gui`)
- mIRC-style layout: treebar, scrollback, nick list, topic bar, input bar, status bar.
- Connect dialog with host, port, nick, username, realname, TLS toggle.
- Multi-server support: connect to multiple servers simultaneously.
- Per-server status windows showing numerics, MOTD, errors.
- Light/dark theme toggle via status bar button or `/theme` command.
- Commands: `/connect`, `/server`, `/join`, `/part`, `/nick`, `/msg`, `/topic`, `/list`, `/quit`, `/raw`, `/theme`, `/help`.
- `/help` command showing all available commands and community guidelines.
- Welcome message shown when joining a channel.
- Desktop notifications for PMs and nick highlights (`notify-rust`).
- Channel list browser with filtering and click-to-join.
- Client-side message echo in scrollback.
- PM routing: messages to your nick open query windows.

#### Client Core (`irc-client-core`)
- Headless multi-network client library.
- Connection manager with TCP and TLS support.
- Auto-registration (NICK + USER) on connect, PING/PONG handling.
- `NetworkState` tracking channels, nicks, topics from incoming messages.
- `ClientEvent` (15 variants) / `ClientCommand` (12 variants) channel API.
- Rhai scripting engine: sandboxed (100k ops limit), IRC helpers (send_msg, join, part, echo, nick, channel), alias registration, event hooks.
- DCC manager: accept/offer chat and file transfers.

#### Bouncer (`irc-bnc`)
- Persistent upstream connections with NICK/USER registration and PING/PONG.
- Per-target message buffering (ring buffer, configurable depth).
- Downstream auth via `PASS user/network:password`.
- Synthetic welcome burst + JOIN replay on client attach.
- Buffered message replay with `@time=` server-time tags.
- `*status` admin pseudo-user: `listnetworks`, `status`, `help`.
- CLI binary with `--config` flag and tracing.

#### Test Kit (`irc-testkit`)
- `Clock` trait with `SystemClock` and `ManualClock`.
- `SmtpTransport` trait with `SmtpSink` for email testing.
- `Store` trait with `InMemoryStore`.
- `DnsblResolver` trait with `NoopDnsblResolver` and `StaticDnsblResolver`.

#### Operations
- Multi-stage Dockerfiles with `cargo-chef` and distroless runtime.
- Docker Compose dev stack: server + Prometheus + Grafana + MailHog.
- Grafana server dashboard (4 panels).
- Prometheus scrape configuration.
- GitHub Actions CI: fmt, clippy, nextest, cargo-deny.
- GitHub Actions nightly fuzz and weekly security audit workflows.
- GitHub Actions release workflow: cross-compile binaries, multi-arch Docker images to GHCR, GitHub Release.

[0.1.0]: https://github.com/hjawhar/irc/releases/tag/v0.1.0
