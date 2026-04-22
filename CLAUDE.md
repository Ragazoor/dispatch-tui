# Dispatch

Terminal kanban board for dispatching Claude Code agents into isolated git worktrees via tmux.

**Stack**: Rust (2021 edition), ratatui TUI, SQLite (rusqlite), Axum HTTP/MCP server, tokio async runtime.

## Build & Test

```bash
cargo build
cargo test
cargo run -- tui
```

Pre-push hook runs `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` automatically before each push. One-time setup:

```bash
git config core.hooksPath .githooks
```

### Running Tests

Run the full suite or target a specific module:

```bash
# Full suite
cargo test

# Module-level tests
cargo test db::tests               # database CRUD and migrations
cargo test service::tests          # domain service layer
cargo test tui::tests              # TUI input/message handling
cargo test mcp::handlers::tests    # MCP JSON-RPC handlers

# Integration tests
cargo test --test lifecycle        # full task lifecycle (create → dispatch → done)
cargo test --test epic_lifecycle   # full epic lifecycle
cargo test --test cli              # CLI subcommand smoke tests

# Single test by name (substring match)
cargo test update_task_params_has_any_field
```

## Test-Driven Development

Always use TDD when working in this repo. Start by expressing the intended behaviour as tests — capture what the code should do before writing the code that does it. Then implement the minimum code to make the tests pass. This applies to all changes — new features, bug fixes, and refactors.

## Allium Specification

The Allium specs in `docs/specs/` are the **source of truth** for domain logic:
- `core.allium` — domain model (entities, enums, config, VisualColumn)
- `tasks.allium` — task lifecycle, agent health, hooks, notifications, split pane, MCP task tools
- `epics.allium` — epic lifecycle and MCP epic tools

Consult the relevant spec before changing core behavior. Use `allium:tend` and `allium:weed` skills to keep spec and code aligned.

## Agent Working Directory

Dispatched agents always work from their worktree folder. Every prompt includes an instruction to stay in the worktree and not `cd` to the parent repo. This is enforced in `dispatch_with_prompt()` in `src/dispatch.rs`.

## Architecture

Key patterns that aren't obvious from reading the code:

- **Message → Command**: `App::update()` processes input messages and returns `Command`s (side effects). Keep rendering pure, effects in commands.
- **Inline-mutation convention**: Input handlers in `input.rs` directly mutate `self.input.mode`, cursor positions, and other UI-only state, returning `vec![]` (no commands). This is intentional — not an Elm Architecture violation. The rule: if a state change has no side effects (no DB write, no process spawn, no network call), mutate inline and return empty. If it needs a side effect, return a `Command`.
- **ProcessRunner trait**: Abstraction over git/tmux shell commands. Tests use `MockProcessRunner` — never shell out in tests.
- **TaskPatch builder**: Selective field updates for the database. `None` = don't change, `Some(None)` = set field to NULL.
- **MCP server**: Runs on port 3142 (configurable via `DISPATCH_PORT`). Agents call JSON-RPC methods in `src/mcp/handlers/` to update task status.
- **Integration tests**: Use `Database::open_in_memory()` with a real SQLite instance — no mocking the database layer.
- **Command queue draining**: `execute_commands` (`src/runtime.rs`) loads the initial `Vec<Command>` into a `VecDeque` and drains it iteratively. Any handler that produces additional commands (e.g. error-path `app.update()` calls that return extra commands) extends the queue with `queue.extend(extra)`, so a single message can cascade into multiple commands without recursive calls.

### Review Board

The Review Board (`ViewMode::ReviewBoard`) shows GitHub PRs across three modes, toggled with `1`/`2`/`3`:

| Mode | What it shows |
|------|---------------|
| `Reviewer` | PRs where you are a requested reviewer |
| `Author` | PRs you authored |
| `Dependabot` | Dependabot PRs across configured repos |

Each mode has 4 columns representing PR review states. The board auto-refreshes via `REVIEW_REFRESH_INTERVAL`. Dispatching a review agent from a PR opens a worktree for the review.

