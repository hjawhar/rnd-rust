# `ops/grafana/`

Grafana dashboards shipped as JSON. Provisioned by the compose stack; wire into your own Grafana with file-based provisioning. Populated in Phase 7.

Planned dashboards:
- `server-dashboard.json` — connections, msgs/s, flood kicks, auth outcomes, DNSBL hits
- `bnc-dashboard.json` — per-user attached clients, reconnects, buffer depth
- `protocol-dashboard.json` — command mix, error numerics, CAP negotiation outcomes
