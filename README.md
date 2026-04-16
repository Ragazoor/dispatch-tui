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

This registers the dispatch MCP server, installs the dispatch plugin (hooks, skills, commands), adds MCP tool permissions, and enables and persists tmux `focus-events` (needed for the split-view focus indicator).

**3. Open a tmux session** (dispatch must run inside tmux):

```bash
tmux new-session -s dev
```

**4. Start the TUI:**

```bash
cargo run tui
```

## Usage

**Create a task (`n`)** — enter a title, description, tag (`b`=bug, `f`=feature, `c`=chore, `e`=epic), and a repo path. Press `d` to dispatch: a Claude Code agent opens in a tmux window and, depending on the tag, writes a plan before implementing. When the agent moves the task to Review, press `W` to rebase onto main or open a draft PR — or type `/wrap-up` in the agent's session to commit any pending work and do the same.

**Quick dispatch (`D`)** — skip the form entirely. Pick a repo (if more than one is configured) and the agent dispatches immediately with a placeholder title; it renames the task itself after learning what you want.

**Epics (`E`)** — group related work under an epic. The first `d` creates a planning subtask whose agent writes an implementation plan broken into subtasks; each subsequent `d` dispatches the next Backlog subtask in order.

**Navigation** — `Tab` cycles between the task, review, and security boards. `g` jumps to the selected agent's tmux window, `S` opens a side-by-side split with the TUI on the left and the agent pane on the right.

Full key bindings and configuration options are in [docs/reference.md](docs/reference.md).

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

**Split view** — press `S` to enter side-by-side mode: the TUI on the left, the selected agent's tmux pane on the right. Press `G` to pin a different task in the right pane, or `g` to jump directly to an agent window (leaving split view). A colored border shows which pane has focus (cyan = TUI, dim = agent). Requires tmux `focus-events` — enabled automatically by `dispatch setup`.

**Epics** — a group of related tasks. Press `g` on an epic to see its subtasks. Press `d` on the epic to dispatch the next Backlog subtask automatically. Epics can be nested — an epic subtask can itself be an epic.

**Review Board** — press `Tab` to see GitHub PRs in two modes — **Reviewer** (PRs awaiting your review) and **Author** (your own PRs) — toggled with `1`/`2`. Requires `gh` CLI.

**Security Board** — press `Tab` again to see dependency vulnerability alerts across your repos.

## Agentic patterns

Dispatch agents can coordinate with each other through the MCP server:

**Spawning subtasks** — an agent can create a new task on the board with `create_task`, useful when it discovers work that should be tracked separately or handed off to another agent.

**Agent-to-agent messaging** — `send_message` delivers a prompt directly into another running agent's tmux window. Fire-and-forget: the sender doesn't wait for a response. Useful for passing context or unblocking a dependent agent.

**Epic as orchestration** — an epic planning agent writes an implementation plan with subtasks, then each subtask is dispatched in sequence. Agents can call `dispatch_next` to trigger the next subtask themselves once their own work is complete.

## Learn More

- **[Reference](docs/reference.md)** — key bindings, configuration, CLI usage, troubleshooting
- **[CLAUDE.md](CLAUDE.md)** — architecture, testing patterns, contribution guidelines
