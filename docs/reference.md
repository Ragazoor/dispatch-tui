# Dispatch Reference

## Key Bindings

### Navigation

| Key | Action |
|-----|--------|
| `h` / `l` / `←` / `→` | Move between columns |
| `j` / `k` / `↓` / `↑` | Move between tasks |
| `Enter` | Toggle detail panel |
| `Tab` | Cycle boards: Task → Review → Security |
| `?` | Toggle help overlay |
| `q` | Quit (or exit epic view) |

### Tasks

| Key | Action |
|-----|--------|
| `n` | New task |
| `c` | Copy selected task |
| `e` | Edit task in editor |
| `d` | Dispatch agent — behavior depends on tag (see README) / resume (Running task whose window is gone) |
| `D` | Quick dispatch — pick repo and dispatch immediately |
| `m` / `M` | Move task forward / backward |
| `W` | Wrap up — commit, rebase, open PR |
| `g` | Jump to the agent's tmux window |
| `x` | Archive task (with confirmation) |
| `H` | Toggle archive panel |
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
| `m` | Mark epic done (when all subtasks are done) |
| `J` / `K` | Reorder subtasks (determines dispatch order) |
| `q` | Exit epic view |

### Review Board (`Tab`)

| Key | Action |
|-----|--------|
| `h` / `l` / `j` / `k` | Navigate PRs |
| `Enter` | Toggle PR detail |
| `p` | Open PR in browser |
| `d` | Dispatch review agent |
| `r` | Refresh |
| `f` | Filter by repo |
| `Shift+Tab` | Cycle board mode (Review PRs / My PRs / Bot PRs) |
| `Tab` | Next board (Security) |
| `Esc` | Return to kanban |

## How Dispatch Works

Press `d` on a Backlog task:

1. Creates a git worktree at `<repo>/.worktrees/<id>-<slug>`
2. Opens a new tmux window in your current session
3. Writes `.mcp.json` so Claude discovers the MCP server
4. Launches `claude` with the task description and completion instructions

The agent reports progress via the MCP server running on `localhost:3142`. When it finishes, it moves the task to Review. Closing a tmux window does **not** delete the worktree — press `d` again on a Running task to resume.

## Review Board

Press `Tab` to switch to the Review Board, which shows GitHub PRs where you are a requested reviewer. Data is fetched via `gh api graphql` and refreshed every 60 seconds.

Three columns: **Needs Review** → **Changes Requested** → **Approved**

Press `e` to edit the GitHub search queries for the current tab in `$EDITOR`. Each line is a separate GitHub search query. Queries are stored per-category (Review PRs, My PRs, Bot PRs) and persist across sessions.

**Bot PRs tip:** The default bot queries have no org scope, so results may be dominated by public repos. Add `org:your-org` to scope them (e.g. `is:pr is:open author:app/dependabot -is:draft archived:false org:my-org`).

Requires `gh` CLI authenticated:

```bash
gh auth login
```

## Security Board

Press `Tab` from the Review Board to switch to the Security Board, which shows Dependabot and code scanning alerts across your repos.

Columns are grouped by severity: **Critical** → **High** → **Medium** → **Low**

| Key | Action |
|-----|--------|
| `h` / `l` / `j` / `k` | Navigate alerts |
| `Enter` | Toggle alert detail |
| `p` | Open alert in browser |
| `d` | Dispatch agent to fix the alert |
| `r` | Refresh |
| `f` | Filter by repo |
| `t` | Toggle alert kind (Dependabot / code scanning) |
| `Tab` / `Esc` | Return to kanban |

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

The setup is idempotent — safe to run on every install or upgrade.

### Plugin contents

| Component | Purpose |
|-----------|---------|
| `/wrap-up` skill | Commit, rebase or PR when a task is complete |
| `/queue-plan` command | Queue a plan file as a task |
| `task-status-hook` | Automatically transitions task status (running/review/needs_input) |
| `task-usage-hook` | Reports token usage and cost per task |

To verify the plugin is installed:
```bash
ls ~/.claude/plugins/local/dispatch/
```

To reinstall:
```bash
dispatch setup
```

## Tmux Configuration

For the best experience with split-view mode, enable focus events in tmux:

```bash
# ~/.tmux.conf
set -g focus-events on
```

This allows the TUI to show a colored border indicating which pane has focus: cyan when the TUI is focused, dim when the agent pane is focused. Without this setting, the focus border will not respond to pane switches.

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

**Review Board shows no PRs**
Run `gh auth login` and ensure you have open PRs where you are a requested reviewer.

**Agent window disappeared but task is still Running**
Press `d` on the Running task to reopen a tmux window in the existing worktree and resume the agent.
