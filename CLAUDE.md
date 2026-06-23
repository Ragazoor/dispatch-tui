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

The pre-push hook runs `cargo fmt` (auto-formats), `cargo clippy --all-targets -- -D warnings`, `./scripts/check-doc-paths.sh` (validates doc links), and `./scripts/check-no-test-sleep.sh` (rejects `tokio::time::sleep` in test code — see the async-test rule below). Run `cargo test` separately before pushing.

The hook is tracked at `.githooks/pre-push`. A fresh clone must point git at it once — run `cargo run -- doctor hooks --repair` (which sets `core.hooksPath = .githooks`) or `git config core.hooksPath .githooks`. Don't add hooks to `.git/hooks/` directly: that directory is untracked and shared across all worktrees, so changes there aren't version-controlled or reviewed.

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

Suite is green; if a runtime test fails locally, suspect timing — `spawn_blocking`-based tests are timing-sensitive.

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

When writing async tests over `spawn_blocking` or detached `tokio::spawn` work, await a deterministic completion signal (oneshot / `Notify` / an `McpEvent`) or inject a clock — never `tokio::time::sleep`, which is flaky on slow CI. `./scripts/check-no-test-sleep.sh` (in the pre-push hook) enforces this. See the "No `tokio::time::sleep` in tests" section of `docs/conventions.md` for the canonical patterns.

### Coverage

CI runs `cargo tarpaulin --out xml` in the `coverage` job. Run locally with `cargo tarpaulin --out Html`. Not in the pre-push hook. Coverage is **informational** — there is no enforced threshold; it does not gate the build.

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

Dispatched agents always work from their worktree folder. Every prompt includes an instruction to stay in the worktree and not `cd` to the parent repo. This is enforced in `dispatch_with_prompt()` in `src/dispatch/agents.rs` by prompt instruction only — there is no test that asserts agents cannot escape the worktree.

## Tag System

Tags (`TaskTag` in `src/models/tasks.rs`: `Bug`, `Feature`, `Chore`, `PrReview`, `Research`, `Fix`) drive dispatch behavior via `DispatchMode::for_task()`. A task with a plan always routes to `Dispatch` regardless of tag. Without a plan: `PrReview`/`Research`/`Fix` route to dedicated agents; everything else (including no tag) → `Dispatch`. Read `DispatchMode::for_task()` in `src/models/tasks.rs` for the authoritative mapping.

## Timing Constants

- **Tick interval** (2s): `TICK_INTERVAL` in `src/runtime/mod.rs` — captures tmux output, checks staleness.
- **Status TTL** (5s): `STATUS_MESSAGE_TTL` in `src/tui/mod.rs` — transient status bar messages auto-clear.
- **PR poll** (30s): `PR_POLL_INTERVAL` in `src/tui/mod.rs` — polls PR status for tasks in review.

## Documentation

This file is intentionally slim — it is loaded into every agent's context. Read these on demand:

> **Key pattern**: `FieldUpdate` / `TaskPatch` is the most-touched pattern in the codebase (nullable field mutations). Read [docs/conventions.md](docs/conventions.md) before writing any update handler. See also the `OwnedTaskPatch` parity hazard in that doc — parity is now compiler-enforced via exhaustive destructuring.

> Bare `unwrap()`/`expect()` are clippy-warned outside tests — see the soft-fail-decoding section of `docs/conventions.md` for the canonical fallback pattern.

> **Mutation boundary**: reads via `state.db` are fine, but task/epic *mutations* should go through `TaskServiceApi`/`EpicServiceApi`, not the DB directly — the service layer owns invariants like epic-status recalculation. See the service mutation-boundary and `recalculate_epic_status` sections of `docs/conventions.md`.

- [docs/architecture.md](docs/architecture.md) — Message→Command, ProcessRunner, command queue draining, editor session invariant, review/security agent state machine, error handling, quick dispatch
- [docs/conventions.md](docs/conventions.md) — `FieldUpdate`, `TaskPatch`/`EpicPatch` double-Option, DB trait narrowing, `db_call`, service mutation boundary, `recalculate_epic_status` invariant, inline-mutation boundary, `LearningService` injection state, `let _`, dead code, sub-status TOCTOU, immutable `parent_epic_id`, Clippy, visibility, performance footguns (`column_items_for_status` test-only; no `std::fs` in async), prod-vs-test LOC split
- [docs/module-map.md](docs/module-map.md) — file-by-file responsibilities
- [docs/how-to.md](docs/how-to.md) — adding an MCP tool, TUI view, entity, database migration; projects feature; knowledge base MCP tools
- [docs/mcp.md](docs/mcp.md) — MCP notification flow, error codes, debugging handlers, feed epics, knowledge base flow
- [docs/reference.md](docs/reference.md) — key bindings, configuration, environment variables, troubleshooting, learning store
- [docs/specs/](docs/specs/) — Allium specifications for domain logic
- [docs/plans/](docs/plans/) — implementation plans and one-off analysis/review docs (working artifacts, never committed)

Subsystem entry points (no dedicated doc page — read the source):

- `src/feed/mod.rs` — feed system: `FeedRunner` poll loop, exec/parse/ingest pipeline that upserts tasks from external commands (see also `docs/module-map.md`)
- `src/service/repo_index/` (`mod.rs` orchestration + `scan.rs`/`chunking.rs`/`embed.rs`/`search.rs`), `src/service/embeddings.rs`, `src/mcp/handlers/repo_rag.rs` — repo indexing / embeddings / RAG: `index_repo` and `search_docs` MCP tools for semantic doc search
- `src/cli/` — CLI subcommand implementations, including the `doctor` health-check subcommand (`src/cli/doctor.rs`)
- `src/mcp/trajectory.rs` — agent trajectory capture (records the agent's tool-call history for a task)

## Unsafe Policy

Any `unsafe` block requires a `// SAFETY:` comment justifying why the invariant holds, and reviewer sign-off. See `docs/conventions.md` for the full policy.
