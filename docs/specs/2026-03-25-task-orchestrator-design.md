# Design: Task Orchestrator TUI

Status: APPROVED
Date: 2026-03-25

## Problem Statement

A developer working across multiple repositories needs a way to create development tasks and dispatch Claude Code agents to work on them autonomously. Today this means manually creating worktrees, opening terminal sessions, launching Claude, and tracking what state everything is in. There's no unified view and no way for agents to report back when they're done.

## Solution

A Rust TUI application that provides a kanban board for task management and dispatches Claude Code agents into tmux windows with MCP-based status reporting. Single binary, three modes: TUI (kanban + MCP server), CLI update (agent fallback), CLI list (debugging).

## Architecture

### Single binary, three modes

```
task-orchestrator tui       # launch kanban TUI + MCP server
task-orchestrator update    # CLI: update task status (agent fallback)
task-orchestrator list      # CLI: list tasks (debugging/scripting)
```

### Process architecture

```
+---------------------------------------------+
|              task-orchestrator tui           |
|                                             |
|  +--------------+    +-------------------+  |
|  |  Ratatui TUI |    |  MCP Server       |  |
|  |  (kanban UI) |    |  (HTTP/SSE :3142) |  |
|  |              |    |                   |  |
|  |  reads ------+----+-- writes ---+     |  |
|  +--------------+    +-------------+-----+  |
|                                    |        |
|         +--------------------------+        |
|         v                                   |
|  +-------------+                            |
|  |   SQLite    |<-- task-orchestrator update|
|  |  tasks.db   |    (CLI fallback)          |
|  +-------------+                            |
+---------------------------------------------+
         | dispatch
         v
+-------------------------+
|  tmux window (same session) |
|  +-----------------------+  |
|  | worktree: .worktrees/ |  |
|  | claude --mcp ...      |  |
|  +-----------------------+  |
+-----------------------------+
```

### Tokio runtime layout

- **Main task**: Ratatui event loop (crossterm key events + periodic tick for refresh)
- **Spawned task**: Axum HTTP server for MCP on localhost
- **Sync SQLite**: All SQLite reads/writes via `rusqlite` with `spawn_blocking` (sync for now, may migrate to async wrapper later)

## Data Model

### SQLite schema