### Security Board

The Security Board (`ViewMode::SecurityBoard`) shows GitHub security alerts fetched via `gh api`. Alerts are categorized by kind (`AlertKind::Dependabot`, `AlertKind::CodeScanning`) and displayed in 4 severity columns (`Critical`, `High`, `Medium`, `Low`). Filtered by `RepoFilterMode`. Auto-refreshes via `SECURITY_POLL_INTERVAL`. Dispatching a fix agent from an alert creates a worktree to address the vulnerability.

### Board Switching

Switch boards with `Tab` (task → review → security → task). Each board preserves its own selection state independently.

### Error Handling

The codebase uses three error types at different layers:

- **`anyhow::Result`** — infrastructure and IO errors (file operations, shell commands, DB initialization). Used at the outer edges where errors propagate up to the caller.
- **`ServiceError`** (`Validation` / `NotFound` / `Internal`) — business logic errors in `src/service.rs`. MCP handlers match on these to return appropriate JSON-RPC error codes.
- **Domain-specific errors** (`FinishError`, `PrError`) — operations with distinct failure modes that callers need to handle differently (e.g., rebase conflicts vs. push failures).

Rule of thumb: use `ServiceError` for request validation and business rules, domain-specific errors when callers branch on the variant, and `anyhow` for everything else.

## Tag System

Tags (`bug`, `feature`, `chore`, `epic`) drive dispatch behavior via `DispatchMode::for_task()` in `models.rs`:

| Tag | No plan | Has plan |
|-----|---------|----------|
| `epic` | Brainstorm (ideation, no edits) | Dispatch |
| `feature` | Plan (write implementation plan) | Dispatch |
| `bug`, `chore`, none | Dispatch | Dispatch |

A task with a plan always dispatches directly regardless of tag. Tags are selected during task creation: `b`=bug, `f`=feature, `c`=chore, `e`=epic, Enter=none.

## Timing Constants

- **Tick interval** (2s): `TICK_INTERVAL` in `runtime.rs` — captures tmux output, checks staleness.
- **Status TTL** (5s): `STATUS_MESSAGE_TTL` in `tui/mod.rs` — transient status bar messages auto-clear.
- **PR poll** (30s): `PR_POLL_INTERVAL` in `tui/mod.rs` — polls PR status for tasks in review.
- **Review refresh** (30s): `REVIEW_REFRESH_INTERVAL` in `tui/mod.rs` — refreshes Review Board PR data.
- **Security poll** (5m): `SECURITY_POLL_INTERVAL` in `tui/mod.rs` — polls GitHub security alerts.

## Module Map

