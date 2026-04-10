# Dispatch

A terminal kanban board for managing development tasks and dispatching Claude Code agents. Create tasks, dispatch agents into isolated git worktrees and tmux windows, and monitor their progress — all from a single TUI.

## Prerequisites

| Dependency | Required | Install |
|---|---|---|
| Rust toolchain | Yes | [rustup.rs](https://rustup.rs) |
| `tmux` | Yes | `apt install tmux` / `brew install tmux` |
| `git` | Yes | Already installed on most systems |
| `claude` | Yes | [Claude Code CLI](https://claude.ai/code) |
| `gh` | Optional | [GitHub CLI](https://cli.github.com) — needed for Review and Security boards |

## Getting Started

**1. Clone and install:**

```bash
git clone https://github.com/Ragazoor/dispatch-tui
cd dispatch-tui
cargo install --path .
```

**2. Configure Claude Code:**

```bash
dispatch setup
```

This registers the dispatch MCP server, installs the dispatch plugin (hooks, skills, commands), and adds MCP tool permissions.

**3. Open a tmux session** (dispatch must run inside tmux):

```bash
tmux new-session -s dev
```

**Recommended tmux setting** — enable focus events so the split-view focus indicator works:

```bash
# Add to ~/.tmux.conf
set -g focus-events on
```

**4. Start the TUI:**

```bash
cargo run tui
```

## Usage

### Create a task (`n`)

| Step | Key | What happens |
|------|-----|--------------|
| Create task | `n` | Enter title, description, tag, and repo path |
| Dispatch | `d` | Agent explores your codebase, writes a plan, and implements it |
| Agent needs input *(optional)* | `g` | Desktop notification — jump to agent and interact |
| Review the work | `g` | Task is in Review — check the result in the tmux window |
| Wrap up | `W` | Commit, rebase, and open a PR. Or use `/wrap-up` from the agent's session |

### Quick dispatch (`D`)

| Step | Key | What happens |
|------|-----|--------------|
| Quick dispatch | `D` | Pick a repo from the numbered list |
| | | Task created and dispatched — agent sets its own title and description |
| Check on the agent | `g` | Jump to the agent's tmux window |
| Wrap up | `W` | Commit, rebase, and open a PR. Or use `/wrap-up` from the agent's session |

### Work with an epic (`E`)

| Step | Key | What happens |
|------|-----|--------------|
| Create epic | `E` | Enter title, description, and repo path |
| Dispatch planning | `d` | Creates a planning subtask; agent writes an implementation plan with subtasks |
| Dispatch subtasks | `d` | Each press dispatches the next Backlog subtask in order |
| Reorder subtasks | `J` / `K` | Change dispatch order within the epic |
| Wrap up each subtask | `W` | Commit, rebase, and open a PR. Or use `/wrap-up` from the agent's session |

### Wrap up a task

There are two ways to wrap up completed work:

**From the TUI** — press `W` on a task:

| Option | Key | What happens |
|--------|-----|--------------|
| Rebase | `r` | Rebases onto main, fast-forwards main, kills tmux window |
| Create PR | `p` | Pushes branch and opens a draft GitHub PR |

**From Claude Code** — type `/wrap-up` in the agent's session. The agent commits any uncommitted changes, then asks you the same rebase-or-PR question.

## Key Concepts

**Tasks** — the unit of work. Each task has a title, description, status, and optionally a plan and a linked git repo.

**Tags** — optional labels (`b`=bug, `f`=feature, `c`=chore, `e`=epic) chosen during task creation that control what happens when you press `d`:

| Tag | No plan | Has plan |
|-----|---------|----------|
| `epic` | Brainstorm (explore and ideate, no code edits) | Dispatch |
| `feature` | Plan (write implementation plan, no code edits) | Dispatch |
| `bug`, `chore`, none | Dispatch | Dispatch |

**Plans** — markdown files describing what an agent should build. A task with a plan always dispatches directly regardless of tag.

**Kanban columns:** Backlog → Running → Review → Done

- **Backlog** — tasks ready to be dispatched (`▸` = has a plan)
- **Running** — agent is active in a tmux window
- **Review** — agent finished; awaiting your review
- **Done** — merged and wrapped up

**Worktrees** — each dispatched agent gets its own git worktree at `<repo>/.worktrees/<id>-<slug>`, isolating agent work from your main branch. Closing the tmux window does **not** delete the worktree — press `d` again to resume.

**Epics** — a group of related tasks. Press `g` on an epic to see its subtasks. Press `d` on the epic to dispatch the next Backlog subtask automatically.

**Review Board** — press `Tab` to see GitHub PRs where you are a requested reviewer. Requires `gh` CLI.

**Security Board** — press `Tab` again to see dependency vulnerability alerts across your repos.

## Learn More

- **[Reference](docs/reference.md)** — key bindings, configuration, CLI usage, troubleshooting
- **[CLAUDE.md](CLAUDE.md)** — architecture, testing patterns, contribution guidelines
