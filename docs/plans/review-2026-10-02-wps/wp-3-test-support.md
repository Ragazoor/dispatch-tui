# Test Support and Test File Splits

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** One shared task fixture, one unwrap-in-tests setting, smaller test files.

## Context

Findings from `docs/plans/review-2026-10-02/report.md`, section 2.

## Findings

### 💡 Duplicated task fixtures

**Issue:** `make_task`/`test_task` variants in `src/db/tests/mod.rs:73`, `src/dispatch/tests.rs:67`, `src/dispatch/prompts.rs:3282`, `src/feed/ingest/routing.rs:150`, `src/mcp/handlers/tests/poll_ownership.rs:4`, `.../tasks/watch.rs:4`, `src/mcp/handlers/tasks/mod.rs:450`, `src/models/epics.rs:382,653`, `src/service/tasks/tests/mod.rs:88`, `src/tui/tests/helpers.rs:104`, `src/editor.rs:598`.

**Fix:** Shared `TaskBuilder` under `cfg(any(test, feature = "test-support"))`; migrate call sites.

### 💡 Per-module clippy allows

**Issue:** ~106 `allow(clippy::unwrap_used/expect_used)`.

**Fix:** `clippy.toml` `allow-unwrap-in-tests = true`; delete the attributes. Review the 3 `dead_code` allows.

### 💡 Oversized test files and inline test modules

**Issue:** `src/db/tests/migrations.rs` 5665, `db/tests/tasks.rs` 5173, `dispatch/tests.rs` 4874, `service/tasks/tests/crud.rs` 4811, `mcp/handlers/tests/tasks/crud.rs` 4283, `tui/tests/epics.rs` 3662, `rendering.rs` 3549, `input_handlers.rs` 3468. Inline tests in `dispatch/prompts.rs`, `tmux.rs` (~:2249), `cli/agent_tree.rs` (:1482), `startup.rs` (:1016).

**Fix:** Split by behaviour; table-drive migration tests; move inline modules to sibling `tests/`. Convert rendering `contains(` asserts to insta snapshots where cheap.

### 💡 Coverage gaps

**Fix:** Add a fake-connector conformance test that runs without `spacetime`; add tests for `src/sync/writes.rs`, `src/spacetime/snapshot.rs`, `managed_store.rs` after a per-file coverage run. Replace `LocalBoardReads` (`src/sync/board_reads.rs`) with an in-memory `BoardReads`.

## Changes

| File | Change |
|------|--------|
| `clippy.toml` | `allow-unwrap-in-tests` |
| test files above | Split, use `TaskBuilder` |
| `src/sync/board_reads.rs` | In-memory `BoardReads` |

## Verification

- [ ] `cargo test` passes with no fewer tests than before (compare counts)
- [ ] `cargo clippy --all-targets -- -D warnings`; `./scripts/check-no-test-sleep.sh`
