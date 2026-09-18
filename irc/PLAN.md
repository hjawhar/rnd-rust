# IRC Suite — Rust Implementation Plan

Three interoperable components: **server**, **client** (mIRC-style GUI), **bouncer**. Shared protocol crate. Modern IRCv3. Rhai scripting. Single-node server designed so TS6 server-to-server linking can be added later without rewriting internals.

---

## 1. Goals & non-goals

### Goals
- **Protocol**: RFC 1459 / RFC 2812 correct, IRCv3-compliant baseline (CAP 302, SASL PLAIN/EXTERNAL, message-tags, server-time, echo-message, away-notify, account-notify, batch, chathistory).
- **Client UX**: mIRC-style GUI — treebar, MDI window stack, nick list, status window per network, input with tab completion, themes, color/format codes, context menus, Rhai scripting (aliases, event hooks, identifiers, dialogs).
- **Accounts**: `REGISTER`/`VERIFY` over IRC. Email verification via configurable SMTP. Nick protection after verification. Admin-provisioned accounts as fallback.
- **Privacy (cloaks)**: every user's host is masked by default. HMAC-keyed cloak for unregistered, `account/<name>` cloak for registered. Opers with an explicit privilege see real host; all such lookups are audit-logged.
- **Anti-abuse**: per-connection token buckets, per-IP connection limits + rate limits, registration deadline, DNSBL, PROXY protocol v2 for fronting, k-lines/g-lines, lockdown mode.
- **Storage**: SQLite (`sqlx`, WAL mode) abstracted behind a `Store` trait; Postgres adapter is a future drop-in without changing callers.
- **Bouncer**: 24/7 upstream, replays with IRCv3 `server-time`, multi-user, multi-network.
- **Operators**: config-file oper blocks, argon2-hashed secrets, SASL-account-bound, hostmask-restricted, class-based privileges.
- **Observability**: Prometheus `/metrics` endpoint, structured logs with `tracing`, shipped Grafana dashboards.
- **Testability**: layered test suite (unit, property, fuzz, conformance, integration, E2E) driven by a dedicated `irc-testkit` crate. CI gates every change on the full matrix.
- **Engineering**: every public boundary typed; every parser input validated; no `.unwrap()` in production paths; `#![forbid(unsafe_code)]`.

### Non-goals (for v1)
- TS6 server-to-server linking. Internals designed for it; code ships in Phase 11.
- Services daemon (NickServ/ChanServ as separate linked pseudo-server). Account auth is first-class inside `irc-server`.
- Web UI for the bouncer.
- Pixel-perfect mIRC rendering — we match the ergonomics, not the Win32 chrome.

---

## 2. Workspace layout

```
irc/
├── Cargo.toml                     # workspace
├── rust-toolchain.toml            # pinned stable
├── rustfmt.toml
├── clippy.toml
├── deny.toml                      # cargo-deny policy
├── .github/workflows/
│   ├── ci.yml                     # fmt, clippy, unit, property, conformance
│   ├── integration.yml            # integration + E2E
│   ├── fuzz.yml                   # nightly fuzz run
│   └── release.yml
├── crates/
│   ├── irc-proto/                 # wire format, types, codec — FOUNDATION
│   ├── irc-server/                # daemon binary
│   ├── irc-client-core/           # headless client library
│   ├── irc-client-gui/            # iced frontend binary (primary client)
│   ├── irc-bnc/                   # bouncer binary
│   ├── irc-cli/                   # optional: ratatui TUI on client-core
│   └── irc-testkit/               # test harness: spin servers, scripted clients, assertion DSL
├── ops/
│   ├── grafana/
│   │   ├── server-dashboard.json
│   │   ├── bnc-dashboard.json
│   │   └── protocol-dashboard.json
│   ├── prometheus/
│   │   └── scrape.yml
│   ├── compose/
│   │   └── docker-compose.yml     # dev stack: server + bnc + prom + grafana + mailhog
│   └── docker/
│       ├── irc-server.Dockerfile
│       ├── irc-bnc.Dockerfile
│       └── irc-cli.Dockerfile
├── docs/
│   ├── architecture.md
│   ├── protocol-notes.md
│   ├── scripting.md
│   ├── accounts-and-cloaks.md
│   ├── ops-and-admins.md
│   ├── security-and-abuse.md
│   ├── metrics.md
│   └── testing.md
└── README.md
```

Cargo workspace, single `Cargo.lock`. Edition 2021 (switch to 2024 when MSRV allows).

---

## 3. Shared crate: `irc-proto`

Every other crate depends on this. Built first, locked down hardest.

### Responsibilities
- **Parser**: line-oriented, zero-copy where possible (`bytes::Bytes`), RFC 1459 grammar + IRCv3 message-tags + escape handling (`\:`, `\s`, `\\`, `\r`, `\n`).
- **Serializer**: inverse of parser. Roundtrip property-tested.
- **Typed messages**: `Message { tags, prefix, command, params }`. `Command` is an enum with structured variants and a `Raw(String)` fallback.
- **Numerics**: `ReplyCode` enum covering RPL_* and ERR_*.
- **Mode parsing**: user and channel modes with parameterized modes (`+o nick`, `+b mask`, `+k key`, `+l limit`). Mode-arg semantics table per server (ISUPPORT `CHANMODES`, `PREFIX`).
- **CTCP**: encode/decode `\x01CMD args\x01`.
- **Formatting codes**: mIRC color, bold, italic, underline, strikethrough, monospace, reverse, reset. Typed `StyledSpan` output.
- **Casemapping**: `Nick`, `ChannelName`, `ServerName`, `AccountName` newtypes. `ascii`, `rfc1459`, `rfc1459-strict`.
- **Codec**: `tokio_util::codec::Decoder`/`Encoder`. 512-byte limit untagged, 8191 bytes tagged (IRCv3). `\r\n`, lenient `\n` on decode.
- **CAP 302** primitives (LS/LIST/REQ/ACK/NAK, values).
- **ISUPPORT** parser.