| File | Responsibility |
|------|---------------|
| `src/main.rs` | CLI entry point (clap), subcommand dispatch (`tui`, `update`, `add`) |
| `src/lib.rs` | Crate root, public module re-exports |
| `src/runtime.rs` | Async event loop (`tokio::select!`), bridges TUI ↔ MCP ↔ shell commands, executes `Command` side effects |
| `src/tui/mod.rs` | `App` struct, `update()` message dispatcher, `column_items_for_status()` render helper |
| `src/tui/input.rs` | Key event handlers, inline-mutation convention for UI-only state |
| `src/tui/ui/mod.rs` | Rendering entry point — re-exports `render()`, thin dispatcher |
| `src/tui/ui/kanban.rs` | Kanban board rendering: task/epic cards, columns, overlays, action hints |
| `src/tui/ui/review.rs` | Review board rendering: PR columns, detail panel, review action hints |
| `src/tui/ui/security.rs` | Security board rendering: Dependabot and alert columns, detail panel |
| `src/tui/ui/shared.rs` | Cross-board helpers: `render_tab_bar`, `refresh_status`, `truncate`, `push_hint_spans` |
| `src/tui/ui/palette.rs` | Tokyo Night color palette constants |
| `src/tui/types.rs` | `Message`, `Command`, `ViewMode`, `InputMode`, `AgentTracking` enums and structs |
| `src/tui/tests.rs` | TUI unit tests |
| `src/models.rs` | Domain types (`Task`, `Epic`, `TaskStatus`, `SubStatus`, `TaskTag`), `DispatchMode::for_task()` tag routing |
| `src/service.rs` | Domain service layer (`TaskService`, `EpicService`): business logic (validation, patch building, epic recalculation) decoupled from MCP/HTTP; also owns `FieldUpdate` and `UpdateTaskParams`/`UpdateEpicParams` |
| `src/db/mod.rs` | `Database` struct, constructor, `TaskStore` trait, `TaskPatch`/`EpicPatch` builders |
| `src/db/migrations.rs` | Versioned schema migrations (`MIGRATIONS` array, `migrate_vN_*` functions) |
| `src/db/queries.rs` | `impl TaskStore for Database` — all CRUD operations, row helpers |
| `src/db/tests.rs` | Database unit tests |
| `src/dispatch.rs` | Worktree creation, tmux session management, agent lifecycle (dispatch/brainstorm/plan/resume/review) |
| `src/process.rs` | `ProcessRunner` trait + `RealProcessRunner` / `MockProcessRunner` for testable shell execution |
| `src/tmux.rs` | Tmux API: create windows, send keys, capture pane output, kill windows |
| `src/github.rs` | GitHub CLI (`gh`) integration: PR creation, review status polling, CI status |
| `src/editor.rs` | External `$EDITOR` integration for editing task/epic fields |
| `src/plan.rs` | Plan file parsing (extract title/description from markdown) |
| `src/setup.rs` | First-run setup: MCP config merging, plugin installation (hooks, skills, commands) |
| `src/mcp/mod.rs` | MCP server bootstrap (Axum router), `McpState`, `McpEvent` notification enum |
| `src/mcp/handlers/dispatch.rs` | JSON-RPC entry point (`handle_mcp`), tool definitions, method routing |
| `src/mcp/handlers/tasks.rs` | Task tool handlers (thin wrappers): parse JSON-RPC args → call `TaskService` → format response |
| `src/mcp/handlers/epics.rs` | Epic tool handlers (thin wrappers): parse JSON-RPC args → call `EpicService` → format response |
| `src/mcp/handlers/types.rs` | JSON-RPC request/response types, flexible integer deserializer |
| `src/mcp/handlers/tests.rs` | MCP handler integration tests |

## MCP Notification Flow

When an MCP handler mutates the database, the TUI must refresh to show the change. This is the propagation path:

```
MCP handler (e.g. handle_update_task)
  → mutates DB via state.db
  → calls state.notify()                          # McpState method
    → sends McpEvent::Refresh via mpsc::UnboundedSender
      → runtime event loop receives it             # tokio::select! in run_event_loop()
        → calls rt.exec_refresh_from_db(app)
          → reads all tasks/epics from DB
          → calls app.update(Message::RefreshTasks(tasks))
            → App replaces its in-memory task list, re-renders
```

Key types in the chain:
- `McpEvent` (`src/mcp/mod.rs`) — enum with `Refresh` and `MessageSent` variants
- `McpState::notify()` — fire-and-forget send on the channel
- `TuiRuntime::exec_refresh_from_db()` (`src/runtime.rs`) — reloads tasks, epics, and usage from DB
- `Message::RefreshTasks` (`src/tui/types.rs`) — carries the fresh task list into the App

The `MessageSent` variant additionally triggers `Message::MessageReceived(task_id)`, which flashes the target task's card in the TUI.

## Visibility Convention

`App` fields use `pub(in crate::tui)` to restrict mutation to the TUI module. External code (runtime, MCP handlers) can only change `App` state by sending a `Message` through `app.update()`, which returns `Command`s. This keeps state transitions auditable in one place and prevents scattered mutation from outside the TUI boundary.

## Quick Dispatch

`Shift+D` creates and immediately dispatches a task without going through the task creation dialog. The flow:

