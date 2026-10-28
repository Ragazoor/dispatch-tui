# Task Write Rules into the Module

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a task update (epic relink, field patch and the Done transition) one module reducer call, so it cannot half-apply and does not depend on the board's cached rows.

## Context

This work package addresses findings S1, S2 and S3 from `docs/plans/2026-10-28-codebase-review.md`. S1 has been open since 2026-10-22. CLAUDE.md: data rules belong in `spacetime/module/src/`, mirrored in `src/sync/memory_caller/` and covered by `tests/memory_caller_conformance.rs`. Read `docs/invariants.md` (FieldUpdate/TaskPatch, mutation boundary) and `docs/specs/store-split.allium` first. Spec first: record the one-call update in the relevant `docs/specs/*.allium` with `allium:tend`.

## Findings

### 🟡 Medium — S1: relink and patch are two writes (`src/service/tasks/crud.rs:222-242`)

**Issue:** `TaskService::update_task` calls `set_task_epic_id` and `patch_task` separately, ordered by whether the repo changes. An `update_tasks` call with `repo_path`, `epic_id` and a `sub_status` illegal for the status relinks first; the module then refuses the patch. The caller gets an error but the task has moved epic and kept its old repo.

**Fix:** One module reducer (or extend `patch_task`) that takes the target epic with the patch, validates everything, then writes. Mirror in the twin; add a conformance case for "relink + illegal patch leaves the task unchanged".

### 🟡 Medium — S2: the Done transition is decided from the subscription cache (`src/service/tasks/crud.rs:197-220, 253`)

**Issue:** The service reads the prior status from the board's cached rows, decides `entering_done`, and clears `patch.completed_at` because the module "lets a completion the patch names win". The same snapshot drives `ClosePrOnDone`. Same pattern in `src/service/epics.rs:380-384` (`EpicService::update_epic`). If another board just reopened the task, this board's stale cache produces a wrong `completed_at` and a misordered Done column.

**Fix:** The module owns `completed_at` on every transition into Done (ignore a client value unless an explicit reorder is flagged) and reports whether the write entered Done; the service uses that report for `ClosePrOnDone`.

### 🟢 Low — S3: service-side existence pre-checks and a stale comment (`src/service/tasks/crud.rs`, `src/service/epics.rs`)

**Issue:** `create_task_inner`, `move_task_to_epic` and `update_task` call `require_epic_accepting_work` / `task_exists` before writing. A delete in between surfaces as `Internal` (epic) or a silent `Ok` (task). The doc comment on `EpicService::delete_epic` says the store "does not check" the deletable guard; `spacetime/module/src/tasks_epics.rs:706` now does.

**Fix:** Map the module's refusal to the same typed error the pre-check returns; correct the comment.

## Changes

| File | Change |
|------|--------|
| `docs/specs/tasks.allium` (or the owning spec) | One-call task update, module-owned `completed_at` |
| `spacetime/module/src/tasks_epics.rs` | Reducer taking epic + patch; Done transition owns `completed_at` |
| `src/sync/memory_caller/tasks_epics.rs` | Twin mirror |
| `src/sync/writes*`, `src/store/` | Call the new reducer |
| `src/service/tasks/crud.rs` | Single call; drop the cache-based `entering_done` and the `completed_at` stripping |
| `src/service/epics.rs` | Same for `update_epic`; fix `delete_epic` comment; error mapping |
| `tests/memory_caller_conformance/` | Relink + illegal patch; stale-reopen Done case |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump both rules versions together |
| `module.wasm`, bindings | `./scripts/build-managed-module.sh`, `./scripts/regenerate-spacetime-bindings.sh` |

## Verification

- [ ] New conformance cases fail first, then pass
- [ ] `cargo test --no-fail-fast` with `tmux` and `spacetime` on `PATH`
- [ ] Module checks from `docs/testing.md`, including `spacetime/module-tests/`
- [ ] `cargo fmt --check && ./scripts/check-doc-paths.sh`; `cargo clippy --all-targets -- -D warnings`
