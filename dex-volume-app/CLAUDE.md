# CLAUDE.md

See @AGENTS.md for the full project reference — architecture, file index, conventions, and commands.

## Policies

**Documentation Sync**: When you modify code that changes architecture, add/remove files, change NATS subjects, modify DB schema, or alter crate structure — update `AGENTS.md` and `README.md` immediately. Keep documentation in sync with code at all times.

**Git Policy**: Never push commits to GitHub unless explicitly told to. Commits are fine; pushing is not.

**Frontend Sync**: When a backend change requires a corresponding frontend change, implement the frontend change immediately without asking. The frontend is at `frontend/`.

**Frontend Build Gate**: Before pushing any frontend changes to GitHub, run `npm run build` in `frontend/` and verify it succeeds. Do not push if the build fails.
