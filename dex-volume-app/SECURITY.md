# Security Policy

## Reporting a Vulnerability

**Do not open public issues for security vulnerabilities.**

Use GitHub's private vulnerability reporting:

1. Go to the [Security tab](../../security/advisories/new) of this repository
2. Click "Report a vulnerability"
3. Describe the issue with reproduction steps

If you cannot use GitHub Security Advisories, encrypt your report to the project maintainers via the contact listed in the repository description.

## Scope

Security issues of interest include, but are not limited to:

- **Wallet encryption / key handling** — flaws in AES-256-GCM usage, key derivation, storage, or JWT signing (`backend/crates/vm-data/src/utils/encryption.rs`, `claims.rs`)
- **Authentication / authorization** — JWT validation, signature verification, project access control bypass
- **Trading logic** — exploits that cause fund loss (incorrect slippage, unsafe arithmetic, race conditions in bundle execution)
- **NATS message validation** — deserialization attacks, subject spoofing
- **SQL injection, command injection, path traversal** in any service

Out of scope:
- Denial of service requiring physical/local access
- Social engineering
- Issues requiring a compromised RPC provider or MITM on TLS

## Response Commitment

This is an open-source project maintained on best-effort. Reporters will receive an acknowledgment within **7 days**. Fix timelines depend on severity and complexity.

## Disclosure

Coordinated disclosure: please wait for a fix (or 90 days, whichever comes first) before publicly disclosing the issue.

## Known Security Notes

- **Key rotation is destructive**: rotating `AES256_GCM_KEY` makes all stored encrypted wallets unreadable. Rotating `ED25519_KEY` invalidates all active JWTs. Plan carefully.
- **`.env` files must never be committed**. The repo's `.gitignore` blocks them; verify before pushing.
