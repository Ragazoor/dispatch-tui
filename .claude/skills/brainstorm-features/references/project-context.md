# Task Orchestrator TUI — Project Context

This reference provides pointers to project files and stable domain context for brainstorming features.

## Files to Read for Current State

These files contain the latest project state. Read them before generating ideas — do not rely on this reference alone for anything that may have changed.

- **`CLAUDE.md`** — Architecture overview, key files, conventions, module structure
- **`TODOS.md`** — Known improvement areas and future phase ideas
- **`Cargo.toml`** — Dependencies and crate versions
- **Recent git log** (`git log --oneline -20`) — Current development momentum

## Source Files by Category

### UX (Interaction & Navigation)
- `src/tui/input.rs` — Keyboard event handling, keybindings per input mode
- `src/tui/mod.rs` — App state machine, input modes, message handlers
- `src/tui/types.rs` — Message, Command, InputMode, TaskDraft enums
- `src/editor.rs` — External editor integration for task editing

### DevX (Developer Workflow)
- `src/dispatch.rs` — Agent dispatch: worktree creation, tmux window, prompt construction
- `src/mcp/handlers.rs` — MCP tool implementations (update_task, add_note, get_task, create_task)
- `src/mcp/mod.rs` — Axum MCP server setup
- `src/runtime.rs` — TUI main loop, command execution, startup/shutdown
- `src/plan.rs` — Plan file metadata parsing for plan-to-task pipeline

### UI (Visual & Layout)
- `src/tui/ui.rs` — Ratatui rendering: columns, detail panel, status bar, task cards
- `src/models.rs` — Task, TaskStatus, Note structs, column definitions

## Stable Domain Context

This context changes rarely and is safe to reference directly.

### Kanban Columns
Backlog → Ready → Running → Review → Done

- **Backlog** — Ideas, captured tasks for later
- **Ready** — Defined enough to dispatch (has title, description, repo path)
- **Running** — Agent actively working in a tmux window (live output shown)
- **Review** — Agent reported completion, awaiting human verification
- **Done** — Human confirmed the work

### Architecture Pattern
Elm Architecture — terminal events produce Messages, `App::update()` returns `Vec<Command>`, commands are executed by the main loop. Side effects (SQLite writes, tmux calls, agent dispatch) happen only in command execution.

### Agent Dispatch Model
Press `d` on a Ready task → creates git worktree (`.worktrees/<id>-<slug>`) → opens tmux window → launches `claude --prompt "<task description>"` in interactive mode. Agent can report status via MCP tools.

### MCP Server
Runs on `localhost:3142` alongside the TUI. Tools: `update_task`, `add_note`, `get_task`, `create_task`.

### Task Creation
Tasks can be created via:
1. TUI inline form (`n` key)
2. CLI: `task-orchestrator create "<title>" "<description>" --repo-path <path>`
3. CLI from plan: `task-orchestrator create --from-plan <path> --repo-path <path>`
4. MCP: `create_task` tool with title, description, repo_path fields