1. Key handler emits `Command::QuickDispatch { draft: TaskDraft { title: DEFAULT_QUICK_TASK_TITLE, repo_path, .. }, epic_id }` (`src/tui/mod.rs`)
2. Runtime handles it in `exec_quick_dispatch()` (`src/runtime.rs`) — calls `create_task()` then immediately dispatches
3. The created task gets title `"Quick task"` (`DEFAULT_QUICK_TASK_TITLE` in `src/models.rs`), no tag, no plan
4. If the board has multiple repo paths, `Shift+D` first enters `InputMode::QuickDispatchRepo` (repo picker), then emits `Message::SelectQuickDispatchRepo(idx)` to resolve the repo before dispatching

Quick dispatch is the same code path as normal dispatch — the difference is the task is created with defaults and skips the creation dialog entirely.

**Command-level shortcut:** `Command::QuickDispatch` bypasses the normal `Command::DispatchAgent` → `Message::Dispatched` round-trip. `exec_quick_dispatch()` calls `create_task()` and immediately dispatches in a single step — there is no intermediate message back through `app.update()`.

## Code Conventions

### `FieldUpdate` — nullable string fields

`FieldUpdate` (`src/service.rs`) replaces the `Option<String>` + empty-string sentinel anti-pattern for fields that need three states: "don't touch", "set to value", "clear to NULL":

```rust
pub enum FieldUpdate {
    Set(String),  // set the field to this value
    Clear,        // set the field to NULL
}
```

Used in `UpdateTaskParams` for `pr_url`, `worktree`, and `tmux_window`. When adding a new nullable field to `UpdateTaskParams`, use `Option<FieldUpdate>` rather than `Option<Option<String>>`.

### `TaskPatch` / `EpicPatch` — double-Option in the DB layer

`TaskPatch` and `EpicPatch` (`src/db/mod.rs`) use `Option<Option<T>>` for nullable fields — the DB-layer equivalent of `FieldUpdate`:

| Value | Meaning |
|-------|---------|
| `None` | Don't touch this field |
| `Some(None)` | Set the field to NULL |
| `Some(Some(v))` | Set the field to `v` |

The service layer bridges the two patterns before writing a patch: `FieldUpdate::Set(v)` becomes `Some(Some(v))` and `FieldUpdate::Clear` becomes `Some(None)`. When adding a new nullable field, use `FieldUpdate` in `UpdateTaskParams`/`UpdateEpicParams` and double-Option in the corresponding patch struct.

### DB trait narrowing — take the narrowest sub-trait you need

`TaskStore` is a supertrait of `TaskAndEpicStore + PrStore + AlertStore + SettingsStore`. New consumers should hold the narrowest sub-trait they actually call:

| Consumer | Holds |
|----------|-------|
| `TaskService` | `Arc<dyn TaskAndEpicStore>` |
| `EpicService` | `Arc<dyn EpicCrud>` |
| `McpState`, `TuiRuntime` | `Arc<dyn TaskStore>` (fans out to all sub-traits) |

`Arc<dyn TaskStore>` coerces to any narrower trait object at call sites via Rust's trait-object upcasting (stabilised in 1.86). If you need to split a wide `Arc<dyn TaskStore>` into a narrower one, use a typed `let` binding: `let d: Arc<dyn EpicCrud> = task_store_arc.clone();`.

### `conn()` — safe database access

Always acquire the SQLite connection via `self.conn()?` (`src/db/mod.rs`). This method locks the mutex and propagates a `Result` error if the lock is poisoned, rather than panicking. Never call `self.conn.lock().unwrap()` directly — that pattern was eliminated and any new code that reintroduces it will panic on a poisoned lock.

### Inline-mutation boundary

Key handlers in `src/tui/input.rs` follow two different patterns:

- **Mutate inline, return `vec![]`** — for UI-only state with no side effects (cursor position, `input.mode`, selected index, text buffer). These changes don't need to be auditable and touching the DB/processes isn't required.
- **Return a `Command`** — for anything that needs a side effect: DB write, process spawn, network call, or waking the runtime.

