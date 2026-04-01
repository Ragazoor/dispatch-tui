# Dispatch

Terminal kanban board for dispatching Claude Code agents into isolated git worktrees via tmux.

**Stack**: Rust (2021 edition), ratatui TUI, SQLite (rusqlite), Axum HTTP/MCP server, tokio async runtime.

## Build & Test

```bash
cargo build
cargo test
cargo run -- tui
```

Pre-commit hook runs `cargo fmt --check` and `cargo clippy -- -D warnings` automatically — no need to run these manually.

## Allium Specification

`docs/specs/dispatch.allium` is the **source of truth** for domain logic: task lifecycle, status transitions, sub-status invariants, dispatch rules, and epic behavior. Consult it before changing core behavior. Use `allium:tend` and `allium:weed` skills to keep spec and code aligned.

## Architecture

Key patterns that aren't obvious from reading the code:

- **Message → Command**: `App::update()` processes input messages and returns `Command`s (side effects). Keep rendering pure, effects in commands.
- **ProcessRunner trait**: Abstraction over git/tmux shell commands. Tests use `MockProcessRunner` — never shell out in tests.
- **TaskPatch builder**: Selective field updates for the database. `None` = don't change, `Some(None)` = set field to NULL.
- **MCP server**: Runs on port 3142 (configurable via `DISPATCH_PORT`). Agents call JSON-RPC methods in `src/mcp/handlers/` to update task status.
- **Integration tests**: Use `Database::open_in_memory()` with a real SQLite instance — no mocking the database layer.

## Documentation

- `docs/reference.md` — Key bindings, configuration, environment variables, troubleshooting
- `docs/specs/` — Allium specifications for domain logic
- `docs/plans/` — Implementation plans (working artifacts, never committed)