### Testing
- Unit tests per grammar rule.
- `proptest`: generate `Message`, serialize, parse, assert equality (modulo documented normalization).
- `cargo fuzz` targets on decoder + casemap. Seeded with real captures.
- Golden tests for mIRC color decode/encode.

### Key dependencies
`bytes`, `tokio-util`, `serde`, `thiserror`, `smallvec`, `arrayvec`. Hand-written parser (no `nom`) for perf and zero-copy.

---

## 4. `irc-server` — daemon

### Runtime
- `tokio` multi-thread.
- TLS via `tokio-rustls` (rustls; no OpenSSL).
- One task per connection; shared state via `Arc<ServerState>` with fine-grained locks (`parking_lot::RwLock` per channel, `DashMap` for user/channel indexes).
- PROXY protocol v2 support on listeners (optional, per-listener) so the server can sit behind HAProxy/nginx and still see real client IPs.

### State model
```
ServerState
├── users:      DashMap<UserId, Arc<User>>
├── nicks:      DashMap<NickKey, UserId>       // casemap-aware
├── channels:   DashMap<ChanKey, Arc<RwLock<Channel>>>
├── accounts:   Arc<dyn AccountStore>          // SQLite impl; trait allows swap
├── cloaks:     CloakEngine                    // HMAC keyed by server secret
├── opers:      OperatorSet                    // config-loaded, argon2-verified
├── klines:     BanList                        // temporary + permanent
├── dnsbl:      DnsblClient                    // optional
├── limiter:    ConnectionLimiter              // per-IP, global
├── config:     ArcSwap<Config>                // hot-reload via /REHASH
├── metrics:    MetricsHandles                 // Prometheus registrations
└── event_bus:  broadcast::Sender<ServerEvent> // TS6 hook
```

### Event bus (TS6 escape hatch)
Every state mutation emits a typed `ServerEvent`. In v1 the only subscriber is the outbound dispatcher. When TS6 lands, a second subscriber translates events to/from the S2S protocol. No core logic changes at that point.

### Commands (v1)
- Registration: `CAP`, `NICK`, `USER`, `PASS`, `AUTHENTICATE` (SASL), `QUIT`, `PING`/`PONG`.
- Accounts: `REGISTER`, `VERIFY`, `UNREGISTER`, `PASSWD`, `RESETPASS`, `CHGEMAIL`, `ACCOUNT` (IRCv3 draft).
- Channel: `JOIN`, `PART`, `TOPIC`, `KICK`, `INVITE`, `NAMES`, `LIST`, `WHO`, `WHOIS`, `WHOWAS`, `MODE`.
- Messaging: `PRIVMSG`, `NOTICE`, `TAGMSG`.
- Ops: `OPER`, `KILL`, `KLINE`, `UNKLINE`, `GLINE`, `UNGLINE`, `LOCKDOWN`, `SAMODE`, `REHASH`, `DIE`, `RESTART`, `SHOWHOST` (see real host for a user — privilege-gated, audit-logged).
- Info: `MOTD`, `VERSION`, `ADMIN`, `TIME`, `LUSERS`, `STATS`.
- IRCv3: `BATCH`, `CHATHISTORY`, `MONITOR` (preferred) or `WATCH`.

### Modes
- **User**: `+i` invisible, `+w` wallops, `+o` oper, `+s` server notices, `+Z` secure (informational), `+x` cloak-enabled (default on, settable off only if config allows), `+R` registered-only messages.
- **Channel**: `+o`, `+v`, `+b`, `+e`, `+I`, `+i`, `+k`, `+l`, `+m`, `+n`, `+s`, `+t`, `+p`, `+r` registered-only, `+M` mod-registered-only. ISUPPORT advertises.

### Accounts (registration + verification)
**Register**:
- `REGISTER <account> <email> <password>` from an unauthenticated session.
- Server:
  - validates account name (casemap, length, not reserved, not already owned),
  - enforces password policy (length, entropy floor, argon2id hashing),
  - inserts `accounts (state='pending', verify_token, token_expires)`,
  - sends email via SMTP (`lettre`) with a verification code,
  - replies with numeric `RPL_REG_VERIFICATION_REQUIRED` and instructions.
- If SMTP is disabled in config, server emits a NOTICE to opers online with the token (for ops-assisted verification) or refuses registration — configurable.

**Verify**:
- `VERIFY <account> <token>` — moves state to `active`. From this point:
  - User can `AUTHENTICATE PLAIN` with `account:password`.
  - Account is eligible for nick protection and cloak.

**Nick protection** (configurable per account):
- `enforce_mode = off | warn | rename | ghost`
  - `off`: no protection.
  - `warn`: notice only, no forced action.
  - `rename`: after grace period, unauthenticated user holding the nick is renamed to `Guest<N>`.
  - `ghost`: authenticated user can `GHOST <nick>` to disconnect the imposter.

**Admin provisioning**: `irc-server admin accounts create|delete|verify|resetpass` CLI acting directly on the SQLite store (offline or via rehash-safe lock).

### Cloaks (IP masking, always on by default)
- All public identifiers (WHOIS, WHO, NAMES, JOIN/PART, NICK prefixes) show the cloaked host, never the real one.
- **Unregistered users**: `HMAC-SHA256(server_secret, ip || account_or_empty)` truncated to readable base32, formatted as three dot-separated segments: `a1b2c3.d4e5f6.ip`.
- **Registered users**: `user/<account>` (or configurable `<network>/<account>`). Stable across sessions; unique per account.
- **Vanity cloaks**: opers can grant custom cloaks (`SETCLOAK <account> <pattern>`), validated against a blocklist.
- **Oper-only real-host**: `SHOWHOST <nick>` returns the real host only to opers with the `see-realhost` privilege. Every call is emitted on a server-notice snomask and written to the audit log with oper identity, target, reason field (required).
- **Cloaks are not security boundaries** against determined adversaries (opers are trusted); this is documented in `docs/accounts-and-cloaks.md`.

