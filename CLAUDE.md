# Dispatch

A terminal kanban board for managing development tasks and dispatching Claude Code agents into isolated git worktrees + tmux windows.

## Build & Test

```bash
cargo build
cargo test
cargo clippy
cargo run -- tui   # launch the TUI (requires running inside a tmux session)
```

Runtime dependencies: `tmux`, `git` (checked at startup). The TUI must be launched from within a tmux session for agent dispatch to work.

> **Tmux prerequisite:** If you see `not running inside a tmux session`, run `tmux new-session -d -s dev` first, then re-run `cargo run -- tui`.

## Architecture

**Elm Architecture** — events produce `Message`s, `App::update()` returns `Vec<Command>`, commands are executed by the main loop.

```
Terminal events ──┐
Async messages ───┤──▶ App::update(Message) ──▶ Vec<Command> ──▶ execute_commands()
Tick timer ───────┘                                                  │
                                                                     ├── PersistTask → SQLite
                                                                     ├── Dispatch → worktree + tmux + claude
                                                                     ├── CaptureTmux → tmux capture-pane
                                                                     └── RefreshFromDb → re-read tasks
```

## Key Files

```
src/
├── main.rs          # Entry point, CLI argument parsing
├── models.rs        # Task, TaskStatus, slugify, COLUMN_COUNT
├── db.rs            # TaskStore trait + SQLite Database impl (Mutex<Connection>)
├── dispatch.rs      # Agent dispatch: worktree creation, tmux window, MCP config, prompt
├── tmux.rs          # tmux subprocess wrappers (new-window, send-keys, capture-pane, has-window)
├── runtime.rs       # TUI main loop, TuiRuntime with exec_* command handlers
├── editor.rs        # External editor integration (format/parse task content)
├── tui/
│   ├── mod.rs       # App state, handle_* message handlers, update() routing
│   ├── types.rs     # Message, Command, InputMode, TaskDraft enums
│   ├── input.rs     # Keyboard input handling per mode (normal, text input, confirm)
│   ├── ui.rs        # Ratatui rendering (columns, detail panel, status bar)
│   └── tests.rs     # Unit tests for App state machine
└── mcp/
    ├── mod.rs       # Axum router + server setup
    └── handlers.rs  # JSON-RPC MCP handlers (update_task, get_task, create_task)
```

## Kanban Columns

Backlog → Running → Review → Done

- **Backlog** = tasks not yet started. Tasks with a plan show `▸` icon; `d` dispatches an implementation agent. Tasks without a plan get `d` for brainstorm.
- **Running** = agent dispatched in interactive mode, tmux output shown on card
- **Dispatch** (`d` on a Backlog task with a plan): creates a fresh git worktree + tmux window and launches Claude with the task prompt
- **Brainstorm** (`d` on a Backlog task without a plan): creates a worktree and launches Claude in brainstorm mode to explore and plan
- **Resume** (`d` on a Running task whose window is gone): re-opens a tmux window in the existing worktree and runs `claude --continue`. Closing a tmux window does **not** delete the worktree.
- Status transitions (running/review) are handled by hooks in `.claude/settings.json` that extract the task ID from the git branch name (`{id}-{slug}` pattern)
- Press `g` to jump to an agent's tmux window

## Hooks & Branch Naming

Status update hooks in `.claude/settings.json` run when Claude Code starts or stops in a worktree. They parse the branch name, extract the task ID, and call `dispatch update <id> <status>`.

**Requirements:**
- Worktree branches must follow `{id}-{slug}` (e.g. `42-fix-login-bug`). Non-conforming names silently skip status updates.
- `dispatch` must be in `PATH`. Add the debug binary: `export PATH="$PATH:$(pwd)/target/debug"`

## MCP Server

Starts alongside TUI on `localhost:3142`. Agents use it to query and update tasks.
Tools: `update_task`, `get_task`, `create_task`, `list_tasks`, `claim_task`, `create_epic`, `get_epic`, `list_epics`, `update_epic`.

To test tools manually (while TUI is running):
```bash
curl -s -X POST http://localhost:3142/mcp \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_task","arguments":{"task_id":1}}}'
```

## Configuration

| Flag | Env Var | Default |
|------|---------|---------|
| `--db` | `DISPATCH_DB` | `~/.local/share/dispatch/tasks.db` |
| `--port` | `DISPATCH_PORT` | `3142` |

## Adding a New Message/Command

1. Add the `Message` variant to `src/tui/types.rs`
2. Add a `handle_<name>` private method on `App` in `src/tui/mod.rs`
3. Add the routing arm in `App::update()` (one line)
4. Add tests in `src/tui/tests.rs`
5. If side effects needed: add a `Command` variant in `src/tui/types.rs`
6. Add an `exec_<name>` method on `TuiRuntime` in `src/runtime.rs`
7. Add the routing arm in `execute_commands()` (one line)

## MCP Notification Pattern

MCP handlers (`src/mcp/handlers.rs`) run in a separate Axum context and cannot send `Message`s directly to the TUI event loop. Instead they:

1. Write to the database directly via `TaskStore`
2. Send a `()` notification on a channel
3. The TUI main loop receives the notification and calls `exec_refresh_from_db()`, which re-reads all tasks and sends `Message::RefreshTasks`

