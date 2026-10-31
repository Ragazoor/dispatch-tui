# Epic Write and Task Residue

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make an epic update one write, and remove the last cache-based decisions and pre-checks from task writes.

## Context

This work package addresses findings R2, R5 and R6 from `docs/plans/2026-10-31-codebase-review.md`. Pattern to follow: 5a5209b7 (`update_task` reducer: relink, patch and Done stamp in one transaction) and its twin `src/sync/memory_caller/tasks_epics.rs::apply_update_task`.

## Findings

### 🟡 Medium — R2: `EpicService::update_epic` makes three writes (`src/service/epics.rs:417`)

**Issue:** `patch_epic`, then `move_epic_to_workspace`, then `set_epic_visibility`, each its own store call. A non-creator who edits an epic's title and visibility in the editor gets the title saved and the visibility refused; `runtime/editor.rs::finalize_epic_edit` returns early and shows an error while the store keeps the new title. MCP `update_epic` with `workspace_id` plus other fields has the same shape.

**Fix:** One reducer (or `patch_epic` carrying workspace and visibility) that checks every refusal before the first write; mirror it in the twin and add a conformance case "refused visibility leaves the title unchanged".

### 🟢 Low — R5: Done decisions still read the cache (`src/service/tasks/crud.rs::update_task`, `src/tui/commands/task.rs::PersistFields::from_task`)

**Issue:** ClosePrOnDone is still decided from the cached prior row, so a move into Done after another board reopened the task can skip closing the PR. `PersistFields::from_task` forwards `completed_at` on every persist, so a Done→Done write (a sort swap) can overwrite another board's reorder.

**Fix:** Have the module's `update_task` report the prior status (or "entered Done") back to the caller and decide ClosePrOnDone from that; forward `completed_at` only on an explicit reorder.

### 🟢 Low — R6: `move_task_to_epic` still pre-checks (`src/service/tasks/crud.rs::move_task_to_epic`)

**Issue:** It checks `task_exists`, then calls `set_task_epic`, which answers `Ok` for a missing task. A task deleted in between gives a silent success; an epic deleted in between surfaces as `Internal`. `create_task_inner` also still pre-checks.

**Fix:** Route through `update_task` with an empty patch (which refuses a missing task), or reuse `refusal_as_not_found`; drop the pre-checks.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/tasks_epics.rs` (or `visibility.rs`) | One epic-update reducer; `update_task` reports the transition |
| `src/sync/memory_caller/tasks_epics.rs` | Mirror |
| `src/service/epics.rs` | One store call in `update_epic` |
| `src/service/tasks/crud.rs` | ClosePrOnDone from the reported transition; drop pre-checks |
| `src/tui/commands/task.rs` | Forward `completed_at` only on reorder |
| `tests/memory_caller_conformance/` | Partial-refusal cases |
| `rules.rs` / `rules_gate.rs`, `module.wasm`, bindings | Bump together; rebuild |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Refused visibility change leaves the epic title unchanged (test)
- [ ] After merge to main: `./scripts/rotate-previous-store.sh`