### SASL
- `PLAIN`: against `accounts` table, argon2id.
- `EXTERNAL`: TLS client-cert fingerprint bound to account. Fingerprints stored per account.

### Chathistory
- Per-channel ring buffer (default 1000 msgs) + optional SQLite persistence.
- `CHATHISTORY LATEST / BEFORE / AFTER / AROUND / BETWEEN`.

### Operators (admins)
Config (TOML):
```toml
[[oper]]
name = "alice"
# argon2id hash of the oper password
password_hash = "$argon2id$..."
# SASL account required; OPER refused unless session is authenticated as this account
require_account = "alice"
# Connection must match one of these masks (nick!user@real-host; real-host, not cloak)
allowed_hosts = ["*!*@192.0.2.0/24", "*!alice@trusted.example.com"]
# Privilege class
class = "netadmin"

[oper_class.netadmin]
privileges = [
  "kline", "gline", "kill", "samode", "rehash", "die",
  "see-realhost", "lockdown", "setcloak", "register-bypass",
]
```
- `OPER <name> <password>` is the authentication command. Password never travels as cleartext on-wire past TLS; denied unless session is TLS + SASL-authenticated against `require_account`.
- Every oper action is a `tracing` span with `oper.account`, `oper.class`, `target`, `reason` and appears on the audit log snomask.
- Oper password + cert-fingerprint-bound opers are both supported.

### Configuration (TOML)
Listener bindings (plain/TLS/PROXY), server secret (for cloaks), MOTD, oper blocks + classes, SMTP settings, SASL mechanisms, ISUPPORT overrides, chathistory sizes, flood buckets, connection limiter, DNSBL zones, k-line defaults, metrics endpoint, storage path/DSN. `REHASH` reloads without dropping clients.

### Testing (for this crate specifically)
Integration tests spin up a server on an ephemeral port (using `irc-testkit`), connect scripted clients, assert numerics, state transitions, IRCv3 behavior, registration + verification flow, cloak stability, oper gating, flood cutoff, CHATHISTORY replay. See §12.

---

## 5. Storage

Single trait, multiple backends; keeps us honest about portability without committing to the complexity of a remote DB on day one.

### `Store` trait (defined in a `storage` module of `irc-server` and `irc-bnc`, or in a tiny shared crate if reused)
```rust
#[async_trait]
pub trait Store: Send + Sync + 'static {
    async fn account_create(&self, req: CreateAccount) -> Result<Account>;
    async fn account_verify(&self, name: &AccountName, token: &str) -> Result<Account>;
    async fn account_auth(&self, name: &AccountName, password: &str) -> Result<Account>;
    async fn account_fingerprint_add(&self, name: &AccountName, fp: CertFingerprint) -> Result<()>;
    async fn account_fingerprint_get(&self, fp: &CertFingerprint) -> Result<Option<Account>>;
    async fn kline_add(&self, kl: Kline) -> Result<()>;
    async fn kline_active(&self, ts: OffsetDateTime) -> Result<Vec<Kline>>;
    async fn chathistory_append(&self, target: &ChanKey, msg: HistoryMsg) -> Result<()>;
    async fn chathistory_fetch(&self, q: HistoryQuery) -> Result<Vec<HistoryMsg>>;
    // bouncer:
    async fn bnc_user_create(&self, user: BncUser) -> Result<()>;
    async fn bnc_buffer_append(&self, user: UserId, net: NetId, target: &str, msg: BncMsg) -> Result<()>;
    async fn bnc_buffer_fetch(&self, q: BncBufferQuery) -> Result<Vec<BncMsg>>;
    // ...
}
```

### Recommendation: SQLite for v1
- **Why SQLite**: single-node architecture, embedded (zero ops), WAL mode gives great concurrent read + single writer throughput, file-level backups trivial, schema migrations with `sqlx migrate`. Matches the workload: predominantly small writes + point lookups + small range scans on `(target, ts)`.
- **Driver**: `sqlx` (async, compile-time-checked queries, connection pool). `rusqlite` was considered but `sqlx` removes the need to `spawn_blocking` throughout.
- **Mode**: WAL, `synchronous=NORMAL`, `journal_size_limit`, `busy_timeout=5000`. Documented in `docs/ops-and-admins.md`.

### Postgres path (future)
- A `PostgresStore` implementation of the same trait exists as a stub from day one (compiles, `unimplemented!` bodies) so the trait is genuinely portable, not SQLite-shaped.
- Tests in `irc-testkit` parameterize over store impls; when the Postgres impl fills in, the same tests run against a test Postgres container.
- When to switch:
  - Multi-process deployment sharing state.
  - Need for external tooling to query live data.
  - Dataset size where SQLite VACUUM windows become awkward.

---

## 6. `irc-client-core` — headless client library

Consumed by both the GUI and the BNC (which uses it for upstream connections). Zero UI concerns.

### Shape
```
Client
├── networks:   HashMap<NetworkId, NetworkSession>
├── event_tx:   mpsc::Sender<ClientEvent>       // to frontend
└── command_rx: mpsc::Receiver<ClientCommand>   // from frontend

NetworkSession
├── connection:       TLS/plain socket + codec
├── state:            nick, caps, isupport, joined chans, own modes, account
├── windows:          HashMap<WindowId, WindowState>  // status + channel + query
├── scrollback:       per-window ring buffer of rendered lines
├── reconnect_policy: exponential backoff with cap + jitter
└── scripting:        Rhai engine handle
```

### Events emitted to frontend
`Connected`, `Disconnected`, `Registered`, `Joined`, `Parted`, `Kicked`, `NickChanged`, `TopicChanged`, `ModeChanged`, `Message { window, from, text, tags, styled_spans }`, `NoticeMessage`, `NameList`, `Motd`, `NumericRaw`, `CapNegotiated`, `SaslResult`, `AccountNotice`, `Error`.

