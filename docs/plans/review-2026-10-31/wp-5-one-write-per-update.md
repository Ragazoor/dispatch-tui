# One Write per Epic and Package Update

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `update_epic` and `update_work_package` one module call each, so a refused part leaves nothing applied, and remove the service's remaining cache-based decisions.

## Context

Finding R3 and standing item S2/S3 residue from `docs/plans/2026-10-31-codebase-review.md`. The same mistake was fixed for tasks in 5a5209b7 (`update_task` one reducer; `store-split.allium: UpdateTaskIsOneTransaction`). Follow that pattern: spec rule, module reducer, twin, conformance case, both rules versions bumped, wasm and bindings rebuilt.

## Findings

### 🟡 Medium — R3: `update_epic` and `update_work_package` are several store writes (`src/service/epics.rs::EpicService::update_epic`; `src/service/work_packages.rs::WorkPackageService::update_work_package`)

**Issue:** `update_epic` calls `patch_epic`, then `move_epic_to_workspace`, then `set_epic_visibility`; `update_work_package` calls `update_work_package` then `set_work_package_visibility`. A member who is not the creator sends `update_epic {title, visibility}`: the title is saved, the visibility refused, the tool returns an error and `notify_epic_changed` is skipped. In the TUI epic editor (`src/runtime/editor.rs`) the feed take-over prompt is skipped too.

**Fix:** one module reducer per entity taking the patch, workspace and visibility; the service makes one call. Conformance case "a refused move leaves the title unchanged".

### 🟡 Medium — S2/S3 residue (standing): the service still decides from the cache (`src/service/tasks/crud.rs::TaskService::update_task`, `::move_task_to_epic`, `::create_task_inner`)

**Issue:** `update_task` reads `prior` from the subscription cache for `close_pr_on_done` and `was_pr_finalisation`; `move_task_to_epic` and `create_task_inner` pre-check with `require_epic_accepting_work`/`task_exists`; `set_task_epic` answers Ok for a task deleted in between. Raised in the 10-22, 10-28 and 10-31 reviews.

**Fix:** let the module's answer drive these: have `update_task` return the prior status (or the PR-close decision) and drop the pre-checks in favour of mapping the module's refusal (`refusal_as_not_found`).

## Changes

| File | Change |
|------|--------|
| `docs/specs/store-split.allium` (or `epics.allium`) | `UpdateEpicIsOneTransaction`, same for packages |
| `spacetime/module/src/tasks_epics.rs`, `packages.rs` | New combined reducers |
| `src/sync/memory_caller/` | Mirror |
| `src/service/epics.rs`, `work_packages.rs` | One call each |
| `src/service/tasks/crud.rs` | Drop cache reads and pre-checks |
| `tests/memory_caller_conformance/` | Refused-part cases |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump together |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass
- [ ] Conformance: refused visibility leaves title unchanged (epic and package)
- [ ] `cargo clippy --all-targets -- -D warnings` clean
