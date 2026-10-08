# Moving the agent-tree companion pane into Claude Code mods

Task #28713. Status: **abandoned on 2026-10-08** (task #28720).

The spike, the agent list pane and the tree pane were built and then reverted.
The diff viewer is why. A mod `Pane` docks beside the chat at a width Claude
Code picks; this build's mod API has no way to widen it by hand and nothing
like tmux's zoom. The tmux diff pane can be resized and zoomed with keys the
user already knows, so moving the diff into a mod is a step back. Without the
diff viewer, the move no longer removes the tmux plumbing, which was its main
payoff. The tmux companion pane stays. Revisit if mod panes gain resizing or
a full-screen mode.

## Why

The agent-tree, its diff viewer and the active agent list are one tmux side
pane: a separate ratatui process that `dispatch agent-tree <task_id>` runs and
that `src/dispatch/agents.rs::spawn_agent_tree_pane` splits in beside each
agent window. A large part of `docs/specs/agent-tree.allium` is tmux
plumbing: split, hide, toggle, discard, and the second split for the diff pane.

Claude Code now lets a plugin ship a *mod*: a hooks module that draws a
`Pane` (docked beside the chat) or an `AbovePrompt` band inside the session
itself. The three features fit that model.

## What exists today

| Feature | Data source | Code |
| --- | --- | --- |
| Agent tree (changed files, +/- counts) | git only: unstaged work against the index (`diff --name-status`, `--numstat`, untracked) or one picked commit; merge-base only for the commit list (changed after this doc, in ecf87c45) | `src/cli/agent_tree.rs::git_changes`, `src/agent_tree.rs` |
| Diff viewer | git, same baseline; open files shared through `<git dir>/dispatch-agent-tree.json` | `src/cli/agent_diff.rs`, `src/agent_tree_open_set.rs`, `src/agent_tree_diff_pane.rs` |
| Active agent list | board `pane_view` HTTP call, `live_agents` (Running or Review with a tmux window) | `src/cli/agent_tree_agents.rs`, `src/mcp/handlers/hooks.rs::handle_pane_view` |

The pane reads the worktree path, base branch and live agents from the running
board over loopback HTTP, never from the store (`PanesReadThroughTheBoard`).

## What a mod can do (Claude Code mod API)

- `Pane`: docked beside the transcript in fullscreen from 110 columns, inline
  above the prompt otherwise. Opened by a command the person types, it seats at
  any width. Opened unasked it needs 144 columns.
- `AbovePrompt` band: always available, size capped at half the terminal.
- `$.process.run` / `$.process.spawn`: run git and `tmux select-window`.
- `$.http.fetch`: call the board's loopback port.
- `Code` element with `format: 'diff'`: draws a unified diff with gutters and
  colours. We would not draw diffs ourselves.
- `$.state`: values the drawing reads, redrawn on write. `$.store` keeps values
  across sessions.

## Direction

1. **Active agent list first.** Smallest piece. The data is one HTTP call.
   Jumping is a `tmux select-window`. It would show inside every session.
2. **Agent tree and diff viewer as one pane.** Tree on top, diff of the open
   files below, in the same `Pane`. This removes the second tmux split, the
   open-set file and the diff-pane reconcile logic.
3. **Retire the Rust pane** once the mod matches it: tmux toggle key, split on
   launch, `agent-tree` and `agent-diff` subcommands, and the matching spec
   rules.

Not moving: the kanban board. It orchestrates agents and must outlive them.

A bonus: Claude Code's built-in changed-files panel ignores the task's base
branch (learning #611). The mod diffs against the right base.

## Risks and open questions

- The mod API is early access and moves between releases.
- Hot reload and `--plugin-dir` are for the person's own session. We must
  confirm a dispatched agent can load a hooks module from the dispatch plugin
  (`plugin/`), which today ships command hooks. This is the first spike.
- The mod needs the task id and the board's port. The hook script derives the
  task id from the branch or cwd. The port source is unresolved.
- Below 110 columns the pane sits inline, not docked. Narrow terminals lose
  the side-by-side layout.
- `claude -p` and other headless runs draw nothing. Fine for agents, which are
  interactive.
- Keep the Rust pane working until the mod reaches parity, so the move is
  reversible.

## Work packages

Tracked as an epic. General guideline only; details are decided per task.

1. Spike: load a hooks-module mod in a dispatched agent session and find the
   board port and task id.
2. Mod scaffold and active agent list.
3. Agent tree pane (changed files via git). Built in #28719 as `/dispatch-tree`
   (the git work in Rust behind `dispatch agent-changes`), then reverted with
   the rest of the mod; see the status note at the top.
4. Diff viewer inside the pane.
5. Retire the Rust agent-tree and diff panes, tmux plumbing and spec rules.
