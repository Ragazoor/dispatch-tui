# Dispatch Reference

## Key Bindings

### Navigation

| Key | Action |
|-----|--------|
| `h` / `l` / `←` / `→` | Move between columns |
| `j` / `k` / `↓` / `↑` | Move between tasks |
| `Enter` | Toggle detail panel |
| `Tab` | Cycle through feed epics |
| `?` | Toggle help overlay |
| `q` | Quit (or exit epic view) |

### Tasks

| Key | Action |
|-----|--------|
| `n` | New task |
| `c` | Copy selected task |
| `e` | Edit task in editor (opens in a separate tmux window) |
| `d` | Dispatch agent — behavior depends on tag (see README) / resume (Running task whose window is gone) |
| `D` | Quick dispatch — pick repo and dispatch immediately |
| `Shift+L` / `Shift+H` | Move task forward / backward |
| `W` | Wrap up — commit, rebase, open PR |
| `g` | Jump to the agent's tmux window (swap pane in split view) |
| `G` | Jump to the agent's tmux window (always, ignoring split view) |
| `S` | Toggle split view — side-by-side TUI + agent pane |
| `x` | Archive task (with confirmation) |
| `Space` | Toggle select |
| `a` | Select all in column |
| `J` / `K` | Reorder task up / down |
| `f` | Filter by repo path |
| `N` | Toggle notification panel |

### Epics

| Key | Action |
|-----|--------|
| `E` | New epic |
| `g` | Enter epic view (see subtasks) |
| `d` | Dispatch next backlog subtask |
| `D` | Quick dispatch subtask for this epic |
| `Shift+L` / `Shift+H` | Move epic status forward / backward |
| `J` / `K` | Reorder subtasks (determines dispatch order) |
| `q` | Exit epic view |

## How Dispatch Works

Press `d` on a Backlog task:

1. Creates a git worktree at `<repo>/.worktrees/<id>-<slug>`
2. Opens a new tmux window in your current session
3. Writes `.mcp.json` so Claude discovers the MCP server
4. Launches `claude` with the task description and completion instructions

The agent reports progress via the MCP server running on `localhost:3142`. When it finishes, it moves the task to Review. Closing a tmux window does **not** delete the worktree — press `d` again on a Running task to resume.

## CLI Usage

```bash
# Start the TUI (must be inside a tmux session)
dispatch tui

# CLI — used by agents and hooks
dispatch update <task-id> <status>
dispatch list [--status <status>]
dispatch create --from-plan plan.md
```

## Configuration

| Flag | Env Var | Default |
|------|---------|---------|
| `--db` | `DISPATCH_DB` | `~/.local/share/dispatch/tasks.db` |
| `--port` | `DISPATCH_PORT` | `3142` |

## Setup

`dispatch setup` configures Claude Code integration:

1. **MCP server** — registers the dispatch server in `~/.claude/.mcp.json`
2. **Plugin** — installs hooks, skills, and commands to `~/.claude/plugins/local/dispatch/`
3. **Permissions** — adds MCP tool permissions to `~/.claude/settings.json`
4. **Tmux** — enables `focus-events` globally (needed for split-view focus indicator)

The setup is idempotent — safe to run on every install or upgrade.

### Plugin contents

| Component | Purpose |
|-----------|---------|
| `/wrap-up` skill | Commit, rebase or PR when a task is complete |
| `/queue-plan` command | Queue a plan file as a task |
| `task-status-hook` | Automatically transitions task status (running/review/needs_input) |
| `task-usage-hook` | Reports token usage per task |

To verify the plugin is installed:
```bash
ls ~/.claude/plugins/local/dispatch/
```

To reinstall:
```bash
dispatch setup
```

## Tmux Configuration

`dispatch setup` enables `focus-events` for the running tmux server. To persist this across tmux server restarts, add to `~/.tmux.conf`:

```
set -g focus-events on
```

This allows the split-view focus indicator to work: a colored border shows which pane has focus (cyan = TUI, dim = agent pane). Without this setting, the border will not respond to pane switches.

## Troubleshooting

**`not running inside a tmux session`**
Start a tmux session first: `tmux new-session -s dev`

**`dispatch: command not found`**
`~/.local/bin` is not in your PATH. Add to your shell profile:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

**`claude: command not found`**
Install Claude Code from https://claude.ai/code

**Task status not updating automatically**
Verify the dispatch plugin is installed: `ls ~/.claude/plugins/local/dispatch/hooks/hooks.json`. If missing, run `dispatch setup` to reinstall.

**Skills not available (`/wrap-up`, `/queue-plan`)**
The dispatch plugin may not be installed. Run `dispatch setup` to install it.

**Agent window disappeared but task is still Running**
Press `d` on the Running task to reopen a tmux window in the existing worktree and resume the agent.

## Learning Store

Dispatch maintains a learning store — approved knowledge that is injected into agent prompts automatically and can be queried or recorded via MCP tools.

### Scopes

Learnings are tagged with a scope that determines which tasks see them:

| Scope     | Covers                        | Example use |
|-----------|-------------------------------|-------------|
| `user`    | All tasks for this user       | Editor preference, personal workflow rules |
| `project` | All tasks in a project        | Project-specific conventions |
| `repo`    | All tasks in a repository     | Build toolchain, test patterns |
| `epic`    | All tasks in an epic          | Shared design decisions for this feature |
| `task`    | One specific task             | Episodic notes scoped to a single agent run |

### Retrieval at Dispatch Time

When an agent is dispatched, Dispatch queries approved learnings that match the task's context and injects them into the prompt. The union includes:

- **Always**: `user`-scoped learnings
- **Always**: `repo`-scoped learnings where `scope_ref` matches the task's repo path
- **Always**: `project`-scoped learnings where `scope_ref` matches the task's project
- **If task belongs to an epic**: `epic`-scoped learnings for that epic

`task`-scoped learnings are **not** auto-injected. They can be retrieved explicitly via `query_learnings` with a `tag_filter`.

### Ranking

Within the injected set, learnings are ordered:

1. **Kind first**: `procedural` learnings appear before all others (injected verbatim as prompt-prefix instructions)
2. **Scope proximity**: epic → repo → project → user (closest context first)
3. **Confirmation count**: more-confirmed learnings rank higher within the same band

The auto-inject cap is **10 learnings**. Agents can retrieve up to **50** via an explicit `query_learnings` call.

### Recording a Learning

Agents propose learnings via `record_learning`. The `scope_ref` is auto-derived from the task's context when omitted:

```
scope=user    → scope_ref: (none)
scope=repo    → scope_ref: task.repo_path
scope=project → scope_ref: task.project_id
scope=epic    → scope_ref: task.epic_id  (error if task has no epic)
scope=task    → scope_ref: task.id
```

All proposed learnings await human approval before appearing in any agent's context.

### Examples

**User preference** — applies to every task you run:
```
scope=user, kind=preference
summary="Always use uv to run Python scripts, never python directly"
```

**Repo convention** — applies to all tasks in this repository:
```
scope=repo, kind=convention
summary="Integration tests use Database::open_in_memory() — never mock the DB layer"
```

**Epic decision** — applies only to tasks in this epic:
```
scope=epic, kind=procedural
summary="This epic adds the learning store; consult docs/specs/core.allium before changing domain types"
```

**Task episodic note** — scoped to a single task, not auto-injected:
```
scope=task, kind=episodic
summary="Rebase on main resolved the rusqlite version conflict; use that if it recurs"
```