The rule: if you're only changing what the screen looks like without touching external state, mutate inline. If the change needs to outlast the current render cycle or involve I/O, return a `Command`.

### Intentional `let _ =`

`let _ = expr` silences the `#[must_use]` warning on a result or value. In this codebase it appears in two patterns — neither is a bug:

- **Fire-and-forget channel sends** — `let _ = tx.send(McpEvent::Refresh)` in `src/mcp/mod.rs`: the send can only fail if the receiver has dropped (TUI exited), which is fine to ignore
- **Non-critical side-effect patches** — `let _ = self.db.patch_epic(...)` where the caller cannot usefully recover from a transient DB error on a non-primary write

If you see `let _ =` and are unsure whether it's intentional, check the surrounding comment or commit message. Add a comment when adding a new one.

### `#[allow(dead_code)]`

Avoid `#[allow(dead_code)]` — dead code should be removed, not suppressed. If a type or function is unused today but is part of an in-progress feature, document it with a comment pointing at the relevant issue/task rather than silencing the warning.

## How-To Guides

### Adding a New MCP Tool

1. **Define the handler** in `src/mcp/handlers/tasks.rs` (or `epics.rs` for epic tools). Follow the pattern: parse args with `types::parse_args`, call `state.db` methods, call `state.notify()` if mutating, return `JsonRpcResponse::ok`.
2. **Add the tool schema** to `tool_definitions()` in `src/mcp/handlers/dispatch.rs` — add a new entry to the `tools` array with `name`, `description`, and `inputSchema`.
3. **Wire the route** in `handle_mcp()` in `src/mcp/handlers/dispatch.rs` — add a match arm in the `tools/call` section mapping the tool name to your handler.
4. **Add types** if needed in `src/mcp/handlers/types.rs` (argument structs with serde derives, use `#[serde(deserialize_with = "deserialize_flexible_i64")]` for integer fields since Claude Code may send them as strings).
5. **Write tests** in `src/mcp/handlers/tests.rs` using `Database::open_in_memory()`.

### Adding a New TUI View/Mode

1. **Add a `ViewMode` variant** in `src/tui/types.rs` (e.g., `ViewMode::MyNewView { selection, saved_board }`).
2. **Add `Message` variants** for entering/exiting and any view-specific actions.
3. **Add `Command` variants** if the view triggers side effects (DB writes, shell commands).
4. **Handle input** in `src/tui/input.rs` — add key handlers under a new match arm for your `ViewMode`.
5. **Handle messages** in `src/tui/mod.rs` `update()` — process your new messages, return commands.
6. **Render** in the appropriate `src/tui/ui/` module (`kanban.rs`, `review.rs`, or `security.rs`) — add a rendering branch for your view mode in `kanban.rs::render()`.

### Adding a Database Migration

Migrations live in `src/db/migrations.rs` as standalone functions:

1. **Write the migration function**: `fn migrate_vN_description(conn: &Connection) -> Result<()>` in `src/db/migrations.rs`. Use `ALTER TABLE` for additive changes; for destructive changes (column removal, constraint changes), create a new table, copy data, drop old, rename.
2. **Register it** in the `MIGRATIONS` array in `src/db/migrations.rs`: add `(N, migrate_vN_description)`. The loop in `Database::init_schema()` applies any migration where `current_version < N` and bumps `PRAGMA user_version` after each.
3. **Update the schema test**: `fresh_db_has_latest_schema_version` in `src/db/tests.rs` asserts the final version number — bump it to match your new N.
4. **Write a migration test** in `src/db/tests.rs` that creates a DB at the pre-migration schema, inserts test data, runs the migration, and verifies the result.

## Documentation

- `docs/reference.md` — Key bindings, configuration, environment variables, troubleshooting
- `docs/specs/` — Allium specifications for domain logic
- `docs/plans/` — Implementation plans (working artifacts, never committed)
