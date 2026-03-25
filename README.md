# Task Orchestrator TUI

A terminal kanban board for managing development tasks and dispatching Claude Code agents. Create tasks, dispatch agents into isolated git worktrees and tmux windows, and monitor their progress — all from a single TUI.

## Build

```bash
cargo build --release
```

## Usage

### TUI mode (main interface)

```bash
task-orchestrator tui [--port 3142]
```

Launches the kanban board with 5 columns: Backlog, Ready, Running, Review, Done.

### CLI fallback (for agents)

```bash
task-orchestrator update <task-id> <status>
task-orchestrator list [--status <status>]
```

Agents use `update` to report task completion when the MCP server is unavailable.

## Key Bindings

| Key | Action |
|-----|--------|
| `n` | New task (title, description, repo path) |
| `d` | Dispatch agent for selected Ready task |
| `m` / `M` | Move task forward / backward |
| `h/j/k/l` | Navigate (vim-style) |
| `Enter` | Toggle detail panel |
| `x` | Delete task (with confirmation) |
| `q` | Quit |

## How Dispatch Works

When you press `d` on a Ready task:
1. Creates a git worktree in `<repo>/.worktrees/<id>-<slug>`
2. Opens a new tmux window in your current session
3. Writes `.mcp.json` so Claude discovers the MCP server
4. Launches `claude --prompt` with task description and completion instructions
5. Task moves to Running; live output shown on the kanban card

When the agent finishes, it calls `update_task` via MCP to move the task to Review. If the MCP server is down, the agent falls back to `task-orchestrator update <id> review`.

## MCP Server

Starts alongside the TUI on `localhost:3142` (configurable via `--port`). Exposes three tools:

- `update_task(task_id, status)` — move a task to a new status
- `add_note(task_id, note)` — append a note to a task
- `get_task(task_id)` — read task details

## Configuration

| Flag | Env Var | Default |
|------|---------|---------|
| `--db` | `TASK_ORCHESTRATOR_DB` | `~/.local/share/task-orchestrator/tasks.db` |
| `--port` | `TASK_ORCHESTRATOR_PORT` | `3142` |

## Architecture

- **Rust 2021** with Ratatui, Crossterm, Tokio, rusqlite, Axum, Clap
- **Elm architecture** for TUI: events produce messages, update produces commands
- **SQLite** with `Mutex<Connection>` for thread safety (sync rusqlite, may migrate to async wrapper later)
- **MCP Streamable HTTP** transport for agent communication