### Commands from frontend
`Connect`, `Disconnect`, `SendRaw`, `SendPrivmsg`, `SendNotice`, `Join`, `Part`, `ChangeNick`, `SetTopic`, `SetMode`, `OpenQuery`, `CloseWindow`, `Register { account, email, password }`, `Verify { account, token }`, `Identify { account, password }`, `ReloadScripts`, `EvalScript`.

### IRCv3 on the client
- Offer + require when advertised: `cap-notify`, `account-notify`, `away-notify`, `extended-join`, `server-time`, `message-tags`, `echo-message`, `batch`, `chathistory`, `sasl`, `multi-prefix`, `userhost-in-names`, `invite-notify`, `setname`.
- On JOIN, auto-fetch recent chathistory when advertised.

### Logging
- Per-window log files, rotated daily or at size threshold.
- Default format: mIRC-like `[HH:MM:SS] <nick> text`.

### Rhai scripting
- Loaded from `~/.config/irc-suite/scripts/*.rhai`.
- **Identifiers**: `nick()`, `chan()`, `network()`, `now()`, `me()`, `topic()`, `version()`, `account()`.
- **Actions**: `send_msg(target, text)`, `send_raw(line)`, `join(chan)`, `part(chan)`, `set_mode(target, modes)`, `open_window(target)`, `echo(window, text)`.
- **Dialogs**: `dialog::prompt`, `dialog::confirm`, `dialog::form` — async, frontend resolves.
- **Event hooks**: `on("text" | "join" | "notice" | "raw" | ..., |ctx| { ... })` with filters.
- **Aliases**: `alias("j", |args| { join(args[0]) })`.
- **Timers**: `timer::after(ms, fn)`, `timer::every(ms, fn)`.
- Engine runs in a dedicated thread with a bounded mpsc queue; instruction cap per event prevents runaway loops.

---

## 7. `irc-client-gui` — iced frontend (primary client)

Layout (mIRC-inspired):

```
┌────────────────────────────────────────────────────────────────┐
│  Menu: File  Edit  View  Commands  Tools  Window  Help         │
├──────────┬────────────────────────────────────────┬────────────┤
│          │ Topic bar: #rust — "Rust Programming…" │            │
│          ├────────────────────────────────────────┤  Nick list │
│ Treebar  │                                        │  @op1      │
│          │   Scrollback (styled, selectable,      │  +voice1   │
│ Libera   │   links clickable, timestamps)         │   alice    │
│  #rust   │                                        │   bob      │
│  #linux  │                                        │   carol    │
│  (pm)bob │                                        │            │
│ OFTC     │                                        │            │
│  Status  │                                        │            │
│          ├────────────────────────────────────────┤            │
│          │ > input line ─ tab-complete, history   │            │
├──────────┴────────────────────────────────────────┴────────────┤
│ Status: lag 23ms · nick alice · +Zix · connected               │
└────────────────────────────────────────────────────────────────┘
```

### Features
- **Treebar**: networks expandable. Indicators: unread counts, highlight pulses, PM italics.
- **Windows**: tabs + panes. Close/reorder via context menu. Ctrl+number switch.
- **Nick list**: sorted by prefix then casemap name. Right-click: query, whois, op, deop, voice, kick, ban, kickban, ignore.
- **Input**: history, tab completion (nicks in channel → channel names → commands), multi-line shift+enter.
- **Formatting**: renders mIRC codes; input supports Ctrl+K/B/U/I/R.
- **Themes**: TOML palettes; ship classic mIRC + modern dark.
- **Channel browser**: `/list` opens a sortable grid.
- **Registration wizard**: built-in UI for `REGISTER` → email prompt → `VERIFY` on any server advertising account support.
- **SASL manager**: per-network credentials (keychain-backed where OS supports; else encrypted at rest).
- **DCC** (Phase 8): chat + send/get with a transfers window.
- **Notifications**: `notify-rust` for highlights, PMs, watched-nick joins.
- **Dialogs**: Rhai scripts drive modal/inline dialogs; results returned async.

### Keybinds
Ctrl+Tab / Ctrl+Shift+Tab (next/prev window), Ctrl+W (close), Ctrl+F (find in scrollback), Alt+number (network), F1 (help), F2 (options).

---

## 8. `irc-bnc` — bouncer

Composition: `client-core` (for each upstream) + a server-lite listener (downstream), glued by a state/buffer layer.

### Architecture
```
BncServer
├── config:   TOML users + networks
├── store:    Arc<dyn Store>                      // SQLite
├── users:    HashMap<UserId, BncUser>
│    └── networks: HashMap<NetId, BncNet>
│         ├── upstream:    client_core::NetworkSession (persistent)
│         ├── downstreams: Vec<Downstream>
│         └── buffers:     per-channel + per-query ring buffers with server-time tags
├── listener: TCP/TLS accept loop
├── metrics:  MetricsHandles
└── admin:    *status pseudo-user
```

### Downstream protocol
- Auth: SASL PLAIN (preferred) or `PASS user/net:password`.
- Announces caps supported upstream + `soju.im/bouncer-networks` + `draft/chathistory`.
- On attach: synthetic welcome, JOIN replay for currently-joined channels, TOPIC, NAMES, then buffered messages with original `@time=` tag.
- Passes through caps negotiated upstream.

### Buffering
- Ring buffer per target (default 5000) with optional SQLite persistence.
- Replay window: last N, since-last-attach, or time window — configurable per-network.

### Admin `*status`
`addnetwork`, `delnetwork`, `connect`, `disconnect`, `listnets`, `status`, `setnick`, `setsasl`, `adduser`, `passwd`, `metrics` (points at local /metrics). Mirrors ZNC / soju conventions.

### User management
- `irc-bnc admin users create|delete|listnets|passwd` CLI.
- Bouncer users are separate from IRC-server accounts (different trust domain — a bouncer user may have accounts on multiple networks).

---