This is an intentional bypass of the Elm Architecture for cross-process mutations. App state may briefly lag behind the database until the next refresh (triggered immediately on notification, plus every tick at ~2s).

## Tick Interval

The TUI tick fires every ~2 seconds (`src/runtime.rs`). Each tick:
- Captures tmux pane output for all running agents
- Checks for stale agents (no output change beyond `inactivity_timeout`)
- Triggers a `RefreshFromDb` to pick up external changes

## Testing

- **MockProcessRunner** (`src/process.rs`): Pre-queue responses with `MockProcessRunner::new(vec![...])`. Use `MockProcessRunner::ok()`, `::fail(stderr)`, `::ok_with_stdout(bytes)`. Call `recorded_calls()` to verify program names and arguments.
- **In-memory SQLite**: Use `Database::open_in_memory()` for isolated tests with no file I/O.
- **Test helpers** (`src/tui/tests.rs`): `make_app()` creates a default App, `make_task()` creates a task with defaults, `render_to_buffer()` renders to an in-memory terminal, `buffer_contains()` searches rendered output.
- **Runtime tests** (`src/runtime.rs`): Use `test_runtime()` to get a `TuiRuntime` + `App` wired to in-memory DB, mock runner, and real message channels.

### Integration Tests

`tests/` contains CLI integration tests (`tests/cli.rs`) and a full lifecycle test (`tests/lifecycle.rs`). These test the binary's subcommands (create, list, update, plan) against a real SQLite database.

### Property-Based Tests

`proptest` (dev-dependency) is used for fuzzing `parse_plan` in `src/plan.rs`. Use this pattern for any parser or transformer that should handle arbitrary input without panicking.

## MCP Schema Maintenance

Tool definitions in `mcp/handlers/dispatch.rs` (`tool_definitions()`) must be manually kept in sync with the typed argument structs in `tasks.rs` / `epics.rs`. The `tool_schemas_match_arg_structs` test validates property names, required fields, and deserialization for every tool — it will fail if a struct field is added without updating the schema (or vice versa).

## Common Pitfalls

- **`patch_task` / `patch_epic`**: These build dynamic SQL from only the fields set in the patch. When adding a new column to `tasks` or `epics`, add a corresponding field to `TaskPatch` / `EpicPatch` and a new `if let Some(...)` branch in the patch method.
- **MCP tool definitions**: See MCP Schema Maintenance above. The sync test catches drift, but remember to update both the JSON schema in `dispatch.rs` and the arg struct + the test's `cases` vec.
- **`InputMode` carries data**: Some variants like `ConfirmRetry(TaskId)` and `ConfirmWrapUp(TaskId)` carry the target ID. Extract the ID from the mode in the handler — don't re-read from `selected_task()` as the cursor may have moved.
- **`Instant` in tests**: `AgentTracking` uses `std::time::Instant` which cannot be faked. Tests that depend on elapsed time test the handler directly rather than going through `handle_tick`.

## InputMode Transitions

```
Normal ──n──▶ InputTitle ──Enter──▶ InputDescription ──Enter──▶ InputRepoPath ──Enter──▶ Normal
Normal ──E──▶ InputEpicTitle ──Enter──▶ InputEpicDescription ──Enter──▶ InputEpicRepoPath ──Enter──▶ Normal
Normal ──D──▶ QuickDispatch ──1-9──▶ Normal
Normal ──x──▶ ConfirmArchive ──y──▶ Normal
Normal ──m (Review→Done)──▶ ConfirmDone(id) ──y──▶ Normal
Normal ──f──▶ RepoFilter ──Enter/Esc──▶ Normal
Normal ──W──▶ ConfirmWrapUp(id) ──r──▶ Normal (rebase)
                                ──p──▶ Normal (PR)
                                ──Esc──▶ Normal
Normal ──d (stale/crashed)──▶ ConfirmRetry(id) ──r/f──▶ Normal
Normal ──?──▶ Help ──?/Esc──▶ Normal

Any input mode ──Esc──▶ Normal (cancels)
Error popup ──any key──▶ dismisses
```

## Conventions

- Rust edition 2021, SQLite with bundled `libsqlite3-sys`
- Sync `rusqlite` with `Mutex` (not async wrapper)
- All subprocess calls go through `src/tmux.rs` or `src/dispatch.rs`, injected with a `ProcessRunner` (`src/process.rs`). Use `MockProcessRunner` in tests.
- Tests use in-memory SQLite databases
- **App field visibility**: All `App` fields use `pub(in crate::tui)` — accessible from `input.rs`, `ui.rs`, `tests.rs` but not outside the `tui` module. External code uses public accessor methods.
- **Column count**: `TaskStatus::COLUMN_COUNT` is the canonical source. Never hardcode the column count.
- **Database abstraction**: `db::TaskStore` trait abstracts persistence. `TuiRuntime` and `McpState` hold `Arc<dyn TaskStore>`. Tests can provide mock implementations.
- **Task lookup**: Use `App::find_task(id)` / `find_task_mut(id)` instead of inline `.iter().find()`.
- **Error handling**: Message handlers should return `Vec<Command>` with error messages displayed via the status bar, never panic.
