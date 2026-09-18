# `ops/compose/`

Docker Compose dev + e2e stack. Populated in Phase 7 (see [`PLAN.md` §13](../../PLAN.md#13-phased-delivery)).

Planned services:
- `irc-server` — daemon
- `irc-bnc` — bouncer
- `prometheus` — scrapes both
- `grafana` — dashboards preloaded from `../grafana/`
- `mailhog` — SMTP sink for local registration testing

Local configs live in `server-config/` and `bnc-config/` (gitignored, seeded from `../../examples/`).