## 9. Observability — metrics, logs, tracing

### Prometheus
- `metrics` + `metrics-exporter-prometheus` crates.
- Dedicated listener (e.g. `127.0.0.1:9772`) — **never the IRC port**. Optional basic auth / IP allow-list.
- Key metrics exposed:

**irc-server**:
- `irc_server_connections_total{listener, tls}` counter, `_open` gauge.
- `irc_server_registration_duration_seconds` histogram.
- `irc_server_messages_total{command}` counter.
- `irc_server_bytes_in_total`, `irc_server_bytes_out_total` counters.
- `irc_server_channels_open` gauge, `irc_server_users_online` gauge.
- `irc_server_auth_total{mechanism, outcome}` counter.
- `irc_server_flood_kicks_total{reason}` counter.
- `irc_server_klines_active` gauge, `irc_server_klines_added_total{kind}` counter.
- `irc_server_dnsbl_hits_total{zone}` counter.
- `irc_server_oper_actions_total{class, action}` counter.
- `irc_server_chathistory_queries_total`, `irc_server_chathistory_rows_returned_total`.
- `irc_server_rehash_total{outcome}` counter.
- `irc_server_store_query_duration_seconds{op}` histogram.

**irc-bnc**:
- `irc_bnc_upstreams{state}` gauge, `irc_bnc_downstreams` gauge.
- `irc_bnc_buffered_messages_total{user, net}` gauge.
- `irc_bnc_replay_bytes_total` counter.
- `irc_bnc_reconnects_total{reason}` counter.

### Logs
- `tracing` + `tracing-subscriber` (env-filter, JSON in prod, pretty in dev).
- Spans on every connection and every command dispatch, with structured fields (`user.id`, `nick`, `chan`, `cmd`, `oper.account` for oper commands).
- Audit log: dedicated target `audit` with oper-action events (see-realhost lookups, klines, samode, account creation, etc.).

### Grafana
- Dashboards shipped as JSON in `ops/grafana/`:
  - **Server overview**: connections, msgs/s, bytes/s, flood kicks, auth success/fail rate, DNSBL hits.
  - **Bouncer**: per-user attached clients, reconnects, buffer depth.
  - **Protocol**: command mix, error numerics, CAP negotiation outcomes, chathistory queries.
- `docker-compose.yml` in `ops/compose/` brings up Prometheus + Grafana + irc-server + irc-bnc for local demo.

---

## 10. Security & anti-abuse

Defense in depth. No single control is sufficient; layers compensate for each other's failures.

### Layer 1 — Network edge
- **TLS everywhere**: separate plain/TLS listeners; plain can be disabled in config. TLS 1.2+, modern cipher suite, HSTS-equivalent only via STS cap.
- **PROXY protocol v2** support on listeners (optional per-listener) — HAProxy/nginx terminate TLS and forward real IP. Limiter + DNSBL + cloaks all see real IP.
- **kernel-level**: documentation in `docs/security-and-abuse.md` recommends `ufw`/`nftables` rate rules, SYN cookies on, `net.ipv4.tcp_syncookies=1`. Not our code, but we tell operators what to do.

### Layer 2 — Connection admission
- **Per-IP connection limit**: max N concurrent connections per IP (default 5). Configurable. `/24` IPv4 and `/64` IPv6 aggregation to handle NAT / block subnets.
- **Per-IP connection rate**: token bucket (e.g. 3/minute burst 10) at accept time; excess `RST`.
- **Global concurrent-connection cap**: hard ceiling; above it, new connections get `ERROR :Server is full`.
- **DNSBL** (optional): on accept, resolve IP against configured zones (e.g. `dnsbl.dronebl.org`); on hit, immediately close with reason. Cached, async, timeout-bounded.
- **Geographic / CIDR blocklist** (optional): static allow/deny lists.
- **TLS-SNI gating** (optional): require expected SNI on TLS listener.

### Layer 3 — Registration deadline
- **Slow-loris kill**: a connection that doesn't complete registration (NICK+USER) within `registration_deadline_seconds` (default 10) is dropped. Prevents resource exhaustion.
- **Pre-registration flood**: tight token bucket (fewer commands allowed before registration).

### Layer 4 — Post-registration limits
- **Token buckets**: per-connection buckets for messages/sec, bytes/sec, targets/sec (to stop PRIVMSG blast to many channels). Oper override.
- **Channel join rate**: per-connection limit on JOINs/min and total joined channels.
- **Who/WhoIs/List rate**: separate bucket; expensive queries are rate-limited.
- **Penalty mode**: repeat offenders enter a higher-cost bucket, then auto-disconnect.

### Layer 5 — Bans
- **K-line** (local ban): `KLINE <mask> <duration> :reason`. Immediate disconnect + refused re-connect. Persisted.
- **G-line** (global ban, scaffolded): stored; propagated via TS6 in Phase 11.
- **Ban evasion tracking**: on kline, log the cloak and real IP both; alert on reconnection attempts from same /24 or /64.

### Layer 6 — Application-level abuse
- **Lockdown mode** (oper command): refuse new connections except from trusted sources; useful during attacks.
- **Nick flood**: per-account nick changes limited.
- **PRIVMSG to unknown targets**: rate-limited to stop dictionary attacks.
- **Channel invite spam**: per-channel invite budget.
- **CTCP flood**: separate bucket; mass CTCP triggers auto-disconnect.

### Layer 7 — Abuse telemetry
- Every limiter trip is a Prometheus counter (`irc_server_flood_kicks_total`, `irc_server_dnsbl_hits_total`, `irc_server_klines_added_total`) and a structured log event. Grafana dashboards surface attack shapes quickly.

### Things we explicitly choose *not* to do
- Proof-of-work / CAPTCHA on connect — wrecks UX, attackers automate anyway.
- IP reputation scoring beyond DNSBL — out of scope for v1.

---

## 11. Deployment & packaging

