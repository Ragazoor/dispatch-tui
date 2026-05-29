# Architecture

Key patterns that aren't obvious from reading the code:

- **Message → Command**: `App::update()` processes input messages and returns `Command`s (side effects). Keep rendering pure, effects in commands.
- **Inline-mutation convention**: Input handlers in `input.rs` directly mutate `self.input.mode`, cursor positions, and other UI-only state, returning `vec![]` (no commands). This is intentional — not an Elm Architecture violation. The rule: if a state change has no side effects (no DB write, no process spawn, no network call), mutate inline and return empty. If it needs a side effect, return a `Command`. If a UI handler in `src/tui/input.rs` returns `vec![]` after mutating `self.input.mode`, cursor positions, or selected indices, that is intentional — do not change it to a `Message`.
- **ProcessRunner trait**: Abstraction over git/tmux shell commands. Tests use `MockProcessRunner` — never shell out in tests.
- **TaskPatch builder**: Selective field updates for the database. `None` = don't change, `Some(None)` = set field to NULL.
- **MCP server**: Runs on port 3142 (configurable via `DISPATCH_PORT`). Agents call JSON-RPC methods in `src/mcp/handlers/` to update task status. Caller identity is established via `X-Caller-Task-Id` / `X-Caller-Kind` HTTP headers, parsed by the `extract_caller_identity` middleware (`src/mcp/middleware.rs`) and attached to the request as `Result<CallerIdentity, IdentityError>` — every handler that requires authorization extracts this extension rather than accepting an argument.
- **Integration tests**: Use `Database::open_in_memory()` with a real SQLite instance — no mocking the database layer.
- **Command queue draining**: `execute_commands` (`src/runtime/mod.rs`) loads the initial `Vec<Command>` into a `VecDeque` and drains it iteratively. Any handler that produces additional commands (e.g. error-path `app.update()` calls) extends the queue with `queue.extend(extra)`, so a single message can cascade into multiple commands without recursive calls:

  ```rust
  let mut queue = std::collections::VecDeque::from(cmds);
  while let Some(command) = queue.pop_front() {
      let extra = commands::dispatch(command, app, rt);
      queue.extend(extra);
  }
  ```
- **Editor session invariant**: `TuiRuntime` holds an `editor_session: Arc<Mutex<Option<EditorSession>>>` (`src/runtime/mod.rs`). At most one pop-out editor can be open at a time — the runtime refuses to start a new one while the slot is occupied. The slot is `None` when idle.

## Review/Security Agent State Machine

Review agents (dispatched for PRs) and fix agents (dispatched for security alerts) track their lifecycle via `ReviewAgentStatus` (`src/models/review.rs`):

| Status | DB value | Card badge | Meaning |
|--------|----------|------------|---------|
| `Reviewing` | `"reviewing"` | `[reviewing]` yellow | Agent session is active; agent is analyzing |
| *(none)* | `NULL` | *(no badge)* | No agent dispatched |

`FindingsReady` and `Idle` variants still exist on the enum so legacy DB rows parse, but no current code path writes them.

State transitions:

```
dispatch (d) → Reviewing → detach (T) or PR merge → NULL (agent_status cleared)
```

Key bindings on a PR/alert card:
- `d` — dispatch agent (blocked when `Reviewing`)
- `g` — jump to the active tmux session
- `T` — detach: kills the tmux window and clears `tmux_window`, `worktree`, and `agent_status` atomically

`set_pr_agent` / `set_alert_agent` write `ReviewAgentStatus = Reviewing` atomically with `tmux_window` and `worktree`; detach and PR-merge detection clear all three. See `docs/specs/review.allium` for the full specification.

## Error Handling

The codebase uses three error types at different layers:

- **`anyhow::Result`** — infrastructure and IO errors (file operations, shell commands, DB initialization). Used at the outer edges where errors propagate up to the caller.
- **`ServiceError`** (`Validation` / `NotFound` / `Internal`) — business logic errors in `src/service/mod.rs`. MCP handlers match on these to return appropriate JSON-RPC error codes.
- **Domain-specific errors** (`FinishError`, `PrError`) — operations with distinct failure modes that callers need to handle differently (e.g., rebase conflicts vs. push failures).

Rule of thumb: use `ServiceError` for request validation and business rules, domain-specific errors when callers branch on the variant, and `anyhow` for everything else.

## Quick Dispatch

`Shift+D` creates and immediately dispatches a task without going through the task creation dialog. The flow:

1. Key handler emits `Command::QuickDispatch { draft: TaskDraft { title: DEFAULT_QUICK_TASK_TITLE, repo_path, .. }, epic_id }` (`src/tui/mod.rs`)
2. Runtime handles it in `exec_quick_dispatch()` (`src/runtime/tasks.rs`) — calls `create_task()` then immediately dispatches
3. The created task gets title `"Quick task"` (`DEFAULT_QUICK_TASK_TITLE` in `src/models/tasks.rs`), no tag, no plan
4. If the board has multiple repo paths, `Shift+D` first enters `InputMode::QuickDispatchRepo` (repo picker), then emits `Message::SelectQuickDispatchRepo(idx)` to resolve the repo before dispatching

Quick dispatch is the same code path as normal dispatch — the difference is the task is created with defaults and skips the creation dialog entirely.

**Command-level shortcut:** `Command::QuickDispatch` bypasses the normal `Command::DispatchAgent` → `Message::Dispatched` round-trip. `exec_quick_dispatch()` calls `create_task()` and immediately dispatches in a single step — there is no intermediate message back through `app.update()`.