```sql
CREATE TABLE tasks (
    id          INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    description TEXT NOT NULL,
    repo_path   TEXT NOT NULL,  -- absolute path to git repository root
    status      TEXT NOT NULL DEFAULT 'backlog',  -- backlog|ready|running|review|done
    worktree    TEXT,          -- absolute path to created worktree (set on dispatch)
    tmux_window TEXT,          -- tmux window identifier (set on dispatch)
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE notes (
    id         INTEGER PRIMARY KEY,
    task_id    INTEGER NOT NULL REFERENCES tasks(id),
    content    TEXT NOT NULL,
    source     TEXT NOT NULL DEFAULT 'user',  -- user|agent|system
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Status flow

```
Backlog --> Ready --> Running --> Review --> Done
```

- **Backlog**: ideas, captured tasks for later
- **Ready**: defined enough to dispatch an agent
- **Running**: agent is actively working (auto-set on dispatch)
- **Review**: agent reported completion (via MCP or CLI), awaiting human verification
- **Done**: human confirmed the work

## TUI Layout

```
+-- Backlog ------+-- Ready --------+-- Running -------+-- Review --------+-- Done -----------+
|                 |                 |                  |                  |                   |
| +-------------+ | +-------------+ | +--------------+ | +--------------+ | +---------------+ |
| | refactor    | | | add cache   | | | migrate DB   | | | fix auth     | | | update deps   | |
| | auth module | | | layer       | | | > running 5m | | | agent done   | | | completed     | |
| |             | | | repo: svc   | | | > Adding...  | | | > All tests  | | |               | |
| +-------------+ | +-------------+ | +--------------+ | |   pass       | | +---------------+ |
|                 |                 |                  | +--------------+ |                   |
+-----------------+-----------------+------------------+------------------+-------------------+
| Task Detail: migrate DB                                                                    |
| Repo: ~/Code/work/scala/user-service                                                      |
| Description: Migrate user table from UUID to ULID primary keys...                          |
| Worktree: .worktrees/migrate-db | Window: #3                                              |
| +-- Agent Output (last 5 lines) --------------------------------------------------------+ |
| | Analyzing src/main/scala/UserRepository.scala...                                       | |
| | Adding migration file 003_ulid_migration.sql                                           | |
| | Running tests...                                                                       | |
| +----------------------------------------------------------------------------------------+ |
+--------------------------------------------------------------------------------------------+
| [n]ew  [d]ispatch  [Enter] detail  [<>] column  [^v] task  [m]ove  [q]uit                 |
+--------------------------------------------------------------------------------------------+
```

### Key bindings

| Key | Action |
|-----|--------|
| `n` | Create new task (inline form: title, description, repo path) |
| `d` | Dispatch agent for selected task (moves Ready -> Running) |
| `Enter` | Toggle detail panel for selected task |
| `left/right` | Move focus between columns |
| `up/down` | Move focus between tasks in a column |
| `m` | Move selected task to next status column |
| `M` | Move selected task to previous status column |
| `q` | Quit |

### Card content by status

- **Backlog/Ready**: title, repo name
- **Running**: title, duration since dispatch, last line of tmux output
- **Review**: title, last line of agent output (frozen at completion)
- **Done**: title, checkmark

### Refresh cycle

Every 2 seconds:
1. Read task list from SQLite (picks up MCP/CLI updates)
2. For Running tasks: capture last 5 lines from tmux pane via `tmux capture-pane -t <window> -p -S -5`
3. Re-render the board

## Agent Dispatch

### Dispatch flow (pressing `d` on a Ready task)

Only tasks in `ready` status can be dispatched. The `d` key is a no-op for other statuses.

1. Create worktree: `git -C <repo_path> worktree add <repo_path>/.worktrees/<task-id>-<slugified-title>`
2. Create tmux window in current session: `tmux new-window -n task-<task-id> -c <worktree_path>`
3. Write `.mcp.json` into the worktree root (Claude Code discovers MCP servers from the working directory's `.mcp.json`)
4. Send command to the tmux window: `claude --prompt "<task description>"`
5. Update task in SQLite: status -> `running`, set worktree path and tmux window id

### Worktree cleanup

When a task is moved to `done`, the TUI offers to clean up:
- Remove the git worktree (`git -C <repo_path> worktree remove <worktree_path>`)
- Close the tmux window if still open (`tmux kill-window -t <tmux_window>`)

### Process exit detection

Safety net: the TUI's refresh cycle checks if a Running task's tmux window still exists. If the window is gone and the task is still `running`, it gets marked as `review` with a system note indicating auto-detection rather than agent-reported completion.

## MCP Server

Listens on `localhost:3142`. Uses MCP Streamable HTTP transport (the standard for HTTP-based MCP servers). Exposes tools to Claude Code agents:

| Tool | Parameters | Effect |
|------|-----------|--------|
| `update_task` | `task_id`, `status` | Move task to new status |
| `add_note` | `task_id`, `note` | Append a note to the task |
| `get_task` | `task_id` | Read back task details |

### `.mcp.json` written to worktree

```json
{
  "mcpServers": {
    "task-orchestrator": {
      "url": "http://localhost:3142/mcp"
    }
  }
}
```

### Agent prompt template

```
You are working on task #<id>: <title>

<description>

When you have completed the task, call the update_task MCP tool
with task_id=<id> and status="review".