Every service is container-first. Same workflow in dev, CI, and prod: multi-stage build, non-root runtime, read-only rootfs, volumes for state and config.

### Container images
- `irc/server` — `irc-server` binary.
- `irc/bnc` — `irc-bnc` binary.
- `irc/cli` — `irc-cli` (headless TUI on `irc-client-core`) for smoke tests and ops scripting.
- GUI client is **not** containerized — it's a desktop app, packaged via `cargo install`, GitHub Releases prebuilts, and (post-1.0) Homebrew / Flatpak / MSI.

### Build strategy — multi-stage with cargo-chef
All server-side images share the same Dockerfile template in `ops/docker/`. `cargo-chef` caches dependency compilation separately from the workspace so incremental rebuilds touch only changed crates.

```dockerfile
# syntax=docker/dockerfile:1.7
ARG RUST_VERSION=1.80

FROM rust:${RUST_VERSION}-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG BIN=irc-server
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin ${BIN}

FROM gcr.io/distroless/cc-debian12:nonroot
ARG BIN=irc-server
COPY --from=builder /src/target/release/${BIN} /usr/local/bin/app
USER nonroot
EXPOSE 6667 6697 9772
VOLUME ["/var/lib/app", "/etc/app"]
ENTRYPOINT ["/usr/local/bin/app"]
CMD ["--config", "/etc/app/config.toml"]
```

### Runtime hardening
- Non-root UID 65532 (distroless `nonroot`).
- Read-only root filesystem; writable volumes at `/var/lib/*`.
- `cap_drop: [ALL]`, no added capabilities.
- `security_opt: [no-new-privileges:true]`.
- Default seccomp profile; no custom syscalls needed.
- Resource limits (`mem_limit`, `cpus`) set per service in the compose file.

### Volumes
- `irc-server`: `/var/lib/irc-server/` (SQLite, cloak secret, chathistory), `/etc/irc-server/` (config + TLS certs).
- `irc-bnc`: `/var/lib/irc-bnc/` (SQLite, buffers), `/etc/irc-bnc/`.

### Healthcheck + readiness
- Each binary exposes `GET /health` on the metrics listener. Returns 200 when the accept loop is running and the `Store` responds to a ping query.
- Docker `HEALTHCHECK`: `["/usr/local/bin/app", "--healthcheck"]` — the binary is its own healthcheck; `--healthcheck` GETs `127.0.0.1:<metrics-port>/health` and exits 0/1.
- Kubernetes: same `/health` for liveness and readiness.

### Compose dev stack — `ops/compose/docker-compose.yml`
One command brings up the whole topology for development and end-to-end tests:

```yaml
services:
  irc-server:
    build:
      context: ../..
      dockerfile: ops/docker/irc-server.Dockerfile
    ports: ["6667:6667", "6697:6697", "9772:9772"]
    volumes:
      - server-data:/var/lib/irc-server
      - ./server-config:/etc/irc-server:ro
    read_only: true
    tmpfs: ["/tmp"]
    security_opt: ["no-new-privileges:true"]
    cap_drop: ["ALL"]

  irc-bnc:
    build:
      context: ../..
      dockerfile: ops/docker/irc-bnc.Dockerfile
    ports: ["6668:6668", "6699:6699", "9773:9773"]
    volumes:
      - bnc-data:/var/lib/irc-bnc
      - ./bnc-config:/etc/irc-bnc:ro
    depends_on: [irc-server]

  prometheus:
    image: prom/prometheus:latest
    volumes:
      - ../prometheus/scrape.yml:/etc/prometheus/prometheus.yml:ro
    ports: ["9090:9090"]

  grafana:
    image: grafana/grafana:latest
    volumes:
      - ../grafana:/etc/grafana/provisioning/dashboards:ro
    ports: ["3000:3000"]
    environment:
      GF_SECURITY_ADMIN_PASSWORD: admin

  mailhog:
    image: mailhog/mailhog:latest
    ports: ["1025:1025", "8025:8025"]

volumes:
  server-data:
  bnc-data:
```

`docker compose up --build` → IRC on `6667`/`6697`, bouncer on `6668`/`6699`, Prometheus at `http://localhost:9090`, Grafana at `http://localhost:3000`, MailHog inbox at `http://localhost:8025`. Local configs live in `ops/compose/server-config/` and `ops/compose/bnc-config/` (gitignored, seeded from `examples/`).

### Image publishing (CI)
`.github/workflows/release.yml` builds and publishes on tagged releases:
- Multi-arch via `docker buildx`: `linux/amd64` + `linux/arm64`.
- Signed with `cosign` (keyless OIDC).
- SBOM via `syft` attached as an OCI artifact.
- Tags: `ghcr.io/<owner>/irc-server:<semver>`, `:latest`, `:sha-<short>`; same for `irc-bnc` and `irc-cli`.

### Kubernetes (post-1.0)
Helm chart scaffold in `ops/helm/` after release 0.1. Server + BNC as StatefulSets (one replica each; multi-replica waits for TS6). `ServiceMonitor` CRs for kube-prometheus-stack.

---

## 12. Testing strategy

Quality is enforced by a layered suite. Every change runs through the same gates.

### Layers

| Layer | Scope | Tooling | Where |
|---|---|---|---|
| **Unit** | Per-module logic | `cargo test`, `insta` | each crate's `#[cfg(test)]` |
| **Property** | Parser/codec/casemap invariants | `proptest` | `irc-proto/tests/prop/` |
| **Fuzz** | Parser robustness, casemap, mode parsing | `cargo-fuzz` (libfuzzer) | `irc-proto/fuzz/` |
| **Conformance** | RFC 1459/2812 + IRCv3 specs | `irc-testkit` DSL | `crates/irc-testkit/tests/conformance/` |
| **Integration** | End-to-end per crate (real sockets) | `irc-testkit` | `irc-server/tests/`, `irc-bnc/tests/` |
| **E2E** | Full stack: server + bnc + client-core, multi-user flows | `irc-testkit` | `irc-testkit/tests/e2e/` |
| **Load** | Many concurrent clients | custom harness built on `irc-testkit` | `irc-testkit/benches/` (manual) |
| **UI (manual)** | GUI smoke | screenshots / checklist | docs/testing.md |

