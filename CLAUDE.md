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

Tasks are created exclusively via the MCP `create_task` tool — there is
no CLI for task creation. Use the `/queue-plan` slash command (or call
the MCP tool directly) to queue a plan file as a task.

Feed epics are wired to user-owned shell scripts that emit a `FeedItem` JSON
array on stdout. Reference templates live in `scripts/`:

- `scripts/fetch-dependabot.sh` — open Dependabot PRs (gh + jq). The same
  script is embedded in the binary and `dispatch setup` installs it to
  `<data_dir>/scripts/fetch-dependabot.sh` as a working example, wired to a
  seeded "Dependabot" feed epic.
- `scripts/fetch-security.sh` — open Dependabot vulnerability alerts.

Both ship with empty `REPOS` placeholders — fill them in to populate the
feed. `verify-feed` runs the given shell command (via `sh -c`) and checks
that stdout parses as a JSON array of `FeedItem` objects; use it when
writing or debugging a custom `feed_command`.

### First-time setup

Point git at the repo's hooks directory so the pre-push hook runs:

```bash
git config core.hooksPath .githooks
```

The pre-push hook runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` before each push. The first run is slow (cold cargo cache); subsequent runs are quick.

### Running Tests

Run the full suite or target a specific module:

```bash
# Full suite
cargo test

# Module-level tests
cargo test db::tests               # database CRUD and migrations
cargo test service::               # domain service layer (tasks, epics, learnings)
cargo test tui::tests              # TUI input/message handling
cargo test mcp::handlers::tests    # MCP JSON-RPC handlers

# Integration tests
cargo test --test lifecycle        # full task lifecycle (create → dispatch → done)
cargo test --test epic_lifecycle   # full epic lifecycle
cargo test --test cli              # CLI subcommand smoke tests

# Single test by name (substring match)
cargo test update_task_params_has_any_field

# Scenario tests (key-sequence integration tests)
cargo test tui::tests::scenarios

