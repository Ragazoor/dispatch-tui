# Test Fixture Builders

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop every new request/epic field from being a 200-site test edit, the way `TaskBuilder` fixed it for `Task`.

## Context

This work package addresses the "Fixture duplication" section of `docs/plans/review-2026-10-08-followup/report.md`. Test code only (plus a `test-support`-gated builder). Do it in mechanical, reviewable commits — one fixture type per commit. Avoid touching production `CreateTaskRequest` shape; WP1 may move its definition, so if WP1 is running, coordinate or wait.

## Findings

### 💡 `CreateTaskRequest` written out 212 times (`src/store/mod.rs:127`)

**Issue:** No `Default`, no builder. 212 full struct literals in tests: 37 in `src/store/tests/tasks.rs`, 23 in `src/mcp/handlers/tests/tasks/wrap_up.rs`, 23 in `crud/sub_status.rs`, 19 in `crud/base_branch.rs`.

**Fix:** A `test-support`-gated `CreateTaskRequest::fixture(title, repo)` (or builder like `src/models/task_builder.rs`) and `..fixture` struct-update at the sites.

### 💡 `Epic` has no builder

**Issue:** ~11 full 16-field literals: `src/tui/tests/helpers.rs:149`, four in `src/tui/tests/repo_filter.rs`, `src/editor/tests.rs`, `src/feed/tests.rs`, `src/store/tests/shared_writer.rs`, `src/tui/tests/snapshots.rs`, `src/tui/types/tests.rs`, `tests/memory_caller_conformance.rs`.

**Fix:** `EpicBuilder` beside `task_builder.rs`.

### 💡 Smaller repeats

**Issue:** `CreateEpicParams {` 36× (9 in `src/service/tasks/tests/crud/epic_in_epic.rs`), `CreateTaskParams {` 49×, `McpState::new(` hand-assembled 25× (18 in `wrap_up.rs`) despite `test_state()`, ~19 `make_app*` variants across modules.

**Fix:** Fixture constructors for the params; use `test_state()` where it fits; consolidate `make_app*` into `src/tui/tests/helpers.rs` where the variants are equivalent.

### 💡 Copy-paste setup in `src/tui/tests/dispatch.rs`

**Issue:** 100 `App::new(` setups; 131 fully qualified `crate::tui::messages::TaskMessage::`/`TaskCommand::` paths; repeated `task.worktree = Some(…)` / `task.tmux_window = Some(test_tmux_window(…))` right after `make_task` (68 of the latter across `src/tui/tests`).

**Fix:** `use` the enums at the top; a `TaskBuilder::tmux_window()` method if missing; a `running_task_with_window(id, status)` helper.

## Changes

| File | Change |
|------|--------|
| `src/store/mod.rs` (or a test-support module) | `CreateTaskRequest` fixture |
| `src/models/` | `EpicBuilder`; `TaskBuilder::tmux_window` if missing |
| `src/store/tests/**`, `src/mcp/handlers/tests/**`, `src/service/tasks/tests/**`, `src/tui/tests/**` | Use the fixtures |
| `src/tui/tests/dispatch.rs` | Imports and helper |

## Verification

- [ ] `cargo test --no-fail-fast` — same test count, all pass
- [ ] `grep -c 'CreateTaskRequest {' -r src tests` drops by >150
- [ ] No production code behaviour changes (`git diff --stat` touches only tests and gated fixtures)
