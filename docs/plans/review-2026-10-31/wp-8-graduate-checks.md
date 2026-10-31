# Graduate Repeated Findings into Checks

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn two finding classes that keep coming back into mechanical checks, so the next review does not report them again.

## Context

From "Graduate to a check" in `docs/plans/2026-10-31-codebase-review.md`. Class 1, "a move path forgets a rule its sibling has": N1 (10-28) then R2 (10-31). Class 2, "one logical change, several store writes": S1 (10-22, 10-28) then R3 (10-31). The checks may fail on today's tree until WP2 (R2) and WP5 (R3) land; mark known failures as expected (`#[ignore]` with the task id, or an allow-list entry naming it) rather than block.

## Findings

### 🟡 Medium — Class 1: a conformance test over every task-move path (`tests/memory_caller_conformance/`)

**Issue:** each task-move reducer (`set_task_epic`, `update_task` relink, `relink_task` via repo reroute/`regroup_epic`/`flatten_epic`, the feed move in `feed_ingest.rs`, `move_epic_to_workspace`'s cascade) re-implements the "leaving an epic" rules. N1 fixed one; R2 found the sibling.

**Fix:** one table-driven conformance test that runs every move path against the same assertions (unseen mark cleared, sight checked on destination, workspace data consistent), on both module and twin. A new move path must be added to the table.

### 🟡 Medium — Class 2: a check against multi-write service methods (`scripts/check-*.sh` or a test in `tests/`)

**Issue:** service methods that make more than one store write per logical change keep reappearing (`TaskService::update_task` before 5a5209b7, `EpicService::update_epic`, `WorkPackageService::update_work_package`).

**Fix:** a script or test that flags a `src/service/` method with more than one `self.db.<write>` call, with an allow-list naming each sanctioned exception and why. Add it to `.githooks/pre-push`'s list and to `docs/testing.md` ("Toolchain and pre-push hook").

## Changes

| File | Change |
|------|--------|
| `tests/memory_caller_conformance/` | Table-driven move-path test |
| `scripts/check-single-write-service.sh` (or a test) | Multi-write detector with allow-list |
| `.githooks/pre-push`, `.github/workflows/ci.yml`, `docs/testing.md` | Wire and document the check |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass (known failures marked with task ids)
- [ ] The new script exits non-zero on a planted double write and zero on today's tree with the allow-list
- [ ] `cargo clippy --all-targets -- -D warnings` clean
