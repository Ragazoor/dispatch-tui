# Agent-Tree Editor Pane

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop the agent tree from killing an editor pane that may hold unsaved edits, and test the editor flag's transitions.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-23-codebase-review.md` (G1, G2, G3), in the agent-tree views added by a7ebe531. Spec: `docs/specs/agent-tree.allium` (`SplitAgentTreeDiffPane`, `OpenAgentTreeFileInEditor`, `DiffPaneExistsWhenSomethingIsOpen`). Spec first, then tests, then code. Test real tmux only through `tests/tmux_harness/mod.rs`.

## Findings

### 🟡 Medium: The editor key throws away unsaved edits (`src/agent_tree/diff_pane.rs::show_in_editor`, `src/agent_tree/diff_pane.rs::reconcile_diff_pane`, `src/agent_tree/run.rs`)

**Issue:** The user presses `v` on `a.rs`, edits it in the lower pane, goes back to the tree and presses `v` on `b.rs` or Space on a diff. Both run `respawn-pane -k`, which kills the editor with no prompt (`reconcile_diff_pane`'s `(true, Some(pane)) if showing_editor` arm, `show_in_editor`'s `Some(pane)` arm). `a`, `q` or hiding the tree run `kill-pane` (`(false, Some(pane))` arm, `tear_down_diff_pane`). The buffer is lost. The spec's "Reuse, never takeover" guidance in `SplitAgentTreeDiffPane` refuses to kill panes because "killing the process in one of them would cost the user work", yet `OpenAgentTreeFileInEditor` says it replaces the pane with `respawn-pane -k` and never mentions unsaved work. **Decision item** — see below.

**Fix:** Per the decision. Record it in `docs/specs/agent-tree.allium` first.

### 🟢 Low: The editor flag lives only in process memory (`src/agent_tree/state.rs::RenderState::lower_pane_editor`)

**Issue:** If the tree renderer restarts without tearing down while an editor pane with the diff role is open, the flag starts `false`; the next Space takes the "already agreeing" arm and the diff never appears.

**Fix:** Store the pane's content kind (diff or editor) in a tmux pane option next to the existing role marker and read it back instead of the in-memory flag.

### 🟢 Low: No tests for the editor flag's transitions (`src/agent_tree/run.rs`, `src/agent_tree/tests/`)

**Issue:** Nothing tests how `run.rs` sets and resets `lower_pane_editor` (editor, then Space; a failed reconcile keeps it), or `v` with an empty open set followed by a toggle that leaves it empty, which kills the editor pane.

**Fix:** Add tests for each transition alongside the G1 behaviour.

## Decisions

### What happens to an editor pane with possibly unsaved work (`src/agent_tree/diff_pane.rs::show_in_editor`)

**Question:** When the lower pane shows the editor and the user presses `v` on another file, Space on a diff, `a`, `q` or hides the tree, what should dispatch do?

**Options:**
- (a) Refuse to replace or kill the pane while the editor shows, with a notice ("quit the editor first"). Safe and small; the user must quit the editor before browsing diffs again.
- (b) Open diffs in a second pane and leave the editor pane alone. No lost work and no extra step; more panes, more geometry rules in the spec, and teardown on `q` still has to decide about the editor.
- (c) Accept the kill and document it in `OpenAgentTreeFileInEditor`. No code change; users lose unsaved edits.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/agent-tree.allium` | Record the G1 decision; content kind as a pane option (G2) |
| `src/agent_tree/diff_pane.rs` | Per decision; read content kind from tmux |
| `src/agent_tree/run.rs`, `src/agent_tree/state.rs` | Per decision; drop or back the in-memory flag |
| `src/tmux.rs` | Pane-option read/write if not already there |
| `src/agent_tree/tests/`, `tests/tmux_diff_pane.rs` | Tests for G1–G3 |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green (with `tmux` on `PATH`; check no `skipping:` lines)
- [ ] `cargo clippy --all-targets -- -D warnings` clean
