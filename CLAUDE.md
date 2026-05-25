# Dispatch

Terminal kanban board for dispatching Claude Code agents into isolated git worktrees via tmux.

**Stack**: Rust (2021 edition), ratatui TUI, SQLite (rusqlite), Axum HTTP/MCP server, tokio async runtime.

## Build & Test

```bash
cargo build
cargo test
cargo run -- tui
```

Other useful CLI subcommands:

```bash
cargo run -- setup              # configure Claude Code MCP integration
cargo run -- verify-feed 'gh api ...'  # run a feed command and validate its JSON output
```

Tasks are created exclusively via the MCP `create_task` tool — there is no CLI for task creation. Use the `/queue-plan` slash command (or call the MCP tool directly) to queue a plan file as a task.

### First-time setup

Point git at the repo's hooks directory so the pre-push hook runs:

```bash
git config core.hooksPath .githooks
```

The pre-push hook runs `cargo fmt` (auto-formats), `cargo clippy --all-targets -- -D warnings`, and `./scripts/check-doc-paths.sh` (validates doc links). Run `cargo test` separately before pushing.

### Running tests

```bash
cargo test                                # full suite
cargo test db::tests                      # database CRUD and migrations
cargo test service::                      # domain service layer
cargo test tui::tests                     # TUI input/message handling
cargo test mcp::handlers::tests           # MCP JSON-RPC handlers
cargo test --test lifecycle               # integration: full task lifecycle
cargo test --test epic_lifecycle          # integration: full epic lifecycle
cargo test --test cli                     # CLI subcommand smoke tests
cargo test tui::tests::scenarios          # key-sequence integration tests
cargo test tui::tests::snapshots          # ratatui buffer rendering tests
```

### Snapshot tests

Snapshots live in `src/tui/tests/snapshots/` and render to a 120×40 `TestBackend`. **Do not change the backend size** — it breaks all existing diffs.

Agent prompt snapshots live in `src/dispatch/snapshots/` and lock the rendered output of every `build_*_prompt` variant. Agent prompt bodies live in `src/dispatch/prompts/` as markdown files.

To accept intentional UI changes:

```bash
cargo insta review                                  # interactive
INSTA_UPDATE=always cargo test tui::tests::snapshots # auto-accept
INSTA_UPDATE=always cargo test dispatch::prompts_snapshots # auto-accept prompt snapshots
rm src/tui/tests/snapshots/*.snap.new                # always clean up
rm src/dispatch/snapshots/*.snap.new                 # always clean up
```

### Where new tests go

| What you're testing | Where |
|---|---|
| TUI key handling / message flow | `src/tui/tests/` |
| DB schema, CRUD, migrations | `src/db/tests/` |
| Service-layer business rules | inline in `src/service/<domain>/` |
| MCP JSON-RPC handler behaviour | `src/mcp/handlers/tests/` |
| Full task/epic lifecycle | `tests/` (integration tests) |
| Domain-type invariants | inline in the owning module |
| Agent prompt rendering (all variants) | `src/dispatch/prompts_snapshots.rs` |

Property tests live alongside unit tests in a nested `mod property_tests` block.

Inline test modules (`mod tests`, `mod property_tests`) must have `#[allow(clippy::unwrap_used, clippy::expect_used)]` at the top — the workspace `-D warnings` policy otherwise rejects bare `unwrap()`/`expect()` calls. See `src/db/tests/mod.rs` for the canonical pattern.

### Coverage

CI runs `cargo tarpaulin --out xml` in the `coverage` job. Run locally with `cargo tarpaulin --out Html`. Not in the pre-push hook.

## Test-Driven Development

Always use TDD. Express intended behaviour as tests before writing the code that satisfies them — for new features, bug fixes, and refactors alike.

## Allium Specification

The Allium specs in `docs/specs/` are the **source of truth** for domain logic:

- `core.allium` — domain model (entities, enums, config, VisualColumn)
- `tasks.allium` — task lifecycle (creation, status movement, reorder, archive, copy, editor)
- `dispatch.allium` — dispatching tasks, retry flows, repo-path persistence
- `agent-health.allium` — activity classification, crash detection, notifications, Claude Code hooks
- `pr-workflow.allium` — PR creation, polling, merge detection, wrap-up, finish paths
- `split-pane.allium` — split-pane lifecycle, focus border, jump-to-agent, pin, swap, tmux detach
- `mcp-task-tools.allium` — MCP tools for task management and the CLI plan-attachment surface
- `epics.allium` — epic lifecycle and MCP epic tools
- `learnings.allium` — knowledge base rules and MCP learning tools

Consult the relevant spec before changing core behavior. Use `allium:tend` and `allium:weed` skills to keep spec and code aligned.

## Agent Working Directory

Dispatched agents always work from their worktree folder. Every prompt includes an instruction to stay in the worktree and not `cd` to the parent repo. This is enforced in `dispatch_with_prompt()` in `src/dispatch/agents.rs`.

## Tag System

Tags (`TaskTag` in `src/models/tasks.rs`: `Bug`, `Feature`, `Chore`, `PrReview`, `Research`, `Fix`) drive dispatch behavior via `DispatchMode::for_task()`. A task with a plan always routes to `Dispatch` regardless of tag. Without a plan: `PrReview`/`Research`/`Fix` route to dedicated agents; everything else (including no tag) → `Dispatch`. Read `DispatchMode::for_task()` in `src/models/tasks.rs` for the authoritative mapping.

## Timing Constants

- **Tick interval** (2s): `TICK_INTERVAL` in `src/runtime/mod.rs` — captures tmux output, checks staleness.
- **Status TTL** (5s): `STATUS_MESSAGE_TTL` in `src/tui/mod.rs` — transient status bar messages auto-clear.
- **PR poll** (30s): `PR_POLL_INTERVAL` in `src/tui/mod.rs` — polls PR status for tasks in review.

## Documentation

This file is intentionally slim — it is loaded into every agent's context. Read these on demand:

> **Key pattern**: `FieldUpdate` / `TaskPatch` is the most-touched pattern in the codebase (nullable field mutations). Read [docs/conventions.md](docs/conventions.md) before writing any update handler.

- [docs/architecture.md](docs/architecture.md) — Message→Command, ProcessRunner, command queue draining, editor session invariant, review/security agent state machine, error handling, quick dispatch
- [docs/conventions.md](docs/conventions.md) — `FieldUpdate`, `TaskPatch`/`EpicPatch` double-Option, DB trait narrowing, `conn()`, inline-mutation boundary, `let _`, dead code, sub-status TOCTOU, immutable `parent_epic_id`, Clippy, visibility, performance footguns (`column_items_for_status` test-only; no `std::fs` in async)
- [docs/module-map.md](docs/module-map.md) — file-by-file responsibilities
- [docs/how-to.md](docs/how-to.md) — adding an MCP tool, TUI view, entity, database migration; projects feature; knowledge base MCP tools
- [docs/mcp.md](docs/mcp.md) — MCP notification flow, error codes, debugging handlers, feed epics, knowledge base flow
- [docs/reference.md](docs/reference.md) — key bindings, configuration, environment variables, troubleshooting, learning store
- [docs/specs/](docs/specs/) — Allium specifications for domain logic
- [docs/plans/](docs/plans/) — implementation plans (working artifacts, never committed)