If the MCP server is unavailable, run:
task-orchestrator update <id> review
```

## Tech Stack

| Crate | Purpose |
|-------|---------|
| `ratatui` + `crossterm` | TUI rendering + terminal input |
| `tokio` | Async runtime (MCP server + tick timer) |
| `rusqlite` (bundled) | SQLite persistence |
| `axum` | MCP HTTP/SSE server |
| `serde` + `serde_json` | JSON serialization (MCP protocol) |
| `clap` | CLI argument parsing |

## Project Structure

```
task-orchestrator-tui/
+-- Cargo.toml
+-- README.md
+-- src/
|   +-- main.rs           # CLI parsing, entrypoint routing
|   +-- db.rs             # SQLite schema, CRUD operations
|   +-- tui/
|   |   +-- mod.rs        # App state, message/update loop (Elm architecture)
|   |   +-- ui.rs         # Ratatui rendering (kanban board, detail panel)
|   |   +-- input.rs      # Key event handling -> messages
|   +-- mcp/
|   |   +-- mod.rs        # Axum server setup, route registration
|   |   +-- handlers.rs   # update_task, add_note, get_task handlers
|   +-- dispatch.rs       # Worktree creation, tmux window, claude launch
|   +-- tmux.rs           # tmux commands: new-window, capture-pane, list-windows
|   +-- models.rs         # Task struct, Status enum, shared types
+-- docs/
```

### Elm architecture (TUI)

```
Event (key press, tick, MCP update)
    -> Message (CreateTask, DispatchAgent, MoveTask, Tick, ...)
        -> update(state, message) -> (new_state, Option<Command>)
            -> Command (SQLite write, tmux call, etc.)
                -> executed async, may produce new Messages
```

## Phased Build Plan

### Phase 1: Foundation
- Project scaffolding (Cargo.toml, dependencies)
- `models.rs` -- Task struct, Status enum
- `db.rs` -- SQLite schema, create/read/update/list operations
- `main.rs` -- CLI with `list` and `update` subcommands
- Tests for db and models

### Phase 2: TUI
- `tui/mod.rs` -- App state, Elm message/update loop
- `tui/ui.rs` -- Kanban board rendering with all 5 columns
- `tui/input.rs` -- Key bindings, task creation form
- Detail panel with task info
- Refresh tick reading from SQLite
- `main.rs` -- `tui` subcommand launches the app

### Phase 3: Agent Dispatch
- `tmux.rs` -- new-window, capture-pane, has-window
- `dispatch.rs` -- worktree creation, `.mcp.json` writing, claude launch
- Wire `d` keybinding to dispatch flow
- Live output capture on tick for Running tasks
- Process exit detection (safety net)

### Phase 4: MCP Server
- `mcp/mod.rs` -- Axum server on localhost, started alongside TUI
- `mcp/handlers.rs` -- `update_task`, `add_note`, `get_task` tool implementations
- MCP protocol compliance (tool discovery, JSON-RPC)
- Agent prompt template includes MCP instructions + CLI fallback

## Design Decisions

1. **Rust over Python**: Speed, single binary distribution, language learning value. Previous designs used Python/Textual but user prefers Rust + Ratatui.
2. **Dispatch model (push), not claim model (pull)**: Agents are launched with context for a specific task. They don't discover or self-assign work. Claude Code sessions are stateless.
3. **Monolith with CLI fallback**: Single process for simplicity, CLI subcommands for resilience. If TUI dies, agents can still report via `task-orchestrator update`.
4. **Sync SQLite (rusqlite + spawn_blocking)**: Simpler than async wrappers. May migrate to `tokio-rusqlite` later if needed.
5. **tmux windows in same session**: TUI runs in tmux already, so agents get new windows (tabs) in the same session rather than separate sessions.
6. **MCP for agent communication**: Claude Code natively supports MCP. The orchestrator's MCP server lets agents call `update_task` to report completion.
7. **No github-assistant coupling**: Standalone tool focused on task creation and agent dispatch. PR monitoring is a separate concern for later.

## Configuration

CLI flags on the `tui` subcommand:

| Flag | Default | Description |
|------|---------|-------------|
| `--db` | `~/.local/share/task-orchestrator/tasks.db` | SQLite database path |
| `--port` | `3142` | MCP server port |

Environment variables override flags: `TASK_ORCHESTRATOR_DB`, `TASK_ORCHESTRATOR_PORT`.

## Success Criteria

- Create a task from the TUI, see it on the kanban board
- Dispatch an agent: worktree created, tmux window opens, Claude starts working
- See live agent output on the Running task card without switching windows
- Agent calls MCP `update_task` and the card moves to Review
- Kill the TUI, agent finishes and reports via CLI fallback, restart TUI and see task in Review
