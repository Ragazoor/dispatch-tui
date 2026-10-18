# Module, In-Memory Twin and Managed Store

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close rule and test gaps in the module and its in-memory twin, and tighten store adoption.

## Context

This work package addresses findings from the 2026-10-18 codebase review (`docs/plans/2026-10-18-codebase-review.md`, finding numbers match that report). Verify each finding in the code before fixing; it may have moved. Spec first for any behaviour change (see CLAUDE.md "Working With the User"); module rules are mirrored in `src/sync/memory_caller/` and covered in `tests/memory_caller_conformance.rs`. If you change the module, rebuild with `./scripts/build-managed-module.sh`.

## Findings

### Low: Managed store adopts any HTTP 200 on `/v1/ping` (`src/spacetime/managed_store.rs:281`)

**Fix:** Check that the answer is SpacetimeDB (response shape or header) before adopting.

### Low: Rule changes without a `RULES_VERSION` bump (eb73e7eb)

**Fix:** Decide whether the constant's rule requires a bump for the `epic_id` refusal, sub-status check and `close_task` change. If yes, bump `RULES_VERSION` and `BOARD_RULES_VERSION` together.

### Low: Wrong comment in `spacetime/module/src/rules.rs` (`check_task_patch`)

**Issue:** It says `set_task_epic` owns the epic-exists check; a nonexistent `epic_id` is written as a dangling id.

**Fix:** Decide with the user: add the check, or fix the comment. Record it in the spec.

### Low: Twin lacks rollback in `patch_task` (`src/sync/memory_caller/tasks_epics.rs`)

**Fix:** Snapshot and restore on `Refused` after `write_task`; validate after the final adjustments.

### Low: Regroup delete skips `validate_epic_deletable` (`spacetime/module/src/grouping.rs:118`)

**Fix:** Run the check or assert the subtree is shallow.

### Test gaps

- `respawn_phoenix_successor` non-phoenix refusal (module `tasks_epics.rs:485`, twin `agent_state.rs:517`).
- Over-deep subtree refusal through `delete_epic` and `batch_delete`, on both sides, in the conformance tests.

## Changes

| File | Change |
|------|--------|
| `src/spacetime/managed_store.rs` | Ping probe check |
| `spacetime/module/src/rules.rs`, `grouping.rs` | Comment, depth check |
| `src/sync/memory_caller/tasks_epics.rs` | Rollback |
| `tests/memory_caller_conformance/`, `spacetime/module-tests/` | New tests |
| `docs/specs/store-split.allium` | Record decisions |

## Verification

- [ ] Run existing tests — all pass, plus `spacetime/module-tests` via `scripts/check-spacetime-module.sh`
- [ ] Run the repo's verify command from get_task
