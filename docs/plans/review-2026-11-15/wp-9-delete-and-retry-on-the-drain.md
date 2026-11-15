# Delete and RetryFresh: no blocking, no race, honest batches

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** RetryFresh never blocks the command drain, a delete cannot tear down an agent started after its check, and batch operations report and keep what they could not do.

## Context

Findings R2, R3, R5, R6 and R8 from `docs/plans/2026-11-15-codebase-review.md` §3.6. Spec first where behaviour changes (`docs/specs/tasks.allium: BatchDelete`, `BatchMoveToEpic`, RetryFresh). Use the `run_bounded` primitive (`docs/conventions.md`) for any new wait.

## Findings

### 🟡 Medium — R2: RetryFresh blocks the command drain on an untimed `git worktree remove` (`src/runtime/commands.rs::run_cleanup` with `await_teardown` → `dispatch::teardown_task` → `src/dispatch/worktree.rs::remove_worktree_and_branch` → `git::git_in_unbounded`)

**Issue:** chain 5e34bf8f → 10e90b63 / 69cca0cc. The one wait left is on the deliberately unbounded git call; a multi-GB `target/` or a hung mount freezes keys and drawing.

**Fix:** issue the re-dispatch from the cleanup's completion message (`CleanupSucceeded`/`CleanupFailed` with a RetryFresh follow-up) instead of awaiting on the drain.

### 🟡 Medium — R3: the delete pre-check can pass, then the teardown kills an agent started in between (`src/runtime/tasks.rs::exec_batch_delete`, `src/runtime/epics.rs::exec_delete_epic`, `::exec_cleanup_queues`) — plausible

**Issue:** targets are taken at keypress; `pass_delete_guard` checks; teardowns start fire-and-forget before `batch_delete`/`delete_epic` re-checks in the reducer. Another board reopens and resumes a task in between: its new window is killed and its worktree removed, then the reducer refuses and the row points at nothing.

**Fix:** delete first, then tear down from what the delete removed (873287be's archived records carry the worktree and window), so a refused delete tears down nothing.

### 🟢 Low — R5: teardown queues and pre-checks (`src/runtime/tasks.rs::exec_cleanup_queues`, `::pass_delete_guard`)

**Issue:** queues group by the raw `repo_path` string (`~/x` and `/home/u/x` run in parallel); `pass_delete_guard` calls `check_deletable` once per id although the procedure takes vectors (N round trips, N snapshots, against "checks the whole batch once").

**Fix:** key queues on the expanded path; one `check_deletable(task_ids, epic_ids)` call.

### 🟢 Low — R6: the editor's local update still applies the open-time snapshot (`src/runtime/editor.rs::finalize_task_edit` → `src/tui/update/system.rs::handle_task_edited`; `finalize_epic_edit`)

**Issue:** the store gets only changed fields (3908be70), but the board copy gets the snapshot's status, model and wrap-up mode (and `stop_pending` cleared) until the next push.

**Fix:** apply only the changed fields locally.

### 🟢 Low — R8: `m` on a selection (`src/tui/update/move_task.rs` → `src/runtime/tasks.rs::exec_move_task_to_epic`)

**Issue:** N moves each with a full refresh; the selection is cleared before results arrive; epics in the selection are silently ignored.

**Fix:** keep refused ids selected (as d6215146 does for delete); refresh once after the last move; say when epics were skipped.

## Changes

| File | Change |
|------|--------|
| `docs/specs/tasks.allium` | RetryFresh order; delete-then-teardown |
| `src/runtime/{commands,tasks,epics,editor}.rs` | R2, R3, R5, R6 |
| `src/tui/update/{retry,move_task,system}.rs` | follow-up message; selection; local fields |
| `src/runtime/tests/`, `src/tui/tests/` | tests per finding |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] A test shows RetryFresh's drain returns before the teardown completes