# Snapshot tests (ratatui buffer rendering tests)
cargo test tui::tests::snapshots
```

### Snapshot Tests

Snapshot tests in `src/tui/tests/snapshots.rs` render the TUI to a 120×40 `TestBackend` buffer and compare against committed `.snap` files in `src/tui/tests/snapshots/`.

**Updating snapshots intentionally** (e.g. after a deliberate UI change):

1. Run the tests and accept the new output, either interactively or in bulk:

   ```bash
   # Accept all pending new snapshots interactively
   cargo insta review

   # Or auto-accept without interactive review
   INSTA_UPDATE=always cargo test tui::tests::snapshots
   ```

2. **Clean up `.snap.new` files** — `INSTA_UPDATE=always` overwrites the `.snap` files but can leave behind `.snap.new` siblings that show up as untracked. Always run:

   ```bash
   rm src/tui/tests/snapshots/*.snap.new
   ```

3. Review the diff in the committed `.snap` files before pushing — `git diff src/tui/tests/snapshots/` should show only the changes you intended.

Keep snapshots at 120×40 so failure diffs remain readable.

> **Do not change the `TestBackend` size from 120×40.** Resizing breaks all existing snapshot diffs and makes failures unreadable.

### Where New Tests Go

| What you're testing | Where to put the test |
|---------------------|----------------------|
| TUI key handling / message flow | `src/tui/tests/` |
| DB schema, CRUD, migrations | `src/db/tests/` (split by domain: tasks, epics, prs, alerts, projects, learnings, settings, migrations) |
| Business rules in the service layer | inline in `src/service/tasks/` (mod/crud/params/validators), `src/service/epics.rs`, or `src/service/learnings.rs` |
| MCP JSON-RPC handler behaviour | `src/mcp/handlers/tests/` (split per domain) |
| Full task/epic lifecycle (end-to-end) | `tests/` (integration tests) |
| Domain-type invariants and roundtrips | inline in the owning module (`src/models/`, `src/db/mod.rs`) |

Property tests live alongside unit tests in the same module, in a nested `mod property_tests` block.

### Coverage

`cargo-tarpaulin` is configured in CI in the `coverage` job of `.github/workflows/ci.yml` (installed via `cargo install cargo-tarpaulin`, no `Cargo.toml` config — runs `cargo tarpaulin --out xml`). Run locally with:

```bash
cargo tarpaulin --out Html
```

Coverage is not added to the pre-push hook — the run is slow. Check the trend manually or review the CI artifact.

## Test-Driven Development

Always use TDD when working in this repo. Start by expressing the intended behaviour as tests — capture what the code should do before writing the code that does it. Then implement the minimum code to make the tests pass. This applies to all changes — new features, bug fixes, and refactors.

## Allium Specification

The Allium specs in `docs/specs/` are the **source of truth** for domain logic:
- `core.allium` — domain model (entities, enums, config, VisualColumn)
- `tasks.allium` — task lifecycle, agent health, hooks, notifications, split pane, MCP task tools
- `epics.allium` — epic lifecycle and MCP epic tools

Consult the relevant spec before changing core behavior. Use `allium:tend` and `allium:weed` skills to keep spec and code aligned.

## Agent Working Directory

Dispatched agents always work from their worktree folder. Every prompt includes an instruction to stay in the worktree and not `cd` to the parent repo. This is enforced in `dispatch_with_prompt()` in `src/dispatch/agents.rs`.

## Architecture

Key patterns that aren't obvious from reading the code:

- **Message → Command**: `App::update()` processes input messages and returns `Command`s (side effects). Keep rendering pure, effects in commands.
- **Inline-mutation convention**: Input handlers in `input.rs` directly mutate `self.input.mode`, cursor positions, and other UI-only state, returning `vec![]` (no commands). This is intentional — not an Elm Architecture violation. The rule: if a state change has no side effects (no DB write, no process spawn, no network call), mutate inline and return empty. If it needs a side effect, return a `Command`. If a UI handler in `src/tui/input.rs` returns `vec![]` after mutating `self.input.mode`, cursor positions, or selected indices, that is intentional — do not change it to a `Message`.
- **ProcessRunner trait**: Abstraction over git/tmux shell commands. Tests use `MockProcessRunner` — never shell out in tests.
- **TaskPatch builder**: Selective field updates for the database. `None` = don't change, `Some(None)` = set field to NULL.
- **MCP server**: Runs on port 3142 (configurable via `DISPATCH_PORT`). Agents call JSON-RPC methods in `src/mcp/handlers/` to update task status.
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

### Rendering Purity

Code under `src/tui/ui/` must be pure: it reads `App` and shared helpers, writes ratatui buffers, and does nothing else.

**Allowed:**
- Immutable reads of `App` fields and shared helpers from `src/tui/ui/shared.rs` and `src/tui/ui/palette.rs`
- Writes to the ratatui `Buffer` / `Frame` passed in by the caller
- Pure formatting (`format!`, `truncate`, span construction)

**Forbidden:**
- Database access (no `self.db`, no `Database::*`, no `rusqlite`)
- File I/O (`std::fs`, `std::io`, `tokio::fs`)
- Process spawning (`std::process::Command`, `tokio::process`)
- Async runtime calls (`tokio::*`, `block_on`, channel sends/receives)
- MCP calls or network I/O
- `unwrap()` / `expect()` / `panic!` outside `#[cfg(test)]` — render must never crash the TUI

If a render path needs data that isn't on `App`, compute it in the runtime/update layer and stash the result on `App` before rendering — do not reach for it from `src/tui/ui/`.

### Soft-fail Decoding

Schema enum values may be added in a migration before all rows are upgraded. Row decoders in `src/db/queries/` default unknown values and emit `tracing::warn!` rather than panicking — see `row_to_task` in `src/db/queries/mod.rs:20-25` for the canonical example.

Never `panic!` (or `unwrap()`/`expect()`) on an unknown enum value read from the DB. Use `Enum::parse(&s).unwrap_or_else(|| { tracing::warn!(...); Enum::Default })`. This keeps an old DB readable after a partial migration and prevents a poisoned row from killing the TUI.

**Decode-fallback counter:** every soft-fail in `row_to_task`/`row_to_epic` and `read_json_string_vec` bumps a process-wide `AtomicU64` exposed as `crate::db::decode_fallback_count()`. The counter value is included in the `tracing::warn!` message (`count=N`) so the warns are greppable in aggregate, and the accessor lets tests and ad-hoc debugging detect slow-bleeding decode bugs without chasing log lines. When you add a new soft-fail branch, bump the counter from `db::queries::bump_decode_fallback()`.

### Border Parsing

Untrusted inputs — MCP JSON-RPC arguments, editor output, feed-script JSON, plan files — must be parsed into typed domain enums **at the boundary**. Business logic should never see raw `serde_json::Value` or `String` for fields that have a typed shape.

- MCP handlers in `src/mcp/handlers/` parse to typed `*Args` structs (with serde derives) before calling into the service layer.
- Feed scripts produce `FeedItem` JSON which is parsed into the typed struct in `verify-feed` and at runtime ingest.
- Plan files are parsed by `src/plan.rs` into a typed plan structure.

Parse failures must surface to the caller as a `ServiceError::Validation` (or `-32602` at the MCP layer). Silent fallback to a default value is forbidden — if the input is invalid, the caller needs to know.

### MCP State Machines

Some MCP tools drive multi-call handshakes via in-memory state on `McpState`. The state is **not persisted** — a process restart loses it, and the agent will start the handshake from scratch on its next call.

**`exit_session` 3-phase shutdown** (`src/mcp/handlers/tasks.rs:719-803`):

| Phase | Trigger | Side effect | Response |
|-------|---------|-------------|----------|
| `AskQuestion` | First call, task not in either set | Insert `task_id` into `exit_session_pending` (`src/mcp/mod.rs:30`) | Prompts agent to reflect on learnings |
| `RecordPrompt` | Second call with `has_learnings=true`, task in `pending` | Move `task_id` from `pending` to `exit_session_reflecting` (`src/mcp/mod.rs:34`) | Prompts agent to call `record_learning` then re-call `exit_session` |
| `CloseSession` | Either: second call with `has_learnings=false`, OR third call (task in `reflecting`) | Remove from set; patch task to `Done` + clear `tmux_window`; spawn tmux window kill | Confirms close |

Both sets are `Mutex<HashSet<TaskId>>`; entries are also cleared by `state.clear_exit_session_pending(task_id)` when a task is dispatched-next or finished through other paths. A crash mid-handshake leaves no stranded DB state — the task simply hasn't transitioned to `Done` yet, and the agent will re-invoke from `AskQuestion`.

Do not add new ad-hoc state machines on `McpState` without documenting them here.

### Review/Security Agent State Machine

Review agents (dispatched for PRs) and fix agents (dispatched for security alerts) track their lifecycle via `ReviewAgentStatus` (`src/models/review.rs`):

| Status | DB value | Card badge | Meaning |
|--------|----------|------------|---------|
| `Reviewing` | `"reviewing"` | `[reviewing]` yellow | Agent session is active; agent is analyzing |
| `FindingsReady` | `"findings_ready"` | `[ready]` green (flashes) | Agent completed analysis; user should review |
| `Idle` | `"idle"` | `[idle]` dim | Agent is alive but waiting for user input |
| *(none)* | `NULL` | *(no badge)* | No agent dispatched |

State transitions:

```
dispatch (d) → Reviewing → findings_ready (agent MCP call) → idle (agent MCP call)
                                                                  ↓
                                          re-review (r) → Reviewing → ...

detach (T) or PR merge → NULL (agent_status cleared)
```

Key bindings on a PR/alert card:
- `d` — dispatch agent (blocked when `Reviewing`; allowed from `FindingsReady` and `Idle` for a fresh pass)
- `g` — jump to the active tmux session
- `r` — re-review (only when `Idle`; sends `/review-pr {number}` to the live session)
- `T` — detach: kills the tmux window and clears `tmux_window`, `worktree`, and `agent_status` atomically

The agent calls `update_review_status` (MCP tool) to advance its own status. When status becomes `findings_ready`, the runtime also upserts `pr_workflow_states` to `ActionRequired/FindingsReady` and flashes the card. See `docs/specs/review.allium` for the full specification.

### Error Handling

The codebase uses three error types at different layers:

- **`anyhow::Result`** — infrastructure and IO errors (file operations, shell commands, DB initialization). Used at the outer edges where errors propagate up to the caller.
- **`ServiceError`** (`Validation` / `NotFound` / `Internal`) — business logic errors in `src/service/mod.rs`. MCP handlers match on these to return appropriate JSON-RPC error codes.
- **Domain-specific errors** (`FinishError`, `PrError`) — operations with distinct failure modes that callers need to handle differently (e.g., rebase conflicts vs. push failures).

Rule of thumb: use `ServiceError` for request validation and business rules, domain-specific errors when callers branch on the variant, and `anyhow` for everything else.

## Tag System

Tags (`TaskTag` in `src/models/tasks.rs`: `Bug`, `Feature`, `Chore`, `PrReview`, `Research`, `Fix`) drive dispatch behavior via `DispatchMode::for_task()` (`src/models/tasks.rs`).

A task with a plan always routes to `DispatchMode::Dispatch` regardless of tag. Without a plan, the tag selects a dedicated agent: `PrReview` → `DispatchMode::PrReview`, `Research` → `Research`, `Fix` → `Fix`; everything else (including no tag, `Bug`, `Feature`, `Chore`) → `Dispatch`.

For the authoritative mapping, read `DispatchMode::for_task()` in `src/models/tasks.rs` — this table is intentionally a pointer rather than a duplicate so it cannot drift.

## Timing Constants

- **Tick interval** (2s): `TICK_INTERVAL` in `src/runtime/mod.rs` — captures tmux output, checks staleness.
- **Status TTL** (5s): `STATUS_MESSAGE_TTL` in `src/tui/mod.rs` — transient status bar messages auto-clear.
- **PR poll** (30s): `PR_POLL_INTERVAL` in `src/tui/mod.rs` — polls PR status for tasks in review.

## Module Map

| File | Responsibility |
|------|---------------|
| `src/main.rs` | CLI entry point (clap), subcommand dispatch (`tui`, `setup`, `verify-feed`, …) |
| `src/lib.rs` | Crate root, public module re-exports |
| `src/runtime/mod.rs` | Async event loop (`tokio::select!`), bridges TUI ↔ MCP ↔ shell commands |
| `src/runtime/commands.rs` | `Command` side-effect dispatcher (called by `execute_commands`) |
| `src/runtime/tasks.rs` | Per-command runtime handlers for tasks (refresh, dispatch, finish, etc.) |
| `src/runtime/{agents,epics,learnings,pr,settings,split,editor}.rs` | Domain-specific runtime helpers |
| `src/tui/mod.rs` | `App` struct, lifecycle, `update()` entry point, `column_items_for_status()` render helper |
| `src/tui/dispatcher.rs` | `dispatch(app, msg)` routing table — match arm per `Message` variant |
| `src/tui/update/` | Per-message handlers (`agent.rs`, `epics.rs`, `feeds.rs`, `forms.rs`, `learnings.rs`, `lifecycle.rs`, `main_session.rs`, `navigation.rs`, `pr.rs`, `repo_filter.rs`, `retry.rs`, `selection.rs`, `split_pane.rs`, `system.rs`, `tips_projects.rs`, `wrap_up.rs`) |
| `src/tui/input.rs` | Key event entry point, inline-mutation convention for UI-only state |
| `src/tui/input/` | Per-mode key handlers: `normal.rs`, `confirm.rs`, `projects.rs`, `repo_filter.rs` |
| `src/tui/ui/mod.rs` | Rendering entry point — re-exports `render()`, thin dispatcher |
| `src/tui/ui/kanban/mod.rs` | Kanban board: `render()` entry, summary/status bar, action hints, color helpers |
| `src/tui/ui/kanban/cards.rs` | Task and epic card rendering (indicator classification, card list items) |
| `src/tui/ui/kanban/columns.rs` | Column layout — `render_columns`, per-column rendering, archive column |
| `src/tui/ui/kanban/popups.rs` | Overlay rendering (error, tips, help, repo filter, task detail) |
| `src/tui/ui/kanban/projects_panel.rs` | Left-side project filter panel |
| `src/tui/ui/kanban/tests.rs` | Kanban rendering unit tests |
| `src/tui/ui/shared.rs` | Cross-board helpers: `render_tab_bar`, `refresh_status`, `truncate`, `push_hint_spans` |
| `src/tui/ui/palette.rs` | Tokyo Night color palette constants |
| `src/tui/ui/{input_form,learnings}.rs` | Overlay renderers (input forms, knowledge base panel) |
| `src/tui/types.rs` | `Message`, `Command`, `ViewMode`, `InputMode`, `AgentTracking` enums and structs |
| `src/tui/tests/` | TUI unit and scenario tests, snapshots, helpers |
| `src/models/mod.rs` | Re-exports of domain types and shared model tests |
| `src/models/tasks.rs` | `Task`, `TaskStatus`, `SubStatus`, `TaskTag`, `DispatchMode::for_task()` tag routing |
| `src/models/{epics,learnings,projects,review}.rs` | Domain types per area |
| `src/service/mod.rs` | Service module root: `ServiceError`, `FieldUpdate`, re-exports of all sub-module types |
| `src/service/tasks/mod.rs` | `TaskService` — task business logic |
| `src/service/tasks/{crud,params,validators}.rs` | Task CRUD methods, `*Params` request types, validation helpers |
| `src/service/epics.rs` | `EpicService`, `UpdateEpicParams`, `CreateEpicParams` — epic business logic |
| `src/service/learnings.rs` | `LearningService`, `CreateLearningParams`, `UpdateLearningParams` — learning business logic |
| `src/db/mod.rs` | `Database` struct, constructor, `TaskStore` trait, `TaskPatch`/`EpicPatch` builders |
| `src/db/migrations.rs` | Versioned schema migrations (`MIGRATIONS` array, `migrate_vN_*` functions) |
| `src/db/queries/mod.rs` | `impl TaskStore for Database` — fans out across the per-domain query files |
| `src/db/queries/{tasks,epics,prs,alerts,projects,learnings,settings}.rs` | CRUD per domain |
| `src/db/tests/mod.rs` | Database unit tests entry point |
| `src/db/tests/{tasks,epics,prs,alerts,projects,learnings,settings,migrations}.rs` | Tests per domain |
| `src/dispatch/mod.rs` | Worktree creation, tmux session management, agent lifecycle (dispatch/brainstorm/plan/resume/review) |
| `src/dispatch/agents.rs` | Agent-specific dispatch helpers |
| `src/dispatch/prompts.rs` | Prompt construction (with-plan, no-plan variants, learning injection) |
| `src/dispatch/worktree.rs` | Worktree creation/teardown |
| `src/dispatch/finish.rs` | Rebase + fast-forward branch onto base branch, kill tmux window (`finish_task`); defines `FinishError` |
| `src/process.rs` | `ProcessRunner` trait + `RealProcessRunner` / `MockProcessRunner` for testable shell execution |
| `src/tmux.rs` | Tmux API: create windows, send keys, capture pane output, kill windows |
| `src/editor.rs` | External `$EDITOR` integration for editing task/epic fields |
| `src/plan.rs` | Plan file parsing (extract title/description from markdown) |
| `src/setup/mod.rs` | First-run setup entry point |
| `src/setup/{config,plugins,hooks}.rs` | MCP config merging, plugin installation, git hook installation |
| `src/mcp/mod.rs` | MCP server bootstrap (Axum router), `McpState`, `McpEvent` notification enum |
| `src/mcp/handlers/dispatch.rs` | JSON-RPC entry point (`handle_mcp`), tool definitions, method routing |
| `src/mcp/handlers/tasks.rs` | Task tool handlers (thin wrappers): parse JSON-RPC args → call `TaskService` → format response |
| `src/mcp/handlers/epics.rs` | Epic tool handlers (thin wrappers): parse JSON-RPC args → call `EpicService` → format response |
| `src/mcp/handlers/learnings.rs` | Knowledge base tool handlers |
| `src/mcp/handlers/review.rs` | Review/security agent status handlers |
| `src/mcp/handlers/types.rs` | JSON-RPC request/response types, flexible integer deserializer |
| `src/mcp/handlers/tests/mod.rs` | MCP handler integration tests entry point |
| `src/mcp/handlers/tests/{tasks,epics,learnings,projects,review}.rs` | MCP handler tests per domain |

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
- `TuiRuntime::exec_refresh_from_db()` (`src/runtime/tasks.rs`) — reloads tasks, epics, and usage from DB
- `Message::RefreshTasks` (`src/tui/types.rs`) — carries the fresh task list into the App

The `MessageSent` variant additionally triggers `Message::MessageReceived(task_id)`, which flashes the target task's card in the TUI.

### MCP Error Codes

MCP handlers in `src/mcp/handlers/` return JSON-RPC error objects using two codes:

| Code | Meaning | When to use |
|------|---------|-------------|
| `-32602` | Invalid params | Validation failure, missing required field, unknown tool name — maps to `ServiceError::Validation` |
| `-32603` | Internal error | Unexpected DB error, I/O failure — maps to `ServiceError::Internal` or `anyhow` errors |

Use `JsonRpcResponse::err(id, -32602, msg)` for anything the caller can fix; use `-32603` for anything they can't.

### Debugging MCP handlers

The MCP server listens on port 3142 by default (override with `DISPATCH_PORT`). When a handler misbehaves you can reproduce it without going through Claude Code:

```bash
# Tail server logs while the TUI runs (logs go to stderr; redirect when launching)
RUST_LOG=dispatch=debug cargo run -- tui 2> /tmp/dispatch.log
tail -f /tmp/dispatch.log

# Send a manual JSON-RPC request to a tool (e.g. list_tasks)
curl -s -X POST http://127.0.0.1:3142 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}}' \
  | jq

# Reproduce a failing update — substitute the offending arguments
curl -s -X POST http://127.0.0.1:3142 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_task","arguments":{"task_id":42,"status":"done"}}}' \
  | jq
```

`tools/list` returns the tool schemas — useful when the argument shape isn't obvious.

## Feed Epics

Feed epics are epics whose tasks are populated externally by a shell command rather than by a human. When an epic has a `feed_command` set, the runtime runs it periodically (`feed_interval_secs`) and calls `upsert_feed_tasks()` to sync the results. Each feed task has an `external_id` that is used as the upsert key — tasks are created on first appearance and updated (but not deleted) on subsequent runs.

Feed tasks appear in their own column on the kanban board (`SubStatus::Feed`). The schema is backed by migration v38. See `docs/specs/feeds.allium` for the full specification.

## Knowledge Base Flow

The Knowledge Base lets dispatched agents record knowledge entries that are automatically injected into future dispatch prompts.

### End-to-end lifecycle

1. **Agent records** — calls `record_learning(task_id, kind, summary, scope, ...)` during a task or at wrap-up. The entry is immediately active and will appear in future dispatch prompts for agents working in the matching scope.
2. **Human manages** — opens the Knowledge Base overlay (`L` key from the main board) and can reject, archive, or edit entries. Only approved entries stay in the active pool.
3. **Future dispatches** — when an agent is launched, `dispatch_with_prompt()` queries approved entries for the task's context and prepends them to the prompt (see `docs/specs/learnings.allium`).
4. **Agent upvotes** — calls `upvote_learning(learning_id, task_id)` when a retrieved entry proves correct. This increments `confirmed_count`, which raises the entry's priority in future results.

### Scope model

Each learning has a `scope` that determines which tasks receive it:

| Scope | Included when | `scope_ref` |
|-------|---------------|-------------|
| `user` | Always | `null` |
| `project` | Task belongs to this project | `str(project_id)` |
| `repo` | Task's repo path matches | `repo_path` |
| `epic` | Task belongs to this epic | `str(epic_id)` |
| `task` | Only via explicit `query_learnings` | `str(task_id)` |

`scope_ref` is auto-derived from the task context when omitted. `task`-scoped entries are excluded from auto-injection (they capture task-specific outcomes and must be fetched on demand).

### Prompt priority order

Within an injected prompt, learnings are ordered (highest first):

1. `procedural` — prepended as verbatim prompt-prefix instructions before the normal learnings block
2. `epic` — most specific to the current work
3. `repo` — repository-wide conventions
4. `project` — project-wide preferences
5. `user` — global preferences

Within each level, entries are sorted by `confirmed_count DESC`.

### Status lifecycle

```
approved → archived (terminal)
         ↘ rejected (terminal)
```

Approved entries affect dispatch. Rejected and archived entries do not.

### Key bindings in the Knowledge Base overlay

| Key | Action |
|-----|--------|
| `L` | Open overlay |
| `j` / `k` | Navigate list |
| `a` | Approve selected |
| `x` | Reject selected |
| `A` | Archive selected (approved only) |
| `e` | Edit (opens `$EDITOR`) |
| `Esc` / `q` | Close |

### Implementation references

- `src/mcp/handlers/learnings.rs` — MCP tool handlers
- `src/service/learnings.rs` — `LearningService` (approval, rejection, archive, edit)
- `src/db/` — `LearningStore` trait, `LearningPatch`, `LearningFilter`
- `src/dispatch/agents.rs` — prompt augmentation in `dispatch_with_prompt()`
- `docs/specs/learnings.allium` — full domain specification

## Visibility Convention

`App` fields use `pub(in crate::tui)` to restrict mutation to the TUI module. **After bootstrap**, external code (runtime, MCP handlers) can only change `App` state by sending a `Message` through `app.update()`, which returns `Command`s. This keeps state transitions auditable in one place and prevents scattered mutation from outside the TUI boundary.

**Bootstrap-only setters** (called from `runtime::run_tui` and helpers in `src/runtime/mod.rs` while loading persisted settings, before the event loop starts):

- `App::set_notifications_enabled` — invoked from `load_notifications_pref` (`src/runtime/mod.rs:440`)
- `App::set_main_session` / `App::set_main_session_dir` — invoked from `load_main_session` (`src/runtime/mod.rs:422`, `src/runtime/mod.rs:427`)
- `App::set_per_project_filter` — invoked from `load_per_project_repo_filters` (`src/runtime/mod.rs:487`, `src/runtime/mod.rs:514`)

`App::set_main_session(None)` is also called mid-runtime from `src/runtime/split.rs:34` to clear a stale main-session reference when the tmux window has gone away — this is a deliberate carve-out because the runtime, not the TUI, owns the tmux lifecycle.

Do not add new direct-mutation setters. Bootstrap loaders are the only legitimate users; everything else must go through a `Message`.

## Quick Dispatch

`Shift+D` creates and immediately dispatches a task without going through the task creation dialog. The flow:

1. Key handler emits `Command::QuickDispatch { draft: TaskDraft { title: DEFAULT_QUICK_TASK_TITLE, repo_path, .. }, epic_id }` (`src/tui/mod.rs`)
2. Runtime handles it in `exec_quick_dispatch()` (`src/runtime/tasks.rs`) — calls `create_task()` then immediately dispatches
3. The created task gets title `"Quick task"` (`DEFAULT_QUICK_TASK_TITLE` in `src/models/tasks.rs`), no tag, no plan
4. If the board has multiple repo paths, `Shift+D` first enters `InputMode::QuickDispatchRepo` (repo picker), then emits `Message::SelectQuickDispatchRepo(idx)` to resolve the repo before dispatching

Quick dispatch is the same code path as normal dispatch — the difference is the task is created with defaults and skips the creation dialog entirely.

**Command-level shortcut:** `Command::QuickDispatch` bypasses the normal `Command::DispatchAgent` → `Message::Dispatched` round-trip. `exec_quick_dispatch()` calls `create_task()` and immediately dispatches in a single step — there is no intermediate message back through `app.update()`.

## Code Conventions

### `FieldUpdate` — nullable string fields

`FieldUpdate` (`src/service/mod.rs`) replaces the `Option<String>` + empty-string sentinel anti-pattern for fields that need three states: "don't touch", "set to value", "clear to NULL":

```rust
pub enum FieldUpdate {
    Set(String),  // set the field to this value
    Clear,        // set the field to NULL
}
```

Used in `UpdateTaskParams` for `pr_url`, `worktree`, and `tmux_window`. When adding a new nullable field to `UpdateTaskParams`, use `Option<FieldUpdate>` rather than `Option<Option<String>>`.

**When to use:** if the caller can clear the field to NULL (nullable column, user-clearable), use `FieldUpdate`. If the field is non-nullable or the update path only ever sets a value, use a plain `String` (or `Option<String>` to mean "don't touch / set"). Reserve the three-state pattern for genuinely tri-valued updates.

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

### Sub-status validation TOCTOU

`TaskService::update_task()` (`src/service/tasks/crud.rs`) reads the existing task to validate the requested sub-status before applying the patch. This is a TOCTOU window: a concurrent MCP call could change the task status between the read and the write. This is intentional and accepted — simultaneous status changes from two agents on the same task are considered a user error, and the window is too small to be worth a transaction-level fix.

### Immutable `parent_epic_id`

`EpicPatch` intentionally omits `parent_epic_id`. Reparenting an epic is not supported: the parent is set at creation time and never changed. This keeps the parent chain immutable and prevents accidental cycle introduction. The database enforces `CHECK (parent_epic_id != id)` (migration v35) as a final guard. See the doc comment at `src/db/mod.rs` (`EpicPatch` definition) for the full rationale.

### Clippy lint rules

Custom lint rules are configured in `[lints.clippy]` in `Cargo.toml`. The pre-push hook enforces them via `cargo clippy --all-targets --fix -- -D warnings`. When you discover a pattern worth enforcing, add a new entry with a structured comment explaining why. Consult the `/lint` skill for the full workflow.

## How-To Guides

### Adding a New MCP Tool

1. **Define the handler** in `src/mcp/handlers/tasks.rs` (or `epics.rs` for epic tools). Follow the pattern: parse args with `types::parse_args`, call `state.db` methods, call `state.notify()` if mutating, return `JsonRpcResponse::ok`.
2. **Add the tool schema** to `tool_definitions()` in `src/mcp/handlers/dispatch.rs` — add a new entry to the `tools` array with `name`, `description`, and `inputSchema`.
3. **Wire the route** in `handle_mcp()` in `src/mcp/handlers/dispatch.rs` — add a match arm in the `tools/call` section mapping the tool name to your handler.
4. **Add types** if needed in `src/mcp/handlers/types.rs` (argument structs with serde derives, use `#[serde(deserialize_with = "deserialize_flexible_i64")]` for integer fields since Claude Code may send them as strings).
5. **Write tests** in `src/mcp/handlers/tests/` (the file matching the tool's domain) using `Database::open_in_memory()`.

### Adding a New TUI View/Mode

1. **Add a `ViewMode` variant** in `src/tui/types.rs` (e.g., `ViewMode::MyNewView { selection, saved_board }`).
2. **Add `Message` variants** for entering/exiting and any view-specific actions.
3. **Add `Command` variants** if the view triggers side effects (DB writes, shell commands).
4. **Handle input** in `src/tui/input.rs` — add key handlers under a new match arm for your `ViewMode`.
5. **Handle messages** in `src/tui/mod.rs` `update()` — process your new messages, return commands.
6. **Render** in the appropriate `src/tui/ui/` module (`kanban.rs`, `review.rs`, or `security.rs`) — add a rendering branch for your view mode in `kanban.rs::render()`.

### Adding a New Entity (with patch builder and sub-trait)

Adding a fully integrated entity involves five layers. Work through them in order:

1. **Domain model** (`src/models/`) — define the struct and any enums in the appropriate domain file. For nullable fields that agents or the TUI can set/clear, plan to use `FieldUpdate` (service layer) and `Option<Option<T>>` double-Option (DB layer); see the [FieldUpdate](#fieldupdate--nullable-string-fields) and [TaskPatch/EpicPatch](#taskpatch--epicpatch--double-option-in-the-db-layer) conventions.

2. **Database migration** (`src/db/migrations.rs`) — write `migrate_vN_description(conn)` and register it in `MIGRATIONS`. See [Adding a Database Migration](#adding-a-database-migration) for the full procedure.

3. **DB trait and queries** (`src/db/mod.rs`, `src/db/queries/`):
   - Define a narrow sub-trait (e.g., `trait NewEntityCrud`) with CRUD methods. Follow the [trait-narrowing convention](#db-trait-narrowing--take-the-narrowest-sub-trait-you-need).
   - Add `NewEntityCrud` as a supertrait of `TaskStore` so existing holders (`McpState`, `TuiRuntime`) get it automatically.
   - Implement `impl NewEntityCrud for Database` under `src/db/queries/` (a new file per domain, wired into `src/db/queries/mod.rs`) using `self.conn()?`.
   - Define a `NewEntityPatch` builder struct with `Option<Option<T>>` for nullable fields; implement the `UPDATE` query.
   - Write a corresponding `NewEntityFilter` if list queries need filtering.

4. **Service layer** (`src/service/<entity>.rs`) — create `NewEntityService` holding `Arc<dyn NewEntityCrud>`. Add `create_`, `get_`, `list_`, `update_`, and any lifecycle methods. Use `ServiceError::Validation` for input errors, `ServiceError::NotFound` for missing rows, and `anyhow` for DB I/O errors. Accept `FieldUpdate` for nullable string fields, map to `Option<Option<T>>` before writing the patch. Declare the new module in `src/service/mod.rs` and add `pub use` re-exports so callers are unaffected.

5. **MCP handler** (if agents need to interact) — follow [Adding a New MCP Tool](#adding-a-new-mcp-tool). For read-only tools, hold the narrowest sub-trait; for mutating tools, call `state.notify()` after the write.

6. **Tests**:
   - DB-layer tests in `src/db/tests/` (the file matching the entity's domain) using `Database::open_in_memory()`.
   - Service-layer tests inline in the corresponding `src/service/<entity>.rs` file.
   - MCP handler tests in `src/mcp/handlers/tests/` (the file matching the tool's domain) for any new tools.

7. **Spec** (`docs/specs/`) — write or extend an Allium spec to document the entity's lifecycle, rules, and invariants. Use the `allium:tend` skill and run `allium check` to validate syntax.

### Adding a Database Migration

Migrations live in `src/db/migrations.rs` as standalone functions. We do **not** squash migrations — see the module-level doc comment in `src/db/migrations.rs` for the policy.

1. **Write the migration function**: `fn migrate_vN_description(conn: &Connection) -> Result<()>` in `src/db/migrations.rs`. Use `ALTER TABLE` for additive changes; for destructive changes (column removal, constraint changes), create a new table, copy data, drop old, rename.
2. **Register it** in the `MIGRATIONS` array in `src/db/migrations.rs`: add `(N, migrate_vN_description)`. The loop in `Database::init_schema()` applies any migration where `current_version < N` and bumps `PRAGMA user_version` after each.
3. **Update the schema test**: `fresh_db_has_latest_schema_version` in `src/db/tests/migrations.rs` asserts the final version number — bump it to match your new N.
4. **Write a migration test** in `src/db/tests/migrations.rs` that creates a DB at the pre-migration schema, inserts test data, runs the migration, and verifies the result.

### Projects Feature

Projects group tasks and epics for board filtering. See `docs/specs/projects.allium` for the full domain specification.

**Filter semantics:**
- `App.active_project: ProjectId` is the active board filter.
- **Default project active** → show all tasks/epics regardless of `project_id` (catch-all view).
- **Any other project active** → show only items where `item.project_id == active_project.id`.
- The filter is applied in `project_matches()` at four call sites in `tui/mod.rs`: task column rendering, epic column rendering, archive view, and search results.

**Default project pinning:**
- The Default project is seeded at DB init (migration v39, `is_default = 1`). There is exactly one default.
- The Default project cannot be deleted. `delete_project_and_move_items` checks `is_default` before proceeding.
- Deleting any other project moves all its tasks and epics to Default in a single DB transaction, preventing orphaned items.
- Users can rename the Default project but cannot change `is_default`.

**Why TUI-only admin state:**
- Projects are never mutated by MCP agents — there are no MCP tools for project management. Only humans create, rename, reorder, and delete projects from the TUI panel.
- The project list is refreshed only after explicit project-mutating commands (`CreateProject`, `RenameProject`, `DeleteProject`, `ReorderProject`), not on every MCP tick.

**Panel behavior:**
- The projects panel is a left-side overlay opened with `h` (or `Left`) from column 0 (Backlog). While visible it intercepts all input before normal board key handling.
- Moving the cursor with `j`/`k` immediately activates the hovered project (hover-to-filter). `Enter`, `g`, `l`, `Right`, and `Esc` close the panel, keeping the currently activated project.
- The panel cursor resets to the active project on each open.

**Delete confirmation:**
- Deleting a project is a two-step confirmation: first `D` opens `ConfirmDeleteProject1`; after confirming, `ConfirmDeleteProject2` shows the count of tasks/epics that will be moved to Default. The user types `y` or presses Enter to proceed.

**Implementation details:**
- `ProjectId = i64` (type alias, not newtype) — simpler rusqlite integration. No FK constraint in the schema; integrity is enforced at the service/runtime layer.
- `exec_refresh_projects_from_db` follows the `exec_refresh_*_from_db` naming pattern (see `src/runtime/tasks.rs`).

### Knowledge Base MCP Tools

Three MCP tools manage the knowledge base from within an agent session:

- **`record_learning`** — record a new entry in the knowledge base (immediately active in future dispatch prompts)
- **`query_learnings`** — retrieve approved entries relevant to the current task's context; supports `tag_filter` and `limit`
- **`upvote_learning`** — increment `confirmed_count` on an entry that proved useful

**When to call these tools:**
- Call `query_learnings` via the action-specific skills (`/codebase-knowledge`, `/code-conventions`, `/pr-workflow`, etc.) at the right moment — not just at task start.
- Call `record_learning` when you discover a pattern worth capturing for future agents (pitfall, convention, landscape, etc.).
- Call `upvote_learning` when a retrieved entry turns out to be correct and useful.

**Scope auto-derivation:** omit `scope_ref` — the MCP handler derives it from the task's project, repo, or epic automatically. Pass `scope_ref` explicitly only to override.

**Task-scoped learnings** are not auto-injected into dispatch prompts. Use `query_learnings` with `tag_filter` to retrieve them when needed.

**Scopes at retrieval time**: a `query_learnings` call for a task returns the union of all approved learnings where:
- `scope = user` (always included)
- `scope = repo` and `scope_ref` matches the task's repo path
- `scope = project` and `scope_ref` matches the task's project
- `scope = epic` and `scope_ref` matches the task's epic (only if the task belongs to an epic)

See `docs/reference.md` → *Learning Store* for the full scoping model with examples.

## Documentation

- `docs/reference.md` — Key bindings, configuration, environment variables, troubleshooting, learning store
- `docs/specs/` — Allium specifications for domain logic
- `docs/plans/` — Implementation plans (working artifacts, never committed)
