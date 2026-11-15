# Task window kills spare the split leftover; clear the pane home tag

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** every way a task window is killed spares the user's shell in a split leftover window, and a respawned pinned pane forgets its old home.

## Context

Findings R1 and R4 from `docs/plans/2026-11-15-codebase-review.md` §3.6. R1 is the sibling 3f5cd27c missed; R4 a regression of d87b938d. Spec first (`docs/specs/split-pane.allium` "Leftover window", `startup.allium` retire). Test real tmux only through `tests/tmux_harness/mod.rs` (private socket); never drive tmux by hand without `-L`.

## Findings

### 🟡 Medium — R1: only the board's kill spares the split leftover window (`src/service/tasks/session.rs::kill_row_window`, `src/service/tasks/wrap_up.rs::kill_session_window`, `src/dispatch/worktree.rs::teardown_task` → `kill_window_if_present`)

**Issue:** only `src/runtime/split.rs` (`KillTmuxWindow`) goes through `dispatch::kill_task_window`. Pin task 7 with a shell split beside it; the agent calls `exit_session`; the reactor's plain `kill_window("task-7")` kills the shell and its scrollback.

**Fix:** route every task-window kill through `kill_task_window` (or move the awaiting-tag check into `tmux::kill_window`). Graduate: a `scripts/check-*.sh` (with self-test, run by `gates.sh`) that fails on a `tmux::kill_window`/`kill_window_if_present` call on a task window outside `kill_task_window`. Mutation-test it by restoring one plain call.

### 🟢 Low — R4: the pinned pane's home tag is never cleared (`src/dispatch/split_panes.rs::tag_pinned_pane`, `src/startup/retire.rs::return_pinned_agents`)

**Issue:** after the pinned task loses its window, the respawned shell (`respawn-pane -k` keeps the pane and its options) still carries `@dispatch_home_window=task-7`. If `task-7` exists again, retire's break-out is refused and the launch aborts as `BoardWindowSurvived`; if not, the relaunch creates a phantom `task-7` holding a shell.

**Fix:** `set-option -p -u @dispatch_home_window` on respawn and on restore; a tmux-harness test.

## Changes

| File | Change |
|------|--------|
| `docs/specs/split-pane.allium` | leftover window spared on every kill path; tag cleared on respawn |
| `src/service/tasks/{session,wrap_up}.rs`, `src/dispatch/worktree.rs` | use `kill_task_window` |
| `src/dispatch/split_panes.rs`, `src/runtime/split.rs` | unset the tag |
| `scripts/check-*.sh` (new), `scripts/gates.sh` | kill-path check |
| `tests/` (tmux harness) | exit_session with a split leftover; respawn clears the tag |

## Verification

- [ ] `./scripts/verify.sh` passes (with `tmux` on `PATH`; no `skipping` lines for the new tests)
- [ ] The kill-path check fails on a restored plain call
