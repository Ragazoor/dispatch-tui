# Module Rule Gaps and Request Pruning

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Every module write path enforces the task invariants, and request tables stop growing forever.

## Context

This work package addresses findings 17–19, 51 and 53–56 of the 2026-10-17
code review (`docs/plans/2026-10-17-codebase-review.md`). `spacetime/module/`
is a separate wasm crate: see `docs/testing.md` ("SpacetimeDB module and CI
details"). Mirror every change in `src/sync/memory_caller/` and cover the
pair in `tests/memory_caller_conformance.rs`. Rebuild with
`./scripts/build-managed-module.sh`, regenerate bindings if the schema
changes, and bump `RULES_VERSION` and `BOARD_RULES_VERSION` together if the
module constant's rules say so. Spec: `docs/specs/store-split.allium`,
`docs/specs/tasks.allium`. Caller-check findings are out of scope (trust
model question).

## Findings

### 🟡 Two reducers write `sub_status` without the legality check (`spacetime/module/src/tasks_epics.rs:516`, `spacetime/module/src/agent_state.rs:443`, `:472`)

**Issue:** `batch_patch_sub_status` writes `ctx.db.tasks()` directly;
`record_pre_tool_use` (and `record_notification`) go through `write_task`,
which does not validate. Only `check_task_patch` (from `patch_task`) does. A
stale board can store backlog/approved.

**Fix:** Put the legality check in `write_task` (the one write seam) and have
the batch go through it, skipping illegal entries; let `patch_task` drop its
special case if it becomes redundant. Mirror in the memory caller.

### 🟡 `patch_task` applies `epic_id` raw (`spacetime/module/src/tasks_epics.rs:339`)

**Issue:** Skips `route_to_repo_group` and the epic-exists check that
`create_task` and `set_task_epic` do. A patch can land a task on a
`group_by_repo` root, or on a missing epic (invisible orphan).

**Fix:** Route `patch.epic_id` through `set_task_epic`'s logic, or refuse
`epic_id` in `patch_task` if no host caller needs it (grep `src/`).

### 🟡 Request rows never pruned (`spacetime/module/src/dispatch_request.rs`, `spacetime/module/src/session_request.rs`)

**Issue:** Only timer rows are ever deleted. Both public tables grow forever,
are downloaded on every connect, and `check_exit_token` scans a task's whole
history.

**Fix:** Delete a task's request rows in `delete_task_side_effects`; prune
terminal rows older than N days in the scheduled backstop (keep the latest
done wrap-up per task if the exit token needs it). Spec the retention.

### 🟢 Low findings

- `src/sync/memory_caller/tasks_epics.rs` (`apply_patch_task`) and
  `src/sync/memory_caller/dispatch_requests.rs` — the double mutates before a
  late fallible write and does not roll back on `Refused`, unlike a reducer.
  Snapshot and restore `Tables` at the single dispatch point.
- `spacetime/module/src/tasks_epics.rs:478` (`respawn_phoenix_successor`) —
  no check that the predecessor has the phoenix flag. Check whether any host
  path still calls it; remove if dead, guard if not.
- `spacetime/module/src/session_request.rs:992` (`close_task`) — fails open
  session requests but not open dispatch requests; call
  `fail_open_dispatch_requests_of`.
- `spacetime/module/src/tasks_epics.rs` (`delete_epic_subtree`) — silently
  returns past `MAX_EPIC_DEPTH`; refuse in the validation pass instead.
- `spacetime/module/src/rules.rs:134` (`validate_epic_parent`) — returns
  `Ok` after the hop cap; return the cycle error.

## Changes

| File | Change |
|------|--------|
| `docs/specs/store-split.allium` / `tasks.allium` | Legality at the write seam; request retention |
| `spacetime/module/src/tasks_epics.rs`, `agent_state.rs`, `rules.rs` | Validation, routing, guards |
| `spacetime/module/src/dispatch_request.rs`, `session_request.rs` | Pruning; close fails dispatch requests |
| `src/sync/memory_caller/` | Mirror every change; rollback on refusal |
| `tests/memory_caller_conformance.rs` | Pair coverage |
| `spacetime/module.wasm`, bindings | Rebuild / regenerate as needed |

## Verification

- [ ] Tests first in the conformance suite for each rule
- [ ] Module crate checks per `docs/testing.md`; `./scripts/check-spacetime-module.sh`
- [ ] `cargo test --no-fail-fast` (with `spacetime` on PATH) and the task's verify command pass
