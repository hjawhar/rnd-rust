# Security and Abuse Posture

## TLS Enforcement

All client and server-to-server connections require TLS 1.2+. Plaintext listeners are not exposed by default. The server refuses to negotiate deprecated cipher suites. Certificate validation is enforced on S2S links; clients may use self-signed certificates but are encouraged to use valid ones for SASL EXTERNAL.

## Cloaks

User host cloaks replace the visible hostname with a hashed or vanity string. **Cloaks are not a security boundary.** They reduce casual IP exposure but do not protect against a determined attacker with access to server logs or network-level visibility. Operators can always see the real host.

## Flood Control

Flood protection operates at three layers:

1. **Connection rate limiting** — new connections per IP per interval, enforced before registration completes.
2. **Command throttling** — per-client token bucket applied after registration. Excess commands are queued or dropped, not buffered indefinitely.
3. **Message target limits** — caps on distinct PRIVMSG/NOTICE targets within a window to limit spam fan-out.

Clients that persistently exceed limits are disconnected. Repeat offenders may be automatically KLINE'd.

## KLINE Enforcement

KLINE bans match on `user@host` patterns and are evaluated at connection time and on rehash. Active sessions matching a new KLINE are terminated immediately. KLINE entries are persisted to disk and survive server restarts. Only operators with the `ban` privilege can set or remove KLINEs.

## SASL Credential Storage

User credentials for SASL PLAIN are hashed with argon2id using per-user salts. Plaintext passwords are never stored or logged. The server zeroes password buffers after hashing. SASL EXTERNAL delegates authentication to the client TLS certificate fingerprint.

## Oper Authentication

Operator blocks require both a password (argon2id-hashed, same as SASL) and an explicit host mask match. Oper passwords are never transmitted in cleartext — they travel over the mandatory TLS connection and are verified server-side against the stored hash. Failed OPER attempts are logged with source details.

## What We Don't Do

The following mechanisms are deliberately not implemented:

- **Proof-of-Work challenges** — adds latency and complexity with marginal benefit against modern bot tooling.
- **CAPTCHA** — IRC clients have no standard mechanism to present or solve CAPTCHAs.
- **IP reputation / DNSBL lookups** — external dependency with false-positive risk. Operators can integrate external blocklists via KLINE tooling if desired.

These decisions may be revisited if the threat model changes.
