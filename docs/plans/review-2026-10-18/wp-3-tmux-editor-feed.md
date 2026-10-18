# tmux, Editor and Feed Soft-Fails

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop transient tmux errors being read as "absent", and tidy feed process cleanup.

## Context

This work package addresses findings from the 2026-10-18 codebase review (`docs/plans/2026-10-18-codebase-review.md`, finding numbers match that report). Verify each finding in the code before fixing; it may have moved.

## Findings

### Low: `window_target` reads any `list-panes` failure as "no windows" (`src/tmux.rs:~187`)

**Fix:** Return `Err` except for "no server running". Fix the stale comment (~179) and the `rename_window` doc (~961).

### Low: Remaining soft-fail callers (`src/dispatch/agents.rs:789`, `src/tmux.rs:234`)

**Fix:** Stop using `has_window().unwrap_or(false)`; surface the error. Narrow `is_no_server` so "Permission denied" is not an empty list.

### Low: Feed exec edge cases (`src/feed/exec.rs`)

**Fix:** Kill the process group on a stdout read error and on the stderr cap; kill without shelling out to `kill(1)` inside the async fn.

### Low: Failed editor-window create can kill a pre-existing window (`src/runtime/editor.rs`)

**Fix:** Do not arm `cleanup_runner` until the create succeeds, or skip it when the failure was a duplicate name.

## Changes

| File | Change |
|------|--------|
| `src/tmux.rs` | Error handling, comments, `is_no_server` |
| `src/dispatch/agents.rs` | `resume_agent` error handling |
| `src/feed/exec.rs` | Process-group kill paths |
| `src/runtime/editor.rs` | Cleanup arming |

## Verification

- [ ] Run existing tests — all pass (tmux tests need `tmux` on PATH; use the private-socket harness)
- [ ] New tests for each changed error path
- [ ] Run the repo's verify command from get_task