### `irc-testkit`
A dedicated crate that owns every mechanism the tests need:

- `TestServer`: builder that spins up a real `irc-server` on an ephemeral port with a provided `Config`, returns handles + shutdown guard. In-memory `Store` + in-memory SMTP sink (we assert emails were sent with the expected token).
- `TestBnc`: same for `irc-bnc`.
- `ScriptedClient`: speaks the wire protocol using `irc-proto`. Fluent DSL:
  ```rust
  let mut c = srv.connect_client().await?;
  c.send("NICK alice").await?;
  c.send("USER alice 0 * :Alice").await?;
  c.expect_numeric(ReplyCode::RPL_WELCOME).await?;
  c.send("REGISTER alice alice@example.com hunter2hunter2").await?;
  c.expect_numeric(ReplyCode::RPL_REG_VERIFICATION_REQUIRED).await?;
  let token = srv.smtp_sink.last_token_for("alice@example.com").await?;
  c.send(&format!("VERIFY alice {token}")).await?;
  c.expect_numeric(ReplyCode::RPL_REG_SUCCESS).await?;
  ```
- `SmtpSink`: trait implementation of the server's outbound email transport; captures messages in tests.
- `ClockOverride`: deterministic time source so we can test ban expiry, token expiry, flood buckets, chathistory ranges.
- `CaptureStore`: `Store` implementation that records every call (for invariant checks) while delegating to a real backend.
- Scenario modules, each a self-contained story:
  - `registration_happy_path`, `registration_expired_token`, `registration_collision`
  - `cloak_is_stable_across_reconnect`, `cloak_changes_on_auth`
  - `oper_requires_sasl_and_hostmask`, `oper_action_is_audited`
  - `flood_triggers_kick_and_metric_increments`
  - `kline_blocks_reconnect`, `kline_expires`
  - `dnsbl_hit_rejects_connection` (mock DNSBL resolver)
  - `sasl_plain_success`, `sasl_plain_failure`, `sasl_external_via_fingerprint`
  - `chathistory_roundtrip`
  - `bnc_replays_missed_messages_with_server_time`
  - `bnc_multi_downstream_sees_live_traffic`

### Running
- `cargo nextest run --workspace` is the default test command.
- `cargo test -p irc-proto --features fuzz-smoke` runs a short fuzz sweep as part of CI.
- A separate CI job runs a long nightly fuzz session and uploads the corpus.
- Integration + E2E run in a separate CI workflow with a bigger timeout; PRs gate on all layers except load.

### Coverage & regression
- `cargo llvm-cov` in CI; target 80% on libraries, advisory on binaries.
- `insta` snapshot tests on wire bytes for every numeric we emit — any accidental format change fails loudly.
- Every fixed bug lands a regression test referencing the issue ID.

### What testability costs us in the design
- `Store`, `Clock`, `SmtpTransport`, `DnsblResolver` are all trait objects (behind `Arc<dyn ...>`) so tests inject sinks/mocks. This is the one place we accept the trait-object cost.
- The server exposes an internal `control` API (feature-gated or behind a dev-only socket) for tests to query state without screen-scraping numerics.

---

## 13. Phased delivery

Each phase ends with green tests, working binary, and a demo path.

| Phase | Scope | Exit criteria |
|------|------|---------------|
| **0** | Workspace, CI, tooling, rust-toolchain, clippy+fmt+deny, skeleton READMEs, `irc-testkit` skeleton with `Store` + `Clock` + `SmtpSink` traits | `cargo check --workspace` passes; CI green |
| **1** | `irc-proto` complete: parser, serializer, codec, numerics, modes, CTCP, colors, casemap, ISUPPORT | Unit + proptest + fuzz targets pass; roundtrip property holds |
| **2** | `irc-server` MVP: registration (nick/user), channels, messaging, modes, MOTD, PING, TLS, config, per-IP limiter, registration deadline, token-bucket flood control, PROXY protocol v2 | HexChat connects, joins, chats; flood kick test passes |
| **3** | **Storage + Accounts + Cloaks**: SQLite via `sqlx`, `Store` trait, `REGISTER`/`VERIFY` with SMTP (`lettre`), cloak engine (HMAC), nick protection modes, SASL PLAIN+EXTERNAL, argon2 hashes | Registration happy-path + cloak stability + SASL tests green |
| **4** | **Opers + K-lines + Audit**: config-driven oper blocks, argon2, SASL-bound, hostmask-restricted, class-based privileges, `KLINE`/`UNKLINE`, audit log, `SHOWHOST` | Oper gating tests, audit log tests, kline tests green |
| **5** | **IRCv3 full baseline**: CAP 302, message-tags, server-time, echo-message, away-notify, account-notify, batch, chathistory, MONITOR | Conformance suite for IRCv3 specs green |
| **6** | **Anti-abuse expansion**: DNSBL, lockdown, penalty mode, nick/invite/CTCP rate limits, g-lines scaffolded | Scenario tests (DNSBL hit, lockdown, penalty escalation) green |
| **7** | **Metrics & Ops**: `metrics` crate wired, `/metrics` + `/health`, Grafana dashboards, per-binary Dockerfiles (multi-stage + distroless), docker-compose demo stack (server + bnc + prom + grafana + mailhog) | `curl /metrics` returns expected series; dashboards render; `docker compose up` brings full stack to green |
| **8** | `irc-client-core`: connection manager, state, events, scrollback, IRCv3 client, color/format, logging, SASL, registration client flow | Headless CLI connects to our server and to Libera; registration wizard works |
| **9** | `irc-client-gui` v1: treebar, window stack, nick list, input + completion, scrollback rendering, themes, SASL manager, registration wizard UI | Daily-driver against our server |
| **10** | Rhai scripting: aliases, event hooks, identifiers, dialogs, timers, security caps | Example scripts; `docs/scripting.md` complete |
| **11** | `irc-bnc`: upstream + downstream + buffers + replay + admin + multi-user + metrics | E2E test green; GUI through BNC transparently |
| **12** | DCC CHAT + SEND/GET; notifications; channel list browser; extras | DCC works between two GUI clients |
| **13** | Hardening & release: expanded fuzzing, load test (10k clients), security review, `cargo audit`, cosign-signed multi-arch OCI images (`linux/amd64` + `linux/arm64`) published to GHCR, SBOM via syft, release 0.1 | Tagged release; signed images pull + run clean; benchmarks documented |
| **14 (opt)** | TS6 server-to-server | Two linked nodes burst + stay in sync; netsplit simulated |

