# `examples/`

Reference configurations. The compose stack in `ops/compose/` seeds its local `server-config/` and `bnc-config/` directories from the files here.

Files (populated as features land):

- `server-config.toml` — full `irc-server` config with every section annotated
- `bnc-config.toml` — full `irc-bnc` config
- `scripts/` — example Rhai scripts (aliases, event hooks, dialogs)

Until the corresponding feature lands, each file is a minimal stub pointing at the relevant `PLAN.md` section.