---

## 14. Dependencies (locked early, reviewed by cargo-deny)

| Purpose | Crate |
|---|---|
| async runtime | `tokio` (rt-multi-thread, macros, net, sync, io-util, time, signal) |
| codec | `tokio-util` (codec), `bytes` |
| TLS | `tokio-rustls`, `rustls`, `rustls-pemfile`, `rustls-native-certs` |
| PROXY protocol | `ppp` or hand-rolled v2 parser |
| logging / tracing | `tracing`, `tracing-subscriber` (env-filter, fmt, json), `tracing-appender` |
| metrics | `metrics`, `metrics-exporter-prometheus` |
| errors | `thiserror` (libs), `anyhow` (binaries) |
| config | `serde`, `serde_derive`, `toml`, `figment` (optional env overlay), `arc-swap` |
| storage | `sqlx` (runtime-tokio-rustls, sqlite, macros), `sqlx-cli` for migrations |
| hashing | `argon2`, `blake3`, `hmac`, `sha2` |
| email | `lettre` (tokio1-rustls-tls) |
| DNS | `hickory-resolver` (for DNSBL) |
| concurrency | `parking_lot`, `dashmap`, `arc-swap` |
| collections | `smallvec`, `indexmap`, `ahash` |
| GUI | `iced`, `iced_aw` |
| TUI (optional) | `ratatui`, `crossterm` |
| scripting | `rhai` |
| notifications | `notify-rust` |
| property / fuzz | `proptest`, `arbitrary`, `cargo-fuzz` |
| testing | `tokio-test`, `insta`, `cargo-nextest`, `cargo-llvm-cov` |
| time | `jiff` (preferred) or `time` |
| CLI | `clap` |
| secrets at rest (client) | `keyring`, `aes-gcm-siv` fallback |

---

## 15. Engineering practices

- `#![forbid(unsafe_code)]` everywhere.
- `#![warn(clippy::pedantic, clippy::nursery)]` with targeted allows; CI fails on new warnings.
- No `.unwrap()` / `.expect()` outside tests and `main.rs` bootstrap.
- Newtype every protocol identifier (`Nick`, `ChannelName`, `AccountName`, `CloakHost`, `RealHost`). Conversions explicit.
- Public API docs required on every `pub` item in library crates (`#![deny(missing_docs)]` on `irc-proto` and `irc-client-core`).
- Audit-sensitive actions flow through a dedicated `audit` tracing target; CI checks every oper command emits an audit event.
- Network I/O always behind timeouts; no unbounded reads.
- All queues bounded (`mpsc` with fixed capacity); backpressure is part of the design.
- Snapshot-tested wire bytes for every numeric.
- `cargo-deny`: license allow-list (MIT/Apache-2.0/BSD), no unmaintained, duplicate-major-version review.
- `cargo-nextest` for parallelism; `cargo-llvm-cov` for coverage.
- `sqlx` prepared queries checked at build (offline mode via `.sqlx/` dir committed).

---

## 16. Risks & open questions

- **iced MDI-style feel**: Elm-architecture doesn't give native MDI. Pane + tabs emulation is the plan; if it feels wrong in Phase 9, egui is the fallback.
- **Rhai throughput**: fine for event handlers, not great for heavy text munging. Expose Rust helpers for hot paths.
- **SMTP reliability**: retries + dead-letter queue for failed verification emails; document a "verify via oper" path when SMTP is unavailable.
- **Cloak collisions**: astronomically unlikely with HMAC-SHA256 truncated to 60 bits per segment, but we log collisions if detected and provide a `RECLOAK <account>` oper tool.
- **DNSBL false positives**: log-only mode available for gradual rollout.
- **Chathistory storage growth**: enforced retention per channel (size and time); backup guidance in ops docs.
- **Casemapping correctness**: history is littered with bugs here; dedicated fuzz target.
- **sqlx compile-time offline metadata**: must commit `.sqlx/` for hermetic builds; documented in CONTRIBUTING.
- **Multi-line messages (IRCv3 draft)**: punt to post-1.0; stub in codec.

---

## 17. First concrete steps (when execution begins)

1. Initialize Cargo workspace with seven crates as empty `lib.rs` / `main.rs` shells (includes `irc-testkit`).
2. Add `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`, GitHub Actions CI (fmt, clippy, test, deny).
3. In `irc-testkit`: define `Store` / `Clock` / `SmtpTransport` / `DnsblResolver` traits + in-memory impls.
4. In `irc-proto`: define `Message`, `Prefix`, `Command`, `ReplyCode`; line parser with property tests; codec; fuzz target.
5. In `irc-server`: minimal listener (TLS + plain), parses incoming, responds to PING, emits hardcoded MOTD on registration, wires `irc-testkit::TestServer`.
6. Prove the stack: write the first integration test (`registration_happy_path_minimal`) that connects a `ScriptedClient` via `irc-testkit`, sees RPL_WELCOME, and disconnects.
7. Wire CI so every PR runs fmt, clippy, unit, property, conformance, integration — full gate from day one.

From step 7 onward, phases 2–13 proceed as laid out in §13.
